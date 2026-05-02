use std::sync::Arc;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncBufReadExt;
use tokio_stream::StreamExt;

use crate::modes::sql::client::SqlConnectionManager;
use crate::modes::nosql::client::NoSqlConnections;
use crate::shared::ai::context::truncate_str;
use crate::shared::ai::dispatch::{self, ToolContext};
use crate::shared::ai::types::ChatContext;
use crate::shared::ai::ProviderConfig;

pub async fn stream_openai(
    client: &reqwest::Client,
    app: &AppHandle,
    pool: &SqlitePool,
    api_key: &str,
    mut conversation_msgs: Vec<serde_json::Value>,
    context: &ChatContext,
    session_id: &str,
    system_prompt: &str,
    tools: &[serde_json::Value],
    config: &ProviderConfig,
    sql_manager: &Arc<SqlConnectionManager>,
    nosql_conns: &NoSqlConnections,
) -> Result<(), String> {
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;

    // Convert Anthropic-format tools to OpenAI format
    let openai_tools: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t["name"].as_str().unwrap_or(""),
                    "description": t["description"].as_str().unwrap_or(""),
                    "parameters": t["input_schema"],
                }
            })
        })
        .collect();

    // The keyword-heuristic optimization below is REST-specific. SQL/NoSQL/SSH
    // modes must always send their tools and full system prompt — without them
    // the assistant cannot do its job (e.g. SSH cannot execute_shell).
    // Detect non-REST modes by content of the system prompt.
    let is_rest_mode = !system_prompt.contains("SQL assistant")
        && !system_prompt.contains("NoSQL assistant")
        && !system_prompt.contains("SSH operations assistant")
        && !system_prompt.contains("Explorer assistant");

    // For REST: keep the legacy heuristic (saves tokens on Groq's tight limits).
    // For SQL/NoSQL/SSH: always include tools and the full system prompt.
    let needs_tools = if !is_rest_mode {
        true
    } else {
        let last_user_msg = conversation_msgs.iter().rev()
            .find(|m| m["role"].as_str() == Some("user"))
            .and_then(|m| m["content"].as_str())
            .unwrap_or("")
            .to_lowercase();
        let tool_keywords = ["execute", "run", "send", "test", "create", "list", "collection",
            "apply", "request", "history", "environment", "api", "fetch", "call", "invoke",
            "show me", "what request", "current", "response", "status", "error", "debug",
            "headers", "body", "url", "method", "get", "post", "put", "delete", "patch"];
        let matched = tool_keywords.iter().any(|kw| last_user_msg.contains(kw));
        log::info!("[AI OpenAI] needs_tools={} last_msg_preview=\"{}\"", matched, &last_user_msg[..last_user_msg.len().min(100)]);
        matched
    };

    // Use the actual system prompt unless we're in REST + the keyword heuristic skipped tools
    let sys_content = if needs_tools || !is_rest_mode {
        system_prompt.to_string()
    } else {
        "You are a REST API assistant. Answer briefly. No emojis. No markdown headers.".to_string()
    };

    // Filter out assistant messages with empty content (Mistral rejects them)
    let mut conversation_msgs = conversation_msgs;
    conversation_msgs.retain(|m| {
        let role = m["role"].as_str().unwrap_or("");
        if role == "assistant" {
            let content = m["content"].as_str().unwrap_or("");
            !content.is_empty() || m.get("tool_calls").is_some()
        } else {
            true
        }
    });

    let mut full_msgs = vec![serde_json::json!({
        "role": "system",
        "content": sys_content,
    })];
    full_msgs.extend(conversation_msgs.clone());

    let mut tool_rounds: u32 = 0;
    const MAX_TOOL_ROUNDS: u32 = 10;

    // Reduced output cap when tools aren't needed: legacy heuristic.
    // 1024 was the original "no-tool" cap; preserved verbatim for parity.
    let no_tool_max_tokens: u32 = 1024.min(config.max_output_tokens);

    loop {
        let mut body = serde_json::json!({
            "model": config.model_id,
            "max_tokens": if needs_tools { config.max_output_tokens } else { no_tool_max_tokens },
            "stream": true,
            "temperature": config.default_temperature,
            "messages": full_msgs,
        });

        if needs_tools && !openai_tools.is_empty() {
            body["tools"] = serde_json::json!(openai_tools);
            body["tool_choice"] = serde_json::json!("auto");
            if config.supports_parallel_tools {
                body["parallel_tool_calls"] = serde_json::json!(true);
            }
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|e| e.to_string())?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        log::info!("[AI OpenAI] POST {} model={}", config.api_url, config.model_id);

        let response = client
            .post(config.api_url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                log::error!("[AI OpenAI] Connection failed: {}", e);
                let _ = app.emit(
                    &format!("ai:error:{}", session_id),
                    serde_json::json!({"error": format!("Connection failed: {}", e)}),
                );
                format!("Connection failed: {}", e)
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let retry_after = response.headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<f64>().ok());
            let remaining_tokens = response.headers()
                .get("x-ratelimit-remaining-tokens")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string());
            let error_body = response.text().await.unwrap_or_default();
            log::error!("[AI OpenAI] Error {}: {}", status, truncate_str(&error_body, 500));
            let msg = match status {
                401 => "Invalid API key".to_string(),
                429 => {
                    let mut m = "Rate limited".to_string();
                    if let Some(secs) = retry_after {
                        m.push_str(&format!(" — retry in {:.0}s", secs));
                    } else {
                        m.push_str(" — try again in a moment");
                    }
                    if let Some(rem) = remaining_tokens {
                        m.push_str(&format!(" ({} tokens remaining)", rem));
                    }
                    m
                }
                _ => {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&error_body) {
                        // OpenAI format: {"error": {"message": "..."}}
                        if let Some(msg) = parsed["error"]["message"].as_str() {
                            msg.to_string()
                        }
                        // Mistral format: {"message": {"detail": [{"msg": "..."}]}}
                        else if let Some(detail) = parsed["message"]["detail"].as_array() {
                            detail.iter()
                                .filter_map(|d| d["msg"].as_str())
                                .collect::<Vec<_>>()
                                .join("; ")
                        }
                        // Mistral format: {"message": "string"}
                        else if let Some(msg) = parsed["message"].as_str() {
                            msg.to_string()
                        }
                        else {
                            format!("API error ({}): {}", status, truncate_str(&error_body, 200))
                        }
                    } else {
                        format!("API error ({}): {}", status, truncate_str(&error_body, 200))
                    }
                }
            };
            let _ = app.emit(
                &format!("ai:error:{}", session_id),
                serde_json::json!({"error": msg}),
            );
            return Err(msg);
        }

        // Parse SSE stream
        let byte_stream = response.bytes_stream();
        let stream_reader = tokio_util::io::StreamReader::new(
            byte_stream
                .map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
        );
        let mut lines = tokio::io::BufReader::new(stream_reader).lines();

        let mut current_text = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_id = String::new();
        let mut current_tool_json = String::new();
        let mut tool_calls_collected: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
        let mut finish_reason = String::new();

        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            let line = line.trim().to_string();
            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            if data == "[DONE]" {
                break;
            }

            let event: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let choice = &event["choices"][0];
            let delta = &choice["delta"];
            let finish = choice["finish_reason"].as_str();

            // Text content
            if let Some(text) = delta["content"].as_str() {
                if !text.is_empty() {
                    current_text.push_str(text);
                    let _ = app.emit(
                        &format!("ai:text:{}", session_id),
                        serde_json::json!({"text": text}),
                    );
                }
            }

            // Tool calls
            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                for tc in tool_calls {
                    if let Some(name) = tc["function"]["name"].as_str() {
                        // Finalize previous tool call if any
                        if !current_tool_name.is_empty() {
                            tool_calls_collected.push((
                                current_tool_id.clone(),
                                current_tool_name.clone(),
                                current_tool_json.clone(),
                            ));
                        }
                        current_tool_name = name.to_string();
                        current_tool_id = tc["id"].as_str().unwrap_or("").to_string();
                        current_tool_json.clear();
                        let _ = app.emit(
                            &format!("ai:tool_start:{}", session_id),
                            serde_json::json!({"toolName": current_tool_name}),
                        );
                    }
                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        current_tool_json.push_str(args);
                    }
                }
            }

            // Finish reason
            if let Some(fr) = finish {
                finish_reason = fr.to_string();
            }

            // Usage from Groq (x_groq.usage or usage)
            if let Some(usage) = event["x_groq"]["usage"].as_object() {
                total_input_tokens += usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                total_output_tokens += usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
            } else if let Some(usage) = event["usage"].as_object() {
                total_input_tokens += usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                total_output_tokens += usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
            }
        }

        // Finalize last tool call if any
        if !current_tool_name.is_empty() {
            tool_calls_collected.push((
                current_tool_id.clone(),
                current_tool_name.clone(),
                current_tool_json.clone(),
            ));
        }

        log::info!("[AI OpenAI] finish_reason=\"{}\" tool_calls_count={} text_len={}", finish_reason, tool_calls_collected.len(), current_text.len());
        if finish_reason == "tool_calls" && !tool_calls_collected.is_empty() {
            // Build assistant message with tool_calls
            let assistant_tool_calls: Vec<serde_json::Value> = tool_calls_collected
                .iter()
                .map(|(id, name, args)| {
                    serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": args,
                        }
                    })
                })
                .collect();

            let assistant_msg = serde_json::json!({
                "role": "assistant",
                "content": "",
                "tool_calls": assistant_tool_calls,
            });
            full_msgs.push(assistant_msg.clone());
            conversation_msgs.push(assistant_msg);

            // Execute each tool and add results
            for (id, name, args_str) in &tool_calls_collected {
                let tool_input: serde_json::Value =
                    serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));

                let tool_result = dispatch::execute(
                    name,
                    ToolContext {
                        tool_use_id: id,
                        input: &tool_input,
                        context,
                        pool,
                        app,
                        session_id,
                        sql_manager,
                        nosql_conns,
                    },
                )
                .await;

                let _ = app.emit(
                    &format!("ai:tool_end:{}", session_id),
                    serde_json::json!({"toolName": name}),
                );

                let tool_msg = serde_json::json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": tool_result,
                });
                full_msgs.push(tool_msg.clone());
                conversation_msgs.push(tool_msg);
            }

            tool_calls_collected.clear();
            current_tool_name.clear();
            current_tool_id.clear();
            current_tool_json.clear();

            tool_rounds += 1;
            if tool_rounds >= MAX_TOOL_ROUNDS {
                let _ = app.emit(
                    &format!("ai:text:{}", session_id),
                    serde_json::json!({"text": "\n\n[Stopped: too many tool calls in a row]"}),
                );
                let _ = app.emit(
                    &format!("ai:done:{}", session_id),
                    serde_json::json!({"inputTokens": total_input_tokens, "outputTokens": total_output_tokens}),
                );
                break;
            }
            continue;
        }

        // Done
        let _ = app.emit(
            &format!("ai:done:{}", session_id),
            serde_json::json!({
                "inputTokens": total_input_tokens,
                "outputTokens": total_output_tokens,
            }),
        );
        break;
    }

    Ok(())
}

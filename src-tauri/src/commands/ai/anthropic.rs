use std::sync::Arc;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncBufReadExt;
use tokio_stream::StreamExt;

use super::tools::execute_tool;
use super::types::ChatContext;
use crate::commands::sql_client::SqlConnectionManager;
use crate::commands::nosql_client::NoSqlConnections;

pub const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";

pub async fn stream_anthropic(
    client: &reqwest::Client,
    app: &AppHandle,
    pool: &SqlitePool,
    api_key: &str,
    mut conversation_msgs: Vec<serde_json::Value>,
    context: &ChatContext,
    session_id: &str,
    system_prompt: &str,
    tools: &[serde_json::Value],
    sql_manager: &Arc<SqlConnectionManager>,
    nosql_conns: &NoSqlConnections,
) -> Result<(), String> {
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut tool_rounds: u32 = 0;
    const MAX_TOOL_ROUNDS: u32 = 10;

    loop {
        let mut tools_with_cache = tools.to_vec();
        if let Some(last_tool) = tools_with_cache.last_mut() {
            if let Some(obj) = last_tool.as_object_mut() {
                obj.insert(
                    "cache_control".to_string(),
                    serde_json::json!({"type": "ephemeral"}),
                );
            }
        }

        let body = serde_json::json!({
            "model": DEFAULT_MODEL,
            "max_tokens": 4096,
            "stream": true,
            "system": [{
                "type": "text",
                "text": system_prompt,
                "cache_control": {"type": "ephemeral"}
            }],
            "tools": tools_with_cache,
            "messages": conversation_msgs,
        });

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(api_key).map_err(|e| e.to_string())?,
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = client
            .post(ANTHROPIC_API_URL)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                let _ = app.emit(
                    &format!("ai:error:{}", session_id),
                    serde_json::json!({"error": format!("Connection failed: {}", e)}),
                );
                format!("Connection failed: {}", e)
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_body = response.text().await.unwrap_or_default();
            let msg = match status {
                401 => "Invalid API key".to_string(),
                429 => "Rate limited — try again in a moment".to_string(),
                _ => {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&error_body) {
                        parsed["error"]["message"]
                            .as_str()
                            .unwrap_or("API error")
                            .to_string()
                    } else {
                        format!("API error ({})", status)
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
        let mut tool_uses: Vec<serde_json::Value> = Vec::new();
        let mut tool_results: Vec<serde_json::Value> = Vec::new();
        let mut stop_reason = String::new();

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

            let event_type = event["type"].as_str().unwrap_or("");

            match event_type {
                "content_block_start" => {
                    let block = &event["content_block"];
                    if block["type"].as_str() == Some("tool_use") {
                        current_tool_name =
                            block["name"].as_str().unwrap_or("").to_string();
                        current_tool_id =
                            block["id"].as_str().unwrap_or("").to_string();
                        current_tool_json.clear();
                        let _ = app.emit(
                            &format!("ai:tool_start:{}", session_id),
                            serde_json::json!({"toolName": current_tool_name}),
                        );
                    }
                }
                "content_block_delta" => {
                    let delta = &event["delta"];
                    match delta["type"].as_str().unwrap_or("") {
                        "text_delta" => {
                            let text = delta["text"].as_str().unwrap_or("");
                            if !text.is_empty() {
                                current_text.push_str(text);
                                let _ = app.emit(
                                    &format!("ai:text:{}", session_id),
                                    serde_json::json!({"text": text}),
                                );
                            }
                        }
                        "input_json_delta" => {
                            let partial =
                                delta["partial_json"].as_str().unwrap_or("");
                            current_tool_json.push_str(partial);
                        }
                        _ => {}
                    }
                }
                "content_block_stop" => {
                    if !current_tool_name.is_empty() {
                        let tool_input: serde_json::Value =
                            serde_json::from_str(&current_tool_json)
                                .unwrap_or(serde_json::json!({}));

                        let tool_result = execute_tool(
                            &current_tool_name,
                            &current_tool_id,
                            &tool_input,
                            context,
                            pool,
                            app,
                            session_id,
                            sql_manager,
                            nosql_conns,
                        )
                        .await;

                        let _ = app.emit(
                            &format!("ai:tool_end:{}", session_id),
                            serde_json::json!({"toolName": current_tool_name}),
                        );

                        tool_uses.push(serde_json::json!({
                            "type": "tool_use",
                            "id": current_tool_id,
                            "name": current_tool_name,
                            "input": tool_input,
                        }));

                        tool_results.push(serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": current_tool_id,
                            "content": tool_result,
                        }));

                        current_tool_name.clear();
                        current_tool_id.clear();
                        current_tool_json.clear();
                    }
                }
                "message_delta" => {
                    if let Some(sr) = event["delta"]["stop_reason"].as_str() {
                        stop_reason = sr.to_string();
                    }
                    if let Some(usage) = event["usage"].as_object() {
                        total_output_tokens += usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                    }
                }
                "message_start" => {
                    if let Some(usage) = event["message"]["usage"].as_object() {
                        total_input_tokens += usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let cache_read = usage
                            .get("cache_read_input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let cache_creation = usage
                            .get("cache_creation_input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        if cache_read > 0 || cache_creation > 0 {
                            log::info!(
                                "[AI] Cache read: {} tokens, cache creation: {} tokens",
                                cache_read,
                                cache_creation
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        if stop_reason == "tool_use" {
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

            let mut assistant_blocks: Vec<serde_json::Value> = Vec::new();
            if !current_text.is_empty() {
                assistant_blocks.push(
                    serde_json::json!({"type": "text", "text": current_text.clone()}),
                );
                current_text.clear();
            }
            for tu in &tool_uses {
                assistant_blocks.push(tu.clone());
            }

            conversation_msgs.push(serde_json::json!({
                "role": "assistant",
                "content": assistant_blocks,
            }));

            conversation_msgs.push(serde_json::json!({
                "role": "user",
                "content": tool_results.clone(),
            }));

            tool_uses.clear();
            tool_results.clear();
            continue;
        }

        // end_turn — done
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

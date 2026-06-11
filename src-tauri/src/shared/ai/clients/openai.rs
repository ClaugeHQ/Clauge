use std::sync::Arc;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncBufReadExt;
use tokio_stream::StreamExt;

use super::errors;
use crate::cloud::auth::AuthState;
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
    // Extra headers to attach to every request. Used by the Clauge AI
    // provider to send `X-Provider: github|google` so our worker can
    // validate the bearer against the right JWKS.
    extra_headers: &std::collections::HashMap<String, String>,
    // Auth state — only meaningful for the Clauge AI provider, where it
    // enables auto-refresh of the Google id_token on 401 so a stale
    // session doesn't surface as a user-facing "sign in again" error.
    // `None` for BYOK providers (no refresh path).
    auth_state: Option<&AuthState>,
) -> Result<(), String> {
    let mut api_key = api_key.to_string();
    // We only attempt the Clauge AI refresh+retry dance once per chat to
    // avoid loops: if refresh succeeds but the new token also 401s,
    // something is genuinely wrong (provider revoked, JWKS mismatch, etc.)
    // and the user has to re-sign-in.
    let mut clauge_refresh_attempted = false;
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
    // Loop-discipline state. See the matching block in anthropic.rs.
    let mut recent_calls: std::collections::VecDeque<(String, u64)> = std::collections::VecDeque::with_capacity(8);
    let mut consecutive_introspection_rounds: u32 = 0;
    const INTROSPECTION_BUDGET: u32 = 4;

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

        // Clauge AI worker requires a fresh UUID v4 per call for replay
        // defense (prevents re-streaming a paid-for response). Generate a
        // new one on every loop iteration — each tool round-trip is a
        // distinct billable request from the worker's perspective.
        // Also send the originating mode so the worker can attribute the
        // deduction to the right mode in its usage log.
        if matches!(
            config.provider_id,
            crate::shared::ai::providers::ProviderId::Clauge
        ) {
            body["request_id"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
            if !context.mode.is_empty() {
                body["mode"] = serde_json::json!(context.mode);
            }
            // Stable session_id so the worker can group the per-round
            // request_id inserts back into one chat session. Lets the
            // operator compute rounds-per-session as the primary signal
            // for loop-discipline effectiveness.
            body["session_id"] = serde_json::json!(session_id);
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|e| e.to_string())?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        // Insert any caller-supplied extra headers (e.g. X-Provider for Clauge AI).
        for (k, v) in extra_headers.iter() {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                headers.insert(name, value);
            }
        }

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

            // Clauge AI auto-refresh on 401: the user's cloud bearer is a
            // Google id_token (or GitHub token) and Google id_tokens rotate
            // every ~1 hour. Catch the first 401 of a chat, refresh the
            // token via `/api/auth/google/refresh`, swap in the new bearer,
            // and retry the same request. Mirrors `with_google_refresh_retry`
            // in cloud/client.rs but at the streaming-request level here.
            // For GitHub or non-Clauge providers there's no refresh path,
            // so we fall through to the normal error mapping.
            if status == 401
                && matches!(
                    config.provider_id,
                    crate::shared::ai::providers::ProviderId::Clauge
                )
                && !clauge_refresh_attempted
                && auth_state.is_some()
            {
                clauge_refresh_attempted = true;
                let state = auth_state.unwrap();
                log::info!("[AI Clauge] 401 received — attempting Google token refresh");
                // Try the refresh BEFORE we consume the response body. If the
                // refresh succeeds we drain + drop the response and continue
                // the outer loop with the new bearer; if it fails we leave
                // `response` intact so the normal error-mapping path can
                // read its body / headers below.
                if crate::cloud::auth::refresh_google_and_store(state, pool)
                    .await
                    .is_ok()
                {
                    if let Some((new_tok, _)) = state.active_token_and_provider() {
                        api_key = new_tok;
                        // Drain the response so the connection can be reused.
                        let _ = response.bytes().await;
                        log::info!("[AI Clauge] refresh succeeded — retrying the request");
                        continue;
                    }
                }
                log::warn!("[AI Clauge] token refresh failed — surfacing 401 to caller");
            }

            let (retry_after, remaining_tokens) = errors::rate_limit_detail(response.headers());
            let error_body = response.text().await.unwrap_or_default();
            log::error!("[AI OpenAI] Error {}: {}", status, truncate_str(&error_body, 500));
            let msg = errors::map_upstream_error(
                status,
                &error_body,
                retry_after,
                remaining_tokens.as_deref(),
            );
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
        // Index-keyed accumulators. Gemini's OpenAI-compat layer streams the
        // function name in one chunk, arguments across several, and the
        // thought_signature in a separate (often final, often empty-text)
        // chunk under `extra_content`. Tracking by `index` lets us merge
        // these out-of-order pieces correctly across providers.
        #[derive(Default, Clone)]
        struct ToolAccum {
            id: String,
            name: String,
            args: String,
            thought_sig: Option<String>,
            announced: bool,
        }
        let mut tool_calls_by_idx: std::collections::BTreeMap<i64, ToolAccum> = std::collections::BTreeMap::new();
        let mut last_tool_idx: i64 = -1;
        let mut logged_sample_delta = false;
        let mut finish_reason = String::new();

        // Track the most recent `event:` line so the data: that follows can
        // be routed to the right handler. Default upstream chat SSE has no
        // event: line at all (everything is `data: {...}`); our Clauge AI
        // worker prefixes credit notifications with `event: balance`.
        let mut current_event: Option<String> = None;

        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            let line = line.trim().to_string();
            if line.is_empty() {
                current_event = None;
                continue;
            }
            if let Some(rest) = line.strip_prefix("event: ") {
                current_event = Some(rest.to_string());
                continue;
            }
            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            if data == "[DONE]" {
                break;
            }

            // Clauge-AI-worker-specific: live credit balance after each chat.
            // Patch ProStateManager directly — it persists the snapshot and
            // emits cloud:pro-state, which the frontend's proState
            // subscription consumes. Replaces the legacy clauge_ai:balance
            // event so there's exactly one event surface for credit updates.
            if current_event.as_deref() == Some("balance") {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(remaining) = parsed.get("remaining").and_then(|v| v.as_i64()) {
                        if let Some(manager) =
                            app.try_state::<crate::cloud::pro_state::ProStateManager>()
                        {
                            let _ = manager
                                .patch_credits_remaining(remaining, app, pool)
                                .await;
                        }
                    }
                }
                current_event = None;
                continue;
            }

            let event: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            current_event = None;

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

            // Tool calls — merge by index across SSE chunks.
            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                if !logged_sample_delta {
                    log::info!("[AI OpenAI] sample tool_call delta: {}", serde_json::to_string(tool_calls).unwrap_or_default());
                    logged_sample_delta = true;
                }
                for tc in tool_calls {
                    let idx = tc["index"].as_i64().unwrap_or_else(|| if last_tool_idx < 0 { 0 } else { last_tool_idx });
                    last_tool_idx = idx;
                    let entry = tool_calls_by_idx.entry(idx).or_default();

                    if let Some(id) = tc["id"].as_str() {
                        if !id.is_empty() && entry.id.is_empty() { entry.id = id.to_string(); }
                    }
                    if let Some(name) = tc["function"]["name"].as_str() {
                        if !name.is_empty() && entry.name.is_empty() {
                            entry.name = name.to_string();
                        }
                    }
                    if !entry.announced && !entry.name.is_empty() {
                        let _ = app.emit(
                            &format!("ai:tool_start:{}", session_id),
                            serde_json::json!({"toolName": entry.name}),
                        );
                        entry.announced = true;
                    }
                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        entry.args.push_str(args);
                    }
                    if entry.thought_sig.is_none() {
                        let sig = tc["extra_content"]["google"]["thought_signature"].as_str()
                            .or_else(|| tc["extra_content"]["thought_signature"].as_str())
                            .or_else(|| tc["function"]["extra_content"]["thought_signature"].as_str())
                            .or_else(|| tc["function"]["extra_content"]["google"]["thought_signature"].as_str())
                            .or_else(|| tc["thought_signature"].as_str())
                            .or_else(|| tc["function"]["thought_signature"].as_str());
                        if let Some(s) = sig {
                            entry.thought_sig = Some(s.to_string());
                        }
                    }
                }
            }

            // Gemini sometimes streams the thought_signature on a separate
            // delta with empty text content. Attach to most-recent tool call.
            if last_tool_idx >= 0 {
                let delta_sig = delta["extra_content"]["google"]["thought_signature"].as_str()
                    .or_else(|| delta["extra_content"]["thought_signature"].as_str())
                    .or_else(|| delta["thought_signature"].as_str());
                if let Some(s) = delta_sig {
                    if let Some(entry) = tool_calls_by_idx.get_mut(&last_tool_idx) {
                        if entry.thought_sig.is_none() {
                            entry.thought_sig = Some(s.to_string());
                        }
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

        let tool_calls_collected: Vec<(String, String, String, Option<String>)> = tool_calls_by_idx
            .into_values()
            .filter(|a| !a.name.is_empty())
            .map(|a| (a.id, a.name, a.args, a.thought_sig))
            .collect();
        let sigs_captured = tool_calls_collected.iter().filter(|(_, _, _, s)| s.is_some()).count();

        log::info!(
            "[AI OpenAI] finish_reason=\"{}\" tool_calls_count={} thought_sigs={} text_len={}",
            finish_reason, tool_calls_collected.len(), sigs_captured, current_text.len()
        );
        // Gemini's OpenAI-compat endpoint reports finish_reason="stop" even when
        // the response includes tool_calls. OpenAI uses "tool_calls". Presence
        // of tool_calls is the canonical signal — finish_reason is unreliable.
        if !tool_calls_collected.is_empty() {
            // Echo each tool_call back exactly as received. Gemini 3.x rejects
            // requests where prior function calls lack `extra_content.google.thought_signature`.
            let assistant_tool_calls: Vec<serde_json::Value> = tool_calls_collected
                .iter()
                .map(|(id, name, args, sig)| {
                    let mut tc = serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": args,
                        }
                    });
                    if let Some(s) = sig {
                        tc["extra_content"] = serde_json::json!({
                            "google": { "thought_signature": s }
                        });
                    }
                    tc
                })
                .collect();

            let assistant_msg = serde_json::json!({
                "role": "assistant",
                "content": "",
                "tool_calls": assistant_tool_calls,
            });
            full_msgs.push(assistant_msg.clone());
            conversation_msgs.push(assistant_msg);

            // Compute loop-discipline steer BEFORE we execute, so we can
            // append it to the last tool_result (model sees the nudge
            // inline with the result it's about to read).
            let this_round_calls: Vec<(String, u64)> = tool_calls_collected
                .iter()
                .map(|(_, name, args_str, _)| {
                    let parsed: serde_json::Value =
                        serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                    (name.clone(), dispatch::hash_tool_args(&parsed))
                })
                .collect();
            let duplicate_call = this_round_calls
                .iter()
                .find(|c| recent_calls.contains(c))
                .cloned();
            let all_introspection = !this_round_calls.is_empty()
                && this_round_calls
                    .iter()
                    .all(|(name, _)| dispatch::is_introspection_tool(name));
            if all_introspection {
                consecutive_introspection_rounds += 1;
            } else {
                consecutive_introspection_rounds = 0;
            }
            for c in this_round_calls {
                if recent_calls.len() >= 8 {
                    recent_calls.pop_front();
                }
                recent_calls.push_back(c);
            }
            let steer: Option<String> = if let Some((dup_name, _)) = duplicate_call {
                Some(format!(
                    "\n\nSTEER: you already called `{}` with these exact arguments in a recent round. Re-running it returns the same result. Use the prior tool_result, or stop calling tools and either answer the user or ask one clarifying question.",
                    dup_name
                ))
            } else if consecutive_introspection_rounds >= INTROSPECTION_BUDGET {
                let n = consecutive_introspection_rounds;
                consecutive_introspection_rounds = 0;
                Some(format!(
                    "\n\nSTEER: you have spent {} consecutive rounds inspecting metadata without executing or applying anything. STOP. Either (a) call the execute/apply tool with what you have now, or (b) stop using tools and ask the user ONE clarifying question. Do not introspect further.",
                    n
                ))
            } else {
                None
            };

            // Execute each tool and add results. Append steer to the LAST
            // tool's result so the model reads it next to the data it has
            // to act on. Inline is more reliable than a separate message
            // (avoids reorder/filter by upstream OpenAI-compat layers).
            let last_idx = tool_calls_collected.len().saturating_sub(1);
            for (i, (id, name, args_str, _sig)) in tool_calls_collected.iter().enumerate() {
                let tool_input: serde_json::Value =
                    serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));

                let mut tool_result = dispatch::execute(
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

                if i == last_idx {
                    if let Some(s) = &steer {
                        tool_result.push_str(s);
                    }
                }

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

            tool_rounds += 1;
            if tool_rounds >= MAX_TOOL_ROUNDS {
                let _ = app.emit(
                    &format!("ai:text:{}", session_id),
                    serde_json::json!({"text": "\n\n[Stopped: too many tool calls in a row]"}),
                );
                let _ = app.emit(
                    &format!("ai:done:{}", session_id),
                    serde_json::json!({"inputTokens": total_input_tokens, "outputTokens": total_output_tokens, "toolRounds": tool_rounds}),
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
                "toolRounds": tool_rounds,
            }),
        );
        break;
    }

    Ok(())
}

// One-shot completion that returns the collected text instead of
// streaming it to the frontend. Used by background generation flows
// (meeting notes) that have no chat session to emit into. Reuses the
// provider registry, the shared HTTP client, and the same request
// shapes as the streaming clients in this directory.

use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};

use super::errors;
use crate::cloud::auth::AuthState;
use crate::shared::ai::context::truncate_str;
use crate::shared::ai::providers::ProviderId;
use crate::shared::ai::{ApiKind, ProviderConfig};

pub struct CollectParams<'a> {
    pub system: &'a str,
    pub user: &'a str,
    /// Originating mode, sent to the Clauge AI worker for usage attribution.
    pub mode: &'a str,
    /// Stable id grouping the per-call request_ids of one generation run
    /// on the Clauge AI worker.
    pub session_id: &'a str,
    /// Extra HTTP headers (e.g. `X-Provider` for Clauge AI).
    pub extra_headers: &'a HashMap<String, String>,
    /// Only meaningful for the Clauge AI provider: enables the one-shot
    /// Google id_token refresh-and-retry on 401. `None` for BYOK.
    pub auth_state: Option<&'a AuthState>,
}

pub struct CollectedCompletion {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub async fn collect_completion(
    client: &reqwest::Client,
    app: &AppHandle,
    pool: &SqlitePool,
    api_key: &str,
    config: &'static ProviderConfig,
    params: &CollectParams<'_>,
) -> Result<CollectedCompletion, String> {
    match config.api_kind {
        ApiKind::AnthropicMessages => collect_anthropic(client, api_key, config, params).await,
        ApiKind::OpenAICompat => {
            collect_openai(client, app, pool, api_key, config, params).await
        }
    }
}

/// Non-streaming Anthropic /v1/messages call. Same headers and body
/// shape as `stream_anthropic`, minus tools and `stream`.
async fn collect_anthropic(
    client: &reqwest::Client,
    api_key: &str,
    config: &'static ProviderConfig,
    params: &CollectParams<'_>,
) -> Result<CollectedCompletion, String> {
    let body = serde_json::json!({
        "model": config.model_id,
        "max_tokens": config.max_output_tokens,
        "system": [{"type": "text", "text": params.system}],
        "messages": [{"role": "user", "content": params.user}],
    });

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key).map_err(|e| e.to_string())?,
    );
    headers.insert(
        "anthropic-version",
        HeaderValue::from_str(config.anthropic_version.unwrap_or("2023-06-01"))
            .map_err(|e| e.to_string())?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    log::info!("[AI collect] POST {} model={}", config.api_url, config.model_id);

    let response = client
        .post(config.api_url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let (retry_after, remaining_tokens) = errors::rate_limit_detail(response.headers());
        let error_body = response.text().await.unwrap_or_default();
        log::error!(
            "[AI collect] Error {}: {}",
            status.as_u16(),
            truncate_str(&error_body, 500)
        );
        return Err(errors::map_upstream_error(
            status.as_u16(),
            &error_body,
            retry_after,
            remaining_tokens.as_deref(),
        ));
    }

    let v: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;
    let text = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        return Err("Model returned an empty response".to_string());
    }
    Ok(CollectedCompletion {
        text,
        input_tokens: v["usage"]["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: v["usage"]["output_tokens"].as_u64().unwrap_or(0),
    })
}

/// OpenAI-compatible /chat/completions call. Streams (`stream: true` —
/// the Clauge AI worker is SSE-only) and collects the text deltas. Same
/// Clauge-specific behaviour as `stream_openai`: per-call request_id +
/// mode + session_id in the body, `event: balance` credit patching, and
/// the single Google refresh-and-retry on 401.
async fn collect_openai(
    client: &reqwest::Client,
    app: &AppHandle,
    pool: &SqlitePool,
    api_key: &str,
    config: &'static ProviderConfig,
    params: &CollectParams<'_>,
) -> Result<CollectedCompletion, String> {
    use tokio::io::AsyncBufReadExt;
    use tokio_stream::StreamExt;

    let mut api_key = api_key.to_string();
    let mut refresh_attempted = false;
    let is_clauge = matches!(config.provider_id, ProviderId::Clauge);

    loop {
        let mut body = serde_json::json!({
            "model": config.model_id,
            "max_tokens": config.max_output_tokens,
            "stream": true,
            "temperature": config.default_temperature,
            "messages": [
                {"role": "system", "content": params.system},
                {"role": "user", "content": params.user},
            ],
        });
        if is_clauge {
            body["request_id"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
            if !params.mode.is_empty() {
                body["mode"] = serde_json::json!(params.mode);
            }
            body["session_id"] = serde_json::json!(params.session_id);
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|e| e.to_string())?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        for (k, v) in params.extra_headers.iter() {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                headers.insert(name, value);
            }
        }

        log::info!("[AI collect] POST {} model={}", config.api_url, config.model_id);

        let response = client
            .post(config.api_url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Connection failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();

            if status == 401 && is_clauge && !refresh_attempted {
                if let Some(state) = params.auth_state {
                    refresh_attempted = true;
                    log::info!("[AI collect] 401 — attempting Google token refresh");
                    if crate::cloud::auth::refresh_google_and_store(state, pool)
                        .await
                        .is_ok()
                    {
                        if let Some((new_tok, _)) = state.active_token_and_provider() {
                            api_key = new_tok;
                            let _ = response.bytes().await;
                            continue;
                        }
                    }
                    log::warn!("[AI collect] token refresh failed — surfacing 401");
                }
            }

            let (retry_after, remaining_tokens) = errors::rate_limit_detail(response.headers());
            let error_body = response.text().await.unwrap_or_default();
            log::error!("[AI collect] Error {}: {}", status, truncate_str(&error_body, 500));
            return Err(errors::map_upstream_error(
                status,
                &error_body,
                retry_after,
                remaining_tokens.as_deref(),
            ));
        }

        let byte_stream = response.bytes_stream();
        let stream_reader = tokio_util::io::StreamReader::new(
            byte_stream.map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
        );
        let mut lines = tokio::io::BufReader::new(stream_reader).lines();

        let mut text = String::new();
        let mut input_tokens: u64 = 0;
        let mut output_tokens: u64 = 0;
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

            if current_event.as_deref() == Some("balance") {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(remaining) = parsed.get("remaining").and_then(|v| v.as_i64()) {
                        if let Some(manager) =
                            app.try_state::<crate::cloud::pro_state::ProStateManager>()
                        {
                            let _ = manager.patch_credits_remaining(remaining, app, pool).await;
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

            if let Some(t) = event["choices"][0]["delta"]["content"].as_str() {
                text.push_str(t);
            }
            let usage = if event["x_groq"]["usage"].is_object() {
                &event["x_groq"]["usage"]
            } else {
                &event["usage"]
            };
            if usage.is_object() {
                input_tokens += usage["prompt_tokens"].as_u64().unwrap_or(0);
                output_tokens += usage["completion_tokens"].as_u64().unwrap_or(0);
            }
        }

        if text.trim().is_empty() {
            return Err("Model returned an empty response".to_string());
        }
        return Ok(CollectedCompletion {
            text,
            input_tokens,
            output_tokens,
        });
    }
}

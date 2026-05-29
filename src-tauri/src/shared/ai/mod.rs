pub mod clients;
pub mod context;
pub mod dispatch;
pub mod providers;
pub mod sample;
pub mod types;
pub mod usage;

pub use providers::{
    default_model_for, get_provider_config, list_all_providers, list_models_for, ApiKind,
    ProviderConfig, ProviderId,
};
pub use types::*;
pub use usage::*;

use std::sync::Arc;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};

use crate::modes::sql::client::SqlConnectionManager;
use crate::modes::nosql::client::NoSqlConnections;

use self::context::build_api_messages;

/// Look up the registry entry for a frontend-supplied provider slug, falling
/// back to the provider's default model when no model is specified.
///
/// Centralised here so `test_ai_key` and `ai_chat` resolve identically.
fn resolve_config(
    provider_slug: &str,
    model: Option<&str>,
) -> Result<&'static ProviderConfig, String> {
    let provider = ProviderId::from_slug(provider_slug)
        .ok_or_else(|| format!("Unknown AI provider: {}", provider_slug))?;
    let cfg = match model {
        Some(m) => get_provider_config(provider, m).or_else(|| default_model_for(provider)),
        None => default_model_for(provider),
    };
    cfg.ok_or_else(|| format!("No registered model for provider: {}", provider_slug))
}

/// Read a settings value, returning `None` for missing keys, DB errors, or
/// blank strings. Used to fetch the `local` provider's runtime endpoint/model
/// overrides.
async fn local_setting(pool: &SqlitePool, key: &str) -> Option<String> {
    crate::shared::repos::settings::get_by_key(pool, key)
        .await
        .ok()
        .flatten()
        .map(|s| s.value.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Canonicalize a user-entered local base URL into the chat-completions
/// endpoint we POST to. Trims whitespace, drops a trailing slash, and appends
/// `/chat/completions` when missing. Shared by `ai_chat` (request URL) and
/// `test_local_endpoint` (which derives the `/models` listing from it) so both
/// agree on the same canonical form regardless of how the user typed it.
fn normalize_local_base_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{}/chat/completions", trimmed)
    }
}

/// Helper to test an OpenAI-compatible API key (Groq, Mistral, etc.)
async fn test_openai_key(
    pool: &SqlitePool,
    api_key: &str,
    config: &ProviderConfig,
) -> Result<String, String> {
    let client = crate::shared::http::build_app_http_client(pool).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|e| e.to_string())?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let body = serde_json::json!({
        "model": config.model_id,
        "max_tokens": 10,
        "messages": [{"role": "user", "content": "Hi"}]
    });

    let response = client
        .post(config.api_url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    let status = response.status();
    if status.is_success() {
        Ok("Connected successfully".to_string())
    } else {
        let error_body = response.text().await.unwrap_or_default();
        let msg = match status.as_u16() {
            401 => "Invalid API key — please check and try again".to_string(),
            403 => "Access denied — your API key may not have permission".to_string(),
            429 => "Rate limited — please try again in a moment".to_string(),
            _ => {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&error_body) {
                    parsed["error"]["message"]
                        .as_str()
                        .unwrap_or("Unknown error")
                        .to_string()
                } else {
                    format!("API error ({})", status.as_u16())
                }
            }
        };
        Err(msg)
    }
}

/// Helper to test an Anthropic /v1/messages key.
async fn test_anthropic_key(
    pool: &SqlitePool,
    api_key: &str,
    config: &ProviderConfig,
) -> Result<String, String> {
    let client = crate::shared::http::build_app_http_client(pool).await?;
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

    let body = serde_json::json!({
        "model": config.model_id,
        "max_tokens": 10,
        "messages": [{"role": "user", "content": "Hi"}]
    });

    let response = client
        .post(config.api_url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    let status = response.status();
    if status.is_success() {
        Ok("Connected successfully".to_string())
    } else {
        let error_body: String = response.text().await.unwrap_or_default();
        let msg = match status.as_u16() {
            401 => "Invalid API key — please check and try again".to_string(),
            403 => "Access denied — your API key may not have permission".to_string(),
            429 => "Rate limited — please try again in a moment".to_string(),
            _ => {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&error_body) {
                    parsed["error"]["message"]
                        .as_str()
                        .unwrap_or("Unknown error")
                        .to_string()
                } else {
                    format!("API error ({})", status.as_u16())
                }
            }
        };
        Err(msg)
    }
}

/// Health-check a local OpenAI-compatible server (Ollama, LM Studio, …) by
/// GETting its `/v1/models` listing. Friendlier than a dummy-key chat call:
/// it needs no key and confirms the server is reachable. Reads the endpoint
/// from the `ai_local_base_url` setting, falling back to the registry default.
async fn test_local_endpoint(pool: &SqlitePool, api_key: &str) -> Result<String, String> {
    let base = local_setting(pool, "ai_local_base_url")
        .await
        .unwrap_or_else(|| "http://localhost:11434/v1/chat/completions".to_string());
    // Canonicalize to the same form `ai_chat` POSTs to, then derive the
    // sibling `/models` listing from its root so the health check and the
    // real request target the same server path.
    let canonical = normalize_local_base_url(&base);
    let root = canonical.trim_end_matches("/chat/completions");
    let models_url = format!("{}/models", root.trim_end_matches('/'));

    let client = crate::shared::http::build_app_http_client(pool).await?;
    let mut req = client.get(&models_url);
    if !api_key.trim().is_empty() {
        req = req.header(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|e| e.to_string())?,
        );
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Could not reach local server at {} — is it running? ({})", models_url, e))?;

    if response.status().is_success() {
        Ok("Connected to local server".to_string())
    } else {
        Err(format!(
            "Local server responded with HTTP {} — check the base URL",
            response.status().as_u16()
        ))
    }
}

#[tauri::command]
pub async fn test_ai_key(
    pool: State<'_, SqlitePool>,
    api_key: String,
    provider: String,
) -> Result<String, String> {
    if provider == "local" {
        return test_local_endpoint(pool.inner(), &api_key).await;
    }

    let config = resolve_config(&provider, None)?;

    if let Some(prefix) = config.key_prefix {
        if !api_key.starts_with(prefix) {
            let label = match config.provider_id {
                ProviderId::Claude => "Claude",
                ProviderId::Groq => "Groq",
                ProviderId::OpenAI => "OpenAI",
                _ => provider.as_str(),
            };
            return Err(format!(
                "Invalid key format — {} API keys start with '{}'",
                label, prefix
            ));
        }
    }

    match config.api_kind {
        ApiKind::AnthropicMessages => test_anthropic_key(pool.inner(), &api_key, config).await,
        ApiKind::OpenAICompat => test_openai_key(pool.inner(), &api_key, config).await,
    }
}

#[tauri::command]
pub async fn ai_chat(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    sql_manager: State<'_, Arc<SqlConnectionManager>>,
    nosql_connections: State<'_, NoSqlConnections>,
    auth_state: State<'_, crate::cloud::auth::AuthState>,
    api_key: String,
    messages: Vec<ChatMessage>,
    context: ChatContext,
    session_id: String,
    system_prompt: String,
    tools: Vec<serde_json::Value>,
    provider: String,
    // Optional extra HTTP headers to attach to each upstream request.
    // Currently used by the `clauge` provider to send `X-Provider:
    // github|google` so the worker can validate the cloud bearer token
    // against the correct provider's JWKS.
    extra_headers: Option<std::collections::HashMap<String, String>>,
) -> Result<(), String> {
    let client = crate::shared::http::build_app_http_client(pool.inner()).await?;
    let conversation_msgs = build_api_messages(&messages, &context);
    let sql_mgr = sql_manager.inner().clone();
    let nosql_mgr = nosql_connections.inner().clone();
    let extra_headers = extra_headers.unwrap_or_default();
    // Pass AuthState only for the Clauge provider — that's the only path
    // where 401 + Google refresh-and-retry is meaningful. BYOK providers
    // get None so the streaming client falls straight through to the
    // normal error mapping.
    let auth_for_stream = if provider == "clauge" {
        Some(auth_state.inner())
    } else {
        None
    };

    let config = match resolve_config(&provider, None) {
        Ok(cfg) => cfg,
        Err(msg) => {
            let _ = app.emit(
                &format!("ai:error:{}", session_id),
                serde_json::json!({"error": msg}),
            );
            return Err(msg);
        }
    };

    // The `local` provider stores its endpoint + model in user settings
    // (they vary per machine), overriding the registry placeholders.
    let (url_override, model_override) = if provider == "local" {
        (
            local_setting(pool.inner(), "ai_local_base_url")
                .await
                .map(|b| normalize_local_base_url(&b)),
            local_setting(pool.inner(), "ai_local_model").await,
        )
    } else {
        (None, None)
    };

    match config.api_kind {
        ApiKind::AnthropicMessages => {
            clients::anthropic::stream_anthropic(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools, config, &sql_mgr, &nosql_mgr,
            )
            .await
        }
        ApiKind::OpenAICompat => {
            clients::openai::stream_openai(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools, config, &sql_mgr, &nosql_mgr,
                &extra_headers, auth_for_stream,
                url_override.as_deref(), model_override.as_deref(),
            )
            .await
        }
    }
}

/// Resolve a pending frontend-handled tool (e.g. SSH `execute_shell`).
/// Frontend calls this with the captured + redacted command output (or an
/// error/cancellation message). The Rust AI chat loop unblocks and feeds
/// the result back to the model as a tool_result.
#[tauri::command]
pub fn ai_resolve_pending_tool(
    pending: State<'_, PendingFrontendTools>,
    tool_use_id: String,
    output: String,
) -> Result<(), String> {
    let sender = pending.map.lock().remove(&tool_use_id);
    match sender {
        Some(tx) => {
            tx.send(output).map_err(|_| "Receiver dropped".to_string())
        }
        None => Err(format!("No pending tool with id {}", tool_use_id)),
    }
}

pub mod types;
pub mod usage;
pub mod context;
pub mod tools;
pub mod tools_sql;
pub mod tools_nosql;
pub mod anthropic;
pub mod openai;

pub use types::*;
pub use usage::*;

use std::sync::Arc;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};

use super::sql_client::SqlConnectionManager;
use super::nosql_client::NoSqlConnections;

use self::anthropic::{ANTHROPIC_API_URL, ANTHROPIC_VERSION, DEFAULT_MODEL};
use self::context::build_api_messages;
use self::openai::{
    GROQ_API_URL, GROQ_DEFAULT_MODEL,
    MISTRAL_API_URL, MISTRAL_DEFAULT_MODEL,
    OPENAI_GH_API_URL, OPENAI_GH_DEFAULT_MODEL,
    NVIDIA_API_URL, NVIDIA_DEFAULT_MODEL,
    OPENROUTER_API_URL, OPENROUTER_DEFAULT_MODEL,
    OPENAI_API_URL, OPENAI_DEFAULT_MODEL,
    GEMINI_API_URL, GEMINI_DEFAULT_MODEL,
};

/// Helper to test an OpenAI-compatible API key (Groq, Mistral, etc.)
async fn test_openai_key(
    api_key: &str,
    api_url: &str,
    model: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|e| e.to_string())?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 10,
        "messages": [{"role": "user", "content": "Hi"}]
    });

    let response = client
        .post(api_url)
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

#[tauri::command]
pub async fn test_ai_key(api_key: String, provider: String) -> Result<String, String> {
    match provider.as_str() {
        "groq" => {
            if !api_key.starts_with("gsk_") {
                return Err("Invalid key format — Groq API keys start with 'gsk_'".to_string());
            }
            test_openai_key(&api_key, GROQ_API_URL, GROQ_DEFAULT_MODEL).await
        }
        "mistral" => {
            test_openai_key(&api_key, MISTRAL_API_URL, MISTRAL_DEFAULT_MODEL).await
        }
        "openai_gh" => {
            test_openai_key(&api_key, OPENAI_GH_API_URL, OPENAI_GH_DEFAULT_MODEL).await
        }
        "nvidia" => {
            test_openai_key(&api_key, NVIDIA_API_URL, NVIDIA_DEFAULT_MODEL).await
        }
        "openrouter" => {
            test_openai_key(&api_key, OPENROUTER_API_URL, OPENROUTER_DEFAULT_MODEL).await
        }
        "openai_direct" => {
            if !api_key.starts_with("sk-") {
                return Err("Invalid key format — OpenAI API keys start with 'sk-'".to_string());
            }
            test_openai_key(&api_key, OPENAI_API_URL, OPENAI_DEFAULT_MODEL).await
        }
        "gemini" => {
            test_openai_key(&api_key, GEMINI_API_URL, GEMINI_DEFAULT_MODEL).await
        }
        _ => {
            // Claude (default)
            if !api_key.starts_with("sk-ant-") {
                return Err(
                    "Invalid key format — Claude API keys start with 'sk-ant-'".to_string(),
                );
            }

            let client = reqwest::Client::new();
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(&api_key).map_err(|e| e.to_string())?,
            );
            headers.insert(
                "anthropic-version",
                HeaderValue::from_static(ANTHROPIC_VERSION),
            );
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

            let body = serde_json::json!({
                "model": DEFAULT_MODEL,
                "max_tokens": 10,
                "messages": [{"role": "user", "content": "Hi"}]
            });

            let response = client
                .post(ANTHROPIC_API_URL)
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
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&error_body)
                        {
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
    }
}

#[tauri::command]
pub async fn ai_chat(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    sql_manager: State<'_, Arc<SqlConnectionManager>>,
    nosql_connections: State<'_, NoSqlConnections>,
    api_key: String,
    messages: Vec<ChatMessage>,
    context: ChatContext,
    session_id: String,
    system_prompt: String,
    tools: Vec<serde_json::Value>,
    provider: String,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let conversation_msgs = build_api_messages(&messages, &context);
    let sql_mgr = sql_manager.inner().clone();
    let nosql_mgr = nosql_connections.inner().clone();

    match provider.as_str() {
        "claude" => {
            anthropic::stream_anthropic(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools, &sql_mgr, &nosql_mgr,
            )
            .await
        }
        "groq" => {
            openai::stream_openai(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools,
                GROQ_API_URL, GROQ_DEFAULT_MODEL, &sql_mgr, &nosql_mgr,
            )
            .await
        }
        "mistral" => {
            openai::stream_openai(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools,
                MISTRAL_API_URL, MISTRAL_DEFAULT_MODEL, &sql_mgr, &nosql_mgr,
            )
            .await
        }
        "openai_gh" => {
            openai::stream_openai(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools,
                OPENAI_GH_API_URL, OPENAI_GH_DEFAULT_MODEL, &sql_mgr, &nosql_mgr,
            )
            .await
        }
        "nvidia" => {
            openai::stream_openai(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools,
                NVIDIA_API_URL, NVIDIA_DEFAULT_MODEL, &sql_mgr, &nosql_mgr,
            )
            .await
        }
        "openrouter" => {
            openai::stream_openai(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools,
                OPENROUTER_API_URL, OPENROUTER_DEFAULT_MODEL, &sql_mgr, &nosql_mgr,
            )
            .await
        }
        "openai_direct" => {
            openai::stream_openai(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools,
                OPENAI_API_URL, OPENAI_DEFAULT_MODEL, &sql_mgr, &nosql_mgr,
            )
            .await
        }
        "gemini" => {
            openai::stream_openai(
                &client, &app, pool.inner(), &api_key, conversation_msgs,
                &context, &session_id, &system_prompt, &tools,
                GEMINI_API_URL, GEMINI_DEFAULT_MODEL, &sql_mgr, &nosql_mgr,
            )
            .await
        }
        unknown => {
            let msg = format!("Unknown AI provider: {}", unknown);
            let _ = app.emit(
                &format!("ai:error:{}", session_id),
                serde_json::json!({"error": msg}),
            );
            return Err(msg);
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

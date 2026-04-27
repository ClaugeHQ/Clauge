use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use parking_lot::Mutex;
use tokio::sync::oneshot;

/// Pending frontend-handled tool calls.
/// Used by tools like `execute_shell` (SSH mode): when the model invokes the
/// tool, the Rust chat loop inserts a oneshot sender keyed by the tool_use_id,
/// emits an `ai:tool_pending:<session>` event, and awaits the receiver. The
/// frontend shows a confirmation modal, runs the command, captures output, and
/// calls `ai_resolve_pending_tool` to unblock with the (redacted) result.
#[derive(Default)]
pub struct PendingFrontendTools {
    pub map: Mutex<HashMap<String, oneshot::Sender<String>>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageStat {
    pub mode: String,
    pub total_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderStat {
    pub model: String,
    pub total_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatContext {
    pub mode: String,
    pub current_request: Option<ContextRequest>,
    pub current_response: Option<ContextResponse>,
    pub env_vars: Vec<ContextEnvVar>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<ContextKV>,
    pub params: Vec<ContextKV>,
    pub body: String,
    pub body_type: String,
    pub auth_type: String,
    pub auth_data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub duration_ms: u64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextKV {
    pub key: String,
    pub value: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEnvVar {
    pub key: String,
    pub value: String,
    pub is_secret: bool,
}

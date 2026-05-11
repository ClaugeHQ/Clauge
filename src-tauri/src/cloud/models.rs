use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudUser {
    pub user_id: i64,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudProvider {
    pub provider: String,
    pub provider_user_id: String,
    pub provider_login: Option<String>,
    pub email: Option<String>,
    pub linked_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudEntitlements {
    pub features: Vec<String>,
    pub limits: serde_json::Value,
}

/// Response from /api/auth/{provider}/exchange and /api/auth/me.
/// `token`/`refresh`/`id_token` only populated on /exchange paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub refresh: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    pub user: CloudUser,
    pub providers: Vec<CloudProvider>,
    pub plan: String,
    pub entitlements: CloudEntitlements,
}

/// Response from /api/auth/me — same shape minus tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub user: CloudUser,
    pub providers: Vec<CloudProvider>,
    pub plan: String,
    pub entitlements: CloudEntitlements,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStateRow {
    pub kind: String,
    pub content_hash: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPullResponse {
    pub kind: String,
    pub content_hash: String,
    pub updated_at: String,
    pub payload: String, // base64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPushResponse {
    pub kind: String,
    pub content_hash: String,
    pub updated_at: String,
}

/// Snapshot returned to the frontend by `cloud_get_status`.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CloudStatus {
    pub connected: bool,
    pub active_provider: Option<String>,
    pub user: Option<CloudUser>,
    pub providers: Vec<CloudProvider>,
    pub plan: String,
    pub last_synced: std::collections::HashMap<String, String>,
}

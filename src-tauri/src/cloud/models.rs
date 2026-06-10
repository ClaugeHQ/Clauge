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
    /// ISO timestamp of the user row creation — drives "Member since" in
    /// the subscription card. May be absent on legacy rows.
    #[serde(default)]
    pub created_at: Option<String>,
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
    pub plan: String,
    #[serde(default)]
    pub credits: Option<CloudCredits>,
    #[serde(default)]
    pub subscription: Option<CloudSubscription>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudCredits {
    pub remaining: i64,
    pub allowance: i64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSubscription {
    pub status: String,
    pub cancel_at_period_end: bool,
    /// True for users on the one-time Lifetime plan. Recurring users
    /// (monthly/yearly) get this as false. Lifetime credits refill on
    /// the purchase anniversary via lazy refill at worker side.
    #[serde(default)]
    pub is_lifetime: bool,
    /// ISO timestamp for the next renewal / cancellation cutoff. Same value
    /// as credits.resets_at on a recurring plan but exposed explicitly so
    /// the subscription card doesn't have to reach across objects.
    #[serde(default)]
    pub current_period_end: Option<String>,
    /// ISO timestamp the current cycle started — drives the period-length
    /// math in the UI if needed (we already get `interval` from the worker).
    #[serde(default)]
    pub current_period_start: Option<String>,
    /// "monthly" | "yearly" | "lifetime" — derived on the worker.
    #[serde(default)]
    pub interval: Option<String>,
    /// Display price in USD (whole dollars), e.g. 12, 100, or 299.
    #[serde(default)]
    pub price_usd: Option<i64>,
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
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
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

/// One archived version of a kind, from /api/sync/history/:kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncHistoryEntry {
    pub content_hash: String,
    #[serde(default)]
    pub device_name: Option<String>,
    pub replaced_at: String,
}

/// Archived blob payload, from /api/sync/history/:kind/:hash.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncHistoryBlob {
    pub payload: String, // base64
    pub content_hash: String,
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
    /// Entitlements snapshot (credits balance + subscription details) from
    /// /api/auth/me. Optional so the default `CloudStatus` (signed-out) can
    /// omit it; populated by `build_status` on any authenticated refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entitlements: Option<CloudEntitlements>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudPricing {
    pub schema_version: i64,
    pub plans: Vec<CloudPricingPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudPricingPlan {
    pub id: String,
    pub price_usd: i64,
    #[serde(default)]
    pub credits: i64,
    pub discount: Option<CloudPricingDiscount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudPricingDiscount {
    pub percent: i64,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCheckoutRequest {
    pub plan: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCheckoutResponse {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudPortalResponse {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudAiBalance {
    pub remaining: i64,
    pub allowance: i64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudAiUsage {
    pub entries: Vec<CloudAiUsageEntry>,
    pub next_before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudAiUsageEntry {
    pub occurred_at: String,
    pub operation: String,
    pub clauge_credits: i64,
    pub cost_usd_micros: i64,
    pub request_id: String,
    /// Originating app mode (rest / sql / nosql / ssh / explorer / agent /
    /// workspace). Optional — older log rows from before the migration
    /// don't carry it. Drives the per-mode breakdown card.
    #[serde(default)]
    pub mode: Option<String>,
}

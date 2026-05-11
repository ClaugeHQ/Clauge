// Compile-time API base. Override at build with CLAUGE_API_URL.
pub const API_BASE_URL: &str = match option_env!("CLAUGE_API_URL") {
    Some(v) => v,
    None => "https://clauge.in",
};

// OS keyring service for cloud auth tokens. Separate service from SSH /
// Explorer secrets so wiping cloud auth doesn't touch user-saved credentials.
pub const KEYRING_SERVICE: &str = "Clauge Cloud Auth";

pub const KEY_GITHUB_ACCESS: &str = "github:access_token";
pub const KEY_GOOGLE_ACCESS: &str = "google:access_token";
pub const KEY_GOOGLE_REFRESH: &str = "google:refresh_token";
pub const KEY_GOOGLE_ID: &str = "google:id_token";

// settings table keys for non-secret bookkeeping.
pub const SETTINGS_KEY_USER_ID: &str = "cloud:user_id";
pub const SETTINGS_KEY_ACTIVE_PROVIDER: &str = "cloud:active_provider";
pub const SETTINGS_KEY_HAS_SYNCED: &str = "cloud:has_synced";

// Per-kind last-pushed hash; key format `cloud:hash:<kind>`.
pub fn settings_key_hash(kind: &str) -> String {
    format!("cloud:hash:{}", kind)
}
// Per-kind last-synced ISO timestamp; key format `cloud:synced_at:<kind>`.
pub fn settings_key_synced_at(kind: &str) -> String {
    format!("cloud:synced_at:{}", kind)
}

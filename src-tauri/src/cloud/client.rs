// Typed HTTP client for the Clauge Worker API. Uses the shared proxy-aware
// reqwest client built by `shared::http`.

use serde::de::DeserializeOwned;
use sqlx::SqlitePool;

use crate::cloud::auth::AuthState;
use crate::cloud::config::API_BASE_URL;
use crate::cloud::models::{
    AuthResponse, MeResponse, SyncHistoryBlob, SyncHistoryEntry, SyncPullResponse,
    SyncPushResponse, SyncStateRow,
};
use crate::shared::http::build_app_http_client;

/// Errors all callers see — unified shape so commands can map to user-friendly messages.
#[derive(Debug)]
pub enum CloudError {
    NotAuthenticated,
    Network(String),
    Server { status: u16, body: String },
    /// 412 Precondition Failed from `/api/sync/push/:kind` — remote hash
    /// doesn't match the `prevHash` we sent. Caller decides whether to
    /// pull, prompt the user, or force-push.
    Conflict { current_hash: Option<String>, current_updated_at: Option<String> },
}

impl std::fmt::Display for CloudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloudError::NotAuthenticated => write!(f, "Not signed in to Clauge cloud"),
            CloudError::Network(e) => write!(f, "Network error: {}", e),
            CloudError::Server { status, body } => write!(f, "Cloud API {}: {}", status, body),
            CloudError::Conflict { .. } => write!(f, "Remote has changed since this device last synced."),
        }
    }
}

impl From<CloudError> for String {
    fn from(e: CloudError) -> String {
        e.to_string()
    }
}

// ─── Exchange endpoints (no auth required) ──────────────────────────────────

pub async fn exchange_github(pool: &SqlitePool, code: &str) -> Result<AuthResponse, CloudError> {
    post_json_no_auth(
        pool,
        "/api/auth/github/exchange",
        serde_json::json!({ "code": code }),
    )
    .await
}

pub async fn exchange_google(
    pool: &SqlitePool,
    code: &str,
    redirect_uri: &str,
) -> Result<AuthResponse, CloudError> {
    post_json_no_auth(
        pool,
        "/api/auth/google/exchange",
        serde_json::json!({ "code": code, "redirectUri": redirect_uri }),
    )
    .await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleRefreshResponse {
    pub token: String,
    pub id_token: Option<String>,
    pub expires_in: Option<i64>,
}

pub async fn refresh_google(
    pool: &SqlitePool,
    refresh_token: &str,
) -> Result<GoogleRefreshResponse, CloudError> {
    post_json_no_auth(
        pool,
        "/api/auth/google/refresh",
        serde_json::json!({ "refreshToken": refresh_token }),
    )
    .await
}

// ─── Refresh-and-retry wrapper ──────────────────────────────────────────────
//
// Every auth-required endpoint runs through this wrapper. When the
// Worker returns 401 (id_token expired / signature invalid), we:
//   1. If the active provider is Google AND we have a refresh token,
//      call /api/auth/google/refresh and persist the rotated id_token.
//   2. Retry the original op exactly once.
//   3. Either retry succeeds → return its result, or it 401s again →
//      collapse to `CloudError::NotAuthenticated` so callers can clear
//      auth and surface a re-login UI cleanly.
// For non-Google providers (GitHub today), 401 always means "re-login
// required" — there's no refresh flow — so we collapse to
// NotAuthenticated immediately. Network / 5xx errors are propagated
// as-is so the caller can keep partial state and try again later.

async fn with_google_refresh_retry<T, F, Fut>(
    pool: &SqlitePool,
    state: &AuthState,
    mut op: F,
) -> Result<T, CloudError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, CloudError>>,
{
    let first = op().await;
    let is_unauth = matches!(&first, Err(CloudError::Server { status: 401, .. }));
    if !is_unauth {
        return first;
    }
    let provider = state.snapshot().active_provider;
    if provider.as_deref() != Some("google") {
        // No refresh path → caller must re-login. Drop the verbose
        // server body in favour of the canonical signal.
        return Err(CloudError::NotAuthenticated);
    }
    if crate::cloud::auth::refresh_google_and_store(state, pool)
        .await
        .is_err()
    {
        return Err(CloudError::NotAuthenticated);
    }
    let retry = op().await;
    if matches!(&retry, Err(CloudError::Server { status: 401, .. })) {
        return Err(CloudError::NotAuthenticated);
    }
    retry
}

// ─── Auth-required endpoints ────────────────────────────────────────────────

pub async fn me(pool: &SqlitePool, state: &AuthState) -> Result<MeResponse, CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        get_json_auth(pool, "/api/auth/me", &token, &provider).await
    })
    .await
}

pub async fn update_profile(
    pool: &SqlitePool,
    state: &AuthState,
    display_name: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
) -> Result<MeResponse, CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        let client = build_app_http_client(pool).await.map_err(CloudError::Network)?;
        let url = format!("{}{}", API_BASE_URL, "/api/auth/me");
        // Only include fields the caller passed — `null` clears, missing = leave alone.
        let mut body = serde_json::Map::new();
        if let Some(ref v) = display_name { body.insert("displayName".into(), serde_json::Value::String(v.clone())); }
        if let Some(ref v) = first_name   { body.insert("firstName".into(),   serde_json::Value::String(v.clone())); }
        if let Some(ref v) = last_name    { body.insert("lastName".into(),    serde_json::Value::String(v.clone())); }
        let resp = client
            .patch(url)
            .header("Authorization", format!("Bearer {}", token))
            .header("X-Provider", provider)
            .header("Content-Type", "application/json")
            .json(&serde_json::Value::Object(body))
            .send()
            .await
            .map_err(|e| CloudError::Network(e.to_string()))?;
        check_ok(resp).await
    })
    .await
}

pub async fn delete_account(
    pool: &SqlitePool,
    state: &AuthState,
    confirm_slug: &str,
) -> Result<(), CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        let client = build_app_http_client(pool).await.map_err(CloudError::Network)?;
        let url = format!("{}{}", API_BASE_URL, "/api/auth/me");
        let resp = client
            .delete(url)
            .header("Authorization", format!("Bearer {}", token))
            .header("X-Provider", provider)
            .header("X-Confirm", confirm_slug)
            .send()
            .await
            .map_err(|e| CloudError::Network(e.to_string()))?;
        check_ok(resp).await.map(|_: serde_json::Value| ())
    })
    .await
}

pub async fn link(
    pool: &SqlitePool,
    state: &AuthState,
    provider: &str,
    code: &str,
    redirect_uri: Option<&str>,
) -> Result<MeResponse, CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, active_provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        let body = serde_json::json!({
            "provider": provider,
            "code": code,
            "redirectUri": redirect_uri.unwrap_or("https://clauge.in/auth/google-callback.html"),
        });
        post_json_auth(pool, "/api/auth/link", body, &token, &active_provider).await
    })
    .await
}

pub async fn unlink(
    pool: &SqlitePool,
    state: &AuthState,
    provider: &str,
) -> Result<MeResponse, CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, active_provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        post_json_auth(
            pool,
            "/api/auth/unlink",
            serde_json::json!({ "provider": provider }),
            &token,
            &active_provider,
        )
        .await
    })
    .await
}

// ─── Sync endpoints ─────────────────────────────────────────────────────────

pub async fn sync_state(
    pool: &SqlitePool,
    state: &AuthState,
) -> Result<Vec<SyncStateRow>, CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        get_json_auth(pool, "/api/sync/state", &token, &provider).await
    })
    .await
}

pub async fn sync_pull(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
) -> Result<SyncPullResponse, CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        let path = format!("/api/sync/pull/{}", kind);
        get_json_auth(pool, &path, &token, &provider).await
    })
    .await
}

/// Archived versions of a kind, newest first (≤5).
pub async fn sync_history_list(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
) -> Result<Vec<SyncHistoryEntry>, CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        let path = format!("/api/sync/history/{}", kind);
        get_json_auth(pool, &path, &token, &provider).await
    })
    .await
}

/// Fetch one archived blob by content hash. 404 surfaces as `CloudError::Server`.
pub async fn sync_history_blob(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
    hash: &str,
) -> Result<SyncHistoryBlob, CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        let path = format!("/api/sync/history/{}/{}", kind, hash);
        get_json_auth(pool, &path, &token, &provider).await
    })
    .await
}

/// Push a kind blob with optimistic concurrency.
///
/// `prev_hash` semantics (matches the Worker side):
///   - `None`     → first push of this kind (server requires row to not exist).
///   - `Some("*")` → force overwrite (post-conflict "Keep my changes").
///   - `Some(hex)` → only succeed if remote currently has that hash.
///
/// On 412 the server returns the current remote state; we surface it as
/// `CloudError::Conflict` so the orchestrator can flip the kind into
/// conflict-locked mode and the UI can show the resolver.
pub async fn sync_push(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
    content_hash: &str,
    payload_b64: &str,
    prev_hash: Option<&str>,
) -> Result<SyncPushResponse, CloudError> {
    let device_id = crate::telemetry::device::ensure_device_id(pool).await;
    let device_name = crate::shared::repos::settings::get_by_key(pool, "cloud:device_name")
        .await
        .ok()
        .flatten()
        .map(|r| r.value)
        .unwrap_or_else(|| "This device".to_string());
    with_google_refresh_retry(pool, state, || async {
        let (token, provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        let client = build_app_http_client(pool).await.map_err(CloudError::Network)?;
        let url = format!("{}{}/{}", API_BASE_URL, "/api/sync/push", kind);

        let mut body = serde_json::Map::new();
        body.insert("contentHash".into(), serde_json::Value::String(content_hash.into()));
        body.insert("payload".into(), serde_json::Value::String(payload_b64.into()));
        if let Some(p) = prev_hash {
            body.insert("prevHash".into(), serde_json::Value::String(p.into()));
        }
        body.insert("deviceId".into(), serde_json::Value::String(device_id.clone()));
        body.insert("deviceName".into(), serde_json::Value::String(device_name.clone()));

        let resp = client
            .put(url)
            .header("Authorization", format!("Bearer {}", token))
            .header("X-Provider", provider)
            .header("Content-Type", "application/json")
            .json(&serde_json::Value::Object(body))
            .send()
            .await
            .map_err(|e| CloudError::Network(e.to_string()))?;

        // 412 → conflict. Parse the body to surface the current
        // remote hash so the resolver can show "this device vs
        // other device" stats.
        if resp.status().as_u16() == 412 {
            #[derive(serde::Deserialize)]
            struct ConflictBody {
                #[serde(rename = "currentHash")] current_hash: Option<String>,
                #[serde(rename = "currentUpdatedAt")] current_updated_at: Option<String>,
            }
            let body: ConflictBody = resp.json().await.unwrap_or(ConflictBody {
                current_hash: None,
                current_updated_at: None,
            });
            return Err(CloudError::Conflict {
                current_hash: body.current_hash,
                current_updated_at: body.current_updated_at,
            });
        }

        check_ok(resp).await
    })
    .await
}

pub async fn sync_wipe(pool: &SqlitePool, state: &AuthState) -> Result<(), CloudError> {
    with_google_refresh_retry(pool, state, || async {
        let (token, provider) = state
            .active_token_and_provider()
            .ok_or(CloudError::NotAuthenticated)?;
        let client = build_app_http_client(pool).await.map_err(CloudError::Network)?;
        let url = format!("{}{}", API_BASE_URL, "/api/sync/wipe");
        let resp = client
            .delete(url)
            .header("Authorization", format!("Bearer {}", token))
            .header("X-Provider", provider)
            .header("X-Confirm", "yes")
            .send()
            .await
            .map_err(|e| CloudError::Network(e.to_string()))?;
        check_ok(resp).await.map(|_: serde_json::Value| ())
    })
    .await
}

// ─── Internals ──────────────────────────────────────────────────────────────

pub(crate) async fn get_json_no_auth<T: DeserializeOwned>(
    pool: &SqlitePool,
    path: &str,
) -> Result<T, CloudError> {
    let client = build_app_http_client(pool).await.map_err(CloudError::Network)?;
    let resp = client
        .get(format!("{}{}", API_BASE_URL, path))
        .send()
        .await
        .map_err(|e| CloudError::Network(e.to_string()))?;
    check_ok(resp).await
}

pub(crate) async fn post_json_no_auth<T: DeserializeOwned>(
    pool: &SqlitePool,
    path: &str,
    body: serde_json::Value,
) -> Result<T, CloudError> {
    let client = build_app_http_client(pool).await.map_err(CloudError::Network)?;
    let resp = client
        .post(format!("{}{}", API_BASE_URL, path))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CloudError::Network(e.to_string()))?;
    check_ok(resp).await
}

pub(crate) async fn post_json_auth<T: DeserializeOwned>(
    pool: &SqlitePool,
    path: &str,
    body: serde_json::Value,
    token: &str,
    provider: &str,
) -> Result<T, CloudError> {
    let client = build_app_http_client(pool).await.map_err(CloudError::Network)?;
    let resp = client
        .post(format!("{}{}", API_BASE_URL, path))
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Provider", provider)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CloudError::Network(e.to_string()))?;
    check_ok(resp).await
}

pub(crate) async fn get_json_auth<T: DeserializeOwned>(
    pool: &SqlitePool,
    path: &str,
    token: &str,
    provider: &str,
) -> Result<T, CloudError> {
    let client = build_app_http_client(pool).await.map_err(CloudError::Network)?;
    let resp = client
        .get(format!("{}{}", API_BASE_URL, path))
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Provider", provider)
        .send()
        .await
        .map_err(|e| CloudError::Network(e.to_string()))?;
    check_ok(resp).await
}

async fn check_ok<T: DeserializeOwned>(
    resp: tauri_plugin_http::reqwest::Response,
) -> Result<T, CloudError> {
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        if status >= 500 {
            crate::telemetry::bump("err.api_5xx");
        }
        let body = resp.text().await.unwrap_or_default();
        return Err(CloudError::Server { status, body });
    }
    resp.json::<T>().await.map_err(|e| CloudError::Network(e.to_string()))
}

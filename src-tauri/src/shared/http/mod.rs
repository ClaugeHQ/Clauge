//! Centralised `reqwest::Client` construction.
//!
//! Two builders, both backed by user settings stored in the `settings` table:
//!
//! - `build_app_http_client(pool)` — applies the global proxy setting and
//!   nothing else. All non-REST outbound traffic (AI providers, GitHub
//!   OAuth + Gist sync, the updater, ClickHouse HTTP, agent usage analytics)
//!   should go through this so corporate-network users behind a mandatory
//!   proxy can reach those services.
//!
//! - `build_rest_http_client(pool)` — extends the app client with REST-only
//!   knobs: `request_timeout`, `follow_redirects`, `ssl_verification`. The
//!   `max_response_size` setting is enforced separately in the executor
//!   (reqwest doesn't have a per-request body cap).
//!
//! All settings have safe fallbacks; missing or unparsable values fall back
//! to the documented defaults rather than failing the build.
//!
//! `ssl_verification` defaults to `true` — this is the safe default. The
//! UI exposes a toggle to flip it OFF (with a clear retry-on-cert-error
//! flow on the frontend), which then passes through as
//! `danger_accept_invalid_certs(true)` to reqwest. Previously this flag
//! was unconditionally hardcoded `true` (= verify OFF), which was wrong.
//!
//! Errors are bubbled up rather than swallowed: if `proxy_url` is malformed
//! the whole client build fails so the user sees a real error instead of
//! silent fallback.

use std::time::Duration;

use sqlx::SqlitePool;

use crate::shared::repos::settings as settings_repo;

/// Default values used when a setting is missing or unparseable.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_REDIRECT_LIMIT: usize = 10;

/// Read a single setting value as `String`. Returns `None` if the key is
/// absent OR the stored value is empty (the UI stores empty strings for
/// "no proxy" / "no auth username" rather than deleting the row).
async fn get_setting(pool: &SqlitePool, key: &str) -> Option<String> {
    settings_repo::get_by_key(pool, key)
        .await
        .ok()
        .flatten()
        .map(|s| s.value)
        .filter(|v| !v.is_empty())
}

async fn get_bool(pool: &SqlitePool, key: &str, default_: bool) -> bool {
    match get_setting(pool, key).await.as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => default_,
    }
}

async fn get_u64(pool: &SqlitePool, key: &str, default_: u64) -> u64 {
    get_setting(pool, key)
        .await
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_)
}

/// Apply the global proxy setting to a builder. Shared between the app and
/// REST builders so there's exactly one place that knows the proxy keys.
async fn apply_proxy(
    pool: &SqlitePool,
    builder: reqwest::ClientBuilder,
) -> Result<reqwest::ClientBuilder, String> {
    let proxy_url = match get_setting(pool, "proxy_url").await {
        Some(u) => u,
        None => return Ok(builder), // no proxy configured
    };

    let mut proxy = reqwest::Proxy::all(&proxy_url)
        .map_err(|e| format!("Invalid proxy URL '{}': {}", proxy_url, e))?;

    if get_bool(pool, "proxy_auth", false).await {
        let username = get_setting(pool, "proxy_username").await.unwrap_or_default();
        let password = get_setting(pool, "proxy_password").await.unwrap_or_default();
        proxy = proxy.basic_auth(&username, &password);
    }

    Ok(builder.proxy(proxy))
}

/// Build a generic HTTP client suitable for non-user-driven traffic
/// (AI providers, GitHub, updater, etc.).
///
/// Always uses safe defaults: SSL verification ON, sane connect/total
/// timeout (60s — these calls can be larger than user REST requests),
/// redirects followed up to 10 hops. Only the proxy is configurable.
pub async fn build_app_http_client(pool: &SqlitePool) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(DEFAULT_REDIRECT_LIMIT));

    let builder = apply_proxy(pool, builder).await?;

    builder
        .build()
        .map_err(|e| format!("HTTP client build failed: {}", e))
}

/// Build the HTTP client for large file downloads (whisper models).
///
/// Deliberately has NO total timeout — a 466 MB model on a slow link can
/// take far longer than the app client's 60s budget. Stall protection comes
/// from `connect_timeout` (30s) + `read_timeout` (60s between chunks,
/// reqwest ≥ 0.12.5). Picks up the global proxy like every other client.
pub async fn build_download_http_client(pool: &SqlitePool) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(DEFAULT_REDIRECT_LIMIT));

    let builder = apply_proxy(pool, builder).await?;

    builder
        .build()
        .map_err(|e| format!("HTTP client build failed: {}", e))
}

/// Build the HTTP client for one-shot AI completion calls (the collect
/// path used by background generation flows such as meeting notes).
///
/// Deliberately has NO total timeout — a non-streaming completion with a
/// large `max_tokens` budget can take well over the app client's 60s
/// total. Stall protection comes from `connect_timeout` (30s) +
/// `read_timeout` (120s of socket silence; for SSE-collected responses
/// this applies between chunks). Picks up the global proxy like every
/// other client.
pub async fn build_ai_oneshot_http_client(pool: &SqlitePool) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(DEFAULT_REDIRECT_LIMIT));

    let builder = apply_proxy(pool, builder).await?;

    builder
        .build()
        .map_err(|e| format!("HTTP client build failed: {}", e))
}

/// Build the HTTP client for user-driven REST requests. Layers REST-only
/// knobs (timeout, redirects, SSL verification, gzip/brotli/deflate)
/// on top of the app-client base.
///
/// `max_response_size` is enforced by the executor after `.bytes()` rather
/// than here — reqwest has no built-in body cap.
pub async fn build_rest_http_client(pool: &SqlitePool) -> Result<reqwest::Client, String> {
    let timeout_ms = get_u64(pool, "request_timeout", DEFAULT_TIMEOUT_MS).await;
    let follow_redirects = get_bool(pool, "follow_redirects", true).await;
    let verify_ssl = get_bool(pool, "ssl_verification", true).await;

    let redirect_policy = if follow_redirects {
        reqwest::redirect::Policy::limited(DEFAULT_REDIRECT_LIMIT)
    } else {
        reqwest::redirect::Policy::none()
    };

    let builder = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .redirect(redirect_policy)
        .danger_accept_invalid_certs(!verify_ssl)
        .gzip(true)
        .brotli(true)
        .deflate(true);

    let builder = apply_proxy(pool, builder).await?;

    builder
        .build()
        .map_err(|e| format!("HTTP client build failed: {}", e))
}

/// HTTP client for SQL HTTP drivers (D1, ClickHouse). Honors the
/// user-configurable `sql_http_query_timeout_sec` setting; falls back to
/// 60s when unset. Also picks up the app proxy.
pub async fn build_sql_http_client(pool: &SqlitePool) -> Result<reqwest::Client, String> {
    let timeout_sec = crate::shared::repos::settings::get_u64_or(pool, "sql_http_query_timeout_sec", 60).await;
    let builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_sec))
        .redirect(reqwest::redirect::Policy::limited(DEFAULT_REDIRECT_LIMIT));

    let builder = apply_proxy(pool, builder).await?;

    builder
        .build()
        .map_err(|e| format!("HTTP client build failed: {}", e))
}

pub async fn max_response_bytes(pool: &SqlitePool) -> u64 {
    let mb = get_u64(pool, "max_response_size", 10).await;
    mb.saturating_mul(1024 * 1024)
}

/// Build a stripped-down HTTP client used solely to probe whether a
/// failed REST request was caused by TLS cert verification. Mirrors
/// the proxy settings of the real REST client (so the probe traverses
/// the same network path) but pins a short timeout, disables redirects,
/// and turns cert verification OFF.
async fn build_probe_http_client(pool: &SqlitePool) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(true);

    let builder = apply_proxy(pool, builder).await?;

    builder
        .build()
        .map_err(|e| format!("Probe client build failed: {}", e))
}

/// Probe whether the URL would succeed if cert verification were disabled.
/// Returns `true` ⇒ the original failure was a TLS cert problem; the caller
/// should surface the `ssl-error:` prefix so the UI offers the disable-
/// verification guide.
///
/// Why a probe instead of inspecting the error: reqwest erases the TLS
/// backend's typed error behind `Box<dyn Error>`, and the textual phrasing
/// drifts between native-tls (different again on Linux/macOS/Windows),
/// rustls, and reqwest versions. A live retry sidesteps all of that.
///
/// Sends HEAD so the user's request body isn't replayed against a server
/// we just failed to verify. Any HTTP response — even 4xx/5xx — means the
/// TLS handshake completed, which is the only signal we need.
pub async fn is_ssl_failure(pool: &SqlitePool, url: &str) -> bool {
    let Ok(client) = build_probe_http_client(pool).await else {
        return false;
    };
    client.head(url).send().await.is_ok()
}

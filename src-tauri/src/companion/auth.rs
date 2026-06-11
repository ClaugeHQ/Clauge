// Bearer auth for everything under /v1. Tokens are 32B random values
// held only by the phone; the DB stores SHA-256 hex digests. Lookups
// scan all non-revoked rows with a constant-time comparison instead of
// an indexed equality probe so the matching path is timing-uniform —
// the device count is tiny, the scan is free.

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json as JsonResponse, Response},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::sync::Arc;

use super::devices;
use super::server::CompanionAppState;

/// The authenticated device id, stashed in request extensions by the
/// middleware so /v1 handlers know who is talking (push suppression in
/// D4 needs this).
// dead_code: nothing extracts the extension until the /v1 session
// routes land in D2 — the middleware contract is defined here so they
// don't have to re-derive identity.
#[allow(dead_code)]
#[derive(Clone)]
pub struct AuthedDevice(pub String);

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Branch-free byte comparison — the early-exit of `==` would leak how
/// many leading bytes matched.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Resolve a bearer token to a non-revoked device id, bumping
/// last_seen_at on success. None = unauthorized.
pub async fn authorize(pool: &SqlitePool, token: &str) -> Option<String> {
    if token.is_empty() {
        return None;
    }
    let candidate = hash_token(token);
    let rows = match devices::active_token_hashes(pool).await {
        Ok(rows) => rows,
        Err(e) => {
            log::error!("[companion] auth lookup failed: {}", e);
            return None;
        }
    };
    let matched = rows
        .into_iter()
        .find(|(_, hash)| ct_eq(candidate.as_bytes(), hash.as_bytes()))
        .map(|(id, _)| id)?;
    let _ = devices::bump_last_seen(pool, &matched).await;
    Some(matched)
}

pub async fn require_bearer(
    State(state): State<Arc<CompanionAppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    match authorize(&state.pool, token).await {
        Some(device_id) => {
            req.extensions_mut().insert(AuthedDevice(device_id));
            next.run(req).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            JsonResponse(json!({ "error": "unauthorized" })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIGRATION_22: &str = include_str!("../../migrations/22_companion_devices.sql");

    /// In-memory pool pinned to one connection — each sqlite ::memory:
    /// connection is its own database, so a second pool connection
    /// would see no schema.
    async fn test_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::raw_sql(MIGRATION_22)
            .execute(&pool)
            .await
            .expect("apply migration 22");
        pool
    }

    #[tokio::test]
    async fn token_hash_verify_roundtrip() {
        let pool = test_pool().await;
        let token = "a".repeat(64);
        devices::insert(&pool, "dev-1", "Pixel", "android", &hash_token(&token))
            .await
            .unwrap();

        assert_eq!(authorize(&pool, &token).await.as_deref(), Some("dev-1"));
        assert_eq!(authorize(&pool, "wrong-token").await, None);
        assert_eq!(authorize(&pool, "").await, None);

        // Successful auth must bump last_seen_at.
        let devs = devices::list(&pool).await.unwrap();
        assert!(devs[0].last_seen_at.is_some());
    }

    #[tokio::test]
    async fn authorize_rejects_revoked_device() {
        let pool = test_pool().await;
        let token = "b".repeat(64);
        devices::insert(&pool, "dev-2", "Pixel", "android", &hash_token(&token))
            .await
            .unwrap();
        assert!(authorize(&pool, &token).await.is_some());

        devices::revoke(&pool, "dev-2").await.unwrap();
        assert_eq!(authorize(&pool, &token).await, None);
    }

    #[test]
    fn ct_eq_basic() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
    }
}

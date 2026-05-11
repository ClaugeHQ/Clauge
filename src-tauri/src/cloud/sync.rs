// Orchestrates push/pull across all kinds. Stateless — call sites pass pool +
// AuthState; per-kind last-pushed hash bookkeeping lives in the `settings` table.

use sqlx::SqlitePool;

use crate::cloud::auth::AuthState;
use crate::cloud::client;
use crate::cloud::config::{settings_key_hash, settings_key_synced_at};
use crate::cloud::domains::{export_kind, import_kind, ALL_KINDS};
use crate::shared::repos::settings;

/// Push a single kind. Skips the request entirely if the hash matches what we
/// last pushed (true no-op, no network call).
pub async fn push_kind(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
) -> Result<bool, String> {
    let (hash, payload_b64) = export_kind(pool, kind).await?;

    let last = settings::get_by_key(pool, &settings_key_hash(kind))
        .await
        .map_err(|e| format!("read last hash: {}", e))?
        .map(|s| s.value);
    if last.as_deref() == Some(hash.as_str()) {
        return Ok(false); // no change
    }

    let resp = client::sync_push(pool, state, kind, &hash, &payload_b64)
        .await
        .map_err(String::from)?;

    settings::upsert(pool, &settings_key_hash(kind), &hash)
        .await
        .map_err(|e| format!("store hash: {}", e))?;
    settings::upsert(pool, &settings_key_synced_at(kind), &resp.updated_at)
        .await
        .map_err(|e| format!("store synced_at: {}", e))?;
    Ok(true)
}

/// Push every kind that's currently flagged dirty (or, if `force`, all of them).
pub async fn push_all(
    pool: &SqlitePool,
    state: &AuthState,
    kinds: &[&str],
) -> Result<Vec<String>, String> {
    let mut pushed = Vec::new();
    for k in kinds {
        if push_kind(pool, state, k).await? {
            pushed.push(k.to_string());
        }
    }
    Ok(pushed)
}

/// Pull one kind from the server, decode, import. Updates the local hash to
/// match the remote so the next auto-push is a no-op.
pub async fn pull_kind(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
) -> Result<(), String> {
    let resp = client::sync_pull(pool, state, kind).await.map_err(String::from)?;
    import_kind(pool, kind, &resp.payload).await?;
    settings::upsert(pool, &settings_key_hash(kind), &resp.content_hash)
        .await
        .map_err(|e| format!("store hash: {}", e))?;
    settings::upsert(pool, &settings_key_synced_at(kind), &resp.updated_at)
        .await
        .map_err(|e| format!("store synced_at: {}", e))?;
    Ok(())
}

/// Pull every kind that has a remote blob.
pub async fn pull_all(
    pool: &SqlitePool,
    state: &AuthState,
) -> Result<Vec<String>, String> {
    let rows = client::sync_state(pool, state).await.map_err(String::from)?;
    let mut pulled = Vec::new();
    for row in rows {
        pull_kind(pool, state, &row.kind).await?;
        pulled.push(row.kind);
    }
    Ok(pulled)
}

/// True if the user has any locally-created data in the synced tables.
/// Used by the first-sign-in flow to decide whether to auto-pull or prompt.
pub async fn local_has_data(pool: &SqlitePool) -> Result<bool, String> {
    // OR'd counts across the synced tables. If any has > 0 rows → user has data.
    let row: (i64,) = sqlx::query_as(
        "SELECT \
           (SELECT COUNT(*) FROM collections) + \
           (SELECT COUNT(*) FROM sql_connections) + \
           (SELECT COUNT(*) FROM nosql_connections) + \
           (SELECT COUNT(*) FROM ssh_profiles) + \
           (SELECT COUNT(*) FROM explorer_connections) + \
           (SELECT COUNT(*) FROM agent_contexts) + \
           (SELECT COUNT(*) FROM agent_sessions WHERE origin = 'manual' OR origin IS NULL) \
           AS n",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("count local: {}", e))?;
    Ok(row.0 > 0)
}

/// Kinds reference, for callers that want to iterate.
pub fn all_kinds() -> &'static [&'static str] {
    ALL_KINDS
}

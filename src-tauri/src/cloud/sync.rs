// Orchestrates push/pull across all kinds. Stateless — call sites pass pool +
// AuthState; per-kind last-pushed hash bookkeeping lives in the `settings` table.

use sqlx::SqlitePool;

use crate::cloud::auth::AuthState;
use crate::cloud::client::{self, CloudError};
use crate::cloud::config::{settings_key_conflict, settings_key_hash, settings_key_synced_at};
use crate::cloud::domains::{export_kind, import_kind, ALL_KINDS};
use crate::shared::repos::settings;

async fn clear_conflict_flag(pool: &SqlitePool, kind: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(settings_key_conflict(kind))
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| format!("clear conflict flag: {}", e))
}

/// Outcome of a push attempt for one kind. Push paths return this instead
/// of a bare bool so the scheduler can keep going on remaining kinds when
/// one hits a 412.
pub enum PushOutcome {
    /// Hash matched the server / local — no network or no-op.
    NoChange,
    /// Pushed and accepted.
    Pushed,
    /// Server returned 412 — kind is now conflict-locked locally.
    Conflict { remote_hash: Option<String> },
}

/// Push a single kind.
///
/// `force_overwrite=true` sends `prevHash:'*'` — used by the conflict
/// resolver's "Keep my changes" path. Otherwise we send the last hash
/// we know the server had so concurrent writes from another device are
/// detected as 412.
pub async fn push_kind(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
    force_overwrite: bool,
) -> Result<PushOutcome, String> {
    let (hash, payload_b64) = export_kind(pool, kind).await?;

    // Worker rejects payloads > 900 KB gzipped (413). Catch oversize
    // client-side with margin and surface a per-kind warning instead of
    // failing every push silently.
    let gz_bytes = payload_b64.len() * 3 / 4;
    if gz_bytes > 850_000 {
        settings::upsert(pool, &format!("cloud:too_large:{}", kind), &gz_bytes.to_string())
            .await
            .map_err(|e| format!("store too_large: {}", e))?;
        return Ok(PushOutcome::NoChange);
    }
    let _ = sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(format!("cloud:too_large:{}", kind))
        .execute(pool)
        .await;

    let last = settings::get_by_key(pool, &settings_key_hash(kind))
        .await
        .map_err(|e| format!("read last hash: {}", e))?
        .map(|s| s.value);
    if !force_overwrite && last.as_deref() == Some(hash.as_str()) {
        return Ok(PushOutcome::NoChange);
    }

    // `prev_hash` choice:
    //   - force overwrite (resolver "Keep my changes") → "*"
    //   - we have a previous hash on file               → that hash
    //   - no previous hash (first push on this device)  → None
    let prev_hash_owned: Option<String> = if force_overwrite {
        Some("*".to_string())
    } else {
        last
    };
    let prev_hash = prev_hash_owned.as_deref();

    match client::sync_push(pool, state, kind, &hash, &payload_b64, prev_hash).await {
        Ok(resp) => {
            settings::upsert(pool, &settings_key_hash(kind), &hash)
                .await
                .map_err(|e| format!("store hash: {}", e))?;
            settings::upsert(pool, &settings_key_synced_at(kind), &resp.updated_at)
                .await
                .map_err(|e| format!("store synced_at: {}", e))?;
            // Successful push clears any pre-existing conflict flag for this kind.
            let _ = clear_conflict_flag(pool, kind).await;
            Ok(PushOutcome::Pushed)
        }
        Err(CloudError::Conflict { current_hash, current_updated_at }) => {
            // Lost-ack case: the server already has EXACTLY the content we
            // just sent (a previous push committed but its response was
            // lost). Not a conflict — record success and move on.
            if current_hash.as_deref() == Some(hash.as_str()) {
                settings::upsert(pool, &settings_key_hash(kind), &hash)
                    .await
                    .map_err(|e| format!("store hash: {}", e))?;
                if let Some(ts) = &current_updated_at {
                    let _ = settings::upsert(pool, &settings_key_synced_at(kind), ts).await;
                }
                let _ = clear_conflict_flag(pool, kind).await;
                return Ok(PushOutcome::Pushed);
            }
            // Park this kind in conflict-locked state. The flag's value is the
            // remote hash, so the resolver can fetch & summarise the right blob.
            let marker = current_hash.clone().unwrap_or_else(|| "unknown".to_string());
            settings::upsert(pool, &settings_key_conflict(kind), &marker)
                .await
                .map_err(|e| format!("store conflict marker: {}", e))?;
            Ok(PushOutcome::Conflict { remote_hash: current_hash })
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Push every kind in `kinds`. Returns the list of kinds that actually
/// pushed bytes (excludes no-ops and conflicts). Conflicts don't short-
/// circuit — sibling kinds still push.
pub async fn push_all(
    pool: &SqlitePool,
    state: &AuthState,
    kinds: &[&str],
) -> Result<Vec<String>, String> {
    let mut pushed = Vec::new();
    for k in kinds {
        // Skip kinds that are already in conflict — they must be resolved
        // before another push attempt makes sense.
        let conflicted = settings::get_by_key(pool, &settings_key_conflict(k))
            .await
            .map_err(|e| format!("read conflict flag: {}", e))?
            .is_some();
        if conflicted {
            continue;
        }
        match push_kind(pool, state, k, false).await? {
            PushOutcome::Pushed => pushed.push(k.to_string()),
            PushOutcome::NoChange | PushOutcome::Conflict { .. } => {}
        }
    }
    Ok(pushed)
}

/// Force-push a kind in conflict — used by the resolver's "Keep my changes"
/// action. Returns the new server-reported updated_at on success.
pub async fn force_push_kind(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
) -> Result<(), String> {
    match push_kind(pool, state, kind, true).await? {
        PushOutcome::Pushed => Ok(()),
        PushOutcome::NoChange => {
            // NoChange is ambiguous here: either the export genuinely matches
            // the server, or push_kind skipped an oversize payload. The latter
            // must NOT count as success — resolve_merge would clear the
            // conflict flag while the remote still differs.
            if let Some(row) = settings::get_by_key(pool, &format!("cloud:too_large:{}", kind))
                .await
                .map_err(|e| format!("read too_large flag: {}", e))?
            {
                return Err(format!(
                    "'{}' is too large to sync ({} bytes gzipped)",
                    kind, row.value
                ));
            }
            Ok(())
        }
        PushOutcome::Conflict { .. } => {
            // 412 even with prev='*' shouldn't happen, but surface a clear error.
            Err("server refused force-push".to_string())
        }
    }
}

/// Resolve a conflict by adopting the remote — pulls the remote blob and
/// imports it locally. pull_kind clears the conflict flag itself.
pub async fn resolve_use_remote(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
) -> Result<(), String> {
    crate::cloud::snapshots::snapshot_kind(pool, kind, "pre-conflict").await?;
    pull_kind(pool, state, kind).await
}

/// Merge-resolve one kind: snapshot → pull remote blob → UPSERT-union into
/// local → force-push the union → clear conflict flag.
pub async fn resolve_merge(
    pool: &SqlitePool,
    state: &AuthState,
    kind: &str,
) -> Result<(), String> {
    crate::cloud::snapshots::snapshot_kind(pool, kind, "pre-merge").await?;
    let resp = client::sync_pull(pool, state, kind).await.map_err(String::from)?;
    crate::cloud::domains::merge_kind(pool, kind, &resp.payload).await?;
    force_push_kind(pool, state, kind).await?;
    let _ = clear_conflict_flag(pool, kind).await;
    Ok(())
}

/// Safe auto-pull triggered on app focus.
///
/// For each kind the server has, compare the remote hash to our last-known
/// remote hash. If they differ AND local hasn't diverged (current export
/// hash == last-known remote hash → no unpushed changes), pull the new
/// remote. Kinds with local divergence are skipped — those need to go
/// through the normal push path, where 412 will surface as a conflict
/// the user resolves explicitly.
pub async fn pull_if_remote_newer(
    pool: &SqlitePool,
    state: &AuthState,
) -> Result<Vec<String>, String> {
    let remote_rows = client::sync_state(pool, state).await.map_err(String::from)?;
    let mut pulled = Vec::new();
    for row in remote_rows {
        // Skip kinds this client no longer understands (retired domains whose
        // blobs still linger on the server).
        if !is_known_kind(&row.kind) {
            log::debug!("[cloud:sync] skipping unknown kind on auto-pull: {}", row.kind);
            continue;
        }
        // Local last-known remote hash for this kind.
        let last_known = settings::get_by_key(pool, &settings_key_hash(&row.kind))
            .await
            .map_err(|e| format!("read last hash: {}", e))?
            .map(|s| s.value);

        // Skip if server hash equals what we last saw — nothing new.
        if last_known.as_deref() == Some(row.content_hash.as_str()) {
            continue;
        }

        // Skip if local has diverged — the push path is responsible for that
        // case, and will surface it as a conflict if needed.
        let (local_hash, _) = export_kind(pool, &row.kind).await?;
        if Some(local_hash.as_str()) != last_known.as_deref() {
            continue;
        }

        pull_kind(pool, state, &row.kind).await?;
        pulled.push(row.kind);
    }
    Ok(pulled)
}

/// List kinds currently in conflict-locked state.
pub async fn conflicted_kinds(pool: &SqlitePool) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for k in ALL_KINDS {
        let row = settings::get_by_key(pool, &settings_key_conflict(k))
            .await
            .map_err(|e| format!("read conflict flag: {}", e))?;
        if row.is_some() {
            out.push((*k).to_string());
        }
    }
    Ok(out)
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
    // Local now equals remote — any prior divergence is resolved.
    let _ = clear_conflict_flag(pool, kind).await;
    Ok(())
}

/// Pull every kind that has a remote blob. FK-aware order — kinds that
/// other kinds reference must apply first, or SQLite raises a foreign
/// key constraint failure on insert (e.g. an `explorer_connections` row
/// with `ssh_profile_id` set, applied before the matching `ssh_profiles`
/// row exists locally → SQLITE_CONSTRAINT_FOREIGNKEY).
pub async fn pull_all(
    pool: &SqlitePool,
    state: &AuthState,
) -> Result<Vec<String>, String> {
    let rows = client::sync_state(pool, state).await.map_err(String::from)?;
    let mut kinds: Vec<String> = rows
        .into_iter()
        .map(|r| r.kind)
        .filter(|k| {
            let known = is_known_kind(k);
            if !known {
                log::debug!("[cloud:sync] skipping unknown kind on restore: {}", k);
            }
            known
        })
        .collect();
    kinds.sort_by_key(|k| pull_order_rank(k));
    let mut pulled = Vec::new();
    for kind in kinds {
        // Restore is destructive and user-initiated — snapshot before pulling.
        crate::cloud::snapshots::snapshot_kind(pool, &kind, "pre-restore").await?;
        pull_kind(pool, state, &kind).await?;
        pulled.push(kind);
    }
    Ok(pulled)
}

/// Lower rank = applied earlier. Parents (referenced by FKs in other
/// kinds) get a smaller number. Anything not listed sorts to the end —
/// add new kinds here only when they're FK targets. Current ordering:
/// ssh (0) → coworkers (5) → everything else, including the workspace
/// kinds whose cards/comments reference workspace_coworkers (10).
pub fn pull_order_rank(kind: &str) -> u8 {
    match kind {
        // ssh_profiles is referenced by sql_connections, nosql_connections,
        // and explorer_connections, so it must restore first.
        k if k == crate::cloud::domains::ssh::KIND => 0,
        // workspace_coworkers is referenced by workspace_board_cards
        // (*_by_coworker_id) and workspace_card_comments (coworker_id),
        // so it must restore before the workspace kinds.
        k if k == crate::cloud::domains::coworkers::KIND => 5,
        _ => 10,
    }
}

/// True if the user has any locally-created data in the synced tables.
/// Used by the first-sign-in flow to decide whether to auto-pull or prompt.
///
/// IMPORTANT: this function must list EVERY table that the sync domains
/// export. Whenever a new sync domain (or new table within an existing
/// domain) is added, add a corresponding `(SELECT COUNT(*) FROM <table>)`
/// term here, otherwise the first-sync empty-check will mis-classify
/// devices that only have data in the new table.
pub async fn local_has_data(pool: &SqlitePool) -> Result<bool, String> {
    // Summed counts across all synced tables. If any has > 0 rows → user has data.
    let row: (i64,) = sqlx::query_as(
        "SELECT \
           (SELECT COUNT(*) FROM collections) + \
           (SELECT COUNT(*) FROM sql_connections) + \
           (SELECT COUNT(*) FROM nosql_connections) + \
           (SELECT COUNT(*) FROM ssh_profiles) + \
           (SELECT COUNT(*) FROM explorer_connections) + \
           (SELECT COUNT(*) FROM agent_contexts) + \
           (SELECT COUNT(*) FROM agent_sessions WHERE origin = 'manual' OR origin IS NULL) + \
           (SELECT COUNT(*) FROM environments) + \
           (SELECT COUNT(*) FROM sql_scripts) + \
           (SELECT COUNT(*) FROM workspace_notes) \
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

/// True if `kind` is a sync kind this client understands. The server may hold
/// blobs for kinds we've since retired (e.g. workspace_boards); those must be
/// skipped on pull so import_kind doesn't error with "unknown kind".
fn is_known_kind(kind: &str) -> bool {
    ALL_KINDS.contains(&kind)
}

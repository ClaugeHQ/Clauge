use sqlx::SqlitePool;

use crate::cloud::domains::util::{empty_payload, encode, insert_row, select_rows_as_json, SyncPayload};

pub const KIND: &str = "agent";

pub async fn build_payload(pool: &SqlitePool) -> Result<SyncPayload, String> {
    let mut payload = empty_payload(KIND);
    payload.tables.insert(
        "agent_contexts".into(),
        select_rows_as_json(pool, "SELECT id, name, content, created_at, updated_at FROM agent_contexts ORDER BY name").await?,
    );
    // Excludes claude_session_id, project_path/name, worktree_*, last_used_at, origin, card_id, coworker_id —
    // all machine-local. Only the user-defined session metadata travels.
    payload.tables.insert(
        "agent_sessions".into(),
        select_rows_as_json(
            pool,
            "SELECT id, title, purpose, context_prompt, skip_permissions, git_name, git_email, created_at FROM agent_sessions WHERE origin = 'manual' OR origin IS NULL ORDER BY created_at, id",
        )
        .await?,
    );
    Ok(payload)
}

pub async fn export(pool: &SqlitePool) -> Result<(String, String), String> {
    encode(&build_payload(pool).await?)
}

pub async fn import(pool: &SqlitePool, payload: &SyncPayload) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| format!("begin: {}", e))?;

    // Clear contexts entirely; sessions are merged by id (we don't delete to
    // avoid wiping in-progress sessions tied to claude_session_id/worktree).
    sqlx::query("DELETE FROM agent_contexts").execute(&mut *tx).await.map_err(|e| format!("clear contexts: {}", e))?;

    if let Some(rows) = payload.tables.get("agent_contexts") {
        for r in rows {
            insert_row(&mut tx, "agent_contexts", &[
                "id","name","content","created_at","updated_at",
            ], r).await?;
        }
    }

    // Sessions: insert-or-update by id; we don't touch machine-local cols.
    if let Some(rows) = payload.tables.get("agent_sessions") {
        for r in rows {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM agent_sessions WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| format!("probe: {}", e))?;
            if exists.is_some() {
                sqlx::query(
                    "UPDATE agent_sessions SET title=?, purpose=?, context_prompt=?, skip_permissions=?, git_name=?, git_email=? WHERE id=?",
                )
                .bind(r.get("title").and_then(|v| v.as_str()))
                .bind(r.get("purpose").and_then(|v| v.as_str()))
                .bind(r.get("context_prompt").and_then(|v| v.as_str()))
                .bind(r.get("skip_permissions").and_then(|v| v.as_i64()).unwrap_or(0))
                .bind(r.get("git_name").and_then(|v| v.as_str()))
                .bind(r.get("git_email").and_then(|v| v.as_str()))
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("update agent_sessions: {}", e))?;
            } else {
                // Insert with project_path/project_name placeholders — user will
                // re-attach a project on this machine when they open the session.
                sqlx::query(
                    "INSERT INTO agent_sessions (id, title, purpose, project_path, project_name, context_prompt, skip_permissions, git_name, git_email, created_at, last_used_at, origin) \
                     VALUES (?, ?, ?, '', '', ?, ?, ?, ?, ?, ?, 'manual')",
                )
                .bind(id)
                .bind(r.get("title").and_then(|v| v.as_str()).unwrap_or(""))
                .bind(r.get("purpose").and_then(|v| v.as_str()).unwrap_or(""))
                .bind(r.get("context_prompt").and_then(|v| v.as_str()).unwrap_or(""))
                .bind(r.get("skip_permissions").and_then(|v| v.as_i64()).unwrap_or(0))
                .bind(r.get("git_name").and_then(|v| v.as_str()))
                .bind(r.get("git_email").and_then(|v| v.as_str()))
                .bind(r.get("created_at").and_then(|v| v.as_str()).unwrap_or(""))
                .bind(r.get("created_at").and_then(|v| v.as_str()).unwrap_or(""))
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("insert agent_sessions: {}", e))?;
            }
        }
    }

    tx.commit().await.map_err(|e| format!("commit: {}", e))?;
    Ok(())
}

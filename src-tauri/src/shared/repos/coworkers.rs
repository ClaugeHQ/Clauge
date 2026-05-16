// Coworkers (personas) — global to the user, not workspace-scoped.
// Each row drives both the @<name> chat experience in the drawer and
// the persona system_prompt appended to every agent run for that
// coworker.

use sqlx::SqlitePool;

use crate::modes::workspace::models::WorkspaceCoworker;

pub async fn list_coworkers(
    pool: &SqlitePool,
) -> Result<Vec<WorkspaceCoworker>, sqlx::Error> {
    sqlx::query_as::<_, WorkspaceCoworker>(
        "SELECT * FROM workspace_coworkers ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_coworker(
    pool: &SqlitePool,
    id: &str,
) -> Result<WorkspaceCoworker, sqlx::Error> {
    sqlx::query_as::<_, WorkspaceCoworker>(
        "SELECT * FROM workspace_coworkers WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_coworker(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    role: &str,
    system_prompt: &str,
    provider: &str,
    avatar_seed: &str,
    avatar_style: &str,
    created_at: &str,
    created_by: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO workspace_coworkers \
         (id, name, role, system_prompt, provider, avatar_seed, avatar_style, \
          created_at, created_by) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(role)
    .bind(system_prompt)
    .bind(provider)
    .bind(avatar_seed)
    .bind(avatar_style)
    .bind(created_at)
    .bind(created_by)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_coworker(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    role: &str,
    system_prompt: &str,
    provider: &str,
    avatar_seed: &str,
    avatar_style: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE workspace_coworkers \
         SET name = ?, role = ?, system_prompt = ?, provider = ?, \
             avatar_seed = ?, avatar_style = ? \
         WHERE id = ?",
    )
    .bind(name)
    .bind(role)
    .bind(system_prompt)
    .bind(provider)
    .bind(avatar_seed)
    .bind(avatar_style)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_coworker(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM workspace_coworkers WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Cards where this coworker is currently engaged AND the card is not
/// in a Done-class column. Used to gate `workspace_coworker_delete` —
/// we refuse the delete with a meaningful list when this returns
/// non-empty so the user knows where to look.
pub async fn list_active_cards_for_coworker(
    pool: &SqlitePool,
    coworker_id: &str,
) -> Result<Vec<(String, String, String)>, sqlx::Error> {
    // Returns (card_id, title, column_name). "Active" = the coworker
    // either currently HOLDS the claim, OR has authored at least one
    // comment, AND the card's column name doesn't match a Done-style
    // string. Done detection is case-insensitive substring match
    // against {"done", "complete", "closed", "shipped", "archived"}.
    sqlx::query_as::<_, (String, String, String)>(
        "SELECT DISTINCT c.id, c.title, col.name \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         WHERE LOWER(col.name) NOT LIKE '%done%' \
           AND LOWER(col.name) NOT LIKE '%complete%' \
           AND LOWER(col.name) NOT LIKE '%closed%' \
           AND LOWER(col.name) NOT LIKE '%shipped%' \
           AND LOWER(col.name) NOT LIKE '%archived%' \
           AND ( \
                c.claimed_coworker_id = ? \
             OR c.id IN (SELECT card_id FROM workspace_card_comments WHERE coworker_id = ?) \
           )",
    )
    .bind(coworker_id)
    .bind(coworker_id)
    .fetch_all(pool)
    .await
}

/// Stamp `disabled_at` on all coworkers ordered by created_at ASC beyond
/// the first `keep_count` — used when a Pro subscription lapses, freezing
/// extra coworkers without deleting them. Returns the number stamped.
pub async fn disable_beyond_first_n(
    pool: &SqlitePool,
    keep_count: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE workspace_coworkers
            SET disabled_at = datetime('now')
          WHERE disabled_at IS NULL
            AND id NOT IN (
                SELECT id FROM workspace_coworkers
                ORDER BY created_at ASC LIMIT ?
            )",
    )
    .bind(keep_count)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Clear `disabled_at` on every coworker — used when Pro is restored.
pub async fn enable_all(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE workspace_coworkers SET disabled_at = NULL WHERE disabled_at IS NOT NULL",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Count coworkers that are currently active (not disabled).
pub async fn count_active(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM workspace_coworkers WHERE disabled_at IS NULL",
    )
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

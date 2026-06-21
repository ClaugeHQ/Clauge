use sqlx::SqlitePool;

use crate::modes::workspace::models::{
    Workspace, WorkspaceBoard, WorkspaceBoardCard, WorkspaceBoardColumn,
    WorkspaceCardComment, WorkspaceNote,
};

/// Default board column shape applied to every newly-created board.
/// "In Progress" is the active-work column where agents normally chat;
/// "In Review" is the safety gate the user clears to Done. Single source
/// of truth — both `commands::workspace_create` (UI path) and
/// `mcp::upsert_workspace_for_project` (agent path) seed boards from
/// here so the two paths can never drift.
pub const DEFAULT_BOARD_COLUMNS: &[(&str, &str)] = &[
    ("Backlog", "#5b6776"),
    ("Todo", "#6aa9ff"),
    ("In Progress", "#f4c150"),
    ("In Review", "#a78bfa"),
    ("Done", "#2ee08a"),
];

/// Preconditions a mutation must satisfy before the row is touched.
/// Caller-supplied so UI calls can blast through (default), while MCP
/// calls can opt into freeze enforcement and optimistic concurrency.
/// All mutating helpers that accept a guard return `rows_affected`;
/// `0` means a precondition wasn't met (call `diagnose_card_failure`
/// or `diagnose_note_failure` for the reason).
#[derive(Default, Clone, Copy)]
pub struct MutationGuard<'a> {
    /// When `Some`, the row's `updated_at` must match exactly. Drives
    /// optimistic concurrency: agents pass the value they just read,
    /// and a concurrent write makes the precondition fail.
    pub expected_updated_at: Option<&'a str>,
    /// When `true`, the row's `frozen` column must be `0`. Lets the
    /// MCP layer enforce the freeze atomically with the mutation
    /// instead of via a TOCTOU pre-check.
    pub respect_frozen: bool,
}

/// Why a mutation with `rows_affected == 0` failed. Returned by the
/// diagnostic helpers so callers can produce a precise error message
/// without a second query each.
pub enum MutationFailureReason {
    NotFound,
    Frozen,
    Conflict { current_updated_at: String },
    /// Lost the race between UPDATE and diagnosis (rare). Caller can
    /// retry or treat as `NotFound`.
    Unknown,
}

/// Inspect a card and decide why a guarded mutation returned 0 rows.
pub async fn diagnose_card_failure(
    pool: &SqlitePool,
    card_id: &str,
    guard: MutationGuard<'_>,
) -> Result<MutationFailureReason, sqlx::Error> {
    let row: Option<(i64, String)> = sqlx::query_as(
        "SELECT frozen, updated_at FROM workspace_board_cards WHERE id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        None => MutationFailureReason::NotFound,
        Some((frozen, _)) if guard.respect_frozen && frozen != 0 => MutationFailureReason::Frozen,
        Some((_, updated_at)) => match guard.expected_updated_at {
            Some(expected) if expected != updated_at => {
                MutationFailureReason::Conflict { current_updated_at: updated_at }
            }
            _ => MutationFailureReason::Unknown,
        },
    })
}

/// Inspect a note and decide why a guarded mutation returned 0 rows.
pub async fn diagnose_note_failure(
    pool: &SqlitePool,
    note_id: &str,
    guard: MutationGuard<'_>,
) -> Result<MutationFailureReason, sqlx::Error> {
    let row: Option<(i64, String)> = sqlx::query_as(
        "SELECT frozen, updated_at FROM workspace_notes WHERE id = ?",
    )
    .bind(note_id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        None => MutationFailureReason::NotFound,
        Some((frozen, _)) if guard.respect_frozen && frozen != 0 => MutationFailureReason::Frozen,
        Some((_, updated_at)) => match guard.expected_updated_at {
            Some(expected) if expected != updated_at => {
                MutationFailureReason::Conflict { current_updated_at: updated_at }
            }
            _ => MutationFailureReason::Unknown,
        },
    })
}

/// Compose the optional precondition tail for a guarded UPDATE on a
/// `workspace_board_cards` / `workspace_notes` row. Caller binds
/// `expected_updated_at` after the row id when it's `Some`.
fn guard_clause(guard: MutationGuard<'_>) -> String {
    let mut s = String::new();
    if guard.respect_frozen {
        s.push_str(" AND frozen = 0");
    }
    if guard.expected_updated_at.is_some() {
        s.push_str(" AND updated_at = ?");
    }
    s
}

/// Optional pagination tail for list queries. UI callers pass
/// `Pagination::default()` (no limit) and get the full result set;
/// MCP callers pass the agent-supplied `limit` + `offset` clamped
/// to a sane max so the agent's context window can't be flooded by
/// a list-everything call on a busy workspace.
#[derive(Default, Clone, Copy)]
pub struct Pagination {
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// Compose the optional `LIMIT ? OFFSET ?` tail. Caller binds the
/// values after the WHERE-clause params when they're `Some`.
fn pagination_clause(p: Pagination) -> String {
    let mut s = String::new();
    if p.limit.is_some() {
        s.push_str(" LIMIT ?");
        if p.offset.is_some() {
            s.push_str(" OFFSET ?");
        }
    }
    s
}

// ---------------------------------------------------------------------------
// workspaces
// ---------------------------------------------------------------------------

pub async fn list_workspaces(
    pool: &SqlitePool,
    page: Pagination,
) -> Result<Vec<Workspace>, sqlx::Error> {
    let sql = format!(
        "SELECT * FROM workspaces ORDER BY updated_at DESC{}",
        pagination_clause(page)
    );
    let mut q = sqlx::query_as::<_, Workspace>(&sql);
    if let Some(l) = page.limit {
        q = q.bind(l);
        if let Some(o) = page.offset {
            q = q.bind(o);
        }
    }
    q.fetch_all(pool).await
}

pub async fn get_workspace_by_id(pool: &SqlitePool, id: &str) -> Result<Workspace, sqlx::Error> {
    sqlx::query_as::<_, Workspace>("SELECT * FROM workspaces WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Find a workspace whose `project_path` matches exactly. Returns
/// None if there isn't one. Used by the MCP convenience tools so an
/// agent can resolve "current project" → workspace in one call.
pub async fn find_workspace_by_project_path(
    pool: &SqlitePool,
    project_path: &str,
) -> Result<Option<Workspace>, sqlx::Error> {
    sqlx::query_as::<_, Workspace>(
        "SELECT * FROM workspaces WHERE project_path = ? LIMIT 1",
    )
    .bind(project_path)
    .fetch_optional(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_workspace(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    project_path: Option<&str>,
    project_name: Option<&str>,
    color: Option<&str>,
    actor: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO workspaces \
         (id, name, project_path, project_name, color, \
          created_at, created_by, updated_at, updated_by) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(project_path)
    .bind(project_name)
    .bind(color)
    .bind(now)
    .bind(actor)
    .bind(now)
    .bind(actor)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_workspace(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    project_path: Option<&str>,
    project_name: Option<&str>,
    color: Option<&str>,
    actor: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE workspaces \
         SET name = ?, project_path = ?, project_name = ?, color = ?, \
             updated_at = ?, updated_by = ? \
         WHERE id = ?",
    )
    .bind(name)
    .bind(project_path)
    .bind(project_name)
    .bind(color)
    .bind(now)
    .bind(actor)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_workspace(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM workspaces WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// notes
// ---------------------------------------------------------------------------

pub async fn list_notes_in_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
    page: Pagination,
) -> Result<Vec<WorkspaceNote>, sqlx::Error> {
    let sql = format!(
        "SELECT * FROM workspace_notes WHERE workspace_id = ? ORDER BY updated_at DESC{}",
        pagination_clause(page)
    );
    let mut q = sqlx::query_as::<_, WorkspaceNote>(&sql).bind(workspace_id);
    if let Some(l) = page.limit {
        q = q.bind(l);
        if let Some(o) = page.offset {
            q = q.bind(o);
        }
    }
    q.fetch_all(pool).await
}

pub async fn get_note_by_id(pool: &SqlitePool, id: &str) -> Result<WorkspaceNote, sqlx::Error> {
    sqlx::query_as::<_, WorkspaceNote>("SELECT * FROM workspace_notes WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Find a note by exact (case-insensitive) title within a single
/// workspace. Used by `notes_upsert_for_project` — lets the agent
/// evolve a single doc ("Overview", "Architecture") across calls
/// instead of stacking duplicates.
pub async fn find_note_by_title_in_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
    title: &str,
) -> Result<Option<WorkspaceNote>, sqlx::Error> {
    sqlx::query_as::<_, WorkspaceNote>(
        "SELECT * FROM workspace_notes \
         WHERE workspace_id = ? AND LOWER(title) = LOWER(?) \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(workspace_id)
    .bind(title)
    .fetch_optional(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_note(
    pool: &SqlitePool,
    id: &str,
    workspace_id: &str,
    title: &str,
    content: &str,
    tags_json: &str,
    linked_session_id: Option<&str>,
    actor: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO workspace_notes \
         (id, workspace_id, title, content, tags, linked_session_id, \
          created_at, created_by, updated_at, updated_by) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(title)
    .bind(content)
    .bind(tags_json)
    .bind(linked_session_id)
    .bind(now)
    .bind(actor)
    .bind(now)
    .bind(actor)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_note(
    pool: &SqlitePool,
    id: &str,
    title: &str,
    content: &str,
    tags_json: &str,
    linked_session_id: Option<&str>,
    actor: &str,
    now: &str,
    guard: MutationGuard<'_>,
) -> Result<u64, sqlx::Error> {
    let sql = format!(
        "UPDATE workspace_notes \
         SET title = ?, content = ?, tags = ?, linked_session_id = ?, \
             updated_at = ?, updated_by = ? \
         WHERE id = ?{}",
        guard_clause(guard)
    );
    let mut q = sqlx::query(&sql)
        .bind(title)
        .bind(content)
        .bind(tags_json)
        .bind(linked_session_id)
        .bind(now)
        .bind(actor)
        .bind(id);
    if let Some(eu) = guard.expected_updated_at {
        q = q.bind(eu);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

pub async fn delete_note(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM workspace_notes WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// boards + columns
// ---------------------------------------------------------------------------

pub async fn list_boards_in_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
    page: Pagination,
) -> Result<Vec<WorkspaceBoard>, sqlx::Error> {
    let sql = format!(
        "SELECT * FROM workspace_boards WHERE workspace_id = ? ORDER BY position ASC, created_at ASC{}",
        pagination_clause(page)
    );
    let mut q = sqlx::query_as::<_, WorkspaceBoard>(&sql).bind(workspace_id);
    if let Some(l) = page.limit {
        q = q.bind(l);
        if let Some(o) = page.offset {
            q = q.bind(o);
        }
    }
    q.fetch_all(pool).await
}

pub async fn get_board_by_id(pool: &SqlitePool, id: &str) -> Result<WorkspaceBoard, sqlx::Error> {
    sqlx::query_as::<_, WorkspaceBoard>("SELECT * FROM workspace_boards WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

pub async fn insert_board(
    pool: &SqlitePool,
    id: &str,
    workspace_id: &str,
    name: &str,
    source: &str,
    source_config: Option<&str>,
    position: i32,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO workspace_boards \
         (id, workspace_id, name, source, source_config, position, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(name)
    .bind(source)
    .bind(source_config)
    .bind(position)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_board_name(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE workspace_boards SET name = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Set or clear the board's source_config JSON. Used to override the
/// parent workspace's project_path on a per-board basis.
pub async fn update_board_source_config(
    pool: &SqlitePool,
    id: &str,
    source_config: Option<&str>,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE workspace_boards SET source_config = ?, updated_at = ? WHERE id = ?",
    )
    .bind(source_config)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_board(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM workspace_boards WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_columns_in_board(
    pool: &SqlitePool,
    board_id: &str,
) -> Result<Vec<WorkspaceBoardColumn>, sqlx::Error> {
    sqlx::query_as::<_, WorkspaceBoardColumn>(
        "SELECT * FROM workspace_board_columns WHERE board_id = ? ORDER BY position ASC",
    )
    .bind(board_id)
    .fetch_all(pool)
    .await
}

pub async fn insert_column(
    pool: &SqlitePool,
    id: &str,
    board_id: &str,
    name: &str,
    color: Option<&str>,
    position: i32,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO workspace_board_columns \
         (id, board_id, name, color, position, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(board_id)
    .bind(name)
    .bind(color)
    .bind(position)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// cards
// ---------------------------------------------------------------------------

pub async fn list_cards_in_board(
    pool: &SqlitePool,
    board_id: &str,
    page: Pagination,
) -> Result<Vec<WorkspaceBoardCard>, sqlx::Error> {
    // Fetch every card whose column belongs to this board, ordered by
    // (column.position, card.position) so the frontend can group without
    // a second pass. Comment count via correlated subquery so the kanban
    // tile can render a "💬 N" chip without a second roundtrip.
    let sql = format!(
        "SELECT c.*, \
                (SELECT COUNT(*) FROM workspace_card_comments cc \
                 WHERE cc.card_id = c.id) AS comment_count \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         WHERE col.board_id = ? \
         ORDER BY col.position ASC, c.position ASC{}",
        pagination_clause(page)
    );
    let mut q = sqlx::query_as::<_, WorkspaceBoardCard>(&sql).bind(board_id);
    if let Some(l) = page.limit {
        q = q.bind(l);
        if let Some(o) = page.offset {
            q = q.bind(o);
        }
    }
    q.fetch_all(pool).await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_card(
    pool: &SqlitePool,
    id: &str,
    column_id: &str,
    title: &str,
    description: &str,
    priority: Option<&str>,
    tags_json: &str,
    position: i32,
    external_id: Option<&str>,
    external_url: Option<&str>,
    linked_session_id: Option<&str>,
    coworker_id: Option<&str>,
    actor: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    // coworker_id stamps BOTH created_by_coworker_id and
    // updated_by_coworker_id — the row is fresh, so creator == last
    // mutator at insert time.
    sqlx::query(
        "INSERT INTO workspace_board_cards \
         (id, column_id, title, description, priority, tags, position, \
          external_id, external_url, linked_session_id, \
          review_pending, review_checklist, \
          created_at, created_by, created_by_coworker_id, \
          updated_at, updated_by, updated_by_coworker_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, NULL, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(column_id)
    .bind(title)
    .bind(description)
    .bind(priority)
    .bind(tags_json)
    .bind(position)
    .bind(external_id)
    .bind(external_url)
    .bind(linked_session_id)
    .bind(now)
    .bind(actor)
    .bind(coworker_id)
    .bind(now)
    .bind(actor)
    .bind(coworker_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_card(
    pool: &SqlitePool,
    id: &str,
    title: &str,
    description: &str,
    priority: Option<&str>,
    tags_json: &str,
    review_checklist: Option<&str>,
    coworker_id: Option<&str>,
    actor: &str,
    now: &str,
    guard: MutationGuard<'_>,
) -> Result<u64, sqlx::Error> {
    let sql = format!(
        "UPDATE workspace_board_cards \
         SET title = ?, description = ?, priority = ?, tags = ?, \
             review_checklist = ?, updated_at = ?, updated_by = ?, \
             updated_by_coworker_id = ? \
         WHERE id = ?{}",
        guard_clause(guard)
    );
    let mut q = sqlx::query(&sql)
        .bind(title)
        .bind(description)
        .bind(priority)
        .bind(tags_json)
        .bind(review_checklist)
        .bind(now)
        .bind(actor)
        .bind(coworker_id)
        .bind(id);
    if let Some(eu) = guard.expected_updated_at {
        q = q.bind(eu);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

/// Move a card to a new column + position. Sets `review_pending` to 1
/// when the actor is an AI agent (not the user) — that drives the
/// "Pending review" badge on the destination column. The user can then
/// approve (clear flag) or request changes (clear flag, move back).
pub async fn move_card(
    pool: &SqlitePool,
    id: &str,
    column_id: &str,
    position: i32,
    review_pending: i32,
    actor: &str,
    now: &str,
    guard: MutationGuard<'_>,
) -> Result<u64, sqlx::Error> {
    let sql = format!(
        "UPDATE workspace_board_cards \
         SET column_id = ?, position = ?, review_pending = ?, \
             updated_at = ?, updated_by = ? \
         WHERE id = ?{}",
        guard_clause(guard)
    );
    let mut q = sqlx::query(&sql)
        .bind(column_id)
        .bind(position)
        .bind(review_pending)
        .bind(now)
        .bind(actor)
        .bind(id);
    if let Some(eu) = guard.expected_updated_at {
        q = q.bind(eu);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

pub async fn clear_review_pending(
    pool: &SqlitePool,
    id: &str,
    actor: &str,
    now: &str,
    guard: MutationGuard<'_>,
) -> Result<u64, sqlx::Error> {
    let sql = format!(
        "UPDATE workspace_board_cards \
         SET review_pending = 0, updated_at = ?, updated_by = ? \
         WHERE id = ?{}",
        guard_clause(guard)
    );
    let mut q = sqlx::query(&sql).bind(now).bind(actor).bind(id);
    if let Some(eu) = guard.expected_updated_at {
        q = q.bind(eu);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

/// Delete a card. Two side effects worth knowing about:
///   1. Sync-tombstone — if the card carries an `external_id`, we
///      record (board_id, external_id) in `workspace_dismissed_externals`
///      so the next gh/glab sync skips it. Local cards skip this.
///   2. Cascade hidden sessions — any drawer-spawned `('card'-origin)`
///      session pointing at this card is deleted alongside, so we
///      don't leave orphan rows in `agent_sessions`.
pub async fn delete_card(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    // 1. Look up external_id + board_id (via column→board) so we can
    //    tombstone before the cascade nukes the row.
    let meta: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT c.external_id, col.board_id \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         WHERE c.id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    if let Some((Some(ext), board_id)) = meta {
        let trimmed = ext.trim();
        if !trimmed.is_empty() {
            // INSERT OR IGNORE — re-deleting an already-dismissed
            // external is a no-op, not an error.
            sqlx::query(
                "INSERT OR IGNORE INTO workspace_dismissed_externals \
                 (board_id, external_id) VALUES (?, ?)",
            )
            .bind(&board_id)
            .bind(trimmed)
            .execute(pool)
            .await?;
        }
    }

    // 2. Drop hidden sessions tied to this card (drawer-spawned
    //    `'card'`-origin rows). Manual sessions are never tied to a
    //    single card so they're untouched.
    sqlx::query(
        "DELETE FROM agent_sessions WHERE card_id = ? AND origin = 'card'",
    )
    .bind(id)
    .execute(pool)
    .await?;

    // 3. Drop the card. Comments + FTS rows cascade via FK.
    sqlx::query("DELETE FROM workspace_board_cards WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// `(board_id, external_id)` pairs to skip on the next sync. Used by
/// `workspace_scan_project_issues*` to filter out tombstoned issues.
pub async fn list_dismissed_externals(
    pool: &SqlitePool,
    board_id: &str,
) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT external_id FROM workspace_dismissed_externals WHERE board_id = ?",
    )
    .bind(board_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Re-allow a previously-dismissed external on a board. Not exposed
/// to the UI yet — placeholder for a future "Restore from sync" tool.
#[allow(dead_code)]
pub async fn undismiss_external(
    pool: &SqlitePool,
    board_id: &str,
    external_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM workspace_dismissed_externals \
         WHERE board_id = ? AND external_id = ?",
    )
    .bind(board_id)
    .bind(external_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Inbox — items recently mutated by an agent. We don't keep a separate
// activity log; we query the existing tables filtered by `updated_by`
// not starting with 'user'. The Inbox view UNIONs notes, boards, and
// cards into a single chronological list. Workspace_id resolution for
// cards goes through their column → board → workspace.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub kind: String,           // 'note' | 'board' | 'card'
    pub id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub label: String,          // note title / board name / card title
    pub board_id: Option<String>,    // for cards: parent board id
    pub board_name: Option<String>,  // for cards: parent board name
    pub updated_by: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// v1.1 helpers — search, summary, freeze, link, repo URL, surgical edits.
// All back the MCP tools added 2026-05-08 (FTS5 in migration 12).
// ---------------------------------------------------------------------------

pub async fn search_notes(
    pool: &SqlitePool,
    query: &str,
    workspace_id: Option<&str>,
    limit: i32,
) -> Result<Vec<WorkspaceNote>, sqlx::Error> {
    // FTS5 matches on title + content; we then join back to the base
    // table to return the full row. `bm25(workspace_notes_fts)` ranks
    // by relevance — lower = better.
    let sql = if workspace_id.is_some() {
        "SELECT n.* FROM workspace_notes n \
         JOIN workspace_notes_fts f ON f.note_id = n.id \
         WHERE workspace_notes_fts MATCH ? AND n.workspace_id = ? \
         ORDER BY bm25(workspace_notes_fts) ASC LIMIT ?"
    } else {
        "SELECT n.* FROM workspace_notes n \
         JOIN workspace_notes_fts f ON f.note_id = n.id \
         WHERE workspace_notes_fts MATCH ? \
         ORDER BY bm25(workspace_notes_fts) ASC LIMIT ?"
    };
    let mut q = sqlx::query_as::<_, WorkspaceNote>(sql).bind(query);
    if let Some(ws) = workspace_id { q = q.bind(ws); }
    q.bind(limit).fetch_all(pool).await
}

pub async fn search_cards(
    pool: &SqlitePool,
    query: &str,
    workspace_id: Option<&str>,
    limit: i32,
) -> Result<Vec<WorkspaceBoardCard>, sqlx::Error> {
    let sql = if workspace_id.is_some() {
        "SELECT c.* FROM workspace_board_cards c \
         JOIN workspace_board_cards_fts f ON f.card_id = c.id \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         WHERE workspace_board_cards_fts MATCH ? AND b.workspace_id = ? \
         ORDER BY bm25(workspace_board_cards_fts) ASC LIMIT ?"
    } else {
        "SELECT c.* FROM workspace_board_cards c \
         JOIN workspace_board_cards_fts f ON f.card_id = c.id \
         WHERE workspace_board_cards_fts MATCH ? \
         ORDER BY bm25(workspace_board_cards_fts) ASC LIMIT ?"
    };
    let mut q = sqlx::query_as::<_, WorkspaceBoardCard>(sql).bind(query);
    if let Some(ws) = workspace_id { q = q.bind(ws); }
    q.bind(limit).fetch_all(pool).await
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ColumnCount {
    pub column_id: String,
    pub column_name: String,
    pub count: i64,
}

pub async fn count_cards_per_column(
    pool: &SqlitePool,
    board_id: &str,
) -> Result<Vec<ColumnCount>, sqlx::Error> {
    sqlx::query_as::<_, ColumnCount>(
        "SELECT col.id AS column_id, col.name AS column_name, \
                COUNT(c.id) AS count \
         FROM workspace_board_columns col \
         LEFT JOIN workspace_board_cards c ON c.column_id = col.id \
         WHERE col.board_id = ? \
         GROUP BY col.id, col.name, col.position \
         ORDER BY col.position ASC",
    )
    .bind(board_id)
    .fetch_all(pool)
    .await
}

pub async fn count_cards_in_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         WHERE b.workspace_id = ?",
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn count_notes_in_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM workspace_notes WHERE workspace_id = ?",
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn count_boards_in_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM workspace_boards WHERE workspace_id = ?",
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// One row per claimed card in the workspace whose claim points at a
/// session with a non-empty `worktree_path`. Used by the workspace
/// delete confirmation: cascade alone removes the card row and nulls
/// the FK to the session, but the worktree on disk + its git branch
/// would leak. Caller batches `agent_remove_worktree` for each.
pub async fn list_active_worktrees_in_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Vec<ActiveWorktree>, sqlx::Error> {
    sqlx::query_as::<_, ActiveWorktree>(
        "SELECT c.id AS card_id, c.title AS card_title, \
                s.id AS session_id, s.project_path, \
                s.worktree_path, s.worktree_branch \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         JOIN agent_sessions s ON s.id = c.claimed_session_id \
         WHERE b.workspace_id = ? \
           AND s.worktree_path IS NOT NULL \
           AND TRIM(s.worktree_path) != ''",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveWorktree {
    pub card_id: String,
    pub card_title: String,
    pub session_id: String,
    pub project_path: String,
    pub worktree_path: Option<String>,
    pub worktree_branch: Option<String>,
}

/// Null out the worktree pointers on an agent_session row. Called
/// after a successful `agent_remove_worktree` so the next view of
/// the session doesn't show a phantom branch / path.
pub async fn clear_session_worktree(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE agent_sessions SET worktree_path = NULL, worktree_branch = NULL WHERE id = ?",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn count_review_pending_in_board(
    pool: &SqlitePool,
    board_id: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         WHERE col.board_id = ? AND c.review_pending = 1",
    )
    .bind(board_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn list_review_pending_cards(
    pool: &SqlitePool,
    workspace_id: Option<&str>,
) -> Result<Vec<WorkspaceBoardCard>, sqlx::Error> {
    let sql = if workspace_id.is_some() {
        "SELECT c.* FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         WHERE c.review_pending = 1 AND b.workspace_id = ? \
         ORDER BY c.updated_at DESC LIMIT 100"
    } else {
        "SELECT * FROM workspace_board_cards \
         WHERE review_pending = 1 \
         ORDER BY updated_at DESC LIMIT 100"
    };
    let mut q = sqlx::query_as::<_, WorkspaceBoardCard>(sql);
    if let Some(ws) = workspace_id { q = q.bind(ws); }
    q.fetch_all(pool).await
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ActorCount {
    pub actor: String,
    pub count: i64,
}

pub async fn count_recent_edits_by_actor(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Vec<ActorCount>, sqlx::Error> {
    sqlx::query_as::<_, ActorCount>(
        "SELECT actor, SUM(c) AS count FROM ( \
            SELECT updated_by AS actor, COUNT(*) AS c \
            FROM workspace_notes WHERE workspace_id = ? GROUP BY updated_by \
            UNION ALL \
            SELECT c.updated_by AS actor, COUNT(*) AS c \
            FROM workspace_board_cards c \
            JOIN workspace_board_columns col ON col.id = c.column_id \
            JOIN workspace_boards b ON b.id = col.board_id \
            WHERE b.workspace_id = ? GROUP BY c.updated_by \
         ) GROUP BY actor ORDER BY count DESC LIMIT 20",
    )
    .bind(workspace_id)
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

pub async fn set_note_frozen(
    pool: &SqlitePool,
    id: &str,
    frozen: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE workspace_notes SET frozen = ? WHERE id = ?")
        .bind(frozen).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn set_card_frozen(
    pool: &SqlitePool,
    id: &str,
    frozen: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE workspace_board_cards SET frozen = ? WHERE id = ?")
        .bind(frozen).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn is_note_frozen(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let row: Option<(i32,)> = sqlx::query_as("SELECT frozen FROM workspace_notes WHERE id = ?")
        .bind(id).fetch_optional(pool).await?;
    Ok(matches!(row, Some((1,))))
}

pub async fn is_card_frozen(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let row: Option<(i32,)> = sqlx::query_as("SELECT frozen FROM workspace_board_cards WHERE id = ?")
        .bind(id).fetch_optional(pool).await?;
    Ok(matches!(row, Some((1,))))
}

pub async fn set_workspace_repo_url(
    pool: &SqlitePool,
    id: &str,
    repo_url: Option<&str>,
    actor: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE workspaces SET repo_url = ?, updated_at = ?, updated_by = ? WHERE id = ?",
    )
    .bind(repo_url).bind(now).bind(actor).bind(id)
    .execute(pool).await?;
    Ok(())
}

pub async fn update_note_linked_session(
    pool: &SqlitePool,
    id: &str,
    session_id: Option<&str>,
    actor: &str,
    now: &str,
    guard: MutationGuard<'_>,
) -> Result<u64, sqlx::Error> {
    let sql = format!(
        "UPDATE workspace_notes \
         SET linked_session_id = ?, updated_at = ?, updated_by = ? \
         WHERE id = ?{}",
        guard_clause(guard)
    );
    let mut q = sqlx::query(&sql).bind(session_id).bind(now).bind(actor).bind(id);
    if let Some(eu) = guard.expected_updated_at {
        q = q.bind(eu);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

pub async fn update_card_linked_session(
    pool: &SqlitePool,
    id: &str,
    session_id: Option<&str>,
    actor: &str,
    now: &str,
    guard: MutationGuard<'_>,
) -> Result<u64, sqlx::Error> {
    let sql = format!(
        "UPDATE workspace_board_cards \
         SET linked_session_id = ?, updated_at = ?, updated_by = ? \
         WHERE id = ?{}",
        guard_clause(guard)
    );
    let mut q = sqlx::query(&sql).bind(session_id).bind(now).bind(actor).bind(id);
    if let Some(eu) = guard.expected_updated_at {
        q = q.bind(eu);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

/// Stamp a PR / MR URL onto a card. Set after `cards_raise_pr`
/// successfully opens (or detects an existing) PR. Subsequent raises
/// short-circuit when this is non-null and just push more commits.
pub async fn update_card_pr_url(
    pool: &SqlitePool,
    id: &str,
    pr_url: &str,
    actor: &str,
    now: &str,
    guard: MutationGuard<'_>,
) -> Result<u64, sqlx::Error> {
    let sql = format!(
        "UPDATE workspace_board_cards \
         SET pr_url = ?, updated_at = ?, updated_by = ? \
         WHERE id = ?{}",
        guard_clause(guard)
    );
    let mut q = sqlx::query(&sql).bind(pr_url).bind(now).bind(actor).bind(id);
    if let Some(eu) = guard.expected_updated_at {
        q = q.bind(eu);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

// ---------------------------------------------------------------------------
// Card comments — migration 13. Replaces the markdown-blockquote-in-
// description pattern.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
pub async fn insert_card_comment(
    pool: &SqlitePool,
    id: &str,
    card_id: &str,
    actor: &str,
    coworker_id: Option<&str>,
    body: &str,
    parent_id: Option<&str>,
    now: &str,
    channel: &str,
    external_id: Option<&str>,
    external_author: Option<&str>,
    guard: MutationGuard<'_>,
) -> Result<u64, sqlx::Error> {
    // INSERT … SELECT … WHERE EXISTS lets us refuse atomically when
    // the parent card is frozen (no separate TOCTOU pre-check). When
    // the guard's freeze bit is off, the EXISTS clause matches any
    // card row.
    let exists_clause = if guard.respect_frozen {
        "AND frozen = 0"
    } else {
        ""
    };
    let sql = format!(
        "INSERT INTO workspace_card_comments \
         (id, card_id, actor, coworker_id, body, parent_id, created_at, channel, external_id, external_author) \
         SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ? \
         WHERE EXISTS (SELECT 1 FROM workspace_board_cards WHERE id = ? {})",
        exists_clause
    );
    let inserted = sqlx::query(&sql)
        .bind(id)
        .bind(card_id)
        .bind(actor)
        .bind(coworker_id)
        .bind(body)
        .bind(parent_id)
        .bind(now)
        .bind(channel)
        .bind(external_id)
        .bind(external_author)
        .bind(card_id)
        .execute(pool)
        .await?
        .rows_affected();
    if inserted == 0 {
        return Ok(0);
    }
    // Mirror the card's last-touch metadata so the inbox + per-card
    // unread tracking pick up comment activity without needing a
    // separate query path. We just verified the card exists and (when
    // guarded) is not frozen, so a second freeze guard here would be
    // redundant.
    sqlx::query(
        "UPDATE workspace_board_cards \
         SET updated_at = ?, updated_by = ? \
         WHERE id = ?",
    )
    .bind(now)
    .bind(actor)
    .bind(card_id)
    .execute(pool)
    .await?;
    Ok(inserted)
}

/// Insert (or refresh) a comment fetched from GitHub/GitLab, keyed by
/// `external_id` so repeated fetches never duplicate. Returns true when a
/// new row was inserted. `created_at` is the ORIGINAL provider timestamp.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_external_comment(
    pool: &SqlitePool,
    id: &str,
    card_id: &str,
    actor: &str,
    body: &str,
    created_at: &str,
    external_id: &str,
    external_author: &str,
) -> Result<bool, sqlx::Error> {
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM workspace_card_comments WHERE external_id = ? LIMIT 1")
            .bind(external_id)
            .fetch_optional(pool)
            .await?;
    if existing.is_some() {
        // Body may have been edited upstream — keep our copy in sync.
        sqlx::query(
            "UPDATE workspace_card_comments \
             SET body = ?, created_at = ?, external_author = ?, actor = ? \
             WHERE external_id = ?",
        )
        .bind(body)
        .bind(created_at)
        .bind(external_author)
        .bind(actor)
        .bind(external_id)
        .execute(pool)
        .await?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO workspace_card_comments \
         (id, card_id, actor, coworker_id, body, parent_id, created_at, channel, external_id, external_author) \
         VALUES (?, ?, ?, NULL, ?, NULL, ?, 'ticket', ?, ?)",
    )
    .bind(id)
    .bind(card_id)
    .bind(actor)
    .bind(body)
    .bind(created_at)
    .bind(external_id)
    .bind(external_author)
    .execute(pool)
    .await?;
    Ok(true)
}

/// Update a comment's body (local edit / mirror of a remote edit).
pub async fn update_comment_body(
    pool: &SqlitePool,
    id: &str,
    body: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE workspace_card_comments SET body = ?, created_at = created_at WHERE id = ?")
        .bind(body)
        .bind(id)
        .execute(pool)
        .await?;
    let _ = now; // created_at preserved; updated tracking lives on the card
    Ok(())
}

/// Remove synced (external) ticket comments on a card whose provider id is
/// no longer in `keep` — i.e. they were deleted upstream. Local comments
/// (external_id NULL) and coworker comments are never touched.
pub async fn prune_external_comments(
    pool: &SqlitePool,
    card_id: &str,
    keep: &[String],
) -> Result<u64, sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, external_id FROM workspace_card_comments \
         WHERE card_id = ? AND channel = 'ticket' AND external_id IS NOT NULL",
    )
    .bind(card_id)
    .fetch_all(pool)
    .await?;
    let mut removed = 0u64;
    for (id, ext) in rows {
        if !keep.iter().any(|k| k == &ext) {
            sqlx::query("DELETE FROM workspace_card_comments WHERE id = ?")
                .bind(&id)
                .execute(pool)
                .await?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub async fn list_card_comments(
    pool: &SqlitePool,
    card_id: &str,
    channel: Option<&str>,
) -> Result<Vec<WorkspaceCardComment>, sqlx::Error> {
    let sql = if channel.is_some() {
        "SELECT * FROM workspace_card_comments \
         WHERE card_id = ? AND channel = ? ORDER BY created_at ASC"
    } else {
        "SELECT * FROM workspace_card_comments \
         WHERE card_id = ? ORDER BY created_at ASC"
    };
    let mut q = sqlx::query_as::<_, WorkspaceCardComment>(sql).bind(card_id);
    if let Some(ch) = channel {
        q = q.bind(ch);
    }
    q.fetch_all(pool)
    .await
}

pub async fn delete_card_comment(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM workspace_card_comments WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Card claims (migration 14). One session at a time can drive an agent
// on a given card. Other surfaces still see the card and can post plain
// comments, but `cards_drawer_chat` refuses unless the caller's session
// matches the claim.
// ---------------------------------------------------------------------------

/// Claim a card: set both the active session and the coworker (persona)
/// owning the conversation. Pass `coworker_id = None` for terminal-side
/// claims that don't have a persona today.
/// Atomically claim an unclaimed, non-frozen card for `session_id`.
/// Returns `true` when the claim was set. `false` means the card is
/// already claimed by someone (or doesn't exist / is frozen) — caller
/// should re-read the row and decide whether to error or no-op
/// (e.g. "same session re-claiming" is idempotent).
pub async fn claim_card(
    pool: &SqlitePool,
    card_id: &str,
    session_id: &str,
    coworker_id: Option<&str>,
    actor: &str,
    now: &str,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query(
        "UPDATE workspace_board_cards \
         SET claimed_session_id = ?, claimed_coworker_id = ?, \
             updated_at = ?, updated_by = ? \
         WHERE id = ? AND claimed_session_id IS NULL AND frozen = 0",
    )
    .bind(session_id)
    .bind(coworker_id)
    .bind(now)
    .bind(actor)
    .bind(card_id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() > 0)
}

pub async fn release_card(
    pool: &SqlitePool,
    card_id: &str,
    actor: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE workspace_board_cards \
         SET claimed_session_id = NULL, claimed_coworker_id = NULL, \
             updated_at = ?, updated_by = ? \
         WHERE id = ?",
    )
    .bind(now)
    .bind(actor)
    .bind(card_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// `(claimed_session_id, claimed_coworker_id, project_path)` for a card.
/// Used by drawer chat and `cards_claim` to enforce/establish locks.
pub async fn get_card_claim_and_project(
    pool: &SqlitePool,
    card_id: &str,
) -> Result<Option<(Option<String>, Option<String>, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.claimed_session_id, c.claimed_coworker_id, w.project_path \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         JOIN workspaces w ON w.id = b.workspace_id \
         WHERE c.id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_inbox(
    pool: &SqlitePool,
    limit: i32,
) -> Result<Vec<InboxItem>, sqlx::Error> {
    sqlx::query_as::<_, InboxItem>(
        "SELECT 'note' AS kind, n.id AS id, n.workspace_id AS workspace_id,
                w.name AS workspace_name, n.title AS label,
                NULL AS board_id, NULL AS board_name,
                n.updated_by AS updated_by, n.updated_at AS updated_at
         FROM workspace_notes n
         JOIN workspaces w ON w.id = n.workspace_id
         WHERE n.updated_by NOT LIKE 'user%'
         UNION ALL
         SELECT 'card' AS kind, c.id AS id, w.id AS workspace_id,
                w.name AS workspace_name, c.title AS label,
                b.id AS board_id, b.name AS board_name,
                c.updated_by AS updated_by, c.updated_at AS updated_at
         FROM workspace_board_cards c
         JOIN workspace_board_columns col ON col.id = c.column_id
         JOIN workspace_boards b ON b.id = col.board_id
         JOIN workspaces w ON w.id = b.workspace_id
         WHERE c.updated_by NOT LIKE 'user%'
         ORDER BY updated_at DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

use sqlx::SqlitePool;
use tauri::State;

use crate::modes::workspace::models::{
    ProjectIssue, ProjectScanResult, Workspace, WorkspaceBoard, WorkspaceBoardCard,
    WorkspaceBoardColumn, WorkspaceNote,
};
use crate::shared::repos::settings as settings_repo;
use crate::shared::repos::workspaces as repo;

fn project_name_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

fn is_git_repo(path: &str) -> bool {
    std::path::Path::new(path).join(".git").exists()
}

/// True for the canonical "In Review" safety-gate column (NOT "In Progress",
/// which is the active-work column). Columns are seeded from
/// `repo::DEFAULT_BOARD_COLUMNS` and can't be renamed by the user, so
/// exact equality is enough. Used to decide if an agent move into a
/// column should auto-flag `review_pending=1`.
pub fn is_review_only_column(name: &str) -> bool {
    name == "In Review"
}

/// Discovered subproject — used when the workspace folder isn't a git
/// repo itself but contains git repos somewhere inside. The caller
/// creates one board per entry. Scan walks down to MAX_SCAN_DEPTH
/// levels and records the FIRST `.git` it hits on each branch (does
/// not recurse into found repos, so submodules / nested worktrees
/// don't show up as separate boards).
struct SubProject {
    /// Path-relative name (e.g. "compliance/compliance-auth") used as
    /// the board name. Forward slashes regardless of host OS so the
    /// board names look the same across macOS / Linux / Windows.
    name: String,
    /// Absolute path to the repo root, written into the board's
    /// `source_config.project_path`.
    path: String,
}

/// How deep to walk under the workspace root looking for repos.
/// Depth 1 = immediate children, 2 = grandchildren, 3 = great-grandchildren.
/// 3 catches the common "monorepo umbrella" pattern (e.g. workspace →
/// compliance/ → compliance-auth/.git) with a level of headroom and
/// caps the scan so a poorly-chosen workspace root can't fan out
/// through a giant directory tree.
const MAX_SCAN_DEPTH: usize = 3;

fn scan_immediate_subprojects(parent: &str) -> Vec<SubProject> {
    let mut out: Vec<SubProject> = Vec::new();
    let root = std::path::Path::new(parent);
    walk_for_repos(root, root, 0, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn walk_for_repos(
    dir: &std::path::Path,
    workspace_root: &std::path::Path,
    depth: usize,
    out: &mut Vec<SubProject>,
) {
    if depth >= MAX_SCAN_DEPTH {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip dotfolders + common build/dep noise that would never be
        // a project root and can be huge (node_modules especially).
        if file_name.starts_with('.') || file_name == "node_modules" || file_name == "target" {
            continue;
        }
        if path.join(".git").exists() {
            // Found a repo. Record with a path-relative name (e.g.
            // "compliance/compliance-auth") so siblings under the same
            // umbrella stay grouped in the alphabetical board list.
            // Don't recurse into it — nested .git dirs (submodules,
            // agent worktrees) shouldn't fan out into more boards.
            let abs = path.to_string_lossy().to_string();
            let rel = path
                .strip_prefix(workspace_root)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.replace(std::path::MAIN_SEPARATOR, "/"))
                .unwrap_or_else(|| file_name.clone());
            out.push(SubProject { name: rel, path: abs });
            continue;
        }
        walk_for_repos(&path, workspace_root, depth + 1, out);
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Insert a board with the standard 5-column layout. Used by
/// `workspace_create` for both single-board and per-subproject cases,
/// and by `workspace_board_create` for user-driven board creation.
async fn create_default_board(
    pool: &sqlx::SqlitePool,
    workspace_id: &str,
    name: &str,
    source_config: Option<&str>,
    position: i32,
    now: &str,
) -> Result<String, String> {
    let board_id = new_id();
    repo::insert_board(
        pool,
        &board_id,
        workspace_id,
        name,
        "manual",
        source_config,
        position,
        now,
    )
    .await
    .map_err(|e| e.to_string())?;
    for (idx, (col_name, col_color)) in repo::DEFAULT_BOARD_COLUMNS.iter().enumerate() {
        repo::insert_column(
            pool,
            &new_id(),
            &board_id,
            col_name,
            Some(col_color),
            idx as i32,
            now,
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(board_id)
}

// ---------------------------------------------------------------------------
// workspaces
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn workspace_list(pool: State<'_, SqlitePool>) -> Result<Vec<Workspace>, String> {
    repo::list_workspaces(pool.inner(), repo::Pagination::default())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_get(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<Workspace, String> {
    repo::get_workspace_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_create(
    pool: State<'_, SqlitePool>,
    name: String,
    project_path: Option<String>,
    color: Option<String>,
    actor: String,
) -> Result<Workspace, String> {
    let id = new_id();
    let now = now_rfc3339();
    let project_name = project_path.as_deref().map(project_name_from_path);

    repo::insert_workspace(
        pool.inner(),
        &id,
        &name,
        project_path.as_deref(),
        project_name.as_deref(),
        color.as_deref(),
        &actor,
        &now,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Project-bound workspaces auto-get one or more default boards so
    // the user has something to look at on first open. Three cases:
    //   1. project_path is itself a git repo  → 1 board ("Tasks") with
    //      no source override; sync uses the workspace project_path.
    //   2. project_path is a folder containing nested git repos as
    //      immediate children → 1 board per subproject, named after
    //      the subfolder, with source_config={project_path:<sub>} so
    //      each board syncs against its own remote.
    //   3. project_path isn't a git repo and has no nested ones → 1
    //      board ("Tasks") with no source override (user can set a
    //      project URL later from the board's overflow menu).
    // Standalone workspaces (no project_path) skip auto-board entirely.
    if let Some(path) = project_path.as_deref() {
        let subprojects = if is_git_repo(path) {
            Vec::new()
        } else {
            scan_immediate_subprojects(path)
        };

        if subprojects.is_empty() {
            create_default_board(pool.inner(), &id, "Tasks", None, 0, &now).await?;
        } else {
            for (idx, sub) in subprojects.iter().enumerate() {
                // Per-board override: store the subproject's absolute
                // path inside source_config JSON so the sync banner
                // and the editor's "Set project" UI can read it back.
                let cfg = serde_json::json!({ "project_path": sub.path }).to_string();
                create_default_board(
                    pool.inner(),
                    &id,
                    &sub.name,
                    Some(cfg.as_str()),
                    idx as i32,
                    &now,
                )
                .await?;
            }
        }
    }

    // The workspaces table is exported by both workspace sync kinds.
    crate::cloud::scheduler::bump("workspace_notes");
    crate::cloud::scheduler::bump("workspace_boards");
    repo::get_workspace_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_update(
    pool: State<'_, SqlitePool>,
    id: String,
    name: String,
    project_path: Option<String>,
    color: Option<String>,
    actor: String,
) -> Result<(), String> {
    let now = now_rfc3339();
    let project_name = project_path.as_deref().map(project_name_from_path);
    repo::update_workspace(
        pool.inner(),
        &id,
        &name,
        project_path.as_deref(),
        project_name.as_deref(),
        color.as_deref(),
        &actor,
        &now,
    )
    .await
    .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_notes");
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

/// Pre-delete summary surfaced in the confirmation dialog. Cards
/// with active worktrees are listed individually so the user can see
/// what's about to be cleaned up off-disk.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDeletePreview {
    pub note_count: i64,
    pub board_count: i64,
    pub card_count: i64,
    pub active_worktrees: Vec<repo::ActiveWorktree>,
}

#[tauri::command]
pub async fn workspace_delete_preview(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<WorkspaceDeletePreview, String> {
    let pool = pool.inner();
    let note_count = repo::count_notes_in_workspace(pool, &id).await.map_err(|e| e.to_string())?;
    let board_count = repo::count_boards_in_workspace(pool, &id).await.map_err(|e| e.to_string())?;
    let card_count = repo::count_cards_in_workspace(pool, &id).await.map_err(|e| e.to_string())?;
    let active_worktrees = repo::list_active_worktrees_in_workspace(pool, &id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(WorkspaceDeletePreview { note_count, board_count, card_count, active_worktrees })
}

#[tauri::command]
pub async fn workspace_delete(
    pool: State<'_, SqlitePool>,
    id: String,
    delete_worktrees: Option<bool>,
) -> Result<(), String> {
    let pool = pool.inner();
    // Best-effort worktree cleanup BEFORE the cascade. Failures are
    // logged but never block the delete — orphaned worktrees can be
    // cleaned by hand, an undeleted workspace is a worse UX.
    if delete_worktrees.unwrap_or(true) {
        let active = repo::list_active_worktrees_in_workspace(pool, &id)
            .await
            .map_err(|e| e.to_string())?;
        for w in active {
            let wt_path = match w.worktree_path.as_ref().filter(|p| !p.trim().is_empty()) {
                Some(p) => p.clone(),
                None => continue,
            };
            let proj = w.project_path.clone();
            let session_id = w.session_id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                crate::modes::agent::worktree::agent_remove_worktree(proj, wt_path)
            })
            .await;
            let _ = repo::clear_session_worktree(pool, &session_id).await;
        }
    }
    repo::delete_workspace(pool, &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_notes");
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

// ---------------------------------------------------------------------------
// notes
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn workspace_note_list(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
) -> Result<Vec<WorkspaceNote>, String> {
    repo::list_notes_in_workspace(pool.inner(), &workspace_id, repo::Pagination::default())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_note_get(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<WorkspaceNote, String> {
    repo::get_note_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_note_create(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    title: String,
    content: Option<String>,
    tags: Option<Vec<String>>,
    linked_session_id: Option<String>,
    actor: String,
) -> Result<WorkspaceNote, String> {
    let id = new_id();
    let now = now_rfc3339();
    let tags_json = serde_json::to_string(&tags.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());
    repo::insert_note(
        pool.inner(),
        &id,
        &workspace_id,
        &title,
        content.as_deref().unwrap_or(""),
        &tags_json,
        linked_session_id.as_deref(),
        &actor,
        &now,
    )
    .await
    .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_notes");
    repo::get_note_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_note_update(
    pool: State<'_, SqlitePool>,
    id: String,
    title: String,
    content: String,
    tags: Vec<String>,
    linked_session_id: Option<String>,
    actor: String,
    expected_updated_at: Option<String>,
) -> Result<(), String> {
    let now = now_rfc3339();
    let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());
    let guard = repo::MutationGuard {
        respect_frozen: false,
        expected_updated_at: expected_updated_at.as_deref(),
    };
    let rows = repo::update_note(
        pool.inner(),
        &id,
        &title,
        &content,
        &tags_json,
        linked_session_id.as_deref(),
        &actor,
        &now,
        guard,
    )
    .await
    .map_err(|e| e.to_string())?;
    if rows == 0 {
        match repo::diagnose_note_failure(pool.inner(), &id, guard).await {
            Ok(repo::MutationFailureReason::Conflict { current_updated_at }) => {
                return Err(format!(
                    "Note was modified concurrently. Current updated_at: {current_updated_at}"
                ));
            }
            Ok(repo::MutationFailureReason::NotFound) => return Err("Note not found".into()),
            _ => return Err("Note update failed".into()),
        }
    }
    crate::cloud::scheduler::bump("workspace_notes");
    Ok(())
}

#[tauri::command]
pub async fn workspace_note_delete(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    repo::delete_note(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_notes");
    Ok(())
}

// ---------------------------------------------------------------------------
// boards
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn workspace_board_list(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
) -> Result<Vec<WorkspaceBoard>, String> {
    repo::list_boards_in_workspace(pool.inner(), &workspace_id, repo::Pagination::default())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_board_get(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<WorkspaceBoard, String> {
    repo::get_board_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_board_create(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    name: String,
) -> Result<WorkspaceBoard, String> {
    let now = now_rfc3339();
    let existing = repo::list_boards_in_workspace(pool.inner(), &workspace_id, repo::Pagination::default())
        .await
        .map_err(|e| e.to_string())?;
    let position = existing.len() as i32;
    let board_id =
        create_default_board(pool.inner(), &workspace_id, &name, None, position, &now).await?;
    crate::cloud::scheduler::bump("workspace_boards");
    repo::get_board_by_id(pool.inner(), &board_id)
        .await
        .map_err(|e| e.to_string())
}

/// Set or clear the per-board project override. Either or both of
/// `project_path` and `project_url` may be set:
///   • path  — local clone; sync runs from cwd, picks up the remote.
///   • url   — direct GitHub/GitLab URL; sync uses `--repo owner/repo`.
/// Both empty → override cleared, board falls back to the parent
/// workspace's project_path.
#[tauri::command]
pub async fn workspace_board_set_project(
    pool: State<'_, SqlitePool>,
    id: String,
    project_path: Option<String>,
    project_url: Option<String>,
) -> Result<(), String> {
    let now = now_rfc3339();
    let path = project_path.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let url = project_url.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let cfg = match (path, url) {
        (None, None) => None,
        _ => {
            let mut obj = serde_json::Map::new();
            if let Some(p) = path { obj.insert("project_path".into(), serde_json::Value::String(p.into())); }
            if let Some(u) = url  { obj.insert("project_url".into(),  serde_json::Value::String(u.into())); }
            Some(serde_json::Value::Object(obj).to_string())
        }
    };
    repo::update_board_source_config(pool.inner(), &id, cfg.as_deref(), &now)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

#[tauri::command]
pub async fn workspace_board_rename(
    pool: State<'_, SqlitePool>,
    id: String,
    name: String,
) -> Result<(), String> {
    let now = now_rfc3339();
    repo::update_board_name(pool.inner(), &id, &name, &now)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

#[tauri::command]
pub async fn workspace_board_delete(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    repo::delete_board(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

#[tauri::command]
pub async fn workspace_column_list(
    pool: State<'_, SqlitePool>,
    board_id: String,
) -> Result<Vec<WorkspaceBoardColumn>, String> {
    repo::list_columns_in_board(pool.inner(), &board_id)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// cards
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn workspace_card_list(
    pool: State<'_, SqlitePool>,
    board_id: String,
) -> Result<Vec<WorkspaceBoardCard>, String> {
    repo::list_cards_in_board(pool.inner(), &board_id, repo::Pagination::default())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn workspace_card_create(
    pool: State<'_, SqlitePool>,
    column_id: String,
    title: String,
    description: Option<String>,
    priority: Option<String>,
    tags: Option<Vec<String>>,
    position: Option<i32>,
    external_id: Option<String>,
    external_url: Option<String>,
    linked_session_id: Option<String>,
    coworker_id: Option<String>,
    actor: String,
) -> Result<WorkspaceBoardCard, String> {
    crate::telemetry::bump("workspace.card_create");
    let id = new_id();
    let now = now_rfc3339();
    let tags_json = serde_json::to_string(&tags.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());

    repo::insert_card(
        pool.inner(),
        &id,
        &column_id,
        &title,
        description.as_deref().unwrap_or(""),
        priority.as_deref(),
        &tags_json,
        position.unwrap_or(0),
        external_id.as_deref(),
        external_url.as_deref(),
        linked_session_id.as_deref(),
        coworker_id.as_deref(),
        &actor,
        &now,
    )
    .await
    .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_boards");

    sqlx::query_as::<_, WorkspaceBoardCard>("SELECT * FROM workspace_board_cards WHERE id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn workspace_card_update(
    pool: State<'_, SqlitePool>,
    id: String,
    title: String,
    description: String,
    priority: Option<String>,
    tags: Vec<String>,
    review_checklist: Option<String>,
    coworker_id: Option<String>,
    actor: String,
    expected_updated_at: Option<String>,
) -> Result<(), String> {
    let now = now_rfc3339();
    let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());
    let guard = repo::MutationGuard {
        respect_frozen: false,
        expected_updated_at: expected_updated_at.as_deref(),
    };
    let rows = repo::update_card(
        pool.inner(),
        &id,
        &title,
        &description,
        priority.as_deref(),
        &tags_json,
        review_checklist.as_deref(),
        coworker_id.as_deref(),
        &actor,
        &now,
        guard,
    )
    .await
    .map_err(|e| e.to_string())?;
    if rows == 0 {
        match repo::diagnose_card_failure(pool.inner(), &id, guard).await {
            Ok(repo::MutationFailureReason::Conflict { current_updated_at }) => {
                return Err(format!(
                    "Card was modified concurrently. Current updated_at: {current_updated_at}"
                ));
            }
            Ok(repo::MutationFailureReason::NotFound) => return Err("Card not found".into()),
            _ => return Err("Card update failed".into()),
        }
    }
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

/// Move a card to a column + position. The actor decides whether the
/// move triggers a "Pending review" flag: when an agent (anything other
/// than `user` / `user:*`) moves to a column whose name matches one of
/// the review-class names below, we set `review_pending = 1`. User moves
/// always clear the flag. Keeping this rule in Rust means the same
/// behaviour applies whether the move comes from the UI or from an MCP
/// tool call later.
#[tauri::command]
pub async fn workspace_card_move(
    pool: State<'_, SqlitePool>,
    id: String,
    column_id: String,
    position: i32,
    actor: String,
) -> Result<(), String> {
    let now = now_rfc3339();
    let is_user = actor == "user" || actor.starts_with("user:");

    let review_pending = if is_user {
        0
    } else {
        // Agent moving into the "In Review" safety-gate column flags
        // pending review. "In Progress" is the active-work column and
        // does NOT trigger the flag.
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM workspace_board_columns WHERE id = ?",
        )
        .bind(&column_id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
        match row {
            Some((name,)) if is_review_only_column(&name) => 1,
            _ => 0,
        }
    };

    repo::move_card(
        pool.inner(),
        &id,
        &column_id,
        position,
        review_pending,
        &actor,
        &now,
        repo::MutationGuard::default(),
    )
    .await
    .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

#[tauri::command]
pub async fn workspace_card_clear_review(
    pool: State<'_, SqlitePool>,
    id: String,
    actor: String,
) -> Result<(), String> {
    let now = now_rfc3339();
    repo::clear_review_pending(pool.inner(), &id, &actor, &now, repo::MutationGuard::default())
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

#[tauri::command]
pub async fn workspace_card_delete(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    repo::delete_card(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

/// External IDs the user has tombstoned for this board — sync skips
/// them so re-imported Done issues don't keep coming back to Todo.
#[tauri::command]
pub async fn workspace_board_dismissed_externals(
    pool: State<'_, SqlitePool>,
    board_id: String,
) -> Result<Vec<String>, String> {
    repo::list_dismissed_externals(pool.inner(), &board_id)
        .await
        .map_err(|e| e.to_string())
}

// ── Card claim + drawer-chat lifecycle (migration 14) ─────────────
// Three concepts:
//   • `claim`      = the single session allowed to drive the agent on
//                    this card. Drawer chat creates a hidden session
//                    + claim on first interaction; terminal claims
//                    explicitly via `cards_claim` MCP tool.
//   • `start_work` = once a claim exists, optionally create a worktree
//                    + branch on that session. Code edits run there;
//                    pure-talk stays at `project_path`.
//   • `release`    = unclaim. Optionally delete the worktree.

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CardClaimState {
    /// The session id that owns this card right now, if any.
    pub claimed_session_id: Option<String>,
    /// The coworker (persona) that owns the active conversation. May
    /// be None even when claimed_session_id is set — terminal sessions
    /// (cards_claim from MCP) don't have a persona today.
    pub claimed_coworker_id: Option<String>,
    /// Full session row for the claim. None = unclaimed.
    pub session: Option<crate::modes::agent::models::AgentSession>,
    /// Full coworker row for the claim. None when manual-claimed or
    /// unclaimed.
    pub coworker: Option<crate::modes::workspace::models::WorkspaceCoworker>,
    /// `true` when the claim is held by a *card-origin* (hidden) session
    /// owned by this card — i.e. drawer chat is allowed to continue
    /// here. `false` for manual sessions = claim conflict from the
    /// drawer's perspective.
    pub drawer_owns: bool,
}

/// Read the current claim for a card. Drawer calls this on mount and
/// after every chat turn so the banner reflects truth.
#[tauri::command]
pub async fn workspace_card_get_claim(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<CardClaimState, String> {
    use crate::shared::repos::{coworkers as coworker_repo, sessions as session_repo};
    let row = repo::get_card_claim_and_project(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Card not found".to_string())?;
    let (claimed_session_id, claimed_coworker_id, _project_path) = row;
    let session = match claimed_session_id.as_deref() {
        Some(sid) => session_repo::get_session_by_id(pool.inner(), sid).await.ok(),
        None => None,
    };
    let coworker = match claimed_coworker_id.as_deref() {
        Some(cwid) => coworker_repo::get_coworker(pool.inner(), cwid).await.ok(),
        None => None,
    };
    let drawer_owns = session
        .as_ref()
        .map(|s| s.origin == "card" && s.card_id.as_deref() == Some(&id))
        .unwrap_or(false);
    Ok(CardClaimState {
        claimed_session_id,
        claimed_coworker_id,
        session,
        coworker,
        drawer_owns,
    })
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DrawerChatResult {
    /// Created user comment row.
    pub user_comment: crate::modes::workspace::models::WorkspaceCardComment,
    /// Created agent reply row (None on agent failure — the user
    /// comment was still persisted for the trail).
    pub reply_comment: Option<crate::modes::workspace::models::WorkspaceCardComment>,
    /// The session that handled (or would have handled) the turn.
    pub session_id: String,
    /// Soft error from the agent run — surfaced as a toast next to
    /// the user comment. Differs from the Tauri-level Err result
    /// (which means the request itself never reached the agent).
    pub agent_error: Option<String>,
}

/// The drawer-chat endpoint. Caller passes the active coworker so the
/// agent gets the right persona at spawn. Handles:
///   • First message on an unclaimed card → create-or-reuse hidden
///     session for (card, coworker), claim, post comment, run agent,
///     post reply.
///   • Same coworker re-engaged → reuse the same hidden session
///     (Claude `--resume` keeps thread continuity).
///   • Different coworker active → returns an Err telling the user to
///     End the work-stream / Switch coworker first.
///   • Manual terminal-claimed card → returns Err.
#[tauri::command]
pub async fn workspace_card_drawer_chat(
    pool: State<'_, SqlitePool>,
    app: tauri::AppHandle,
    id: String,
    coworker_id: String,
    body: String,
    actor: String,
) -> Result<DrawerChatResult, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err("Message is empty".into());
    }
    let result =
        super::agent_spawn::drawer_chat_turn(pool.inner(), Some(&app), &id, &coworker_id, trimmed, &actor)
            .await?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(result)
}

/// Release a card's claim. When `delete_worktree` is true and the
/// claimed session has a worktree, the worktree is removed via git.
/// The hidden session row stays (lets the user re-open later); only
/// the lock + (optionally) the worktree are released.
#[tauri::command]
pub async fn workspace_card_release(
    pool: State<'_, SqlitePool>,
    id: String,
    actor: String,
    delete_worktree: Option<bool>,
) -> Result<(), String> {
    super::agent_spawn::release_card(
        pool.inner(),
        &id,
        &actor,
        delete_worktree.unwrap_or(false),
    )
    .await?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StartWorkResult {
    pub worktree_path: String,
    pub worktree_branch: String,
}

/// Promote a claimed card to "active code work": create a git worktree
/// + branch on the project, attach to the claimed session. Errors when
/// the card isn't claimed by a card-origin (hidden) session — manual
/// sessions own their own worktree lifecycle in Agent mode.
#[tauri::command]
pub async fn workspace_card_start_work(
    pool: State<'_, SqlitePool>,
    id: String,
    actor: String,
) -> Result<StartWorkResult, String> {
    let result = super::agent_spawn::start_work(pool.inner(), &id, &actor).await?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(result)
}

/// Push a local card to its workspace's bound GitHub/GitLab repo as a
/// real issue. Always user-initiated (button in the card editor) —
/// never automatic. Same code path as the `cards_push_to_repo` MCP
/// tool so behaviour stays uniform across UI + agent triggers.
#[tauri::command]
pub async fn workspace_card_push_to_repo(
    pool: State<'_, SqlitePool>,
    id: String,
    actor: String,
) -> Result<serde_json::Value, String> {
    let result = super::push::push_card_to_repo(pool.inner(), &id, &actor).await?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(result)
}

/// Push the card's branch and (if no PR exists yet) open a PR / MR.
/// Idempotent — when the card already has a `pr_url`, this just
/// pushes new commits to the existing PR's branch. Always user-
/// initiated (UI button) or explicit-agent-driven (MCP tool); the
/// persona prompt forbids autonomous calls. Returns the resolved PR
/// URL + a flag telling the caller whether the PR was created or
/// updated, so the toast can say the right thing.
#[tauri::command]
pub async fn workspace_card_raise_pr(
    pool: State<'_, SqlitePool>,
    app: tauri::AppHandle,
    card_id: String,
    title: Option<String>,
    body: Option<String>,
    actor: String,
) -> Result<super::pr::RaisePrResult, String> {
    let result = super::pr::raise_or_update_pr(
        pool.inner(),
        Some(&app),
        &card_id,
        title.as_deref(),
        body.as_deref(),
        &actor,
    )
    .await
    .map_err(|e| e.message())?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(result)
}

/// Read the host's current state for a card's PR / MR. Returns one of
/// `"open" | "merged" | "closed" | "unknown"` (lowercase). Pure read,
/// no state changes — used by the frontend's poll-on-focus loop to
/// detect merged PRs and auto-move the card to the "Done"-like column.
#[tauri::command]
pub async fn workspace_card_check_pr_state(
    pool: State<'_, SqlitePool>,
    card_id: String,
) -> Result<super::pr::PrState, String> {
    super::pr::check_pr_state(pool.inner(), &card_id)
        .await
        .map_err(|e| e.message())
}

/// Add a comment to a card. Each comment is its own row in
/// `workspace_card_comments`; the helper also bumps `card.updated_at`
/// + `updated_by` so the inbox and per-card unread tracking continue
/// to work without query changes.
#[tauri::command]
pub async fn workspace_card_add_comment(
    pool: State<'_, SqlitePool>,
    id: String,
    body: String,
    actor: String,
) -> Result<crate::modes::workspace::models::WorkspaceCardComment, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err("Comment body is empty".into());
    }
    let now = now_rfc3339();
    let comment_id = uuid::Uuid::new_v4().to_string();
    repo::insert_card_comment(
        pool.inner(),
        &comment_id,
        &id,
        &actor,
        None, // plain user/agent comment — no coworker attribution
        trimmed,
        None,
        &now,
        repo::MutationGuard::default(),
    )
    .await
    .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(crate::modes::workspace::models::WorkspaceCardComment {
        id: comment_id,
        card_id: id,
        actor,
        coworker_id: None,
        body: trimmed.to_string(),
        parent_id: None,
        created_at: now,
    })
}

/// List all comments on a card, oldest first. Drives the Thread tab
/// in CardEditorDrawer.
#[tauri::command]
pub async fn workspace_card_comment_list(
    pool: State<'_, SqlitePool>,
    card_id: String,
) -> Result<Vec<crate::modes::workspace::models::WorkspaceCardComment>, String> {
    repo::list_card_comments(pool.inner(), &card_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_card_comment_delete(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    repo::delete_card_comment(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("workspace_boards");
    Ok(())
}

#[tauri::command]
pub async fn workspace_inbox_list(
    pool: State<'_, SqlitePool>,
    limit: Option<i32>,
) -> Result<Vec<crate::shared::repos::workspaces::InboxItem>, String> {
    repo::list_inbox(pool.inner(), limit.unwrap_or(50))
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// MCP server lifecycle. Backed by a global Tokio Mutex<Option<McpHandle>>
// in app state — simple, single-instance. start() returns the port +
// token; stop() drops the handle (which fires the oneshot shutdown).
// ---------------------------------------------------------------------------

use tokio::sync::Mutex as AsyncMutex;
use crate::modes::workspace::mcp::{self, McpHandle};

pub struct McpServerState(pub AsyncMutex<Option<McpHandle>>);

impl Default for McpServerState {
    fn default() -> Self { Self(AsyncMutex::new(None)) }
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub running: bool,
    pub port: Option<u16>,
}

#[tauri::command]
pub async fn workspace_mcp_status(
    server: State<'_, McpServerState>,
) -> Result<McpStatus, String> {
    let g = server.0.lock().await;
    Ok(match &*g {
        Some(h) => McpStatus { running: true, port: Some(h.port) },
        None => McpStatus { running: false, port: None },
    })
}

#[tauri::command]
pub async fn workspace_mcp_start(
    app: tauri::AppHandle,
    pool: State<'_, SqlitePool>,
    server: State<'_, McpServerState>,
    port: u16,
    token: String,
) -> Result<McpStatus, String> {
    let mut g = server.0.lock().await;
    if g.is_some() {
        // Already running — caller should stop first if they want to re-bind.
        return Ok(McpStatus { running: true, port: g.as_ref().map(|h| h.port) });
    }
    // Hand the app handle to the MCP server so MCP-side mutations
    // (cards_call_coworker etc.) can emit `workspace:card-updated`
    // and refresh open drawers live.
    let handle = mcp::start(pool.inner().clone(), port, token.clone(), Some(app)).await?;
    let bound_port = handle.port;
    *g = Some(handle);
    drop(g);
    // If port fallback kicked in, back-write the actually-bound port
    // so the UI + future auto-starts use it. Best-effort — failing to
    // persist doesn't invalidate the running server.
    if bound_port != port {
        let _ = settings_repo::upsert(
            pool.inner(),
            "workspace_mcp_port",
            &bound_port.to_string(),
        )
        .await;
        // ~/.claude.json was registered (or will be) against the
        // requested port; rewrite it with the bound port if a
        // claude-code entry already exists.
        let _ = sync_claude_code_registration(bound_port, &token);
    }
    Ok(McpStatus { running: true, port: Some(bound_port) })
}

#[tauri::command]
pub async fn workspace_mcp_stop(
    server: State<'_, McpServerState>,
) -> Result<McpStatus, String> {
    let mut g = server.0.lock().await;
    if let Some(mut h) = g.take() {
        if let Some(tx) = h.shutdown.take() {
            let _ = tx.send(());
        }
    }
    Ok(McpStatus { running: false, port: None })
}

// ---------------------------------------------------------------------------
// Per-agent registration of the MCP server. Each supported agent has
// its own config-file location and JSON shape; we dispatch on an
// `agent` slug so the API stays stable as new agents (codex, gemini,
// opencode, …) get MCP support. The MCP server itself is shared —
// every agent connects to the same `localhost:<port>/mcp` endpoint;
// only the on-disk registration differs.
// ---------------------------------------------------------------------------

const AGENT_CLAUDE_CODE: &str = "claude-code";
const AGENT_CODEX: &str = "codex";
const AGENT_GEMINI: &str = "gemini";
const AGENT_OPENCODE: &str = "opencode";

/// Env var name the Codex spawn path injects with the workspace MCP
/// token. Mirrored in `modes/agent/terminal.rs` for `provider=='codex'`
/// so codex CLI's `--bearer-token-env-var` registration reads the right
/// variable. Keep this constant in lockstep.
pub(crate) const CODEX_BEARER_ENV: &str = "CLAUGE_WORKSPACE_TOKEN";

/// Best-effort lazy MCP registration for non-Claude providers.
/// Called from `agent_create_session` when the user picks Codex or
/// OpenCode — registers the workspace MCP in that CLI's config so the
/// session's tools are reachable. Idempotent: re-registering with the
/// same port + token is a write of the same bytes. Silently no-ops
/// when the CLI isn't installed (shell-out fails / config path
/// missing). We never auto-register at app boot for these two — that
/// would modify external config files for alpha testers who installed
/// the CLIs for their own use.
pub async fn ensure_provider_mcp_registered(pool: &SqlitePool, provider: &str) {
    if provider == "claude" {
        return; // already handled by maybe_autostart_mcp
    }
    let port: u16 = match settings_repo::get_by_key(pool, "workspace_mcp_port").await {
        Ok(Some(s)) => s.value.parse().unwrap_or(7421),
        _ => 7421,
    };
    let token = match settings_repo::get_by_key(pool, "workspace_mcp_token").await {
        Ok(Some(s)) => s.value,
        _ => return, // no token persisted → MCP isn't enabled; skip
    };
    let _ = match provider {
        "codex" => register_codex(port, &token),
        "gemini" => register_gemini(port, &token),
        "opencode" => register_opencode(port, &token),
        _ => Ok(()),
    };
}

#[tauri::command]
pub fn workspace_mcp_register(
    agent: Option<String>,
    port: u16,
    token: String,
) -> Result<(), String> {
    let agent = agent.unwrap_or_else(|| AGENT_CLAUDE_CODE.to_string());
    match agent.as_str() {
        AGENT_CLAUDE_CODE => register_claude_code(port, &token),
        AGENT_CODEX => register_codex(port, &token),
        AGENT_GEMINI => register_gemini(port, &token),
        AGENT_OPENCODE => register_opencode(port, &token),
        other => Err(format!(
            "Unknown agent '{}'. Supported today: {}, {}, {}, {}",
            other, AGENT_CLAUDE_CODE, AGENT_CODEX, AGENT_GEMINI, AGENT_OPENCODE
        )),
    }
}

#[tauri::command]
pub fn workspace_mcp_unregister(agent: Option<String>) -> Result<(), String> {
    let agent = agent.unwrap_or_else(|| AGENT_CLAUDE_CODE.to_string());
    match agent.as_str() {
        AGENT_CLAUDE_CODE => unregister_claude_code(),
        AGENT_CODEX => unregister_codex(),
        AGENT_GEMINI => unregister_gemini(),
        AGENT_OPENCODE => unregister_opencode(),
        other => Err(format!(
            "Unknown agent '{}'. Supported today: {}, {}, {}, {}",
            other, AGENT_CLAUDE_CODE, AGENT_CODEX, AGENT_GEMINI, AGENT_OPENCODE
        )),
    }
}

// ─── claude-code (~/.claude.json) ────────────────────────────────────
//
// Important: Claude Code reads `mcpServers` from `~/.claude.json` —
// the same file `claude mcp add --scope user` writes to. The
// adjacent `~/.claude/settings.json` is a policy/hook file and does
// NOT load MCP servers from it. Earlier versions of this code wrote
// to the wrong file, so Claude Code silently never saw the workspace
// server. Keep this comment as the trail in case anyone moves it.

fn claude_settings_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

fn register_claude_code(port: u16, token: &str) -> Result<(), String> {
    let path = claude_settings_path().ok_or("home directory not found")?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let map = root.as_object_mut().unwrap();
    let servers = map
        .entry("mcpServers".to_string())
        .or_insert(serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    let s = servers.as_object_mut().unwrap();
    s.insert(
        "clauge-workspace".to_string(),
        serde_json::json!({
            "type": "http",
            "url": format!("http://localhost:{}/mcp", port),
            "headers": { "Authorization": format!("Bearer {}", token) }
        }),
    );
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())?;
    Ok(())
}

fn unregister_claude_code() -> Result<(), String> {
    let path = match claude_settings_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut root: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if let Some(map) = root.as_object_mut() {
        if let Some(servers) = map.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
            servers.remove("clauge-workspace");
        }
    }
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())?;
    Ok(())
}

// ─── codex (`codex mcp add` + env-var bearer) ───────────────────────
//
// Codex's `codex mcp add NAME --url URL --bearer-token-env-var ENV`
// records the *env var name* in `~/.codex/config.toml`; the token
// itself is never written to disk. At runtime, codex reads ENV from
// its process environment. Our spawn path (terminal.rs, provider=codex)
// injects `CLAUGE_WORKSPACE_TOKEN=<persisted-token>` into the
// CommandBuilder, so codex authenticates to the workspace MCP without
// the token ever leaving our SQLite. When the user invokes codex
// outside Clauge the env var isn't set and connection fails — which is
// correct (Clauge's MCP server isn't running there anyway).

fn register_codex(port: u16, _token: &str) -> Result<(), String> {
    // We don't pass the token here — codex stores only the env-var
    // name. The actual value is injected at spawn time. Shell out
    // through the user's login + interactive shell so version-manager
    // PATHs (nvm/fnm/asdf) resolve `codex` correctly. The URL is
    // composed of a fixed scheme/host + a u16 port; no shell-special
    // characters are reachable, so no quoting needed.
    let url = format!("http://localhost:{}/mcp", port);
    let cmd = format!(
        "codex mcp add clauge-workspace --url {} --bearer-token-env-var {}",
        url, CODEX_BEARER_ENV
    );
    run_through_user_shell(&cmd, "codex mcp add")
}

fn unregister_codex() -> Result<(), String> {
    let cmd = "codex mcp remove clauge-workspace";
    // Best-effort: a missing entry isn't an error from our side.
    let _ = run_through_user_shell(cmd, "codex mcp remove");
    Ok(())
}

// ─── opencode (~/.config/opencode/opencode.json) ────────────────────
//
// OpenCode's MCP servers are listed under an `mcp` object in its
// global config (`~/.config/opencode/opencode.json`) or per-project
// `opencode.json`. We register to the global config so the workspace
// MCP is reachable from any project the user opens. Schema lifted from
// opencode.ai/config.json (`type: "remote" | "local"`); we set
// `headers` for bearer-token auth, matching common MCP client
// conventions.

fn opencode_config_path() -> Option<std::path::PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return Some(std::path::PathBuf::from(xdg).join("opencode").join("opencode.json"));
        }
    }
    dirs::home_dir().map(|h| h.join(".config").join("opencode").join("opencode.json"))
}

fn register_opencode(port: u16, token: &str) -> Result<(), String> {
    let path = opencode_config_path().ok_or("home directory not found")?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let map = root.as_object_mut().unwrap();
    let mcp = map.entry("mcp".to_string()).or_insert(serde_json::json!({}));
    if !mcp.is_object() {
        *mcp = serde_json::json!({});
    }
    let m = mcp.as_object_mut().unwrap();
    m.insert(
        "clauge-workspace".to_string(),
        serde_json::json!({
            "type": "remote",
            "url": format!("http://localhost:{}/mcp", port),
            "headers": { "Authorization": format!("Bearer {}", token) }
        }),
    );
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())?;
    Ok(())
}

fn unregister_opencode() -> Result<(), String> {
    let path = match opencode_config_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut root: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if let Some(map) = root.as_object_mut() {
        if let Some(mcp) = map.get_mut("mcp").and_then(|v| v.as_object_mut()) {
            mcp.remove("clauge-workspace");
        }
    }
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())?;
    Ok(())
}

// ─── gemini (~/.gemini/settings.json) ────────────────────────────────
//
// Gemini CLI reads MCP servers from `mcpServers` inside its user
// settings file (`~/.gemini/settings.json` — the same file `gemini mcp
// add -s user` writes to). The on-disk shape matches what the CLI
// produces for `-t http`:
//
//     "mcpServers": {
//       "clauge-workspace": {
//         "url": "http://localhost:<port>/mcp",
//         "type": "http",
//         "headers": { "Authorization": "Bearer <token>" }
//       }
//     }
//
// Writing the file directly (rather than shelling out to `gemini mcp
// add`) avoids three things:
//   1. PATH brittleness when Gemini is installed via a version manager
//      and the bare `gemini` binary isn't on the non-login shell PATH.
//   2. The default scope of `gemini mcp add` is "project" — it would
//      write into `<cwd>/.gemini/settings.json` instead of the user
//      one, polluting random project folders. Forcing `-s user` works
//      but the shell-out is still slower and brittler than a write.
//   3. The CLI rewrites the whole file on every add; reading +
//      merging here lets us preserve any unrelated keys (`hooks`,
//      `security`, …) that the user has set.

fn gemini_settings_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".gemini").join("settings.json"))
}

fn register_gemini(port: u16, token: &str) -> Result<(), String> {
    let path = gemini_settings_path().ok_or("home directory not found")?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let map = root.as_object_mut().unwrap();
    let servers = map
        .entry("mcpServers".to_string())
        .or_insert(serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    let s = servers.as_object_mut().unwrap();
    s.insert(
        "clauge-workspace".to_string(),
        serde_json::json!({
            "url": format!("http://localhost:{}/mcp", port),
            "type": "http",
            "headers": { "Authorization": format!("Bearer {}", token) }
        }),
    );
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())?;
    Ok(())
}

fn unregister_gemini() -> Result<(), String> {
    let path = match gemini_settings_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut root: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if let Some(map) = root.as_object_mut() {
        if let Some(servers) = map.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
            servers.remove("clauge-workspace");
        }
    }
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())?;
    Ok(())
}

/// Shell-out helper that runs a command string through the user's
/// login + interactive shell so version-manager PATHs are honored.
/// Returns Ok(()) on exit-status-success; Err(stderr) otherwise.
fn run_through_user_shell(cmd: &str, label: &str) -> Result<(), String> {
    use crate::shared::platform::shell::default_user_shell;
    let (shell_path, shell_kind) = default_user_shell();
    let argv = shell_kind.exec_command_argv(cmd);
    let out = std::process::Command::new(&shell_path)
        .args(&argv)
        .output()
        .map_err(|e| format!("{label} spawn failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return Err(format!(
            "{label} failed: {}",
            if stderr.is_empty() { stdout } else { stderr }
        ));
    }
    Ok(())
}

/// Generate a fresh random token, persist it under
/// `workspace_mcp_token`, AND rewrite any existing `~/.claude.json`
/// entry so the registered token never goes stale relative to the
/// server's. Returns the new token.
///
/// `port` lets us write the correct URL into `~/.claude.json` —
/// callers pass the port the server is bound on (or the requested
/// port if it isn't running yet; the next start will sync if
/// fallback kicks in).
#[tauri::command]
pub async fn workspace_mcp_new_token(
    pool: State<'_, SqlitePool>,
    port: u16,
) -> Result<String, String> {
    let token = uuid::Uuid::new_v4().to_string();
    settings_repo::upsert(pool.inner(), "workspace_mcp_token", &token)
        .await
        .map_err(|e| e.to_string())?;
    // Best-effort re-register against every provider that's currently
    // registered. The token itself only lands in Claude / OpenCode
    // configs (Codex stores only the env-var name, so its registration
    // doesn't need a rewrite — the new token flows in at spawn time).
    let _ = sync_claude_code_registration(port, &token);
    let _ = sync_gemini_registration(port, &token);
    let _ = sync_opencode_registration(port, &token);
    Ok(token)
}

/// Mirror of `sync_claude_code_registration` for Gemini's
/// `~/.gemini/settings.json`. Only rewrites when `clauge-workspace`
/// is already listed; fresh registrations happen at autostart /
/// enable time.
fn sync_gemini_registration(port: u16, token: &str) -> Result<(), String> {
    let path = match gemini_settings_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let root: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let already_registered = root
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .map(|m| m.contains_key("clauge-workspace"))
        .unwrap_or(false);
    if !already_registered {
        return Ok(());
    }
    register_gemini(port, token)
}

/// Mirror of `sync_claude_code_registration` for OpenCode's JSON
/// config. Only rewrites when `clauge-workspace` is already listed;
/// fresh registrations happen at autostart / enable time.
fn sync_opencode_registration(port: u16, token: &str) -> Result<(), String> {
    let path = match opencode_config_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let root: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let already_registered = root
        .get("mcp")
        .and_then(|v| v.as_object())
        .map(|m| m.contains_key("clauge-workspace"))
        .unwrap_or(false);
    if !already_registered {
        return Ok(());
    }
    register_opencode(port, token)
}

/// If `~/.claude.json` already lists `clauge-workspace`, rewrite the
/// entry with the given port + token. No-op when the file doesn't
/// exist or has no entry — start/enable is the path that creates the
/// initial entry, this is only for keeping an existing one in sync.
fn sync_claude_code_registration(port: u16, token: &str) -> Result<(), String> {
    let path = match claude_settings_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let root: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let already_registered = root
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .map(|m| m.contains_key("clauge-workspace"))
        .unwrap_or(false);
    if !already_registered {
        return Ok(());
    }
    register_claude_code(port, token)
}

/// Best-effort auto-start, called from `lib.rs setup()` on app boot.
/// Reads persisted port + token, generates a token on first run,
/// starts the MCP server, persists the bound port if fallback kicked
/// in, and registers `~/.claude.json`. Skipped entirely when the
/// user has explicitly set `workspace_mcp_enabled` to "false".
///
/// Errors are logged but never bubbled — a failure here must not
/// stop app boot. The user can always toggle from Settings.
pub async fn maybe_autostart_mcp(app: tauri::AppHandle, pool: SqlitePool) {
    // Opt-out: only skip if the user has explicitly disabled.
    if let Ok(Some(s)) = settings_repo::get_by_key(&pool, "workspace_mcp_enabled").await {
        if s.value.eq_ignore_ascii_case("false") {
            return;
        }
    }
    let port: u16 = settings_repo::get_by_key(&pool, "workspace_mcp_port")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.value.parse().ok())
        .unwrap_or(7421);
    let token = match settings_repo::get_by_key(&pool, "workspace_mcp_token").await {
        Ok(Some(s)) => s.value,
        _ => {
            let t = uuid::Uuid::new_v4().to_string();
            let _ = settings_repo::upsert(&pool, "workspace_mcp_token", &t).await;
            t
        }
    };
    use tauri::Manager;
    let server_state: tauri::State<'_, McpServerState> = app.state();
    let mut g = server_state.0.lock().await;
    if g.is_some() {
        return; // already running
    }
    match mcp::start(pool.clone(), port, token.clone(), Some(app.clone())).await {
        Ok(handle) => {
            let bound = handle.port;
            *g = Some(handle);
            drop(g);
            if bound != port {
                let _ = settings_repo::upsert(&pool, "workspace_mcp_port", &bound.to_string()).await;
            }
            // Register Claude on first boot so a fresh install picks
            // up the entry without the user opening Settings. We DO
            // NOT auto-register Codex / OpenCode here — that would
            // touch their config files (~/.codex/config.toml,
            // ~/.config/opencode/opencode.json) for every alpha
            // tester who happens to have those CLIs installed for
            // their own use, even if they only use Clauge as a Claude
            // client. Codex / OpenCode register lazily on first
            // session-create for that provider (see
            // ensure_provider_mcp_registered, called from
            // agent_create_session).
            let _ = register_claude_code(bound, &token);
            // Make sure the persisted "enabled" key reflects the
            // running state — first-boot users default to true.
            let _ = settings_repo::upsert(&pool, "workspace_mcp_enabled", "true").await;
            log::info!(target: "workspace::mcp", "auto-started on 127.0.0.1:{bound}");
        }
        Err(e) => {
            log::warn!(target: "workspace::mcp", "auto-start failed: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Project issue scan — supports GitHub via `gh` and GitLab via `glab`.
// Each failure mode maps to its own UI state; we never throw a generic
// error string when there's a structured one available.
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn workspace_scan_project_issues(
    project_path: String,
) -> Result<ProjectScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || scan_project_issues_sync(&project_path))
        .await
        .map_err(|e| format!("scan thread error: {}", e))?
}

fn scan_project_issues_sync(project_path: &str) -> Result<ProjectScanResult, String> {
    use crate::shared::platform::path::{apply_user_path, find_binary};
    use std::process::Command;

    // 1. Is it a git repo at all?
    let git_bin = find_binary("git").unwrap_or_else(|| "git".into());
    let mut git = Command::new(&git_bin);
    apply_user_path(&mut git);
    let inside = git
        .args(["-C", project_path, "rev-parse", "--is-inside-work-tree"])
        .output();
    let inside_ok = matches!(&inside, Ok(o) if o.status.success());
    if !inside_ok {
        return Ok(ProjectScanResult::NotGitRepo);
    }

    // 2. Read the origin remote URL.
    let mut git = Command::new(&git_bin);
    apply_user_path(&mut git);
    let remote_out = git
        .args(["-C", project_path, "remote", "get-url", "origin"])
        .output()
        .map_err(|e| e.to_string())?;
    if !remote_out.status.success() {
        return Ok(ProjectScanResult::NoRemote);
    }
    let remote_url = String::from_utf8_lossy(&remote_out.stdout).trim().to_string();
    if remote_url.is_empty() {
        return Ok(ProjectScanResult::NoRemote);
    }

    // 3. Decide which CLI to use. We support github.com and any host
    //    whose URL contains "gitlab" (covers gitlab.com + self-hosted).
    let lower = remote_url.to_lowercase();
    let (tool, source, install_url, login_cmd, args): (&str, &str, &str, &str, Vec<&str>) =
        if lower.contains("github.com") {
            (
                "gh",
                "github",
                "https://cli.github.com/",
                "gh auth login",
                vec![
                    "issue",
                    "list",
                    "--state",
                    "open",
                    "--limit",
                    "100",
                    "--json",
                    "number,title,body,url,labels",
                ],
            )
        } else if lower.contains("gitlab") {
            (
                "glab",
                "gitlab",
                "https://gitlab.com/gitlab-org/cli",
                "glab auth login",
                vec!["issue", "list", "-F", "json", "--per-page", "100"],
            )
        } else {
            return Ok(ProjectScanResult::UnsupportedRemote { url: remote_url });
        };

    // 4. Is the CLI on PATH?
    let tool_bin = match find_binary(tool) {
        Some(p) => p,
        None => {
            return Ok(ProjectScanResult::ToolNotInstalled {
                tool: tool.to_string(),
                install_url: install_url.to_string(),
            });
        }
    };

    // 5. Run the issue list. cwd matters — both CLIs read repo context
    //    from the working directory.
    let mut cmd = Command::new(&tool_bin);
    apply_user_path(&mut cmd);
    let out = cmd
        .current_dir(project_path)
        .args(&args)
        .output()
        .map_err(|e| format!("{} failed to spawn: {}", tool, e))?;

    if !out.status.success() {
        // Best-effort owner/repo extraction from the git remote URL,
        // so a NoAccess error can name the specific repo the user
        // can't see ("can't access foo/bar" beats "can't access this repo").
        let repo_hint = parse_owner_repo(&remote_url);
        return Ok(map_cli_error_to_scan_result(
            tool,
            login_cmd,
            repo_hint.as_deref(),
            &out,
        ));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let issues = match source {
        "github" => parse_github_issues(&stdout),
        "gitlab" => parse_gitlab_issues(&stdout),
        _ => Vec::new(),
    };
    Ok(ProjectScanResult::Success {
        issues,
        remote: remote_url,
        source: source.to_string(),
    })
}

/// Convert a failed Command output into the right ProjectScanResult
/// variant via the shared cli_errors classifier. Replaces the old
/// "if stderr contains 'auth'" heuristic — that missed GitHub's
/// GraphQL "could not resolve to a Repository" response (the most
/// common multi-account confusion), so the user got raw stderr in a
/// generic ApiError banner instead of the actionable "wrong account"
/// message.
fn map_cli_error_to_scan_result(
    tool: &str,
    login_cmd: &str,
    repo_hint: Option<&str>,
    out: &std::process::Output,
) -> ProjectScanResult {
    use super::cli_errors::{classify_output, CliError};
    match classify_output(tool, out, repo_hint) {
        Some(CliError::NotAuthenticated { .. }) => ProjectScanResult::NotAuthenticated {
            tool: tool.to_string(),
            login_command: login_cmd.to_string(),
        },
        Some(CliError::NoAccess { repo, .. }) => ProjectScanResult::NoAccess {
            tool: tool.to_string(),
            repo,
            login_command: login_cmd.to_string(),
        },
        Some(CliError::NetworkError { msg }) => ProjectScanResult::NetworkError { message: msg },
        // NotInstalled is handled before the call (which-check), and
        // NoChanges / BranchNotPushed are git-only — none should fire
        // from issue-list. Fall through to ApiError with the raw stderr.
        Some(CliError::NotInstalled { .. })
        | Some(CliError::NoChanges)
        | Some(CliError::BranchNotPushed)
        | Some(CliError::Other { .. })
        | None => ProjectScanResult::ApiError {
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        },
    }
}

/// URL-based scan — used when the board has a `project_url` but no
/// `project_path`. We don't need a local clone; both `gh` and `glab`
/// support a `--repo`/`-R owner/repo` flag. Failure variants match
/// the path-based scan so the banner UI is identical.
#[tauri::command]
pub async fn workspace_scan_project_issues_by_url(
    project_url: String,
) -> Result<ProjectScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || scan_project_url_sync(&project_url))
        .await
        .map_err(|e| format!("scan thread error: {}", e))?
}

fn scan_project_url_sync(url: &str) -> Result<ProjectScanResult, String> {
    use crate::shared::platform::path::{apply_user_path, find_binary};
    use std::process::Command;

    // 1. Pick CLI + parse owner/repo from the URL.
    let lower = url.to_lowercase();
    let (tool, source, install_url, login_cmd, repo_arg, args): (
        &str,
        &str,
        &str,
        &str,
        Vec<String>,
        Vec<&str>,
    ) = if lower.contains("github.com") {
        let or = match parse_owner_repo(url) {
            Some(s) => s,
            None => return Ok(ProjectScanResult::UnsupportedRemote { url: url.to_string() }),
        };
        (
            "gh",
            "github",
            "https://cli.github.com/",
            "gh auth login",
            vec!["--repo".to_string(), or],
            vec!["issue", "list", "--state", "open", "--limit", "100", "--json", "number,title,body,url,labels"],
        )
    } else if lower.contains("gitlab") {
        let or = match parse_owner_repo(url) {
            Some(s) => s,
            None => return Ok(ProjectScanResult::UnsupportedRemote { url: url.to_string() }),
        };
        (
            "glab",
            "gitlab",
            "https://gitlab.com/gitlab-org/cli",
            "glab auth login",
            vec!["-R".to_string(), or],
            vec!["issue", "list", "-F", "json", "--per-page", "100"],
        )
    } else {
        return Ok(ProjectScanResult::UnsupportedRemote { url: url.to_string() });
    };

    // 2. CLI on PATH?
    let tool_bin = match find_binary(tool) {
        Some(p) => p,
        None => {
            return Ok(ProjectScanResult::ToolNotInstalled {
                tool: tool.to_string(),
                install_url: install_url.to_string(),
            });
        }
    };

    // 3. Run with --repo / -R prepended so cwd doesn't matter.
    let mut cmd = Command::new(&tool_bin);
    apply_user_path(&mut cmd);
    for a in &repo_arg { cmd.arg(a); }
    for a in &args { cmd.arg(a); }
    let out = cmd.output().map_err(|e| format!("{} failed to spawn: {}", tool, e))?;

    if !out.status.success() {
        // repo_arg is `["--repo", "owner/repo"]` for gh and `["-R", "owner/repo"]`
        // for glab — index 1 is the actual owner/repo string in both cases.
        let repo_hint = repo_arg.get(1).map(|s| s.as_str());
        return Ok(map_cli_error_to_scan_result(tool, login_cmd, repo_hint, &out));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let issues = match source {
        "github" => parse_github_issues(&stdout),
        "gitlab" => parse_gitlab_issues(&stdout),
        _ => Vec::new(),
    };
    Ok(ProjectScanResult::Success {
        issues,
        remote: url.to_string(),
        source: source.to_string(),
    })
}

/// Extract `owner/repo` from a GitHub/GitLab URL. Tolerant — handles
/// trailing slashes, `.git` suffix, `https://`/`http://`/`git@` URLs,
/// and self-hosted GitLab paths with sub-groups (which `glab` accepts
/// as `group/subgroup/project`).
pub fn parse_owner_repo(url: &str) -> Option<String> {
    let mut s = url.trim();
    // Strip protocol.
    s = s.strip_prefix("https://").unwrap_or(s);
    s = s.strip_prefix("http://").unwrap_or(s);
    s = s.strip_prefix("ssh://").unwrap_or(s);
    s = s.strip_prefix("git://").unwrap_or(s);
    if let Some(rest) = s.strip_prefix("git@") {
        // git@host:owner/repo.git → host/owner/repo
        s = rest;
    }
    // Take everything after the first '/' or ':' (host separator).
    let after_host = s.find(|c: char| c == '/' || c == ':').map(|i| &s[i + 1..])?;
    let path = after_host.trim_end_matches('/').trim_end_matches(".git");
    // Need at least owner/repo.
    if !path.contains('/') || path.is_empty() { return None; }
    Some(path.to_string())
}

fn parse_github_issues(json: &str) -> Vec<ProjectIssue> {
    let parsed: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = match parsed.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|i| {
            let number = i.get("number")?.as_i64()?;
            let title = i.get("title")?.as_str()?.to_string();
            let body = i.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let url = i.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let labels = i
                .get("labels")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(ProjectIssue {
                external_id: format!("#{}", number),
                title,
                body,
                url,
                source: "github".to_string(),
                labels,
            })
        })
        .collect()
}

fn parse_gitlab_issues(json: &str) -> Vec<ProjectIssue> {
    let parsed: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = match parsed.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|i| {
            let iid = i.get("iid").and_then(|v| v.as_i64())?;
            let title = i.get("title")?.as_str()?.to_string();
            let body = i.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let url = i.get("web_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let labels = i
                .get("labels")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| {
                            l.as_str().map(|s| s.to_string()).or_else(|| {
                                l.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(ProjectIssue {
                external_id: format!("!{}", iid),
                title,
                body,
                url,
                source: "gitlab".to_string(),
                labels,
            })
        })
        .collect()
}

// ───────────────────────────────────────────────────────────────────
// Coworkers CRUD (personas built on top of an underlying agent CLI).
// Global scope — not per-workspace.
// ───────────────────────────────────────────────────────────────────

use crate::modes::workspace::models::WorkspaceCoworker;
use crate::shared::repos::coworkers as coworker_repo;

#[tauri::command]
pub async fn workspace_coworker_list(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<WorkspaceCoworker>, String> {
    coworker_repo::list_coworkers(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_coworker_get(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<WorkspaceCoworker, String> {
    coworker_repo::get_coworker(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoworkerInput {
    pub name: String,
    pub role: Option<String>,
    pub system_prompt: Option<String>,
    pub provider: Option<String>,
    pub avatar_seed: Option<String>,
    pub avatar_style: Option<String>,
    pub actor: String,
}

#[tauri::command]
pub async fn workspace_coworker_create(
    pool: State<'_, SqlitePool>,
    pro_state: State<'_, crate::cloud::pro_state::ProStateManager>,
    input: CoworkerInput,
) -> Result<WorkspaceCoworker, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("Coworker name is required".into());
    }
    // Single source of truth: the in-memory ProStateManager. Replaces the
    // SQLite read + the `is_pro_hint` workaround that was needed when the
    // gate raced against an in-flight `cloud_auth` write. The manager is
    // updated atomically on every plan change so this read is never stale.
    if !pro_state.is_pro() {
        let active_count = coworker_repo::count_active(pool.inner())
            .await
            .unwrap_or(0);
        if active_count >= 3 {
            return Err(
                "Free plan supports up to 3 coworkers. Upgrade to Pro for unlimited.".into(),
            );
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let avatar_seed = input
        .avatar_seed
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(name)
        .to_string();
    coworker_repo::insert_coworker(
        pool.inner(),
        &id,
        name,
        input.role.as_deref().unwrap_or("").trim(),
        input.system_prompt.as_deref().unwrap_or("").trim(),
        input.provider.as_deref().unwrap_or("claude"),
        &avatar_seed,
        input.avatar_style.as_deref().unwrap_or("personas"),
        &now,
        &input.actor,
    )
    .await
    .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("coworkers");
    coworker_repo::get_coworker(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoworkerUpdate {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub system_prompt: Option<String>,
    pub provider: Option<String>,
    pub avatar_seed: Option<String>,
    pub avatar_style: Option<String>,
}

#[tauri::command]
pub async fn workspace_coworker_update(
    pool: State<'_, SqlitePool>,
    input: CoworkerUpdate,
) -> Result<WorkspaceCoworker, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("Coworker name is required".into());
    }
    let avatar_seed = input
        .avatar_seed
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(name)
        .to_string();
    coworker_repo::update_coworker(
        pool.inner(),
        &input.id,
        name,
        input.role.as_deref().unwrap_or("").trim(),
        input.system_prompt.as_deref().unwrap_or("").trim(),
        input.provider.as_deref().unwrap_or("claude"),
        &avatar_seed,
        input.avatar_style.as_deref().unwrap_or("personas"),
    )
    .await
    .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("coworkers");
    coworker_repo::get_coworker(pool.inner(), &input.id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_coworker_delete(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    // Block deletion when the coworker is still engaged on cards that
    // aren't in a Done-class column. The error message lists the
    // blocking cards by title so the user knows what to clean up.
    let active = coworker_repo::list_active_cards_for_coworker(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    if !active.is_empty() {
        let total = active.len();
        // Show first 3 titles inline; if more, suffix "+ N more".
        let preview: Vec<String> = active
            .iter()
            .take(3)
            .map(|(_, title, col)| format!("• \"{title}\" ({col})"))
            .collect();
        let suffix = if total > 3 {
            format!("\n• …and {} more", total - 3)
        } else {
            String::new()
        };
        return Err(format!(
            "This coworker is still engaged on {total} active card{}:\n\n{}{suffix}\n\n\
             Move those cards to Done (or remove the coworker from them) before deleting.",
            if total == 1 { "" } else { "s" },
            preview.join("\n"),
        ));
    }
    coworker_repo::delete_coworker(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("coworkers");
    Ok(())
}

/// Write an exported note to the path the user picked in the save
/// dialog. We don't ship `tauri-plugin-fs` (its scope-management cost
/// is high for a single write), so this purpose-built command keeps
/// the surface minimal. The path is fully controlled by the OS file
/// picker — we just dump bytes.
#[tauri::command]
pub async fn workspace_note_export_to_file(
    path: String,
    content: String,
) -> Result<(), String> {
    std::fs::write(&path, content.as_bytes()).map_err(|e| format!("write {path}: {e}"))
}

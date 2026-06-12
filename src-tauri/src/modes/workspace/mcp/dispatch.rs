// `tools/call` dispatch + per-tool helpers (diagnostics, pagination,
// auto-link heuristics, project-scoped upsert).

use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::shared::repos::{coworkers as coworker_repo, sessions as session_repo, workspaces as repo};

/// Resolve the actor to record on a write. When the calling agent passes
/// `coworkerId` (the persona prompt instructs them to, so the work shows
/// up as @<name> instead of "claude"), look the coworker up and return
/// their name as the effective actor. The frontend's `describeActor`
/// matches case-insensitively against the coworker registry and renders
/// the @<name> + avatar badge automatically.
///
/// Returns `(effective_actor, is_coworker)`. Callers gate the
/// `linked_session` auto-link on `is_coworker` — coworker writes come
/// through hidden ('card'-origin) sessions and have no business
/// pointing notes/cards at the user's manual Agent session.
async fn resolve_actor(
    pool: &SqlitePool,
    args: &Value,
    fallback: &str,
) -> (String, bool) {
    let coworker_id = args
        .get("coworkerId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    if let Some(cw_id) = coworker_id {
        if let Ok(cw) = coworker_repo::get_coworker(pool, cw_id).await {
            return (cw.name, true);
        }
    }
    (fallback.to_string(), false)
}

/// Resolve a caller-supplied project path into the canonical form the
/// workspace store uses as its dedup key.
///
/// Two transforms, in order:
///   1. `std::fs::canonicalize` — resolves symlinks, drops trailing slash,
///      normalises case on case-insensitive filesystems. Skipped if the
///      path doesn't exist (falls back to trimmed input).
///   2. Worktree → project root. Every Custom-purpose agent session
///      auto-creates a worktree under `<root>/.clauge-worktrees/<branch>`
///      and ends up cwd'd inside it; the workspace, however, is bound to
///      `<root>`. Without this step the agent's cwd-derived projectPath
///      misses the lookup and a duplicate workspace gets created. We
///      walk path components, and if any segment equals
///      `.clauge-worktrees` we keep only the segments before it.
///
/// Returns a string suitable for both `find_workspace_by_project_path`
/// and `insert_workspace`. Both find and insert use the same canonical
/// form so the dedup property holds going forward.
fn resolve_canonical_project_path(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // 1. Realpath via fs::canonicalize. Strips trailing slash, resolves
    //    symlinks, normalises case where the FS does. Falls back to the
    //    raw trimmed input if the path doesn't exist on disk.
    let realpath: String = std::fs::canonicalize(trimmed)
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
        .map(|s| {
            // Windows canonicalize prefixes paths with `\\?\` (extended-
            // length form). Strip it so the stored path matches what
            // users + tools see elsewhere.
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                s
            }
        })
        .unwrap_or_else(|| trimmed.to_string());

    // 2. Walk path components looking for `.clauge-worktrees`. If found,
    //    return the parent of that segment (the project root). Component
    //    walk handles both `/` and `\` separators uniformly.
    let pb = std::path::PathBuf::from(&realpath);
    let mut before_marker: std::path::PathBuf = std::path::PathBuf::new();
    let mut hit_marker = false;
    for comp in pb.components() {
        let segment = comp.as_os_str().to_string_lossy();
        if segment == ".clauge-worktrees" {
            hit_marker = true;
            break;
        }
        before_marker.push(comp);
    }
    if hit_marker {
        if let Some(s) = before_marker.into_os_string().into_string().ok() {
            return s;
        }
    }
    realpath
}

async fn upsert_workspace_for_project(
    pool: &SqlitePool,
    project_path: &str,
    actor: &str,
) -> Result<crate::modes::workspace::models::Workspace, (i32, String)> {
    let map_db = |e: sqlx::Error| -> (i32, String) { (-32603, format!("DB error: {}", e)) };
    // Canonicalise the incoming path BEFORE looking it up. This is what
    // makes "I'm cwd'd in /root/.clauge-worktrees/clauge/custom-foo-xyz,
    // please add a note for this project" land on the same workspace
    // that was created against /root from the UI. Without it the agent
    // creates a second workspace because the strings don't match.
    let canonical = resolve_canonical_project_path(project_path);
    if !canonical.is_empty() {
        if let Some(ws) = repo::find_workspace_by_project_path(pool, &canonical)
            .await
            .map_err(map_db)?
        {
            return Ok(ws);
        }
    }
    // Back-compat: existing DB rows may have been stored with a
    // non-canonical project_path (e.g. with a trailing slash, or as the
    // worktree path itself before this fix). Try the raw trimmed input
    // before deciding to insert a new workspace.
    let raw = project_path.trim();
    if !raw.is_empty() && raw != canonical {
        if let Some(ws) = repo::find_workspace_by_project_path(pool, raw)
            .await
            .map_err(map_db)?
        {
            return Ok(ws);
        }
    }
    // No hit either way — insert a new workspace, storing the canonical
    // form so future lookups are deterministic regardless of which path
    // representation the caller supplies.
    let stored_path = if !canonical.is_empty() { canonical } else { raw.to_string() };
    let name = std::path::Path::new(&stored_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();
    let project_name = name.clone();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    repo::insert_workspace(
        pool,
        &id,
        &name,
        Some(&stored_path),
        Some(&project_name),
        None,
        actor,
        &now,
    )
    .await
    .map_err(map_db)?;

    // Default board seeded from the shared `repo::DEFAULT_BOARD_COLUMNS`
    // constant — keeps the agent-spawned board layout in lockstep with
    // the UI-created one. See `commands::create_default_board`.
    let board_id = uuid::Uuid::new_v4().to_string();
    repo::insert_board(pool, &board_id, &id, "Tasks", "manual", None, 0, &now)
        .await
        .map_err(map_db)?;
    for (idx, (col_name, col_color)) in repo::DEFAULT_BOARD_COLUMNS.iter().enumerate() {
        repo::insert_column(
            pool,
            &uuid::Uuid::new_v4().to_string(),
            &board_id,
            col_name,
            Some(col_color),
            idx as i32,
            &now,
        )
        .await
        .map_err(map_db)?;
    }

    repo::get_workspace_by_id(pool, &id).await.map_err(map_db)
}

/// Auto-link a card to the agent session that "owns" it whenever an
/// agent mutates it via MCP. The MCP server doesn't natively know the
/// caller's session id (one shared HTTP endpoint, multiple potential
/// sessions per provider), so we use a heuristic: the most recently-
/// used session whose `project_path` matches the workspace's. In the
/// solo-dev / single-session-per-project case this is exact; with
/// multiple parallel sessions per project, "most recent" tracks
/// whichever session the user last activated, which is virtually
/// always the one currently making MCP calls.
///
/// Skipped when:
///   • Card's workspace has no project_path bound (nothing to match).
///   • No agent_session exists for that project_path yet.
///   • Caller is a user actor (UI edits — those don't represent
///     "session activity"). MCP actors are always agents in practice,
///     but we guard explicitly so callers can pass any actor.
///   • The card is already linked to that exact session — saves a
///     no-op write.
async fn auto_link_card_to_recent_session(
    pool: &SqlitePool,
    card_id: &str,
    actor: &str,
    now: &str,
) {
    if actor == "user" || actor.starts_with("user:") {
        return;
    }
    // Walk card → column → board → workspace.project_path. Both columns
    // are nullable so the tuple is (Option, Option).
    let row: Result<Option<(Option<String>, Option<String>)>, _> = sqlx::query_as(
        "SELECT c.linked_session_id, w.project_path \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         JOIN workspaces w ON w.id = b.workspace_id \
         WHERE c.id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await;
    let (current_link, project_path) = match row {
        Ok(Some(r)) => r,
        _ => return,
    };
    let project_path = match project_path.filter(|p| !p.trim().is_empty()) {
        Some(p) => p,
        None => return,
    };
    let session = match session_repo::find_recent_session_for_project(pool, &project_path).await {
        Ok(Some(s)) => s,
        _ => return,
    };
    if current_link.as_deref() == Some(session.id.as_str()) {
        return;
    }
    // Best-effort write — if it fails (deleted card race, etc.) we
    // silently skip; the original mutation already succeeded.
    let _ = repo::update_card_linked_session(pool, card_id, Some(&session.id), actor, now, repo::MutationGuard::default()).await;
}

/// Sibling of `auto_link_card_to_recent_session`, scoped to notes.
/// Same heuristic, same skip rules — the join walk is shorter because
/// notes belong to the workspace directly (no column/board hops).
async fn auto_link_note_to_recent_session(
    pool: &SqlitePool,
    note_id: &str,
    actor: &str,
    now: &str,
) {
    if actor == "user" || actor.starts_with("user:") {
        return;
    }
    let row: Result<Option<(Option<String>, Option<String>)>, _> = sqlx::query_as(
        "SELECT n.linked_session_id, w.project_path \
         FROM workspace_notes n \
         JOIN workspaces w ON w.id = n.workspace_id \
         WHERE n.id = ?",
    )
    .bind(note_id)
    .fetch_optional(pool)
    .await;
    let (current_link, project_path) = match row {
        Ok(Some(r)) => r,
        _ => return,
    };
    let project_path = match project_path.filter(|p| !p.trim().is_empty()) {
        Some(p) => p,
        None => return,
    };
    let session = match session_repo::find_recent_session_for_project(pool, &project_path).await {
        Ok(Some(s)) => s,
        _ => return,
    };
    if current_link.as_deref() == Some(session.id.as_str()) {
        return;
    }
    let _ = repo::update_note_linked_session(pool, note_id, Some(&session.id), actor, now, repo::MutationGuard::default()).await;
}

/// Wrap a value as MCP tool-call content. Single text item — clients
/// re-parse the JSON; that's the simplest, most compatible shape.
fn ok_text(value: Value) -> Value {
    json!({
        "content": [
            { "type": "text", "text": value.to_string() }
        ],
        "isError": false
    })
}

/// Translate a guarded mutation that returned 0 rows into a precise
/// MCP error. Saves every arm from spelling out the same diagnose →
/// match → format dance.
async fn diagnose_card_or_err(
    pool: &SqlitePool,
    card_id: &str,
    guard: repo::MutationGuard<'_>,
) -> (i32, String) {
    match repo::diagnose_card_failure(pool, card_id, guard).await {
        Ok(repo::MutationFailureReason::NotFound) => (-32602, "Card not found".into()),
        Ok(repo::MutationFailureReason::Frozen) => (-32000, "Card is frozen".into()),
        Ok(repo::MutationFailureReason::Conflict { current_updated_at }) => (
            -32000,
            format!("Card was modified concurrently. Current updated_at: {current_updated_at}"),
        ),
        Ok(repo::MutationFailureReason::Unknown) => {
            (-32000, "Card mutation failed (no rows affected)".into())
        }
        Err(e) => (-32603, format!("DB error: {e}")),
    }
}

async fn diagnose_note_or_err(
    pool: &SqlitePool,
    note_id: &str,
    guard: repo::MutationGuard<'_>,
) -> (i32, String) {
    match repo::diagnose_note_failure(pool, note_id, guard).await {
        Ok(repo::MutationFailureReason::NotFound) => (-32602, "Note not found".into()),
        Ok(repo::MutationFailureReason::Frozen) => (-32000, "Note is frozen".into()),
        Ok(repo::MutationFailureReason::Conflict { current_updated_at }) => (
            -32000,
            format!("Note was modified concurrently. Current updated_at: {current_updated_at}"),
        ),
        Ok(repo::MutationFailureReason::Unknown) => {
            (-32000, "Note mutation failed (no rows affected)".into())
        }
        Err(e) => (-32603, format!("DB error: {e}")),
    }
}

/// Default + max page size for the `*_list` tools. The cap exists so
/// an agent calling `notes_list` against a workspace with thousands
/// of notes can't unilaterally fill its context window with the
/// payload — it has to ask for more pages explicitly.
const PAGE_DEFAULT: i32 = 50;
const PAGE_MAX: i32 = 200;

/// Read the agent-supplied `limit` + `offset` args, clamp to sane
/// bounds, and return a `repo::Pagination` ready to pass to a list
/// helper. Missing values fall back to the defaults — every list
/// tool always paginates.
fn parse_pagination(args: &Value) -> repo::Pagination {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_i64())
        .map(|l| (l.clamp(1, PAGE_MAX as i64)) as i32)
        .unwrap_or(PAGE_DEFAULT);
    let offset = args
        .get("offset")
        .and_then(|v| v.as_i64())
        .map(|o| o.max(0) as i32)
        .unwrap_or(0);
    repo::Pagination { limit: Some(limit), offset: Some(offset) }
}

// Sync: MCP mutations must dirty the same kinds as UI mutations.
//
// Called once after a tool call has completed successfully. Read-only
// tools are skipped by suffix (`_list` / `_read` / `_search` /
// `_summary`) plus an explicit list for the reads that don't follow
// the naming convention. Tools that can upsert a workspace on the fly
// (workspaces_*, *_for_project, cards_create_from_branch,
// workspace_link_to_repo) dirty BOTH workspace kinds — the workspaces
// table is exported by both.
fn bump_sync_kinds_for_tool(tool_name: &str) {
    use crate::cloud::scheduler::bump;
    const READ_ONLY: &[&str] = &[
        "activity_feed",
        "coworkers_list",
        "cards_list_pending_review",
        "cards_check_pr_state",
    ];
    if READ_ONLY.contains(&tool_name)
        || tool_name.ends_with("_list")
        || tool_name.ends_with("_read")
        || tool_name.ends_with("_search")
        || tool_name.ends_with("_summary")
    {
        return;
    }
    let touches_workspaces_table = tool_name.starts_with("workspaces_")
        || tool_name == "workspace_link_to_repo"
        || tool_name.ends_with("_for_project")
        || tool_name == "cards_create_from_branch";
    if touches_workspaces_table {
        bump("workspace_notes");
    } else if tool_name.starts_with("notes_") {
        bump("workspace_notes");
    } else if tool_name.starts_with("rest_") {
        bump("rest");
    }
}

pub(super) async fn dispatch_tool(
    pool: &SqlitePool,
    app: Option<&tauri::AppHandle>,
    params: Value,
    actor: &str,
) -> Result<Value, (i32, String)> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((-32602, "Missing tool name".into()))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let now = chrono::Utc::now().to_rfc3339();
    let new_id = || uuid::Uuid::new_v4().to_string();
    let str_arg = |key: &str| -> Option<String> {
        args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
    };
    let req_str = |key: &str| -> Result<String, (i32, String)> {
        str_arg(key).ok_or((-32602, format!("{} required", key)))
    };
    let str_array = |key: &str| -> Vec<String> {
        args.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };

    let map_db = |e: sqlx::Error| -> (i32, String) { (-32603, format!("DB error: {}", e)) };

    let result = match name {
        "workspaces_list" => {
            let page = parse_pagination(&args);
            let v = repo::list_workspaces(pool, page).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "workspaces_upsert_for_project" => {
            let project_path = req_str("projectPath")?;
            let ws = upsert_workspace_for_project(pool, &project_path, actor).await?;
            Ok(ok_text(serde_json::to_value(ws).unwrap_or(Value::Null)))
        }
        "notes_create_for_project" => {
            let project_path = req_str("projectPath")?;
            let title = req_str("title")?;
            let content = str_arg("content").unwrap_or_default();
            let tags = str_array("tags");
            let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into());
            let (eff_actor, is_coworker) = resolve_actor(pool, &args, actor).await;
            let ws = upsert_workspace_for_project(pool, &project_path, &eff_actor).await?;
            let id = new_id();
            repo::insert_note(
                pool, &id, &ws.id, &title, &content, &tags_json, None, &eff_actor, &now,
            )
            .await
            .map_err(map_db)?;
            if !is_coworker {
                auto_link_note_to_recent_session(pool, &id, &eff_actor, &now).await;
            }
            let note = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(json!({
                "workspace": ws,
                "note": note
            })))
        }
        "notes_upsert_for_project" => {
            let project_path = req_str("projectPath")?;
            let title = req_str("title")?;
            let new_content = str_arg("content").unwrap_or_default();
            let mode = str_arg("mode").unwrap_or_else(|| "replace".to_string());
            let tags_provided = args.get("tags").is_some();
            let new_tags = str_array("tags");

            let (eff_actor, is_coworker) = resolve_actor(pool, &args, actor).await;
            let ws = upsert_workspace_for_project(pool, &project_path, &eff_actor).await?;
            let existing = repo::find_note_by_title_in_workspace(pool, &ws.id, &title)
                .await
                .map_err(map_db)?;

            let note = match existing {
                None => {
                    // Create — same path as notes_create_for_project.
                    let tags_json = serde_json::to_string(&new_tags)
                        .unwrap_or_else(|_| "[]".into());
                    let id = new_id();
                    repo::insert_note(
                        pool, &id, &ws.id, &title, &new_content, &tags_json,
                        None, &eff_actor, &now,
                    )
                    .await
                    .map_err(map_db)?;
                    let n = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
                    Ok::<_, (i32, String)>((n, true))
                }
                Some(cur) => {
                    // Frozen notes are off-limits to agents — caller must
                    // unfreeze (or change via the UI) before this call works.
                    if repo::is_note_frozen(pool, &cur.id).await.map_err(map_db)? {
                        return Err((-32000, format!("Note '{}' is frozen", cur.title)));
                    }
                    // Update — apply mode, decide tags policy.
                    let merged_content = match mode.as_str() {
                        "append" => {
                            if cur.content.trim().is_empty() {
                                new_content.clone()
                            } else if new_content.trim().is_empty() {
                                cur.content.clone()
                            } else {
                                format!("{}\n\n---\n\n{}", cur.content.trim_end(), new_content)
                            }
                        }
                        // "replace" + any other value falls through to overwrite.
                        _ => new_content.clone(),
                    };
                    let tags_json = if tags_provided {
                        serde_json::to_string(&new_tags).unwrap_or_else(|_| "[]".into())
                    } else {
                        cur.tags.clone()
                    };
                    repo::update_note(
                        pool, &cur.id, &title, &merged_content, &tags_json,
                        cur.linked_session_id.as_deref(), &eff_actor, &now,
                        repo::MutationGuard { respect_frozen: true, expected_updated_at: None },
                    )
                    .await
                    .map_err(map_db)?;
                    let n = repo::get_note_by_id(pool, &cur.id).await.map_err(map_db)?;
                    Ok::<_, (i32, String)>((n, false))
                }
            }?;

            if !is_coworker {
                auto_link_note_to_recent_session(pool, &note.0.id, &eff_actor, &now).await;
            }
            Ok(ok_text(json!({
                "workspace": ws,
                "note": note.0,
                "created": note.1
            })))
        }
        "notes_list" => {
            let workspace_id = req_str("workspaceId")?;
            let page = parse_pagination(&args);
            let v = repo::list_notes_in_workspace(pool, &workspace_id, page).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "notes_read" => {
            let id = req_str("id")?;
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "notes_create" => {
            let workspace_id = req_str("workspaceId")?;
            let title = req_str("title")?;
            let content = str_arg("content").unwrap_or_default();
            let tags = str_array("tags");
            let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into());
            let (eff_actor, is_coworker) = resolve_actor(pool, &args, actor).await;
            let id = new_id();
            repo::insert_note(pool, &id, &workspace_id, &title, &content, &tags_json, None, &eff_actor, &now)
                .await.map_err(map_db)?;
            if !is_coworker {
                auto_link_note_to_recent_session(pool, &id, &eff_actor, &now).await;
            }
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "notes_update" => {
            let id = req_str("id")?;
            // Read existing so we can patch only the fields the agent passed.
            let cur = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            let title = str_arg("title").unwrap_or_else(|| cur.title.clone());
            let content = str_arg("content").unwrap_or_else(|| cur.content.clone());
            let tags_json = if args.get("tags").is_some() {
                serde_json::to_string(&str_array("tags")).unwrap_or("[]".into())
            } else {
                cur.tags.clone()
            };
            let expected_updated_at = str_arg("expectedUpdatedAt");
            let guard = repo::MutationGuard {
                respect_frozen: true,
                expected_updated_at: expected_updated_at.as_deref(),
            };
            let (eff_actor, is_coworker) = resolve_actor(pool, &args, actor).await;
            let rows = repo::update_note(pool, &id, &title, &content, &tags_json,
                cur.linked_session_id.as_deref(), &eff_actor, &now, guard)
                .await.map_err(map_db)?;
            if rows == 0 {
                return Err(diagnose_note_or_err(pool, &id, guard).await);
            }
            if !is_coworker {
                auto_link_note_to_recent_session(pool, &id, &eff_actor, &now).await;
            }
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "boards_list" => {
            let workspace_id = req_str("workspaceId")?;
            let page = parse_pagination(&args);
            let v = repo::list_boards_in_workspace(pool, &workspace_id, page).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "boards_read" => {
            let id = req_str("id")?;
            let cols = repo::list_columns_in_board(pool, &id).await.map_err(map_db)?;
            let cards = repo::list_cards_in_board(pool, &id, repo::Pagination::default()).await.map_err(map_db)?;
            Ok(ok_text(json!({ "columns": cols, "cards": cards })))
        }
        "cards_create" => {
            let column_id = req_str("columnId")?;
            let title = req_str("title")?;
            let description = str_arg("description").unwrap_or_default();
            let priority = str_arg("priority");
            let tags_json = serde_json::to_string(&str_array("tags")).unwrap_or("[]".into());
            let coworker_id = str_arg("coworkerId");
            // Place at end of the destination column.
            let existing_in_col: Vec<_> = repo::list_cards_in_board(pool, &column_id, repo::Pagination::default())
                .await.unwrap_or_default()
                .into_iter().filter(|c| c.column_id == column_id).collect();
            let pos = existing_in_col.len() as i32;
            let id = new_id();
            repo::insert_card(
                pool, &id, &column_id, &title, &description,
                priority.as_deref(), &tags_json, pos,
                None, None, None,
                coworker_id.as_deref(),
                actor, &now,
            )
            .await.map_err(map_db)?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id })))
        }
        "cards_update" => {
            let id = req_str("id")?;
            let row: Option<(String, String, Option<String>, String, Option<String>)> =
                sqlx::query_as(
                    "SELECT title, description, priority, tags, review_checklist \
                     FROM workspace_board_cards WHERE id = ?",
                )
                .bind(&id)
                .fetch_optional(pool)
                .await
                .map_err(map_db)?;
            let (cur_title, cur_desc, cur_prio, cur_tags, cur_check) =
                row.ok_or((-32602, "card not found".into()))?;
            let title = str_arg("title").unwrap_or(cur_title);
            let description = str_arg("description").unwrap_or(cur_desc);
            let priority = if args.get("priority").is_some() { str_arg("priority") } else { cur_prio };
            let tags_json = if args.get("tags").is_some() {
                serde_json::to_string(&str_array("tags")).unwrap_or("[]".into())
            } else {
                cur_tags
            };
            let review_checklist = if args.get("reviewChecklist").is_some() {
                str_arg("reviewChecklist")
            } else {
                cur_check
            };
            let coworker_id = str_arg("coworkerId");
            let expected_updated_at = str_arg("expectedUpdatedAt");
            let guard = repo::MutationGuard {
                respect_frozen: true,
                expected_updated_at: expected_updated_at.as_deref(),
            };
            let rows = repo::update_card(
                pool, &id, &title, &description, priority.as_deref(),
                &tags_json, review_checklist.as_deref(),
                coworker_id.as_deref(),
                actor, &now, guard,
            )
            .await.map_err(map_db)?;
            if rows == 0 {
                return Err(diagnose_card_or_err(pool, &id, guard).await);
            }
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id, "ok": true })))
        }
        "cards_move" => {
            let id = req_str("id")?;
            let column_id = req_str("columnId")?;
            let position = args.get("position").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            // Reuse the same review-flag rule the Tauri command applies:
            // an agent moving to an "In Review" column triggers pending review.
            let review_pending = {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT name FROM workspace_board_columns WHERE id = ?",
                )
                .bind(&column_id)
                .fetch_optional(pool)
                .await
                .map_err(map_db)?;
                match row {
                    Some((name,)) if crate::modes::workspace::commands::is_review_only_column(&name) => 1,
                    _ => 0,
                }
            };
            let guard = repo::MutationGuard { respect_frozen: true, expected_updated_at: None };
            let rows = repo::move_card(pool, &id, &column_id, position, review_pending, actor, &now, guard)
                .await.map_err(map_db)?;
            if rows == 0 {
                return Err(diagnose_card_or_err(pool, &id, guard).await);
            }
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id, "ok": true, "reviewPending": review_pending == 1 })))
        }

        // ---------------------------------------------------------------
        // v1.1 — search, review workflow, summaries, surgical edits,
        // freeze + linking, activity feed.
        // ---------------------------------------------------------------

        "notes_search" => {
            let query = req_str("query")?;
            let workspace_id = str_arg("workspaceId");
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20) as i32;
            let v = repo::search_notes(pool, &query, workspace_id.as_deref(), limit)
                .await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "cards_search" => {
            let query = req_str("query")?;
            let workspace_id = str_arg("workspaceId");
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20) as i32;
            let v = repo::search_cards(pool, &query, workspace_id.as_deref(), limit)
                .await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }

        "cards_approve" => {
            let id = req_str("id")?;
            let guard = repo::MutationGuard { respect_frozen: true, expected_updated_at: None };
            // Approval comment lands as a regular comment row. The
            // semantics ("this was an approval") are conveyed by the
            // tool name + the review_pending=0 transition, not by
            // markup in the body.
            if let Some(comment) = str_arg("comment") {
                if !comment.trim().is_empty() {
                    let cid = new_id();
                    let rows = repo::insert_card_comment(pool, &cid, &id, actor, None, comment.trim(), None, &now, guard)
                        .await.map_err(map_db)?;
                    if rows == 0 {
                        return Err(diagnose_card_or_err(pool, &id, guard).await);
                    }
                }
            }
            let rows = repo::clear_review_pending(pool, &id, actor, &now, guard).await.map_err(map_db)?;
            if rows == 0 {
                return Err(diagnose_card_or_err(pool, &id, guard).await);
            }
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id, "ok": true, "reviewPending": false })))
        }
        "cards_request_changes" => {
            let id = req_str("id")?;
            let guard = repo::MutationGuard { respect_frozen: true, expected_updated_at: None };
            let feedback = req_str("feedback")?;
            let trimmed = feedback.trim();
            if !trimmed.is_empty() {
                let cid = new_id();
                let rows = repo::insert_card_comment(pool, &cid, &id, actor, None, trimmed, None, &now, guard)
                    .await.map_err(map_db)?;
                if rows == 0 {
                    return Err(diagnose_card_or_err(pool, &id, guard).await);
                }
            }
            // Optional move first (so the column-name review check sees
            // the new column), then clear the pending flag explicitly.
            if let Some(column_id) = str_arg("columnId") {
                let position = args.get("position")
                    .and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                // Move with review_pending=0 — we're explicitly leaving review.
                let rows = repo::move_card(pool, &id, &column_id, position, 0, actor, &now, guard)
                    .await.map_err(map_db)?;
                if rows == 0 {
                    return Err(diagnose_card_or_err(pool, &id, guard).await);
                }
            } else {
                let rows = repo::clear_review_pending(pool, &id, actor, &now, guard)
                    .await.map_err(map_db)?;
                if rows == 0 {
                    return Err(diagnose_card_or_err(pool, &id, guard).await);
                }
            }
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id, "ok": true })))
        }
        "cards_list_pending_review" => {
            let workspace_id = str_arg("workspaceId");
            let v = repo::list_review_pending_cards(pool, workspace_id.as_deref())
                .await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }

        "boards_summary" => {
            let board_id = req_str("boardId")?;
            let columns = repo::count_cards_per_column(pool, &board_id)
                .await.map_err(map_db)?;
            let total: i64 = columns.iter().map(|c| c.count).sum();
            let review_pending = repo::count_review_pending_in_board(pool, &board_id)
                .await.map_err(map_db)?;
            Ok(ok_text(json!({
                "boardId": board_id,
                "totalCards": total,
                "reviewPending": review_pending,
                "columns": columns,
            })))
        }
        "workspace_summary" => {
            let workspace_id = req_str("workspaceId")?;
            let workspace = repo::get_workspace_by_id(pool, &workspace_id)
                .await.map_err(map_db)?;
            let boards = repo::list_boards_in_workspace(pool, &workspace_id, repo::Pagination::default())
                .await.map_err(map_db)?;
            let notes = repo::count_notes_in_workspace(pool, &workspace_id)
                .await.map_err(map_db)?;
            let cards = repo::count_cards_in_workspace(pool, &workspace_id)
                .await.map_err(map_db)?;
            let actors = repo::count_recent_edits_by_actor(pool, &workspace_id)
                .await.map_err(map_db)?;
            let mut review_pending = 0i64;
            for b in &boards {
                review_pending += repo::count_review_pending_in_board(pool, &b.id)
                    .await.map_err(map_db)?;
            }
            Ok(ok_text(json!({
                "workspace": workspace,
                "noteCount": notes,
                "cardCount": cards,
                "boardCount": boards.len(),
                "reviewPending": review_pending,
                "actors": actors,
                "boards": boards,
            })))
        }

        "notes_append_section" => {
            let id = req_str("id")?;
            let heading = req_str("heading")?;
            let body = req_str("content")?;
            let cur = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            let section = format!("## {}\n\n{}", heading.trim(), body);
            let merged = if cur.content.trim().is_empty() {
                section
            } else {
                format!("{}\n\n{}", cur.content.trim_end(), section)
            };
            let guard = repo::MutationGuard { respect_frozen: true, expected_updated_at: None };
            let rows = repo::update_note(pool, &id, &cur.title, &merged, &cur.tags,
                cur.linked_session_id.as_deref(), actor, &now, guard)
                .await.map_err(map_db)?;
            if rows == 0 {
                return Err(diagnose_note_or_err(pool, &id, guard).await);
            }
            auto_link_note_to_recent_session(pool, &id, actor, &now).await;
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "notes_apply_diff" => {
            let id = req_str("id")?;
            let find = req_str("find")?;
            let replace = req_str("replace")?;
            let cur = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            // Enforce uniqueness — same rule as Edit tool — so the agent
            // can't silently rewrite the wrong section.
            let occurrences = cur.content.matches(&find).count();
            if occurrences == 0 {
                return Err((-32000, "find string not present in note".into()));
            }
            if occurrences > 1 {
                return Err((-32000, format!(
                    "find string is not unique ({} matches) — pass more surrounding context",
                    occurrences,
                )));
            }
            let next = cur.content.replacen(&find, &replace, 1);
            let guard = repo::MutationGuard { respect_frozen: true, expected_updated_at: None };
            let rows = repo::update_note(pool, &id, &cur.title, &next, &cur.tags,
                cur.linked_session_id.as_deref(), actor, &now, guard)
                .await.map_err(map_db)?;
            if rows == 0 {
                return Err(diagnose_note_or_err(pool, &id, guard).await);
            }
            auto_link_note_to_recent_session(pool, &id, actor, &now).await;
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }

        "cards_create_from_branch" => {
            let branch = req_str("branchName")?;
            // Pull the leaf segment and humanise: "feature/add-user-login"
            // → "Add user login". The full branch goes into externalId so
            // future tooling can correlate.
            let leaf = branch.rsplit('/').next().unwrap_or(&branch);
            let title = {
                let words: Vec<String> = leaf
                    .split(|c: char| c == '-' || c == '_')
                    .filter(|s| !s.is_empty())
                    .enumerate()
                    .map(|(i, w)| {
                        if i == 0 {
                            let mut chars = w.chars();
                            match chars.next() {
                                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                                None => String::new(),
                            }
                        } else {
                            w.to_string()
                        }
                    })
                    .collect();
                let joined = words.join(" ");
                if joined.is_empty() { branch.clone() } else { joined }
            };
            let description = str_arg("description").unwrap_or_default();
            let column_id = if let Some(col) = str_arg("columnId") {
                col
            } else {
                // Need a project path to find a default column.
                let project_path = str_arg("projectPath").ok_or((
                    -32602,
                    "columnId or projectPath required".into(),
                ))?;
                let ws = upsert_workspace_for_project(pool, &project_path, actor).await?;
                let boards = repo::list_boards_in_workspace(pool, &ws.id, repo::Pagination::default())
                    .await.map_err(map_db)?;
                let board = boards.into_iter().next().ok_or((
                    -32603,
                    "workspace has no boards".into(),
                ))?;
                let cols = repo::list_columns_in_board(pool, &board.id)
                    .await.map_err(map_db)?;
                // Prefer a "Todo" column, fall back to the first column.
                let chosen = cols.iter()
                    .find(|c| c.name.eq_ignore_ascii_case("todo"))
                    .or_else(|| cols.first())
                    .ok_or((-32603, "board has no columns".into()))?;
                chosen.id.clone()
            };
            let id = new_id();
            // Position at end of column.
            let existing: Vec<_> = sqlx::query_as::<_, (i64,)>(
                "SELECT COUNT(*) FROM workspace_board_cards WHERE column_id = ?",
            )
            .bind(&column_id)
            .fetch_all(pool)
            .await
            .map_err(map_db)?;
            let pos = existing.first().map(|r| r.0).unwrap_or(0) as i32;
            repo::insert_card(
                pool, &id, &column_id, &title, &description,
                None, "[]", pos,
                Some(&branch), None, None,
                None, // coworker_id — branches don't carry persona context
                actor, &now,
            )
            .await.map_err(map_db)?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({
                "id": id,
                "title": title,
                "branch": branch,
                "columnId": column_id,
            })))
        }

        "notes_link_to_session" => {
            let id = req_str("id")?;
            let session_id = str_arg("sessionId");
            let guard = repo::MutationGuard { respect_frozen: true, expected_updated_at: None };
            let rows = repo::update_note_linked_session(pool, &id, session_id.as_deref(), actor, &now, guard)
                .await.map_err(map_db)?;
            if rows == 0 {
                return Err(diagnose_note_or_err(pool, &id, guard).await);
            }
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "cards_link_to_session" => {
            let id = req_str("id")?;
            let session_id = str_arg("sessionId");
            let guard = repo::MutationGuard { respect_frozen: true, expected_updated_at: None };
            let rows = repo::update_card_linked_session(pool, &id, session_id.as_deref(), actor, &now, guard)
                .await.map_err(map_db)?;
            if rows == 0 {
                return Err(diagnose_card_or_err(pool, &id, guard).await);
            }
            Ok(ok_text(json!({ "id": id, "ok": true })))
        }

        "activity_feed" => {
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(50) as i32;
            // Pull a generous window from the DB then filter in-memory —
            // keeps the SQL trivial; the result set is small (Inbox-class).
            let raw_limit = (limit as usize).saturating_mul(4).max(50) as i32;
            let mut rows = repo::list_inbox(pool, raw_limit).await.map_err(map_db)?;
            if let Some(since) = str_arg("since") {
                rows.retain(|r| r.updated_at.as_str() > since.as_str());
            }
            if let Some(want) = str_arg("actor") {
                rows.retain(|r| r.updated_by == want);
            }
            rows.truncate(limit as usize);
            Ok(ok_text(serde_json::to_value(rows).unwrap_or(Value::Null)))
        }

        "notes_freeze" => {
            let id = req_str("id")?;
            repo::set_note_frozen(pool, &id, 1).await.map_err(map_db)?;
            Ok(ok_text(json!({ "id": id, "frozen": true })))
        }
        "notes_unfreeze" => {
            let id = req_str("id")?;
            repo::set_note_frozen(pool, &id, 0).await.map_err(map_db)?;
            Ok(ok_text(json!({ "id": id, "frozen": false })))
        }
        "cards_freeze" => {
            let id = req_str("id")?;
            repo::set_card_frozen(pool, &id, 1).await.map_err(map_db)?;
            Ok(ok_text(json!({ "id": id, "frozen": true })))
        }
        "cards_unfreeze" => {
            let id = req_str("id")?;
            repo::set_card_frozen(pool, &id, 0).await.map_err(map_db)?;
            Ok(ok_text(json!({ "id": id, "frozen": false })))
        }

        "workspace_link_to_repo" => {
            let workspace_id = req_str("workspaceId")?;
            let repo_url = str_arg("repoUrl");
            repo::set_workspace_repo_url(pool, &workspace_id, repo_url.as_deref(), actor, &now)
                .await.map_err(map_db)?;
            let ws = repo::get_workspace_by_id(pool, &workspace_id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(ws).unwrap_or(Value::Null)))
        }

        "cards_add_comment" => {
            let id = req_str("id")?;
            let body = req_str("body")?;
            let trimmed = body.trim();
            if trimmed.is_empty() {
                return Err((-32602, "body must not be empty".into()));
            }
            let coworker_id = str_arg("coworkerId");
            let comment_id = new_id();
            let guard = repo::MutationGuard { respect_frozen: true, expected_updated_at: None };
            let rows = repo::insert_card_comment(
                pool, &comment_id, &id, actor,
                coworker_id.as_deref(),
                trimmed, None, &now, guard,
            )
            .await.map_err(map_db)?;
            if rows == 0 {
                return Err(diagnose_card_or_err(pool, &id, guard).await);
            }
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({
                "id": comment_id,
                "cardId": id,
                "actor": actor,
                "coworkerId": coworker_id,
                "body": trimmed,
                "createdAt": now,
            })))
        }

        "cards_push_to_repo" => {
            let id = req_str("id")?;
            // Frozen → no push (would mutate externalId/url).
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let result = crate::modes::workspace::push::push_card_to_repo(pool, &id, actor)
                .await
                .map_err(|e| (-32603, e))?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(result))
        }

        "cards_commit" => {
            let id = req_str("cardId")?;
            let message = req_str("message")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let result = crate::modes::workspace::pr::commit_card(pool, app, &id, &message, actor)
                .await
                .map_err(|e| (-32603, e.message()))?;
            Ok(ok_text(serde_json::to_value(result).unwrap_or(Value::Null)))
        }

        "cards_raise_pr" => {
            let id = req_str("cardId")?;
            let title = str_arg("title");
            let body = str_arg("body");
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let result = crate::modes::workspace::pr::raise_or_update_pr(
                pool, app, &id, title.as_deref(), body.as_deref(), actor,
            )
            .await
            .map_err(|e| (-32603, e.message()))?;
            Ok(ok_text(serde_json::to_value(result).unwrap_or(Value::Null)))
        }

        "cards_check_pr_state" => {
            let id = req_str("cardId")?;
            let state = crate::modes::workspace::pr::check_pr_state(pool, &id)
                .await
                .map_err(|e| (-32603, e.message()))?;
            Ok(ok_text(serde_json::json!({ "cardId": id, "state": state })))
        }

        "cards_link_pr" => {
            let id = req_str("cardId")?;
            let pr_url = req_str("prUrl")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            crate::modes::workspace::pr::link_pr_url(pool, app, &id, &pr_url, actor)
                .await
                .map_err(|e| (-32603, e.message()))?;
            Ok(ok_text(json!({ "cardId": id, "prUrl": pr_url })))
        }

        "coworkers_list" => {
            let v = crate::shared::repos::coworkers::list_coworkers(pool)
                .await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }

        "cards_claim" => {
            // Terminal-side claim. We don't get the agent's session id
            // over the wire, so we use the same auto-link heuristic to
            // resolve "the most recent session for this card's project"
            // and stamp it as the claimer. claim_card is atomic — only
            // one parallel call wins; the rest fall through to the
            // diagnostic branch which decides "same session" (idempotent
            // success) vs "different session" (error).
            let id = req_str("id")?;
            let row = repo::get_card_claim_and_project(pool, &id)
                .await.map_err(map_db)?
                .ok_or((-32602, "Card not found".into()))?;
            let (_existing_session, _existing_coworker, project_path_opt) = row;
            let project_path = project_path_opt
                .filter(|p| !p.trim().is_empty())
                .ok_or((-32602, "Workspace has no project_path bound".into()))?;
            let calling_session = session_repo::find_recent_session_for_project(pool, &project_path)
                .await.map_err(map_db)?
                .ok_or((-32603, "No agent session found for this project — start one in Agent mode first".into()))?;
            // Terminal claims have no persona — pass None for coworker_id.
            let claimed = repo::claim_card(pool, &id, &calling_session.id, None, actor, &now)
                .await.map_err(map_db)?;
            if !claimed {
                // Re-read to decide: same session (idempotent) vs different (error).
                let cur = repo::get_card_claim_and_project(pool, &id)
                    .await.map_err(map_db)?
                    .ok_or((-32602, "Card not found".into()))?;
                match cur.0 {
                    Some(sid) if sid == calling_session.id => { /* idempotent — fall through */ }
                    Some(_) => {
                        return Err((-32000, format!(
                            "Card is already claimed by another session. Release it first.",
                        )));
                    }
                    None => {
                        // Either frozen or a fresh race we lost.
                        return Err(diagnose_card_or_err(
                            pool, &id,
                            repo::MutationGuard { respect_frozen: true, expected_updated_at: None },
                        ).await);
                    }
                }
            }
            Ok(ok_text(json!({
                "id": id,
                "claimedSessionId": calling_session.id,
                "sessionTitle": calling_session.title,
            })))
        }

        "cards_release" => {
            let id = req_str("id")?;
            let row = repo::get_card_claim_and_project(pool, &id)
                .await.map_err(map_db)?
                .ok_or((-32602, "Card not found".into()))?;
            let (existing_session, _existing_coworker, project_path_opt) = row;
            let project_path = project_path_opt.unwrap_or_default();
            let calling_session = session_repo::find_recent_session_for_project(pool, &project_path)
                .await.map_err(map_db)?;
            // Only release if the caller is the holder — otherwise it's a no-op
            // so a misfire from another session can't inadvertently unlock the card.
            if let (Some(holder), Some(caller)) = (existing_session.as_deref(), calling_session.as_ref()) {
                if holder == caller.id {
                    repo::release_card(pool, &id, actor, &now)
                        .await.map_err(map_db)?;
                }
            }
            Ok(ok_text(json!({ "id": id, "ok": true })))
        }

        "cards_start_work" => {
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let r = crate::modes::workspace::agent_spawn::start_work(pool, &id, actor)
                .await
                .map_err(|e| (-32603, e))?;
            Ok(ok_text(json!({
                "ok": true,
                "worktreePath": r.worktree_path,
                "worktreeBranch": r.worktree_branch,
            })))
        }

        "cards_call_coworker" => {
            // Terminal-callable wrapper around drawer_chat_turn — lets the
            // calling agent (e.g. Claude in the user's terminal) invoke a
            // specific persona on a card. The persona's reply is persisted
            // as a comment AND returned in the result so the caller can
            // summarise back to the user.
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let coworker_id = req_str("coworkerId")?;
            let message = req_str("message")?;
            // Pass the Tauri AppHandle through (when available) so any
            // open drawer for this card refreshes live via the
            // `workspace:card-updated` event. Falls back to next-poll
            // refresh when MCP started before Tauri was ready.
            let result = crate::modes::workspace::agent_spawn::drawer_chat_turn(
                pool, app, &id, &coworker_id, &message, actor,
            )
            .await
            .map_err(|e| (-32603, e))?;
            Ok(ok_text(json!({
                "ok": true,
                "sessionId": result.session_id,
                "userCommentId": result.user_comment.id,
                "replyCommentId": result.reply_comment.as_ref().map(|c| c.id.clone()),
                "response": result.reply_comment.as_ref().map(|c| c.body.clone()),
                "agentError": result.agent_error,
            })))
        }

        // ── REST mode CRUD ────────────────────────────────────────
        // Lets an agent sync API endpoints from a project's code into
        // Clauge's REST mode. Five primitives — agent decides which
        // combination matches the user's ask (add to existing
        // collection, create+add to new collection, etc.).
        "rest_collections_list" => {
            let v = crate::shared::repos::collections::list_all(pool)
                .await
                .map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "rest_collection_create" => {
            let collection_name = req_str("name")?;
            if collection_name.trim().is_empty() {
                return Err((-32602, "name cannot be empty".into()));
            }
            let id = new_id();
            let max_order = crate::shared::repos::collections::max_sort_order(pool)
                .await
                .map_err(map_db)?;
            crate::shared::repos::collections::insert(pool, &id, collection_name.trim(), max_order.0 + 1)
                .await
                .map_err(map_db)?;
            // Optional description — if the agent passed one, patch it
            // in. The existing `insert` repo helper doesn't take
            // description (UI never sets it on create either), so we
            // do a focused UPDATE rather than reshape the repo API.
            if let Some(desc) = str_arg("description").filter(|s| !s.is_empty()) {
                sqlx::query("UPDATE collections SET description = ?, updated_at = datetime('now') WHERE id = ?")
                    .bind(&desc)
                    .bind(&id)
                    .execute(pool)
                    .await
                    .map_err(map_db)?;
            }
            emit_rest_changed(app, "collections", None);
            let row = crate::shared::repos::collections::get_by_id(pool, &id)
                .await
                .map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(row).unwrap_or(Value::Null)))
        }
        "rest_requests_list" => {
            let collection_id = req_str("collectionId")?;
            let v = crate::shared::repos::requests::list_by_collection(pool, &collection_id)
                .await
                .map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "rest_request_create" => {
            let collection_id = req_str("collectionId")?;
            let request_name = req_str("name")?;
            if request_name.trim().is_empty() {
                return Err((-32602, "name cannot be empty".into()));
            }
            // Verify the collection exists upfront so we can return a
            // friendly error instead of a foreign-key violation later.
            crate::shared::repos::collections::get_by_id(pool, &collection_id)
                .await
                .map_err(|_| (-32602, format!("collectionId {} does not exist", collection_id)))?;
            let method = req_str("method")?.trim().to_uppercase();
            if method.is_empty() {
                return Err((-32602, "method cannot be empty".into()));
            }
            let url = str_arg("url").unwrap_or_default();
            let body = str_arg("body").unwrap_or_default();
            // body_type default: 'none' when there's no body, 'json' when
            // there is one but the caller didn't pick a type. Storing
            // 'none' alongside a non-empty body would hide the body
            // editor in the UI — user would see the saved body but have
            // no way to edit it. 'json' matches the DB-level default and
            // is the common case for agent-created APIs.
            let body_type = str_arg("bodyType")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| if body.is_empty() { "none" } else { "json" }.to_string());
            let auth_type = str_arg("authType")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "none".to_string());
            // auth_data must be valid JSON — the UI does
            // `JSON.parse(auth_data)` to render the auth editor. '{}'
            // matches the schema default; we validate-parse any caller-
            // provided value to catch garbage before it lands in the DB
            // and crashes the UI later.
            let auth_data = match str_arg("authData") {
                Some(s) if !s.is_empty() => {
                    serde_json::from_str::<serde_json::Value>(&s)
                        .map_err(|e| (-32602, format!("authData must be valid JSON: {}", e)))?;
                    s
                }
                _ => "{}".to_string(),
            };
            let pre_script = String::new();
            let description = str_arg("description").unwrap_or_default();

            let id = new_id();
            let max_order = crate::shared::repos::requests::max_sort_order(pool, &collection_id)
                .await
                .map_err(map_db)?;
            crate::shared::repos::requests::insert_full(
                pool,
                &id,
                &collection_id,
                request_name.trim(),
                &description,
                &method,
                &url,
                &body,
                &body_type,
                &auth_type,
                &auth_data,
                &pre_script,
                max_order.0 + 1,
            )
            .await
            .map_err(map_db)?;
            // Headers + query params, if supplied. Each entry is a
            // {key,value,enabled?} object; `enabled` defaults to true so
            // the agent doesn't have to spell it out for every row.
            apply_kv_list(pool, &id, args.get("headers"), KvKind::Header).await?;
            apply_kv_list(pool, &id, args.get("queryParams"), KvKind::Param).await?;
            emit_rest_changed(app, "requests", Some(&collection_id));
            let row = crate::shared::repos::requests::get_by_id(pool, &id)
                .await
                .map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(row).unwrap_or(Value::Null)))
        }
        "rest_request_update" => {
            let id = req_str("id")?;
            // Existence check before any mutation. Without this an
            // update on a wrong id silently no-ops then the post-update
            // get_by_id throws "no rows" — a confusing error to surface
            // to an agent.
            let existing = crate::shared::repos::requests::get_by_id(pool, &id)
                .await
                .map_err(|_| (-32602, format!("request id {} does not exist", id)))?;
            // Build dynamic SET clause from any of the simple fields
            // the caller provided. Omitting a field leaves it
            // untouched (matches the existing UI behaviour).
            // authData gets JSON-validated up-front so we don't
            // poison the DB with un-parseable strings.
            if let Some(s) = str_arg("authData") {
                if !s.is_empty() {
                    serde_json::from_str::<serde_json::Value>(&s)
                        .map_err(|e| (-32602, format!("authData must be valid JSON: {}", e)))?;
                }
            }
            let mut sets: Vec<String> = Vec::new();
            let mut values: Vec<String> = Vec::new();
            for (k, col) in [
                ("name", "name"),
                ("method", "method"),
                ("url", "url"),
                ("body", "body"),
                ("bodyType", "body_type"),
                ("authType", "auth_type"),
                ("authData", "auth_data"),
                ("description", "description"),
            ] {
                if let Some(v) = str_arg(k) {
                    let v = if k == "method" { v.trim().to_uppercase() } else { v };
                    sets.push(format!("{} = ?", col));
                    values.push(v);
                }
            }
            if !sets.is_empty() {
                crate::shared::repos::requests::update_dynamic(pool, &sets, &values, &id)
                    .await
                    .map_err(map_db)?;
            }
            if args.get("headers").is_some() {
                apply_kv_list(pool, &id, args.get("headers"), KvKind::Header).await?;
            }
            if args.get("queryParams").is_some() {
                apply_kv_list(pool, &id, args.get("queryParams"), KvKind::Param).await?;
            }
            emit_rest_changed(app, "requests", Some(&existing.collection_id));
            let row = crate::shared::repos::requests::get_by_id(pool, &id)
                .await
                .map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(row).unwrap_or(Value::Null)))
        }

        _ => Err((-32601, format!("Tool not found: {}", name))),
    };
    if result.is_ok() {
        bump_sync_kinds_for_tool(name);
    }
    result
}

/// Emit a Tauri event so the REST mode's frontend stores re-fetch.
/// Without this, an agent-driven create/update via MCP doesn't reach
/// the UI until the user manually reloads — the existing Tauri
/// commands skip this because the frontend already refreshes itself
/// after its own calls, but MCP bypasses that loop entirely.
fn emit_rest_changed(
    app: Option<&tauri::AppHandle>,
    kind: &str,
    collection_id: Option<&str>,
) {
    use tauri::Emitter;
    if let Some(a) = app {
        let _ = a.emit(
            "rest:changed",
            serde_json::json!({ "kind": kind, "collectionId": collection_id }),
        );
    }
}

enum KvKind { Header, Param }

/// Replace the header/param list for a request with the supplied
/// array. None / null / non-array → no-op. Matches the existing UI's
/// "delete and re-insert in order" semantics, including `enabled`
/// defaulting to true.
async fn apply_kv_list(
    pool: &SqlitePool,
    request_id: &str,
    value: Option<&Value>,
    kind: KvKind,
) -> Result<(), (i32, String)> {
    let map_db = |e: sqlx::Error| -> (i32, String) { (-32603, format!("DB error: {}", e)) };
    let arr = match value.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Ok(()),
    };
    match kind {
        KvKind::Header => crate::shared::repos::requests::delete_headers_for_request(pool, request_id)
            .await
            .map_err(map_db)?,
        KvKind::Param => crate::shared::repos::requests::delete_params_for_request(pool, request_id)
            .await
            .map_err(map_db)?,
    }
    for (i, item) in arr.iter().enumerate() {
        let key = item.get("key").and_then(|v| v.as_str()).unwrap_or("");
        let val = item.get("value").and_then(|v| v.as_str()).unwrap_or("");
        if key.is_empty() {
            continue;
        }
        let enabled_bool = item
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let enabled = if enabled_bool { 1 } else { 0 };
        let row_id = uuid::Uuid::new_v4().to_string();
        match kind {
            KvKind::Header => crate::shared::repos::requests::insert_header(
                pool, &row_id, request_id, key, val, enabled, i as i32,
            )
            .await
            .map_err(map_db)?,
            KvKind::Param => crate::shared::repos::requests::insert_param(
                pool, &row_id, request_id, key, val, enabled, i as i32,
            )
            .await
            .map_err(map_db)?,
        }
    }
    Ok(())
}

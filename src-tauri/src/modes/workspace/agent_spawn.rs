// Card-driven chat + work-stream lifecycle. Three flows live here:
//
//   1. drawer_chat_turn   — drawer @-box: take a (card, coworker) pair,
//                           create-or-reuse a hidden session for that
//                           pair, post user comment, run claude -p
//                           with the coworker's system_prompt
//                           appended, post reply.
//   2. start_work         — promote a claimed card to "code mode" by
//                           creating a worktree + branch on the
//                           hidden session.
//   3. release_card       — unclaim the card (and optionally delete
//                           the worktree).
//
// All three emit `workspace:card-updated` Tauri events after persisting
// changes so any open drawer / kanban view can refresh without polling.

use serde_json::json;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::modes::workspace::models::WorkspaceCoworker;
use crate::shared::repos::{coworkers as coworker_repo, sessions as session_repo, workspaces as repo};

const ONESHOT_TIMEOUT_SECS: u64 = 300;
const PROMPT_MAX_BYTES: usize = 64 * 1024;

const CARD_UPDATED_EVENT: &str = "workspace:card-updated";

fn emit_updated(app: Option<&AppHandle>, card_id: &str) {
    // MCP-initiated calls (e.g. `cards_call_coworker`) don't carry a
    // Tauri handle — open drawers will refresh via their next poll
    // instead. Drawer-initiated calls always pass Some(&app).
    if let Some(a) = app {
        let _ = a.emit(CARD_UPDATED_EVENT, json!({ "cardId": card_id }));
    }
}

// ─── drawer_chat_turn ──────────────────────────────────────────────

pub async fn drawer_chat_turn(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    card_id: &str,
    coworker_id: &str,
    body: &str,
    actor: &str,
) -> Result<super::commands::DrawerChatResult, String> {
    // 0. Resolve coworker (errors if deleted between picker open + send).
    let coworker = coworker_repo::get_coworker(pool, coworker_id)
        .await
        .map_err(|e| format!("Coworker not found: {e}"))?;

    // 1. Resolve card → claim + project.
    let (claim_session, claim_coworker, project_path) = resolve_claim_and_project(pool, card_id).await?;

    // 2. Resolve the hidden session for (card, coworker).
    //    Three cases:
    //      a) Same coworker holds the claim → reuse the session
    //      b) Different coworker holds it (drawer-side switch) →
    //         release the old claim, find-or-create new one. UX
    //         confirmation lives in the frontend; by the time we're
    //         here, the user has already approved the switch.
    //      c) Manual terminal session holds it → still refuse (the
    //         drawer can't take over a terminal-driven work-stream
    //         without a clean handoff)
    //      d) Unclaimed → new session + claim
    let session = match (claim_session, claim_coworker) {
        (Some(claimed_session_id), Some(claimed_cw)) if claimed_cw == coworker_id => {
            // (a) Same coworker — reuse.
            session_repo::get_session_by_id(pool, &claimed_session_id)
                .await
                .map_err(|e| format!("Linked session not found: {e}"))?
        }
        (Some(_claimed_session_id), Some(_claimed_cw)) => {
            // (b) Different coworker — auto-switch. Release the old
            //     claim (worktree on disk persists for resume), then
            //     find-or-create the new (card, coworker) session.
            let now = chrono::Utc::now().to_rfc3339();
            repo::release_card(pool, card_id, actor, &now)
                .await
                .map_err(|e| format!("DB error releasing prior claim: {e}"))?;
            create_hidden_session_and_claim(pool, card_id, &coworker, &project_path, actor).await?
        }
        (Some(claimed_session_id), None) => {
            // (c) Manual terminal session has the card.
            let s = session_repo::get_session_by_id(pool, &claimed_session_id)
                .await
                .map_err(|e| format!("Linked session not found: {e}"))?;
            return Err(format!(
                "This card is active in terminal session '{}'. Switch to that session, \
                 or End the work-stream from the drawer first.",
                s.title
            ));
        }
        (None, _) => {
            // (d) Unclaimed — new (card, coworker) hidden session, then claim.
            create_hidden_session_and_claim(pool, card_id, &coworker, &project_path, actor).await?
        }
    };

    // 3. Persist user comment first so the trail survives an agent failure.
    let now = chrono::Utc::now().to_rfc3339();
    let user_comment_id = uuid::Uuid::new_v4().to_string();
    repo::insert_card_comment(
        pool,
        &user_comment_id,
        card_id,
        actor,
        Some(coworker_id),    // user comment is in this coworker's thread
        body,
        None,
        &now,
        repo::MutationGuard::default(),
    )
    .await
    .map_err(|e| format!("DB error inserting user comment: {e}"))?;
    emit_updated(app, card_id);

    let user_comment = crate::modes::workspace::models::WorkspaceCardComment {
        id: user_comment_id,
        card_id: card_id.to_string(),
        actor: actor.to_string(),
        coworker_id: Some(coworker_id.to_string()),
        body: body.to_string(),
        parent_id: None,
        created_at: now.clone(),
    };

    // 4. Build prompt — card body + thread (filtered to this coworker's
    //    history + plain user comments) + new user message.
    let card_meta = sqlx::query_as::<_, (String, String)>(
        "SELECT title, description FROM workspace_board_cards WHERE id = ?",
    )
    .bind(card_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error reading card meta: {e}"))?;
    let prior = repo::list_card_comments(pool, card_id)
        .await
        .map_err(|e| format!("DB error reading thread: {e}"))?;
    let prompt = build_prompt(&card_meta.0, &card_meta.1, &prior, body);
    let truncated = truncate_to_bytes(&prompt, PROMPT_MAX_BYTES);

    // 5. Spawn claude. Worktree if Start work was pressed; project_path otherwise.
    let cwd = session
        .worktree_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .unwrap_or(&session.project_path)
        .to_string();
    // Worktree info is injected into the persona prompt so the agent
    // explicitly knows which branch / directory to work in. Without
    // this, the agent has to introspect via `git branch --show-current`
    // — workable but easy to drift on (e.g. agent runs `git checkout`
    // in mid-conversation and loses the thread).
    let worktree = match (session.worktree_path.as_deref(), session.worktree_branch.as_deref()) {
        (Some(p), Some(b)) if !p.is_empty() && !b.is_empty() => Some((p, b)),
        _ => None,
    };
    let argv = oneshot_argv(
        &coworker,
        card_id,
        &truncated,
        session.claude_session_id.as_deref(),
        worktree,
    );
    if argv.is_empty() {
        return Ok(super::commands::DrawerChatResult {
            user_comment,
            reply_comment: None,
            session_id: session.id.clone(),
            agent_error: Some(format!(
                "Provider '{}' is not yet wired for chat.",
                coworker.provider
            )),
        });
    }
    // Pre-flight: confirm the CLI binary resolves on the user's PATH
    // (with rc-file additions sourced — see shared::platform::path).
    // Bundled .app processes inherit a stripped launchd PATH, so a
    // bare `Command::new("claude")` would fail NotFound even when the
    // binary is reachable from a terminal.
    let bin = &argv[0];
    let resolved_bin = crate::shared::platform::path::find_binary(bin);
    let Some(resolved_bin) = resolved_bin else {
        return Ok(soft_err(user_comment, &session.id, format!(
            "{bin} is not installed or not on PATH. Install it from \
             https://claude.com/claude-code and retry."
        )));
    };

    let argv_owned = argv.clone();
    let bin_path = resolved_bin.clone();
    let cwd_owned = cwd.clone();
    let provider_owned = coworker.provider.clone();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(ONESHOT_TIMEOUT_SECS),
        tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&bin_path);
            cmd.args(&argv_owned[1..]).current_dir(&cwd_owned);
            crate::shared::platform::path::apply_user_path(&mut cmd);
            cmd.output()
        }),
    )
    .await;

    let output = match output {
        Err(_) => return Ok(soft_err(user_comment, &session.id, format!(
            "{} timed out after {}s — agent may be stuck", provider_owned, ONESHOT_TIMEOUT_SECS
        ))),
        Ok(Err(e))         => return Ok(soft_err(user_comment, &session.id, format!("spawn_blocking failed: {e}"))),
        Ok(Ok(Err(e)))     => return Ok(soft_err(user_comment, &session.id, format!("{} failed to spawn: {e}", provider_owned))),
        Ok(Ok(Ok(o)))      => o,
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let lower = stderr.to_lowercase();
        let friendly = if lower.contains("auth") || lower.contains("logged in") || lower.contains("token") {
            format!("{provider_owned} is not authenticated. Run `{provider_owned} /login` and retry.")
        } else if stderr.is_empty() {
            format!("{provider_owned} exited with non-zero status (no stderr)")
        } else {
            stderr
        };
        return Ok(soft_err(user_comment, &session.id, friendly));
    }

    let response = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if response.is_empty() {
        return Ok(soft_err(user_comment, &session.id, format!("{provider_owned} returned an empty response")));
    }

    // 6. Persist reply attributed to the coworker (actor = persona name).
    let reply_now = chrono::Utc::now().to_rfc3339();
    let reply_id = uuid::Uuid::new_v4().to_string();
    let reply_actor = coworker.name.clone(); // bubble shows the persona name
    repo::insert_card_comment(
        pool,
        &reply_id,
        card_id,
        &reply_actor,
        Some(coworker_id),
        &response,
        None,
        &reply_now,
        repo::MutationGuard::default(),
    )
    .await
    .map_err(|e| format!("DB error inserting agent reply: {e}"))?;
    let _ = session_repo::update_session_last_used(pool, &session.id, &reply_now).await;

    // 6b. Server-side Todo → In Review fallback. Persona prompt asks
    //     the agent to move the card itself, but Claude follows ~70%
    //     of the time. Belt-and-braces: if the card is sitting in a
    //     Todo-class column AND the board has an "In Review" column,
    //     move it. Best-effort; failures are logged but never fail
    //     the turn.
    let _ = auto_advance_to_active(pool, card_id, &reply_actor, &reply_now).await;

    emit_updated(app, card_id);

    Ok(super::commands::DrawerChatResult {
        user_comment,
        reply_comment: Some(crate::modes::workspace::models::WorkspaceCardComment {
            id: reply_id,
            card_id: card_id.to_string(),
            actor: reply_actor,
            coworker_id: Some(coworker_id.to_string()),
            body: response,
            parent_id: None,
            created_at: reply_now,
        }),
        session_id: session.id,
        agent_error: None,
    })
}

fn soft_err(
    user_comment: crate::modes::workspace::models::WorkspaceCardComment,
    session_id: &str,
    msg: String,
) -> super::commands::DrawerChatResult {
    super::commands::DrawerChatResult {
        user_comment,
        reply_comment: None,
        session_id: session_id.to_string(),
        agent_error: Some(msg),
    }
}

// ─── start_work ────────────────────────────────────────────────────

pub async fn start_work(
    pool: &SqlitePool,
    card_id: &str,
    actor: &str,
) -> Result<super::commands::StartWorkResult, String> {
    let (claim_session, claim_coworker, project_path) = resolve_claim_and_project(pool, card_id).await?;
    let session_id = claim_session.ok_or_else(|| {
        "Card isn't claimed yet. Pick a coworker and start a chat first.".to_string()
    })?;
    let session = session_repo::get_session_by_id(pool, &session_id)
        .await
        .map_err(|e| format!("Claim points at a missing session: {e}"))?;
    if session.origin != "card" {
        return Err("Card is owned by a manual terminal session — manage its worktree from Agent mode.".into());
    }

    if let (Some(p), Some(b)) = (session.worktree_path.as_ref(), session.worktree_branch.as_ref()) {
        if !p.is_empty() {
            return Ok(super::commands::StartWorkResult {
                worktree_path: p.clone(),
                worktree_branch: b.clone(),
            });
        }
    }

    // Branch name = card/<short>-<coworker>-<title-slug> when persona known,
    // else card/<short>-<title-slug>. Stays unique across personas working
    // the same card serially.
    let title = sqlx::query_as::<_, (String,)>("SELECT title FROM workspace_board_cards WHERE id = ?")
        .bind(card_id).fetch_one(pool).await
        .map_err(|e| format!("DB error reading card title: {e}"))?
        .0;
    let short_id: String = card_id.chars().take(6).collect();
    let title_slug = slugify(&title);
    let branch = match claim_coworker {
        Some(cw_id) => match coworker_repo::get_coworker(pool, &cw_id).await {
            Ok(cw) => format!("card/{}-{}-{}", short_id, slugify(&cw.name), title_slug),
            Err(_) => format!("card/{short_id}-{title_slug}"),
        },
        None => format!("card/{short_id}-{title_slug}"),
    };

    let project_path_owned = project_path.clone();
    let branch_owned = branch.clone();
    let worktree_path = tokio::task::spawn_blocking(move || {
        crate::modes::agent::worktree::agent_create_worktree(project_path_owned, branch_owned)
    })
    .await
    .map_err(|e| format!("worktree spawn_blocking failed: {e}"))??;

    sqlx::query(
        "UPDATE agent_sessions SET worktree_path = ?, worktree_branch = ? WHERE id = ?",
    )
    .bind(&worktree_path)
    .bind(&branch)
    .bind(&session.id)
    .execute(pool)
    .await
    .map_err(|e| format!("DB error stamping worktree on session: {e}"))?;

    let now = chrono::Utc::now().to_rfc3339();
    let cid = uuid::Uuid::new_v4().to_string();
    let body = format!(
        "**Work started.**\n\n- Branch: `{branch}`\n- Worktree: `{worktree_path}`\n\nFurther \
         agent runs on this card will use this worktree."
    );
    let _ = repo::insert_card_comment(pool, &cid, card_id, actor, None, &body, None, &now, repo::MutationGuard::default()).await;

    Ok(super::commands::StartWorkResult {
        worktree_path,
        worktree_branch: branch,
    })
}

// ─── release_card ──────────────────────────────────────────────────

pub async fn release_card(
    pool: &SqlitePool,
    card_id: &str,
    actor: &str,
    delete_worktree: bool,
) -> Result<(), String> {
    let (claim_session, _claim_coworker, project_path) = resolve_claim_and_project(pool, card_id).await?;
    let session_id = match claim_session {
        Some(s) => s,
        None => return Ok(()), // already released; idempotent
    };
    let session = session_repo::get_session_by_id(pool, &session_id).await.ok();

    if delete_worktree {
        if let Some(s) = session.as_ref() {
            if let Some(wt) = s.worktree_path.as_ref().filter(|p| !p.is_empty()) {
                let proj = project_path.clone();
                let wt_owned = wt.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    crate::modes::agent::worktree::agent_remove_worktree(proj, wt_owned)
                })
                .await;
                let _ = sqlx::query(
                    "UPDATE agent_sessions SET worktree_path = NULL, worktree_branch = NULL WHERE id = ?",
                )
                .bind(&s.id)
                .execute(pool)
                .await;
            }
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    repo::release_card(pool, card_id, actor, &now)
        .await
        .map_err(|e| format!("DB error releasing card: {e}"))?;
    Ok(())
}

// ─── helpers ───────────────────────────────────────────────────────

async fn resolve_claim_and_project(
    pool: &SqlitePool,
    card_id: &str,
) -> Result<(Option<String>, Option<String>, String), String> {
    let row = repo::get_card_claim_and_project(pool, card_id)
        .await
        .map_err(|e| format!("DB error reading card: {e}"))?
        .ok_or_else(|| "Card not found".to_string())?;
    let project_path = row
        .2
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| "Workspace has no project path bound — set one before chatting.".to_string())?;
    Ok((row.0, row.1, project_path))
}

/// If the card lives in a "Todo" or "Backlog" column AND the same
/// board has an "In Review" column, move the card there at position
/// 0. Columns are seeded from `repo::DEFAULT_BOARD_COLUMNS` and can't
/// be renamed, so exact-name matching is enough.
async fn auto_advance_to_active(
    pool: &SqlitePool,
    card_id: &str,
    actor: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT col.id, col.name, col.board_id \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         WHERE c.id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await?;
    let (cur_col_id, cur_col_name, board_id) = match row {
        Some(r) => r,
        None => return Ok(()),
    };
    if cur_col_name != "Todo" && cur_col_name != "Backlog" {
        return Ok(());
    }
    let target: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM workspace_board_columns WHERE board_id = ? AND name = 'In Review'",
    )
    .bind(&board_id)
    .fetch_optional(pool)
    .await?;
    let target_id = match target {
        Some((id,)) => id,
        None => return Ok(()),
    };
    if target_id == cur_col_id {
        return Ok(());
    }
    repo::move_card(pool, card_id, &target_id, 0, 0, actor, now, repo::MutationGuard::default()).await?;
    Ok(())
}

async fn create_hidden_session_and_claim(
    pool: &SqlitePool,
    card_id: &str,
    coworker: &WorkspaceCoworker,
    project_path: &str,
    actor: &str,
) -> Result<crate::modes::agent::models::AgentSession, String> {
    // First: maybe a previous (card, coworker) session already exists from a
    // prior chat that was released — reuse it so Claude's resume works.
    if let Ok(Some(existing)) = session_repo::find_hidden_session_for_card_and_coworker(
        pool, card_id, &coworker.id,
    ).await {
        let now = chrono::Utc::now().to_rfc3339();
        let claimed = repo::claim_card(pool, card_id, &existing.id, Some(&coworker.id), actor, &now)
            .await
            .map_err(|e| format!("DB error claiming card: {e}"))?;
        if !claimed {
            return Err(
                "Card was claimed by another session between the lookup and the claim. Try again.".into(),
            );
        }
        return Ok(existing);
    }

    let now = chrono::Utc::now().to_rfc3339();
    let session_id = uuid::Uuid::new_v4().to_string();
    let project_name = std::path::Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();
    let card_title = sqlx::query_as::<_, (String,)>(
        "SELECT title FROM workspace_board_cards WHERE id = ?",
    )
    .bind(card_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error reading card title: {e}"))?
    .0;
    let title = format!("Card · {} · @{}", card_title, coworker.name);

    session_repo::insert_hidden_session(
        pool,
        &session_id,
        &title,
        project_path,
        &project_name,
        card_id,
        &coworker.id,
        &now,
        &now,
    )
    .await
    .map_err(|e| format!("DB error creating hidden session: {e}"))?;

    let claimed = repo::claim_card(pool, card_id, &session_id, Some(&coworker.id), actor, &now)
        .await
        .map_err(|e| format!("DB error claiming card: {e}"))?;
    if !claimed {
        return Err(
            "Card was claimed by another session between the session insert and the claim. Try again.".into(),
        );
    }

    session_repo::get_session_by_id(pool, &session_id)
        .await
        .map_err(|e| format!("DB error reading new session: {e}"))
}

fn build_prompt(
    card_title: &str,
    card_desc: &str,
    prior: &[crate::modes::workspace::models::WorkspaceCardComment],
    user_msg: &str,
) -> String {
    let mut prompt = format!("Card: {card_title}\n");
    if !card_desc.trim().is_empty() {
        prompt.push_str("\nDescription:\n");
        prompt.push_str(card_desc.trim());
        prompt.push('\n');
    }
    if !prior.is_empty() {
        prompt.push_str("\nPrior thread (oldest first):\n");
        for c in prior {
            prompt.push_str(&format!("{}: {}\n", c.actor, c.body));
        }
    }
    prompt.push_str("\nNew user message: ");
    prompt.push_str(user_msg);
    prompt.push_str(
        "\n\nRespond conversationally; if you need to make code changes, do them and summarise.",
    );
    prompt
}

fn truncate_to_bytes(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string(); }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    let mut out = s[..end].to_string();
    out.push_str("\n…[truncated]");
    out
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').chars().take(40).collect()
}

/// Per-provider argv for non-interactive ("print mode") invocation,
/// with the coworker's persona + card context injected as an appended
/// system prompt. Adding codex/gemini/opencode = one new arm each when
/// their print flags land.
fn oneshot_argv(
    coworker: &WorkspaceCoworker,
    card_id: &str,
    prompt: &str,
    resume_id: Option<&str>,
    worktree: Option<(&str, &str)>,
) -> Vec<String> {
    let persona = build_persona_prompt(coworker, card_id, worktree);
    match coworker.provider.as_str() {
        "claude" => {
            // `claude -p` runs without a TTY, so any permission prompt
            // hangs the turn — the user never sees it and the agent
            // never gets a response. Skip the gate; safety relies on
            // the card-scoped worktree, the persona prompt's "NEVER
            // commit/raise_pr/push on your own initiative" guardrails,
            // and the Review-column safety gate on agent-initiated moves.
            let mut argv = vec![
                "claude".to_string(),
                "-p".to_string(),
                prompt.to_string(),
                "--dangerously-skip-permissions".to_string(),
            ];
            if !persona.is_empty() {
                argv.push("--append-system-prompt".to_string());
                argv.push(persona);
            }
            if let Some(sid) = resume_id {
                argv.push("--resume".to_string());
                argv.push(sid.to_string());
            }
            argv
        }
        _ => Vec::new(),
    }
}

fn build_persona_prompt(
    coworker: &WorkspaceCoworker,
    card_id: &str,
    worktree: Option<(&str, &str)>,
) -> String {
    let role = coworker.role.trim();
    let prompt = coworker.system_prompt.trim();
    let mut out = format!("You are {}, ", coworker.name);
    if role.is_empty() {
        out.push_str("a member of this team. ");
    } else {
        out.push_str(&format!("the {role} on this team. "));
    }
    out.push_str("Respond in character — short signoffs are fine, but stay consistent across turns.");
    if !prompt.is_empty() {
        out.push_str("\n\n");
        out.push_str(prompt);
    }
    // Identity + workflow block. Tells the agent:
    //   • who it is (so it attributes MCP writes correctly)
    //   • when to move the card through columns (Todo → In Review → Review)
    //   • when to create a worktree (only if it's about to write code)
    //   • how to communicate lineage when spinning off a related card
    // Without this, work happens silently and the kanban drifts from
    // reality.
    out.push_str(&format!(
        "\n\n--- Workspace context ---\n\
         You're working on card id: {card_id}.\n\
         Your coworker_id is: {cw_id}.\n\
         \n\
         Attribution: when you call MCP tools that write — cards (cards_create, \
         cards_add_comment, cards_update, cards_move, etc.) AND notes \
         (notes_create, notes_create_for_project, notes_upsert_for_project, \
         notes_update) — always pass `coworkerId: \"{cw_id}\"` so the work shows \
         up as @<your name> instead of the raw CLI id, and is NOT auto-linked to \
         the user's manual Agent session.\n\
         \n\
         Notes vs files: when the user says 'create / update a note', 'doc', \
         'page', 'md file', 'markdown file' — and references the workspace, the \
         notes, or Clauge — use the notes_*_for_project MCP tools. Do NOT use the \
         filesystem Write tool. Workspace notes live in Clauge's SQLite DB, not on \
         disk; only write a real file if the user explicitly says 'on disk' or \
         gives a filesystem path.\n\
         \n\
         Spinning off related cards: if you discover a separate task while \
         discussing this one, create a normal card via cards_create AND drop a \
         comment on THIS card (cards_add_comment) noting the new card's id. \
         That's how the user traces lineage — there is no first-class \
         parent/child relationship between cards.\n\
         \n\
         Card status: this card lives in a kanban column. If it's currently in a \
         column called 'Todo' (or similar) and you've started actively engaging, \
         move it to 'In Review' via cards_move. When you're handing back for the user \
         to review, move it to 'Review'. Use boards_read to discover the column ids.\n\
         \n\
         Code work: if (and only if) this turn requires you to modify files in the \
         project, call cards_start_work first to create an isolated git worktree + \
         branch for this card. Once done, all your file edits go inside that worktree, \
         keeping the rest of the user's repo clean. For pure discussion / planning \
         turns, skip cards_start_work — no need for a worktree.",
        cw_id = coworker.id,
        card_id = card_id,
    ));
    if let Some((path, branch)) = worktree {
        out.push_str(&format!(
            "\n\nWorktree active: you're already running inside `{path}` on branch \
             `{branch}`. Skip cards_start_work — the worktree is set up. Stay on this \
             branch for all file edits and commits; do NOT `git checkout` to another \
             branch unless the user explicitly asks. If you need to confirm, \
             `git branch --show-current` will return `{branch}`."
        ));
    }
    // Shipping tools + the explicit "never autonomous" guardrail. The
    // user controls when code leaves the worktree — even on a turn
    // where you finished a complete unit of work, do NOT raise a PR
    // or push to main on your own. Wait for an explicit signal.
    out.push_str(
        "\n\n--- Shipping tools ---\n\
         You have four MCP tools for getting work off the local machine:\n\
         • cards_commit { cardId, message }       — stage + commit the \
           worktree with `message`. Use a present-tense imperative summary \
           ('Add login rate-limit guard'). Errors with 'no changes' on a \
           clean tree.\n\
         • cards_raise_pr { cardId, title?, body? } — push branch and open \
           a PR/MR. Idempotent: when the card already has a pr_url, this \
           just pushes new commits to the existing PR's branch (no second \
           PR is opened). Returns { prUrl, alreadyExisted, branch }.\n\
         • cards_push_to_repo { id }              — create a real \
           GitHub/GitLab issue from a local card (when the card has no \
           externalId yet).\n\
         • cards_link_pr { cardId, prUrl }        — escape valve: stamp a \
           PR URL onto a card without running any CLI. Use this if you \
           ran `gh pr create` / `glab mr create` via raw bash so the card \
           UI still gets the 'View PR' link.\n\
         \n\
         Prefer the MCP tools over raw bash for commits / PRs / issues. \
         The MCP tools handle PR-already-exists detection, stamp the URL \
         back on the card, and post a 'PR raised' / 'Pushed update to PR' \
         comment so the user sees the activity in the thread. If you \
         bypass them and run `git commit` / `gh pr create` via bash, none \
         of those side effects fire — the card UI silently desyncs from \
         reality. If you do go via bash anyway, call cards_link_pr right \
         after to attach the PR URL.\n\
         \n\
         Hard rule — NEVER call cards_commit, cards_raise_pr, or \
         cards_push_to_repo on your own initiative, even if you've just \
         finished a complete piece of work. Only use them when the user \
         explicitly asks ('commit this', 'raise a PR', 'ship it', 'push it', \
         'create an issue for this'). Silence is the right default; the \
         user controls when code leaves the worktree.",
    );
    out
}

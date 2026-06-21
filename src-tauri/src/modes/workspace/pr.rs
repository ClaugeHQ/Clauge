// Card → branch → commit → push → PR pipeline. Two public entry
// points:
//
//   commit_card        — agent-driven; auto-commits the worktree
//                        with the supplied message. No-op error when
//                        nothing is staged.
//   raise_or_update_pr — idempotent. Pushes the branch, then EITHER
//                        creates a new PR (and stamps `pr_url` on the
//                        card) OR no-ops because the PR already exists
//                        (push alone is enough — gh/glab auto-update
//                        the PR with new commits on the same branch).
//
// Both use the cli_errors helper so failures bubble up as typed
// CliError variants (NotInstalled / NotAuthenticated / NoAccess /
// NetworkError / etc) — the UI / agent gets a clean toast string
// instead of raw stderr.

use serde::Serialize;
use sqlx::SqlitePool;
use std::process::Command;
use tauri::{AppHandle, Emitter};

use crate::modes::workspace::cli_errors::{
    classify_output, install_url_for, is_on_path, CliError,
};
use crate::shared::repos::{sessions as session_repo, workspaces as repo};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    pub commit_sha: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RaisePrResult {
    /// Resolved PR / MR URL — newly created or pre-existing.
    pub pr_url: String,
    /// True when the PR existed before this call. Caller may surface
    /// "Pushed N new commit(s) to existing PR" instead of "PR raised".
    pub already_existed: bool,
    /// Branch the PR is on. Stamped here so the agent's reply can
    /// reference it without an extra MCP roundtrip.
    pub branch: String,
}

const CARD_UPDATED_EVENT: &str = "workspace:card-updated";

fn emit_updated(app: Option<&AppHandle>, card_id: &str) {
    if let Some(a) = app {
        let _ = a.emit(CARD_UPDATED_EVENT, serde_json::json!({ "cardId": card_id }));
    }
}

/// Resolved per-card context that every PR / commit op needs. Fetched
/// once at the top of each entry point so we don't roundtrip the DB
/// for every shell-out.
struct CardCtx {
    /// Worktree path the agent's session is bound to. Required — refusing
    /// without a worktree is the right call (no worktree → no isolated
    /// branch → committing into the user's main checkout would be a foot-gun).
    worktree: String,
    /// Branch name the worktree is on. Required for the same reason.
    branch: String,
    /// Workspace-level repo URL (https://github.com/owner/repo or similar).
    /// Required for raising PRs; commits don't need it.
    repo_url: Option<String>,
    /// Existing PR URL if one was raised previously. None → first PR.
    existing_pr_url: Option<String>,
}

async fn resolve_card_ctx(pool: &SqlitePool, card_id: &str) -> Result<CardCtx, CliError> {
    // Pull worktree + branch via the claimed session.
    let claim = repo::get_card_claim_and_project(pool, card_id)
        .await
        .map_err(|e| CliError::Other { stderr: format!("DB: {e}") })?
        .ok_or_else(|| CliError::Other { stderr: "Card not found".into() })?;
    let session_id = claim.0.clone().ok_or_else(|| CliError::Other {
        stderr: "Card has no active claim — start a chat with a coworker first.".into(),
    })?;
    let session = session_repo::get_session_by_id(pool, &session_id)
        .await
        .map_err(|e| CliError::Other { stderr: format!("DB: {e}") })?;
    let worktree = session
        .worktree_path
        .clone()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| CliError::Other {
            stderr: "Card has no worktree yet — agent must call cards_start_work first.".into(),
        })?;
    let branch = session
        .worktree_branch
        .clone()
        .filter(|b| !b.is_empty())
        .ok_or_else(|| CliError::Other {
            stderr: "Worktree exists but branch is missing — try restarting the work-stream.".into(),
        })?;

    // repo_url + pr_url come from the workspace + card rows.
    let row: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT w.repo_url, c.pr_url \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         JOIN workspaces w ON w.id = b.workspace_id \
         WHERE c.id = ?",
    )
    .bind(card_id)
    .fetch_one(pool)
    .await
    .map_err(|e| CliError::Other { stderr: format!("DB: {e}") })?;

    Ok(CardCtx {
        worktree,
        branch,
        repo_url: row.0,
        existing_pr_url: row.1,
    })
}

/// Run a git subcommand inside the card's worktree. Returns CliError
/// on non-zero exit, classified via the shared helper.
fn run_git(worktree: &str, args: &[&str]) -> Result<std::process::Output, CliError> {
    let git_bin = crate::shared::platform::path::find_binary("git").ok_or_else(|| {
        CliError::NotInstalled {
            tool: "git".into(),
            install_url: install_url_for("git").into(),
        }
    })?;
    let mut cmd = Command::new(&git_bin);
    crate::shared::platform::path::apply_user_path(&mut cmd);
    cmd.arg("-C").arg(worktree);
    for a in args {
        cmd.arg(a);
    }
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CliError::NotInstalled {
                tool: "git".into(),
                install_url: install_url_for("git").into(),
            }
        } else {
            CliError::Other { stderr: e.to_string() }
        }
    })?;
    if let Some(err) = classify_output("git", &out, None) {
        return Err(err);
    }
    Ok(out)
}

/// True when `git status --porcelain` reports any changes. Used to
/// decide whether to skip the commit step (clean tree → nothing to do).
fn worktree_dirty(worktree: &str) -> Result<bool, CliError> {
    let out = run_git(worktree, &["status", "--porcelain"])?;
    Ok(!String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

/// Commit any pending changes in the card's worktree with the given
/// message. Returns NoChanges when the tree is clean — caller decides
/// whether that's an error or a happy path.
pub async fn commit_card(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    card_id: &str,
    message: &str,
    actor: &str,
) -> Result<CommitResult, CliError> {
    let ctx = resolve_card_ctx(pool, card_id).await?;
    if !worktree_dirty(&ctx.worktree)? {
        return Err(CliError::NoChanges);
    }
    // Stage + commit. `git add -A` covers new + modified + deleted.
    run_git(&ctx.worktree, &["add", "-A"])?;
    run_git(&ctx.worktree, &["commit", "-m", message])?;
    let sha_out = run_git(&ctx.worktree, &["rev-parse", "HEAD"])?;
    let sha = String::from_utf8_lossy(&sha_out.stdout).trim().to_string();

    // Drop a comment on the card so the user sees activity in the thread.
    let now = chrono::Utc::now().to_rfc3339();
    let body = format!(
        "**Commit on `{}`**\n\n`{}` — {}",
        ctx.branch,
        &sha[..sha.len().min(7)],
        message
    );
    let cid = uuid::Uuid::new_v4().to_string();
    let _ = repo::insert_card_comment(pool, &cid, card_id, actor, None, &body, None, &now, "ticket", None, None, repo::MutationGuard::default()).await;
    emit_updated(app, card_id);

    Ok(CommitResult { commit_sha: sha, message: message.to_string() })
}

/// Push the worktree's branch upstream, idempotent. Sets `-u origin
/// <branch>` on the first push so subsequent pushes don't need it.
fn push_branch(worktree: &str, branch: &str) -> Result<(), CliError> {
    // -u sets upstream on first push and is a no-op afterwards.
    run_git(worktree, &["push", "-u", "origin", branch])?;
    Ok(())
}

/// Raise (or push to existing) PR for a card. Idempotent: detects an
/// existing pr_url and short-circuits the create step. The branch is
/// pushed unconditionally so new commits land in the existing PR
/// without an extra step from the caller.
pub async fn raise_or_update_pr(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    card_id: &str,
    title_hint: Option<&str>,
    body_hint: Option<&str>,
    actor: &str,
) -> Result<RaisePrResult, CliError> {
    let ctx = resolve_card_ctx(pool, card_id).await?;

    // 1. Make sure there's something to push. Without this, gh/glab's
    //    error for "no commits to push" is opaque and we'd waste a
    //    network round-trip.
    let head = run_git(&ctx.worktree, &["rev-parse", "HEAD"])?;
    if String::from_utf8_lossy(&head.stdout).trim().is_empty() {
        return Err(CliError::Other {
            stderr: "Branch has no commits yet. Use cards_commit first.".into(),
        });
    }

    // 2. Push branch (creates / updates upstream).
    push_branch(&ctx.worktree, &ctx.branch)?;

    // 3. PR already raised → done. The push above already updated it.
    if let Some(url) = ctx.existing_pr_url.clone() {
        // Comment on the card so the user sees the update flowed through.
        let now = chrono::Utc::now().to_rfc3339();
        let body = format!("**Pushed update to PR**\n\n[{}]({})", url, url);
        let cid = uuid::Uuid::new_v4().to_string();
        let _ = repo::insert_card_comment(pool, &cid, card_id, actor, None, &body, None, &now, "ticket", None, None, repo::MutationGuard::default()).await;
        emit_updated(app, card_id);
        return Ok(RaisePrResult { pr_url: url, already_existed: true, branch: ctx.branch });
    }

    // 4. New PR — pick the right CLI based on the workspace's repo URL.
    let repo_url = ctx.repo_url.clone().filter(|u| !u.trim().is_empty()).ok_or_else(|| {
        CliError::Other {
            stderr: "Workspace has no repo URL set. Use 'Link to repo' first.".into(),
        }
    })?;
    let lower = repo_url.to_lowercase();
    let tool = if lower.contains("github.com") {
        "gh"
    } else if lower.contains("gitlab") {
        "glab"
    } else {
        return Err(CliError::Other {
            stderr: format!("Unsupported repo URL: {repo_url}"),
        });
    };
    if !is_on_path(tool) {
        return Err(CliError::NotInstalled {
            tool: tool.into(),
            install_url: install_url_for(tool).into(),
        });
    }
    let owner_repo = crate::modes::workspace::commands::parse_owner_repo(&repo_url)
        .ok_or_else(|| CliError::Other {
            stderr: format!("Could not parse owner/repo from {repo_url}"),
        })?;

    // Title defaults to the card title; body defaults to a short
    // template. Caller can override either to inject card description
    // or a richer template.
    let card_title: (String,) = sqlx::query_as("SELECT title FROM workspace_board_cards WHERE id = ?")
        .bind(card_id)
        .fetch_one(pool)
        .await
        .map_err(|e| CliError::Other { stderr: format!("DB: {e}") })?;
    let title = title_hint.map(str::to_string).unwrap_or(card_title.0);
    let body = body_hint
        .map(str::to_string)
        .unwrap_or_else(|| format!("Card branch `{}` — see card thread for context.", ctx.branch));

    // gh and glab share the create flow but differ on flag names.
    let tool_bin = crate::shared::platform::path::find_binary(tool).ok_or_else(|| {
        CliError::NotInstalled {
            tool: tool.into(),
            install_url: install_url_for(tool).into(),
        }
    })?;
    let mut cmd = Command::new(&tool_bin);
    crate::shared::platform::path::apply_user_path(&mut cmd);
    cmd.current_dir(&ctx.worktree);
    if tool == "gh" {
        cmd.args([
            "pr", "create",
            "--repo", &owner_repo,
            "--head", &ctx.branch,
            "--title", &title,
            "--body", &body,
        ]);
    } else {
        cmd.args([
            "mr", "create",
            "-R", &owner_repo,
            "--source-branch", &ctx.branch,
            "--title", &title,
            "--description", &body,
        ]);
    }
    let out = cmd.output().map_err(|e| CliError::Other {
        stderr: format!("{tool} failed to spawn: {e}"),
    })?;
    if let Some(err) = classify_output(tool, &out, Some(&owner_repo)) {
        return Err(err);
    }

    // Both CLIs print the URL on success — extract the first http(s)
    // URL we see whose host matches the repo's host.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let host_marker = if tool == "gh" { "github.com" } else { "gitlab" };
    let pr_url = extract_url(&stdout, host_marker)
        .ok_or_else(|| CliError::Other {
            stderr: format!("Could not parse PR URL from {tool} output: {}", stdout.trim()),
        })?;

    // Stamp + comment + emit.
    let now = chrono::Utc::now().to_rfc3339();
    repo::update_card_pr_url(pool, card_id, &pr_url, actor, &now, repo::MutationGuard::default())
        .await
        .map_err(|e| CliError::Other { stderr: format!("DB: {e}") })?;
    let body = format!("**PR raised**\n\n[{}]({})", pr_url, pr_url);
    let cid = uuid::Uuid::new_v4().to_string();
    let _ = repo::insert_card_comment(pool, &cid, card_id, actor, None, &body, None, &now, "ticket", None, None, repo::MutationGuard::default()).await;
    emit_updated(app, card_id);

    Ok(RaisePrResult { pr_url, already_existed: false, branch: ctx.branch })
}

/// Stamp a PR URL on a card without running any CLI commands. Used
/// when an agent raised a PR via raw bash (legacy path) and just
/// wants to attach the link back to the card.
pub async fn link_pr_url(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    card_id: &str,
    pr_url: &str,
    actor: &str,
) -> Result<(), CliError> {
    let now = chrono::Utc::now().to_rfc3339();
    repo::update_card_pr_url(pool, card_id, pr_url, actor, &now, repo::MutationGuard::default())
        .await
        .map_err(|e| CliError::Other { stderr: format!("DB: {e}") })?;
    emit_updated(app, card_id);
    Ok(())
}

/// Host-side state of a card's PR. Driven by `gh pr view --json state`
/// / `glab mr view --output json` so it reflects what's actually true
/// on the host, not any cached Clauge-side state. Consumed by the
/// auto-move-on-merge poll in the frontend and the matching MCP tool.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Open,
    Merged,
    Closed,
    /// gh/glab returned something the parser didn't recognise. Callers
    /// should treat as a no-op (don't auto-move, don't bubble error).
    Unknown,
}

/// Query the host for a card's PR state. Pure read — never mutates.
///
/// Errors with the usual `CliError` variants when gh/glab is missing,
/// the user isn't authed, the network fails, etc. so the UI can surface
/// install-modals the same way the create / push paths do.
pub async fn check_pr_state(
    pool: &SqlitePool,
    card_id: &str,
) -> Result<PrState, CliError> {
    let row: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT c.pr_url, b.workspace_id \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         WHERE c.id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| CliError::Other { stderr: format!("DB: {e}") })?;

    let (pr_url, ws_id) = row.ok_or_else(|| CliError::Other {
        stderr: "Card not found".into(),
    })?;
    let pr_url = pr_url
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| CliError::Other {
            stderr: "Card has no PR URL yet".into(),
        })?;

    let workspace = repo::get_workspace_by_id(pool, &ws_id)
        .await
        .map_err(|e| CliError::Other { stderr: format!("DB: {e}") })?;
    let repo_url = workspace
        .repo_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CliError::Other {
            stderr: "Workspace has no repo URL set".into(),
        })?
        .to_string();

    let lower = repo_url.to_lowercase();
    let tool = if lower.contains("github.com") {
        "gh"
    } else if lower.contains("gitlab") {
        "glab"
    } else {
        return Err(CliError::Other {
            stderr: format!("Unsupported repo URL: {repo_url}"),
        });
    };
    if !is_on_path(tool) {
        return Err(CliError::NotInstalled {
            tool: tool.into(),
            install_url: install_url_for(tool).into(),
        });
    }
    let tool_bin = crate::shared::platform::path::find_binary(tool).ok_or_else(|| {
        CliError::NotInstalled {
            tool: tool.into(),
            install_url: install_url_for(tool).into(),
        }
    })?;

    let mut cmd = Command::new(&tool_bin);
    crate::shared::platform::path::apply_user_path(&mut cmd);
    if tool == "gh" {
        // `gh pr view <url> --json state` → {"state":"OPEN" | "MERGED" | "CLOSED"}
        cmd.args(["pr", "view", &pr_url, "--json", "state"]);
    } else {
        // `glab mr view <url> --output json` → full record including .state ("opened"/"merged"/"closed")
        cmd.args(["mr", "view", &pr_url, "--output", "json"]);
    }
    let out = cmd.output().map_err(|e| CliError::Other {
        stderr: format!("{tool} failed to spawn: {e}"),
    })?;
    if let Some(err) = classify_output(tool, &out, None) {
        return Err(err);
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
    let raw = v
        .get("state")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    Ok(match raw.as_str() {
        "OPEN" | "OPENED" => PrState::Open,
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => PrState::Unknown,
    })
}

/// Pull the first http(s) URL out of stdout whose host contains the
/// given marker. Both `gh pr create` and `glab mr create` print the
/// URL as the only content on the last line of stdout, so this is a
/// strict-enough heuristic.
fn extract_url(stdout: &str, host_marker: &str) -> Option<String> {
    for line in stdout.lines().rev() {
        for tok in line.split_whitespace() {
            if (tok.starts_with("http://") || tok.starts_with("https://"))
                && tok.to_lowercase().contains(host_marker)
            {
                return Some(tok.trim_end_matches(['.', ',', ')']).to_string());
            }
        }
    }
    None
}

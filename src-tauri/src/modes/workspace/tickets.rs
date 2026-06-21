// GitHub/GitLab ticket-comment sync for the card drawer's Ticket section.
//
// - fetch_ticket_comments: pulls real issue comments (author + ORIGINAL
//   timestamp) and upserts them into workspace_card_comments (channel
//   'ticket', keyed by external_id so re-fetch never duplicates).
// - post_ticket_comment: posts a comment back to the issue, then stores
//   it locally. For local (non-linked) cards both calls degrade to plain
//   local ticket comments.
//
// Full 2-way for both providers: GitHub via `gh` (issue-comment endpoints),
// GitLab via `glab api` (issue *notes* REST endpoints). Fetch + post + edit +
// delete are supported on both. Local (non-linked) cards keep plain local
// ticket comments.

use serde_json::Value;
use sqlx::SqlitePool;
use tauri::State;

use crate::modes::workspace::cli_errors::{classify_output, CliError};
use crate::modes::workspace::models::WorkspaceCardComment;
use crate::shared::platform::path::{apply_user_path, find_binary};
use crate::shared::repos::workspaces as repo;

struct TicketRef {
    number: String,
    source: String, // "github" | "gitlab"
    tool: &'static str,
    owner_repo: String,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Resolve a card to its linked-issue coordinates straight from the card's
/// own `external_url` (e.g. https://github.com/owner/repo/issues/85) — the
/// linked card literally contains its repo + number, so this works for any
/// synced card regardless of how the workspace was configured. Returns
/// Ok(None) for a local card (no external_url).
async fn resolve_ticket(pool: &SqlitePool, card_id: &str) -> Result<Option<TicketRef>, String> {
    let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT external_id, external_url FROM workspace_board_cards WHERE id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB error reading card: {e}"))?;

    let (external_id, external_url) = row.ok_or_else(|| "Card not found".to_string())?;
    let url = match external_url {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return Ok(None), // local card
    };

    let (tool, source): (&str, &str) = {
        let l = url.to_lowercase();
        if l.contains("github.com") {
            ("gh", "github")
        } else if l.contains("gitlab") {
            ("glab", "gitlab")
        } else {
            return Err(format!("Unsupported issue URL: {url}"));
        }
    };
    // parse_owner_repo on an ISSUE url yields "owner/repo/issues/85" —
    // keep only the first two segments (owner/repo) for `--repo`.
    let owner_repo = super::commands::parse_owner_repo(&url)
        .map(|p| p.split('/').take(2).collect::<Vec<_>>().join("/"))
        .filter(|s| s.matches('/').count() == 1)
        .ok_or_else(|| format!("Could not parse owner/repo from {url}"))?;
    // Issue number is the last path segment of the URL, with the short id
    // (`#85` / `!42`) as a fallback.
    let number = url
        .rsplit('/')
        .next()
        .filter(|s| s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            external_id
                .as_deref()
                .map(|s| s.trim_start_matches(['#', '!']).trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Could not parse issue number from {url}"))?;

    Ok(Some(TicketRef {
        number,
        source: source.to_string(),
        tool,
        owner_repo,
    }))
}

/// Fetch + upsert the issue's comments, returning the full ticket thread.
#[tauri::command]
pub async fn workspace_card_fetch_ticket_comments(
    pool: State<'_, SqlitePool>,
    card_id: String,
) -> Result<Vec<WorkspaceCardComment>, String> {
    let pool = pool.inner();
    let tref = resolve_ticket(pool, &card_id).await?;

    if let Some(t) = tref {
        let bin = find_binary(t.tool)
            .ok_or_else(|| format!("{} is not installed or not on PATH.", t.tool))?;
        let number = t.number.clone();
        let owner_repo = t.owner_repo.clone();
        let source = t.source.clone();
        let output = tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&bin);
            apply_user_path(&mut cmd);
            if source == "github" {
                cmd.args([
                    "issue", "view", &number, "--repo", &owner_repo, "--json", "comments",
                ]);
            } else {
                // GitLab REST: notes on the issue (newest 100). The project
                // id is the URL-encoded path; iid is the issue number.
                let endpoint = format!(
                    "projects/{}/issues/{}/notes?per_page=100&sort=asc",
                    gitlab_project_id(&owner_repo),
                    number
                );
                cmd.args(["api", &endpoint]);
            }
            cmd.output()
        })
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
        .map_err(|e| format!("{} failed to spawn: {e}", t.tool))?;

        if !output.status.success() {
            let err = classify_output(t.tool, &output, Some(&t.owner_repo)).unwrap_or(
                CliError::Other {
                    stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                },
            );
            return Err(err.message());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        if t.source == "github" {
            upsert_github_comments(pool, &card_id, &stdout).await?;
        } else {
            upsert_gitlab_comments(pool, &card_id, &stdout).await?;
        }
    }

    repo::list_card_comments(pool, &card_id, Some("ticket"))
        .await
        .map_err(|e| e.to_string())
}

async fn upsert_github_comments(
    pool: &SqlitePool,
    card_id: &str,
    stdout: &str,
) -> Result<(), String> {
    let parsed: Value =
        serde_json::from_str(stdout).map_err(|e| format!("Could not parse gh output: {e}"))?;
    let comments = parsed
        .get("comments")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut kept: Vec<String> = Vec::new();
    for c in comments {
        let body = c.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let created_at = c
            .get("createdAt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let author = c
            .get("author")
            .and_then(|a| a.get("login"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        // `url` is unique + stable → our dedupe key.
        let url = c
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{author}:{created_at}"));
        if body.trim().is_empty() && created_at.is_empty() {
            continue;
        }
        kept.push(url.clone());
        let id = new_id();
        repo::upsert_external_comment(
            pool, &id, card_id, &author, &body, &created_at, &url, &author,
        )
        .await
        .map_err(|e| format!("DB error storing comment: {e}"))?;
    }
    // Drop synced comments that were deleted upstream (we just saw the
    // full current set, so anything external + not in `kept` is gone).
    repo::prune_external_comments(pool, card_id, &kept)
        .await
        .map_err(|e| format!("DB error pruning comments: {e}"))?;
    Ok(())
}

/// GitLab project id for the REST API = URL-encoded `group/project` path.
fn gitlab_project_id(owner_repo: &str) -> String {
    owner_repo.replace('/', "%2F")
}

/// Parse `glab api …/notes` output (a JSON array) and upsert real user
/// notes. System notes (label/state changes) are skipped. We key each by
/// `note_<id>` so edit/delete can recover the note id, and preserve the
/// original `created_at`.
async fn upsert_gitlab_comments(
    pool: &SqlitePool,
    card_id: &str,
    stdout: &str,
) -> Result<(), String> {
    let parsed: Value =
        serde_json::from_str(stdout).map_err(|e| format!("Could not parse glab output: {e}"))?;
    let notes = parsed.as_array().cloned().unwrap_or_default();
    let mut kept: Vec<String> = Vec::new();
    for n in notes {
        // Skip auto-generated system notes — only real comments.
        if n.get("system").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        let note_id = match n.get("id").and_then(|v| v.as_i64()) {
            Some(i) => i,
            None => continue,
        };
        let body = n.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if body.trim().is_empty() {
            continue;
        }
        let created_at = n
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let author = n
            .get("author")
            .and_then(|a| a.get("username"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let external_id = format!("note_{note_id}");
        kept.push(external_id.clone());
        let id = new_id();
        repo::upsert_external_comment(
            pool, &id, card_id, &author, &body, &created_at, &external_id, &author,
        )
        .await
        .map_err(|e| format!("DB error storing comment: {e}"))?;
    }
    repo::prune_external_comments(pool, card_id, &kept)
        .await
        .map_err(|e| format!("DB error pruning comments: {e}"))?;
    Ok(())
}

/// REST comment id from a comment URL.
/// GitHub: `…/issues/85#issuecomment-1234567`. GitLab: `note_42`.
fn comment_rest_id(url: &str, source: &str) -> Option<String> {
    let marker = if source == "github" { "issuecomment-" } else { "note_" };
    url.rsplit(marker)
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
}

/// Edit a ticket comment. For a synced GitHub comment this PATCHes it on
/// the host; for a local comment it just updates the local copy.
#[tauri::command]
pub async fn workspace_card_edit_ticket_comment(
    pool: State<'_, SqlitePool>,
    comment_id: String,
    body: String,
) -> Result<(), String> {
    let pool = pool.inner();
    let trimmed = body.trim().to_string();
    if trimmed.is_empty() {
        return Err("Comment body is empty".into());
    }
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT card_id, external_id FROM workspace_card_comments WHERE id = ?",
    )
    .bind(&comment_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let (card_id, external_id) = row.ok_or_else(|| "Comment not found".to_string())?;

    if let Some(url) = external_id.filter(|s| !s.trim().is_empty()) {
        let t = resolve_ticket(pool, &card_id)
            .await?
            .ok_or_else(|| "Card is not linked to an issue".to_string())?;
        run_comment_api(&t, &url, "PATCH", Some(&trimmed)).await?;
    }
    repo::update_comment_body(pool, &comment_id, &trimmed, &now_rfc3339())
        .await
        .map_err(|e| e.to_string())
}

/// Delete a ticket comment. Synced GitHub comments are deleted on the host
/// too; local comments are just removed locally.
#[tauri::command]
pub async fn workspace_card_delete_ticket_comment(
    pool: State<'_, SqlitePool>,
    comment_id: String,
) -> Result<(), String> {
    let pool = pool.inner();
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT card_id, external_id FROM workspace_card_comments WHERE id = ?",
    )
    .bind(&comment_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let (card_id, external_id) = row.ok_or_else(|| "Comment not found".to_string())?;

    if let Some(url) = external_id.filter(|s| !s.trim().is_empty()) {
        let t = resolve_ticket(pool, &card_id)
            .await?
            .ok_or_else(|| "Card is not linked to an issue".to_string())?;
        run_comment_api(&t, &url, "DELETE", None).await?;
    }
    sqlx::query("DELETE FROM workspace_card_comments WHERE id = ?")
        .bind(&comment_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Edit (body=Some) / delete (body=None) a provider comment via
/// `gh api` / `glab api`. GitHub uses PATCH/DELETE on the issue-comment;
/// GitLab uses PUT/DELETE on the issue note.
async fn run_comment_api(
    t: &TicketRef,
    comment_url: &str,
    method: &str,
    body: Option<&str>,
) -> Result<(), String> {
    let rid = comment_rest_id(comment_url, &t.source)
        .ok_or_else(|| "Could not determine the comment id".to_string())?;
    let bin = find_binary(t.tool)
        .ok_or_else(|| format!("{} is not installed or not on PATH.", t.tool))?;
    let (endpoint, method) = if t.source == "github" {
        (
            format!("repos/{}/issues/comments/{}", t.owner_repo, rid),
            method.to_string(),
        )
    } else {
        // GitLab: PUT to edit, DELETE to remove the note.
        let m = if body.is_some() { "PUT" } else { "DELETE" };
        (
            format!(
                "projects/{}/issues/{}/notes/{}",
                gitlab_project_id(&t.owner_repo),
                t.number,
                rid
            ),
            m.to_string(),
        )
    };
    let body = body.map(|s| s.to_string());
    let owner_repo = t.owner_repo.clone();
    let tool = t.tool;
    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&bin);
        apply_user_path(&mut cmd);
        cmd.args(["api", "-X", &method, &endpoint]);
        if let Some(b) = &body {
            cmd.args(["-f", &format!("body={b}")]);
        }
        cmd.output()
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
    .map_err(|e| format!("{tool} failed to spawn: {e}"))?;
    if !output.status.success() {
        let err = classify_output(tool, &output, Some(&owner_repo)).unwrap_or(CliError::Other {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
        return Err(err.message());
    }
    Ok(())
}

/// Post a comment to the linked issue (GitHub/GitLab) and store it
/// locally. Local cards just store a plain local ticket comment.
#[tauri::command]
pub async fn workspace_card_post_ticket_comment(
    pool: State<'_, SqlitePool>,
    card_id: String,
    body: String,
    actor: String,
) -> Result<WorkspaceCardComment, String> {
    let pool = pool.inner();
    let trimmed = body.trim().to_string();
    if trimmed.is_empty() {
        return Err("Comment body is empty".into());
    }
    let now = now_rfc3339();
    let comment_id = new_id();
    let tref = resolve_ticket(pool, &card_id).await?;

    let mut external_id: Option<String> = None;
    if let Some(t) = &tref {
        let bin = find_binary(t.tool)
            .ok_or_else(|| format!("{} is not installed or not on PATH.", t.tool))?;
        let number = t.number.clone();
        let owner_repo = t.owner_repo.clone();
        let source = t.source.clone();
        let body_owned = trimmed.clone();
        let output = tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&bin);
            apply_user_path(&mut cmd);
            if source == "github" {
                cmd.args([
                    "issue", "comment", &number, "--repo", &owner_repo, "--body", &body_owned,
                ]);
            } else {
                // glab: add a note to the issue.
                cmd.args([
                    "issue", "note", &number, "-R", &owner_repo, "-m", &body_owned,
                ]);
            }
            cmd.output()
        })
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
        .map_err(|e| format!("{} failed to spawn: {e}", t.tool))?;

        if !output.status.success() {
            let err = classify_output(t.tool, &output, Some(&t.owner_repo)).unwrap_or(
                CliError::Other {
                    stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                },
            );
            return Err(err.message());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        external_id = extract_first_url(&stdout);
        // Normalise GitLab's printed note URL (…#note_<id>) to the same
        // `note_<id>` key the fetch path stores, so a later refresh updates
        // this row in place instead of duplicating it.
        if t.source == "gitlab" {
            external_id = external_id
                .as_deref()
                .and_then(|u| comment_rest_id(u, "gitlab"))
                .map(|rid| format!("note_{rid}"));
        }
    }

    repo::insert_card_comment(
        pool,
        &comment_id,
        &card_id,
        &actor,
        None,
        &trimmed,
        None,
        &now,
        "ticket",
        external_id.as_deref(),
        None,
        repo::MutationGuard::default(),
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(WorkspaceCardComment {
        id: comment_id,
        card_id,
        actor,
        coworker_id: None,
        body: trimmed,
        parent_id: None,
        created_at: now,
        channel: "ticket".to_string(),
        external_id,
        external_author: None,
    })
}

/// First http(s) URL in CLI stdout (the comment URL gh/glab print).
fn extract_first_url(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        for tok in line.split_whitespace() {
            if tok.starts_with("http://") || tok.starts_with("https://") {
                return Some(tok.trim_end_matches(['.', ',', ')']).to_string());
            }
        }
    }
    None
}

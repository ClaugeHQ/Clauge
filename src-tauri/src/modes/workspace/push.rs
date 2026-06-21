// Push a local kanban card up to its workspace's GitHub or GitLab repo
// as a real issue. Shells out to `gh`/`glab` (same tooling we already
// use for the issue scan) so auth lives where users expect it.
//
// Flow (failure-tolerant; every step returns a typed error string the
// UI can show in a toast):
//   1. Look up the card → column → board → workspace.
//   2. Refuse if the card is already linked (`external_id` set).
//   3. Refuse if the workspace has no `repo_url`.
//   4. Decide source from the URL host (GitHub / GitLab).
//   5. Verify the CLI is on PATH.
//   6. Run `<cli> issue create --repo … --title … --body …`.
//   7. Parse the issue URL out of stdout, derive `#NNN` / `!NNN`.
//   8. Persist back onto the card row.
//   9. Return the updated card as JSON for the caller (UI / MCP).

use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::modes::workspace::cli_errors::{classify_output, CliError};

/// Run the full push pipeline for `card_id`. Returns the patched card
/// as a JSON value on success; an error string on failure. The error
/// shape is deliberately a flat string so the same call serves both
/// the Tauri command (which surfaces it as a toast) and the MCP tool
/// (which wraps it in a JSON-RPC error).
pub async fn push_card_to_repo(
    pool: &SqlitePool,
    card_id: &str,
    actor: &str,
) -> Result<Value, String> {
    // ── 1. Resolve card → workspace.repo_url ───────────────────────
    let card_row: Option<(String, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT c.title, c.description, c.column_id, c.external_id, b.workspace_id \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         WHERE c.id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB error reading card: {e}"))?;

    let (title, description, _column_id, external_id, _workspace_id) =
        card_row.ok_or_else(|| "Card not found".to_string())?;

    if external_id.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false) {
        return Err("Card is already linked to an issue".into());
    }

    // ── 2. Resolve the repo the same way the scan does (board source /
    //    project git remote / repo_url). cwd lets gh/glab infer the repo
    //    even when no explicit owner/repo or repo_url is configured.
    let target = super::repo_target::resolve_for_card(pool, card_id)
        .await?
        .ok_or_else(|| {
            "This board isn't connected to a GitHub/GitLab repo. Set a project or repo URL first."
                .to_string()
        })?;
    let tool = target.tool;
    let source = target.provider.clone();

    // ── 3. CLI on PATH? ────────────────────────────────────────────
    let tool_bin = crate::shared::platform::path::find_binary(tool).ok_or_else(|| {
        format!("{tool} is not installed or not on PATH. Install it and retry.")
    })?;

    // ── 4. Run the issue-create command. We capture stdout so we can
    //    parse the resulting issue URL — both CLIs print it on success.
    let title_owned = title.clone();
    let body_owned = description.clone();
    let source_owned = source.clone();
    let tool_bin_owned = tool_bin.clone();
    let cwd_owned = target.cwd.clone();
    let owner_repo_owned = target.owner_repo.clone();

    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&tool_bin_owned);
        crate::shared::platform::path::apply_user_path(&mut cmd);
        // Prefer running inside the repo checkout so the CLI infers the
        // repo; fall back to an explicit --repo/-R when we only have a URL.
        if let Some(cwd) = &cwd_owned {
            cmd.current_dir(cwd);
        }
        if source_owned == "github" {
            cmd.args(["issue", "create"]);
            if cwd_owned.is_none() {
                if let Some(or) = &owner_repo_owned {
                    cmd.args(["--repo", or]);
                }
            }
            cmd.args(["--title", &title_owned, "--body", &body_owned]);
        } else {
            cmd.args(["issue", "create"]);
            if cwd_owned.is_none() {
                if let Some(or) = &owner_repo_owned {
                    cmd.args(["-R", or]);
                }
            }
            cmd.args(["--title", &title_owned, "--description", &body_owned]);
        }
        cmd.output()
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
    .map_err(|e| format!("{tool} failed to spawn: {e}"))?;

    if !output.status.success() {
        // Use the shared classifier so multi-account / no-access /
        // network failures get clean toast copy. Falls back to raw
        // stderr only when the classifier can't pin the error down.
        let err = classify_output(tool, &output, target.owner_repo.as_deref())
            .unwrap_or(CliError::Other {
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        return Err(err.message());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let issue_url = extract_issue_url(&stdout, &source).ok_or_else(|| {
        format!(
            "Could not parse issue URL from {tool} output: {}",
            stdout.trim()
        )
    })?;
    let external_id = derive_external_id(&issue_url, &source).unwrap_or_else(|| issue_url.clone());

    // ── 5. Persist back onto the card. ─────────────────────────────
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE workspace_board_cards \
         SET external_id = ?, external_url = ?, updated_at = ?, updated_by = ? \
         WHERE id = ?",
    )
    .bind(&external_id)
    .bind(&issue_url)
    .bind(&now)
    .bind(actor)
    .bind(card_id)
    .execute(pool)
    .await
    .map_err(|e| format!("DB error writing back card: {e}"))?;

    Ok(json!({
        "id": card_id,
        "externalId": external_id,
        "externalUrl": issue_url,
        "source": source,
    }))
}

/// Pull the first http(s) URL out of `gh`/`glab` stdout. Both CLIs
/// emit the issue URL on success — typically as the last token of the
/// last non-empty line, but we don't depend on the layout.
fn extract_issue_url(stdout: &str, source: &str) -> Option<String> {
    let host_marker = match source {
        "github" => "github.com",
        "gitlab" => "gitlab",
        _ => return None,
    };
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

/// Reduce an issue URL to the short form we use elsewhere
/// ("#1234" for GitHub, "!42" for GitLab) — matches the format the
/// issue-scan parsers stamp on imported cards so the badge logic
/// stays uniform.
fn derive_external_id(url: &str, source: &str) -> Option<String> {
    // Last `/` segment is the issue number on both providers.
    let tail = url.rsplit('/').next()?;
    let num: i64 = tail.parse().ok()?;
    Some(match source {
        "github" => format!("#{num}"),
        "gitlab" => format!("!{num}"),
        _ => return None,
    })
}

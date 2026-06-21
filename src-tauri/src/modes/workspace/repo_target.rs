// Resolve which GitHub/GitLab repo a board/card is connected to — using
// the SAME precedence the issue scan uses, so "create issue" and
// "sync comments" work wherever scanning works (not only when the
// workspace has an explicit repo_url).
//
// Precedence:
//   1. board.source_config.project_url   (explicit URL)
//   2. board.source_config.project_path  → its `origin` git remote
//   3. workspace.project_path            → its `origin` git remote
//   4. workspace.repo_url                (last resort)

use sqlx::SqlitePool;

use crate::shared::repos::workspaces as repo;

#[derive(Debug, Clone)]
pub struct RepoTarget {
    pub provider: String, // "github" | "gitlab"
    pub tool: &'static str, // "gh" | "glab"
    /// Working directory to run the CLI in (lets gh/glab infer the repo).
    pub cwd: Option<String>,
    /// owner/repo (or group/project) when we could parse it from a URL.
    pub owner_repo: Option<String>,
}

fn provider_for(url: &str) -> Option<(&'static str, &'static str)> {
    let l = url.to_lowercase();
    if l.contains("github.com") {
        Some(("github", "gh"))
    } else if l.contains("gitlab") {
        Some(("gitlab", "glab"))
    } else {
        None
    }
}

fn git_remote_url(path: &str) -> Option<String> {
    let mut cmd = std::process::Command::new("git");
    crate::shared::platform::path::apply_user_path(&mut cmd);
    let out = cmd
        .args(["-C", path, "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn target_from_url(url: &str, cwd: Option<String>) -> Option<RepoTarget> {
    let (provider, tool) = provider_for(url)?;
    Some(RepoTarget {
        provider: provider.to_string(),
        tool,
        cwd,
        owner_repo: super::commands::parse_owner_repo(url),
    })
}

/// Resolve a repo target for a board. Returns None when the board isn't
/// connected to a supported repo by any means.
pub async fn resolve_for_board(
    pool: &SqlitePool,
    board_id: &str,
) -> Result<Option<RepoTarget>, String> {
    let row: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT source_config, workspace_id FROM workspace_boards WHERE id = ?",
    )
    .bind(board_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB error reading board: {e}"))?;
    let (source_config, workspace_id) = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    let mut project_path: Option<String> = None;
    let mut project_url: Option<String> = None;
    if let Some(cfg) = source_config.as_deref() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(cfg) {
            project_url = v
                .get("project_url")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            project_path = v
                .get("project_path")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
        }
    }

    // 1. Explicit project URL.
    if let Some(url) = &project_url {
        if let Some(t) = target_from_url(url, project_path.clone()) {
            return Ok(Some(t));
        }
    }

    // 2/3. A local checkout → its origin remote (board path, else workspace path).
    let workspace = repo::get_workspace_by_id(pool, &workspace_id)
        .await
        .map_err(|e| format!("DB error reading workspace: {e}"))?;
    let path = project_path
        .clone()
        .or_else(|| workspace.project_path.clone())
        .map(|p| p.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(p) = &path {
        if let Some(remote) = git_remote_url(p) {
            if let Some((provider, tool)) = provider_for(&remote) {
                return Ok(Some(RepoTarget {
                    provider: provider.to_string(),
                    tool,
                    cwd: Some(p.clone()),
                    owner_repo: super::commands::parse_owner_repo(&remote),
                }));
            }
        }
    }

    // 4. Workspace repo_url fallback.
    if let Some(url) = workspace.repo_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(t) = target_from_url(url, path) {
            return Ok(Some(t));
        }
    }

    Ok(None)
}

/// Resolve a repo target for a card (looks up its board).
pub async fn resolve_for_card(
    pool: &SqlitePool,
    card_id: &str,
) -> Result<Option<RepoTarget>, String> {
    let board_id: Option<(String,)> = sqlx::query_as(
        "SELECT col.board_id FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         WHERE c.id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB error reading card: {e}"))?;
    match board_id {
        Some((bid,)) => resolve_for_board(pool, &bid).await,
        None => Ok(None),
    }
}

//! Resolve the updater endpoint URL for a given channel.

use serde::Deserialize;
use sqlx::SqlitePool;

const REPO_OWNER: &str = "ansxuman";
const REPO_NAME: &str = "Clauge";

const STABLE_ENDPOINT: &str =
    "https://github.com/ClaugeHQ/Clauge/releases/latest/download/latest.json";

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    prerelease: bool,
    draft: bool,
}

/// Returns the latest.json URL for the given channel.
///
/// `stable` always returns the GitHub "latest" auto-alias.
/// `pre` queries the releases API for the most recent pre-release that is
/// not a draft, then returns its asset URL. If none exists, falls back to
/// stable so the user still sees something.
///
/// Pool is needed because the pre-release lookup goes through our
/// proxy-aware HTTP client so corp users behind a mandatory proxy can
/// still reach the GitHub API.
pub async fn resolve_endpoint(pool: &SqlitePool, channel: &str) -> Result<String, String> {
    match channel {
        "stable" => Ok(STABLE_ENDPOINT.to_string()),
        "pre" => match find_latest_prerelease(pool).await {
            Ok(Some(tag)) => Ok(format!(
                "https://github.com/{}/{}/releases/download/{}/latest.json",
                REPO_OWNER, REPO_NAME, tag
            )),
            // No pre-release exists yet. Fall back to stable so the user
            // still gets meaningful update behaviour rather than an error.
            Ok(None) => Ok(STABLE_ENDPOINT.to_string()),
            Err(e) => Err(e),
        },
        other => Err(format!("unknown update channel: {}", other)),
    }
}

async fn find_latest_prerelease(pool: &SqlitePool) -> Result<Option<String>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases?per_page=10",
        REPO_OWNER, REPO_NAME
    );
    let client = crate::shared::http::build_app_http_client(pool).await?;
    // GitHub API rejects requests without a User-Agent.
    let releases: Vec<GhRelease> = client
        .get(&url)
        .header("User-Agent", "Clauge-Updater")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("github releases fetch: {}", e))?
        .json()
        .await
        .map_err(|e| format!("github releases parse: {}", e))?;

    Ok(releases
        .into_iter()
        .find(|r| r.prerelease && !r.draft)
        .map(|r| r.tag_name))
}

use sqlx::SqlitePool;
use tauri::State;

const GITHUB_TOKEN_KEY: &str = "github_token";

/// Parse a JSON response body from a reqwest Response (text -> serde_json).
async fn parse_json(resp: tauri_plugin_http::reqwest::Response) -> Result<serde_json::Value, String> {
    let text = resp.text().await.map_err(|e| format!("Failed to read response: {}", e))?;
    serde_json::from_str(&text).map_err(|e| format!("Failed to parse JSON: {}", e))
}

/// Store token in SQLite settings table
async fn store_token(pool: &SqlitePool, token: &str) -> Result<(), String> {
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(GITHUB_TOKEN_KEY)
        .bind(token)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to store token: {}", e))?;
    Ok(())
}

/// Read token from SQLite settings table
async fn read_token(pool: &SqlitePool) -> Result<Option<String>, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(GITHUB_TOKEN_KEY)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to read token: {}", e))?;
    Ok(row.map(|r| r.0))
}

/// Delete token from SQLite settings table
async fn delete_token(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(GITHUB_TOKEN_KEY)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to delete token: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn github_connect(pool: State<'_, SqlitePool>, token: String) -> Result<String, String> {
    let client = tauri_plugin_http::reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "Clauge/0.2")
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.status().is_success() {
        return Err("Invalid token — GitHub returned an error".to_string());
    }

    let user = parse_json(resp).await?;
    let username = user["login"].as_str().unwrap_or("unknown").to_string();

    store_token(pool.inner(), &token).await?;

    Ok(username)
}

#[tauri::command]
pub async fn github_disconnect(pool: State<'_, SqlitePool>) -> Result<(), String> {
    delete_token(pool.inner()).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubUserInfo {
    pub username: String,
    pub avatar_url: String,
}

#[tauri::command]
pub async fn github_get_status(pool: State<'_, SqlitePool>) -> Result<Option<GitHubUserInfo>, String> {
    let token = match read_token(pool.inner()).await? {
        Some(t) => t,
        None => return Ok(None),
    };

    let client = tauri_plugin_http::reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "Clauge/0.2")
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let user = parse_json(r).await?;
            Ok(Some(GitHubUserInfo {
                username: user["login"].as_str().unwrap_or("unknown").to_string(),
                avatar_url: user["avatar_url"].as_str().unwrap_or("").to_string(),
            }))
        }
        _ => Ok(None),
    }
}

// ── OAuth Flow ──────────────────────────────────────

const GITHUB_CLIENT_ID: &str = "Ov23lix7oiRoAvHlQmS4";
const GITHUB_OAUTH_URL: &str = "https://github.com/login/oauth/authorize";

#[tauri::command]
pub fn github_get_oauth_url() -> String {
    format!(
        "{}?client_id={}&redirect_uri={}&scope=gist",
        GITHUB_OAUTH_URL,
        GITHUB_CLIENT_ID,
        "https://clauge.dev/auth/callback"
    )
}

#[tauri::command]
pub async fn github_connect_with_token(pool: State<'_, SqlitePool>, token: String) -> Result<String, String> {
    let client = tauri_plugin_http::reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "Clauge/0.2")
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.status().is_success() {
        return Err("Invalid token — GitHub returned an error".to_string());
    }

    let user = parse_json(resp).await?;
    let username = user["login"].as_str().unwrap_or("unknown").to_string();

    store_token(pool.inner(), &token).await?;

    Ok(username)
}

/// Internal helper to retrieve the stored token (used by gist sync).
pub async fn get_token(pool: &SqlitePool) -> Result<String, String> {
    read_token(pool).await?.ok_or_else(|| "No GitHub token found".to_string())
}

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Mutex as StdMutex;
use tauri::State;

use crate::commands::nosql_client::NoSqlConnection;
use crate::commands::sql_client::{SqlSavedConnection, SqlScript};
use crate::db::models::{
    Collection, EnvVariable, Environment, Request, RequestHeader, RequestParam,
};

const GIST_DESCRIPTION: &str = "clauge-data-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionSync {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub context_prompt: String,
    pub skip_permissions: i32,
    pub git_name: Option<String>,
    pub git_email: Option<String>,
}

/// Track hash of last pushed data to avoid redundant pushes
static LAST_PUSH_HASH: StdMutex<Option<u64>> = StdMutex::new(None);

fn hash_data(json: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    json.hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncData {
    pub version: u32,
    pub synced_at: String,
    pub collections: Vec<Collection>,
    pub requests: Vec<Request>,
    pub request_headers: Vec<RequestHeader>,
    pub request_params: Vec<RequestParam>,
    pub environments: Vec<Environment>,
    pub env_variables: Vec<EnvVariable>,
    #[serde(default)]
    pub sql_connections: Vec<SqlSavedConnection>,
    #[serde(default)]
    pub nosql_connections: Vec<NoSqlConnection>,
    #[serde(default)]
    pub sql_scripts: Vec<SqlScript>,
    #[serde(default)]
    pub agent_contexts: Vec<crate::commands::agent_models::AgentContext>,
    #[serde(default)]
    pub agent_sessions_sync: Vec<AgentSessionSync>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncInfo {
    pub gist_id: Option<String>,
    pub synced_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GistResponse {
    id: String,
    description: Option<String>,
    public: Option<bool>,
    files: std::collections::HashMap<String, GistFile>,
}

#[derive(Debug, Deserialize)]
struct GistFile {
    content: Option<String>,
}

/// Export all data from the local database into a SyncData struct.
async fn export_data(pool: &SqlitePool) -> Result<SyncData, String> {
    let collections = sqlx::query_as::<_, Collection>("SELECT * FROM collections ORDER BY sort_order")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to export collections: {}", e))?;

    let requests = sqlx::query_as::<_, Request>("SELECT * FROM requests ORDER BY sort_order")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to export requests: {}", e))?;

    let request_headers =
        sqlx::query_as::<_, RequestHeader>("SELECT * FROM request_headers ORDER BY sort_order")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("Failed to export request_headers: {}", e))?;

    let request_params =
        sqlx::query_as::<_, RequestParam>("SELECT * FROM request_params ORDER BY sort_order")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("Failed to export request_params: {}", e))?;

    let environments =
        sqlx::query_as::<_, Environment>("SELECT * FROM environments ORDER BY sort_order")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("Failed to export environments: {}", e))?;

    let env_variables =
        sqlx::query_as::<_, EnvVariable>("SELECT * FROM env_variables ORDER BY sort_order")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("Failed to export env_variables: {}", e))?;

    let sql_connections =
        sqlx::query_as::<_, SqlSavedConnection>("SELECT * FROM sql_connections ORDER BY sort_order")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("Failed to export sql_connections: {}", e))?;

    let nosql_connections =
        sqlx::query_as::<_, NoSqlConnection>("SELECT * FROM nosql_connections ORDER BY sort_order")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("Failed to export nosql_connections: {}", e))?;

    let sql_scripts =
        sqlx::query_as::<_, SqlScript>("SELECT * FROM sql_scripts ORDER BY sort_order")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("Failed to export sql_scripts: {}", e))?;

    let agent_contexts = sqlx::query_as::<_, crate::commands::agent_models::AgentContext>(
        "SELECT * FROM agent_contexts ORDER BY name"
    ).fetch_all(pool).await.unwrap_or_default();

    let agent_sessions_sync: Vec<AgentSessionSync> = sqlx::query_as::<_, (String, String, String, String, i32, Option<String>, Option<String>)>(
        "SELECT id, title, purpose, context_prompt, skip_permissions, git_name, git_email FROM agent_sessions"
    ).fetch_all(pool).await.unwrap_or_default().into_iter().map(|r| AgentSessionSync {
        id: r.0, title: r.1, purpose: r.2, context_prompt: r.3,
        skip_permissions: r.4, git_name: r.5, git_email: r.6,
    }).collect();

    let now = chrono::Utc::now().to_rfc3339();

    Ok(SyncData {
        version: 1,
        synced_at: now,
        collections,
        requests,
        request_headers,
        request_params,
        environments,
        env_variables,
        sql_connections,
        nosql_connections,
        sql_scripts,
        agent_contexts,
        agent_sessions_sync,
    })
}

/// Find the Clauge gist by description (paginates through all gists).
async fn find_clauge_gist(
    client: &tauri_plugin_http::reqwest::Client,
    token: &str,
) -> Result<Option<String>, String> {
    let mut page = 1u32;
    loop {
        let resp = client
            .get(format!(
                "https://api.github.com/gists?per_page=100&page={}",
                page
            ))
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "Clauge/0.1")
            .send()
            .await
            .map_err(|e| format!("Failed to list gists: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("GitHub API error: {}", resp.status()));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read gists: {}", e))?;
        let gists: Vec<GistResponse> =
            serde_json::from_str(&text).map_err(|e| format!("Failed to parse gists: {}", e))?;

        if gists.is_empty() {
            break;
        }

        for gist in &gists {
            let desc = gist.description.as_deref();
            if desc == Some(GIST_DESCRIPTION) {
                if gist.public.unwrap_or(false) {
                    log::warn!("[Sync] Found PUBLIC gist {} — skipping, user should delete manually", gist.id);
                    continue; // Skip public gists, only use secret ones
                }
                return Ok(Some(gist.id.clone()));
            }
        }

        // If fewer than 100 results, this was the last page
        if gists.len() < 100 {
            break;
        }

        page += 1;
    }

    Ok(None)
}

#[tauri::command]
pub async fn gist_check_exists(pool: State<'_, SqlitePool>) -> Result<bool, String> {
    let token = super::oauth::get_token(pool.inner()).await?;
    let client = tauri_plugin_http::reqwest::Client::new();
    let gist_id = find_clauge_gist(&client, &token).await?;
    Ok(gist_id.is_some())
}

#[tauri::command]
pub async fn gist_sync_push(pool: State<'_, SqlitePool>) -> Result<String, String> {
    let token = super::oauth::get_token(pool.inner()).await?;
    let data = export_data(pool.inner()).await?;
    log::info!(
        "[Sync Push] Exporting: {} collections, {} requests, {} envs, {} sql_conns, {} nosql_conns",
        data.collections.len(), data.requests.len(), data.environments.len(),
        data.sql_connections.len(), data.nosql_connections.len()
    );

    // Never push empty data — protects cloud backup from being wiped
    // Only check user-created data (not environments, which may be empty on fresh install)
    if data.collections.is_empty() && data.requests.is_empty()
        && data.sql_connections.is_empty() && data.nosql_connections.is_empty() && data.sql_scripts.is_empty()
        && data.agent_contexts.is_empty() {
        log::warn!("[Sync Push] Skipped — no user data, not overwriting cloud backup");
        return Ok("Skipped — nothing to push".to_string());
    }

    let json_content =
        serde_json::to_string_pretty(&data).map_err(|e| format!("Serialization error: {}", e))?;

    // Skip push if data hasn't changed since last push
    let current_hash = hash_data(&json_content);
    {
        let last = LAST_PUSH_HASH.lock().unwrap();
        if *last == Some(current_hash) {
            log::info!("[Sync Push] Skipped — data unchanged since last push");
            return Ok("Skipped — no changes".to_string());
        }
    }

    let client = tauri_plugin_http::reqwest::Client::new();

    // Build the files payload
    let files = serde_json::json!({
        "clauge-data.json": {
            "content": json_content
        }
    });

    // Check if gist already exists
    let existing_gist_id = find_clauge_gist(&client, &token).await?;

    let result_msg = if let Some(gist_id) = existing_gist_id {
        // Update existing gist
        let payload = serde_json::json!({
            "description": GIST_DESCRIPTION,
            "files": files
        });

        let payload_str = serde_json::to_string(&payload)
            .map_err(|e| format!("Serialization error: {}", e))?;
        let resp = client
            .patch(format!("https://api.github.com/gists/{}", gist_id))
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "Clauge/0.1")
            .header("Content-Type", "application/json")
            .body(payload_str)
            .send()
            .await
            .map_err(|e| format!("Failed to update gist: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Failed to update gist ({}): {}", status, body));
        }

        format!("Updated gist {}", gist_id)
    } else {
        // Create new gist
        let payload = serde_json::json!({
            "description": GIST_DESCRIPTION,
            "public": false,
            "files": files
        });

        let payload_str = serde_json::to_string(&payload)
            .map_err(|e| format!("Serialization error: {}", e))?;
        let resp = client
            .post("https://api.github.com/gists")
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "Clauge/0.1")
            .header("Content-Type", "application/json")
            .body(payload_str)
            .send()
            .await
            .map_err(|e| format!("Failed to create gist: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Failed to create gist ({}): {}", status, body));
        }

        let resp_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read gist creation response: {}", e))?;
        let created: serde_json::Value = serde_json::from_str(&resp_text)
            .map_err(|e| format!("Failed to parse gist response: {}", e))?;
        let gist_id = created["id"]
            .as_str()
            .ok_or_else(|| "Gist created but response missing ID".to_string())?;
        format!("Created gist {}", gist_id)
    };

    // Update hash after successful push
    {
        let mut last = LAST_PUSH_HASH.lock().unwrap();
        *last = Some(current_hash);
    }

    Ok(result_msg)
}

#[tauri::command]
pub async fn gist_sync_pull(pool: State<'_, SqlitePool>) -> Result<String, String> {
    let token = super::oauth::get_token(pool.inner()).await?;
    log::info!("[Sync Pull] Token found, searching for gist...");
    let client = tauri_plugin_http::reqwest::Client::new();

    // Find the Clauge gist
    let gist_id = find_clauge_gist(&client, &token)
        .await?
        .ok_or_else(|| "No Clauge sync gist found. Push first to create one.".to_string())?;
    log::info!("[Sync Pull] Found gist: {}", gist_id);

    // Fetch the gist content
    let resp = client
        .get(format!("https://api.github.com/gists/{}", gist_id))
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "Clauge/0.1")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch gist: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Failed to fetch gist: {}", resp.status()));
    }

    let gist_text = resp.text().await.map_err(|e| format!("Failed to read gist: {}", e))?;
    let gist: GistResponse = serde_json::from_str(&gist_text)
        .map_err(|e| format!("Failed to parse gist: {}", e))?;

    let file_content = gist
        .files
        .get("clauge-data.json")
        .and_then(|f| f.content.as_deref())
        .ok_or_else(|| "Gist does not contain sync data file".to_string())?;

    let data: SyncData = serde_json::from_str(file_content)
        .map_err(|e| format!("Failed to parse sync data: {}", e))?;

    log::info!(
        "[Sync Pull] Data: {} collections, {} requests, {} envs, {} sql_conns, {} nosql_conns, {} scripts",
        data.collections.len(), data.requests.len(), data.environments.len(),
        data.sql_connections.len(), data.nosql_connections.len(), data.sql_scripts.len()
    );

    if data.version != 1 {
        return Err(format!(
            "Unsupported sync data version {}. This app supports version 1. Please update Clauge.",
            data.version
        ));
    }

    if data.collections.is_empty() && data.requests.is_empty() && data.environments.is_empty() {
        return Err("Cloud backup is empty — nothing to restore.".to_string());
    }

    // Upsert all data into local database
    import_data(pool.inner(), &data).await?;
    log::info!("[Sync Pull] Import complete");

    // Clear push hash so next push will detect the new data
    {
        let mut last = LAST_PUSH_HASH.lock().unwrap();
        *last = None;
    }

    Ok(format!("Restored {} collections, {} requests from cloud (synced at {})",
        data.collections.len(), data.requests.len(), data.synced_at))
}

/// Import sync data into the local database (full replace via transaction).
/// Remote data is treated as source of truth: local data is deleted and replaced.
/// For SQL/NoSQL connections, local passwords are preserved if the connection already exists.
async fn import_data(pool: &SqlitePool, data: &SyncData) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| format!("Failed to begin transaction: {}", e))?;

    // Delete all synced tables (order matters for foreign keys)
    // Child tables first, then parent tables
    sqlx::query("DELETE FROM env_variables").execute(&mut *tx).await
        .map_err(|e| format!("Failed to clear env_variables: {}", e))?;
    sqlx::query("DELETE FROM request_headers").execute(&mut *tx).await
        .map_err(|e| format!("Failed to clear request_headers: {}", e))?;
    sqlx::query("DELETE FROM request_params").execute(&mut *tx).await
        .map_err(|e| format!("Failed to clear request_params: {}", e))?;
    sqlx::query("DELETE FROM requests").execute(&mut *tx).await
        .map_err(|e| format!("Failed to clear requests: {}", e))?;
    sqlx::query("DELETE FROM collections").execute(&mut *tx).await
        .map_err(|e| format!("Failed to clear collections: {}", e))?;
    sqlx::query("DELETE FROM environments").execute(&mut *tx).await
        .map_err(|e| format!("Failed to clear environments: {}", e))?;
    sqlx::query("DELETE FROM sql_connections").execute(&mut *tx).await
        .map_err(|e| format!("Failed to clear sql_connections: {}", e))?;
    sqlx::query("DELETE FROM nosql_connections").execute(&mut *tx).await
        .map_err(|e| format!("Failed to clear nosql_connections: {}", e))?;
    sqlx::query("DELETE FROM sql_scripts").execute(&mut *tx).await
        .map_err(|e| format!("Failed to clear sql_scripts: {}", e))?;

    // Insert collections
    for c in &data.collections {
        sqlx::query(
            "INSERT INTO collections (id, name, description, sort_order, env_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.name)
        .bind(&c.description)
        .bind(c.sort_order)
        .bind(&c.env_id)
        .bind(&c.created_at)
        .bind(&c.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to insert collection: {}", e))?;
    }

    // Insert requests
    for r in &data.requests {
        sqlx::query(
            "INSERT INTO requests (id, collection_id, name, description, method, url, body, body_type, auth_type, auth_data, pre_script, sort_order, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.id)
        .bind(&r.collection_id)
        .bind(&r.name)
        .bind(&r.description)
        .bind(&r.method)
        .bind(&r.url)
        .bind(&r.body)
        .bind(&r.body_type)
        .bind(&r.auth_type)
        .bind(&r.auth_data)
        .bind(&r.pre_script)
        .bind(r.sort_order)
        .bind(&r.created_at)
        .bind(&r.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to insert request: {}", e))?;
    }

    // Insert request headers
    for h in &data.request_headers {
        sqlx::query(
            "INSERT INTO request_headers (id, request_id, key, value, enabled, sort_order) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&h.id)
        .bind(&h.request_id)
        .bind(&h.key)
        .bind(&h.value)
        .bind(h.enabled)
        .bind(h.sort_order)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to insert request header: {}", e))?;
    }

    // Insert request params
    for p in &data.request_params {
        sqlx::query(
            "INSERT INTO request_params (id, request_id, key, value, enabled, sort_order) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&p.id)
        .bind(&p.request_id)
        .bind(&p.key)
        .bind(&p.value)
        .bind(p.enabled)
        .bind(p.sort_order)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to insert request param: {}", e))?;
    }

    // Insert environments
    for e in &data.environments {
        sqlx::query(
            "INSERT INTO environments (id, name, color, is_default, sort_order, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&e.id)
        .bind(&e.name)
        .bind(&e.color)
        .bind(e.is_default)
        .bind(e.sort_order)
        .bind(&e.created_at)
        .bind(&e.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|ee| format!("Failed to insert environment: {}", ee))?;
    }

    // Insert env variables
    for v in &data.env_variables {
        sqlx::query(
            "INSERT INTO env_variables (id, environment_id, key, value, is_secret, sort_order) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&v.id)
        .bind(&v.environment_id)
        .bind(&v.key)
        .bind(&v.value)
        .bind(v.is_secret)
        .bind(v.sort_order)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to insert env variable: {}", e))?;
    }

    // Insert SQL connections (passwords now synced from gist)
    for c in &data.sql_connections {
        sqlx::query(
            "INSERT INTO sql_connections (id, name, driver, host, port, database_name, username, password, ssl, sort_order, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.name)
        .bind(&c.driver)
        .bind(&c.host)
        .bind(c.port)
        .bind(&c.database_name)
        .bind(&c.username)
        .bind(&c.password)
        .bind(c.ssl)
        .bind(c.sort_order)
        .bind(&c.created_at)
        .bind(&c.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to insert sql_connection: {}", e))?;
    }

    // Insert NoSQL connections (passwords now synced from gist)
    for c in &data.nosql_connections {
        sqlx::query(
            "INSERT INTO nosql_connections (id, name, driver, connection_string, host, port, database_name, username, password, ssl, direct_connection, sort_order, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.name)
        .bind(&c.driver)
        .bind(&c.connection_string)
        .bind(&c.host)
        .bind(c.port)
        .bind(&c.database_name)
        .bind(&c.username)
        .bind(&c.password)
        .bind(c.ssl)
        .bind(c.direct_connection)
        .bind(c.sort_order)
        .bind(&c.created_at)
        .bind(&c.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to insert nosql_connection: {}", e))?;
    }

    // Insert SQL scripts
    for s in &data.sql_scripts {
        sqlx::query(
            "INSERT INTO sql_scripts (id, name, connection_id, database_name, query, sort_order, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&s.id)
        .bind(&s.name)
        .bind(&s.connection_id)
        .bind(&s.database_name)
        .bind(&s.query)
        .bind(s.sort_order)
        .bind(&s.created_at)
        .bind(&s.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to insert sql_script: {}", e))?;
    }

    // Import agent contexts
    sqlx::query("DELETE FROM agent_contexts").execute(&mut *tx).await.ok();
    for ctx in &data.agent_contexts {
        sqlx::query("INSERT OR REPLACE INTO agent_contexts (id, name, content, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&ctx.id).bind(&ctx.name).bind(&ctx.content).bind(&ctx.created_at).bind(&ctx.updated_at)
            .execute(&mut *tx).await.ok();
    }

    // Import agent sessions (INSERT OR IGNORE — don't overwrite machine-specific fields)
    for s in &data.agent_sessions_sync {
        sqlx::query("INSERT OR IGNORE INTO agent_sessions (id, title, purpose, project_path, project_name, context_prompt, skip_permissions, git_name, git_email, created_at, last_used_at) VALUES (?, ?, ?, '', '', ?, ?, ?, ?, datetime('now'), datetime('now'))")
            .bind(&s.id).bind(&s.title).bind(&s.purpose).bind(&s.context_prompt)
            .bind(s.skip_permissions).bind(&s.git_name).bind(&s.git_email)
            .execute(&mut *tx).await.ok();
    }

    tx.commit().await.map_err(|e| format!("Failed to commit transaction: {}", e))?;

    Ok(())
}

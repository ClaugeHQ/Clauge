use crate::commands::credential_store::{credential_store, CredentialStore};
use crate::commands::ssh_models::SshProfile;
use sqlx::SqlitePool;
use tauri::State;

#[tauri::command]
pub async fn ssh_list_profiles(pool: State<'_, SqlitePool>) -> Result<Vec<SshProfile>, String> {
    sqlx::query_as::<_, SshProfile>(
        "SELECT id, name, host, port, username, auth_type, key_path, accent_color, last_used_at, created_at \
         FROM ssh_profiles \
         ORDER BY (last_used_at IS NULL), last_used_at DESC, created_at DESC",
    )
    .fetch_all(pool.inner())
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ssh_create_profile(
    pool: State<'_, SqlitePool>,
    name: String,
    host: String,
    port: i64,
    username: String,
    auth_type: String,
    key_path: Option<String>,
    accent_color: Option<String>,
    secret: Option<String>,
) -> Result<SshProfile, String> {
    if auth_type != "key" && auth_type != "password" {
        return Err(format!("invalid auth_type: {}", auth_type));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    sqlx::query(
        "INSERT INTO ssh_profiles (id, name, host, port, username, auth_type, key_path, accent_color, last_used_at, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&host)
    .bind(port)
    .bind(&username)
    .bind(&auth_type)
    .bind(&key_path)
    .bind(&accent_color)
    .bind(&now)
    .execute(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    if let Some(s) = secret {
        if !s.is_empty() {
            credential_store()
                .store(&id, &s)
                .await
                .map_err(|e| format!("credential store: {}", e))?;
        }
    }

    sqlx::query_as::<_, SshProfile>("SELECT * FROM ssh_profiles WHERE id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ssh_update_profile(
    pool: State<'_, SqlitePool>,
    id: String,
    name: Option<String>,
    host: Option<String>,
    port: Option<i64>,
    username: Option<String>,
    auth_type: Option<String>,
    key_path: Option<String>,
    accent_color: Option<String>,
    secret: Option<String>,
) -> Result<SshProfile, String> {
    if let Some(ref n) = name {
        sqlx::query("UPDATE ssh_profiles SET name = ? WHERE id = ?")
            .bind(n)
            .bind(&id)
            .execute(pool.inner())
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref h) = host {
        sqlx::query("UPDATE ssh_profiles SET host = ? WHERE id = ?")
            .bind(h)
            .bind(&id)
            .execute(pool.inner())
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(p) = port {
        sqlx::query("UPDATE ssh_profiles SET port = ? WHERE id = ?")
            .bind(p)
            .bind(&id)
            .execute(pool.inner())
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref u) = username {
        sqlx::query("UPDATE ssh_profiles SET username = ? WHERE id = ?")
            .bind(u)
            .bind(&id)
            .execute(pool.inner())
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref a) = auth_type {
        if a != "key" && a != "password" {
            return Err(format!("invalid auth_type: {}", a));
        }
        sqlx::query("UPDATE ssh_profiles SET auth_type = ? WHERE id = ?")
            .bind(a)
            .bind(&id)
            .execute(pool.inner())
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref kp) = key_path {
        sqlx::query("UPDATE ssh_profiles SET key_path = ? WHERE id = ?")
            .bind(kp)
            .bind(&id)
            .execute(pool.inner())
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref ac) = accent_color {
        sqlx::query("UPDATE ssh_profiles SET accent_color = ? WHERE id = ?")
            .bind(ac)
            .bind(&id)
            .execute(pool.inner())
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(s) = secret {
        let store = credential_store();
        // Replace: delete then store. delete is best-effort idempotent.
        let _ = store.delete(&id).await;
        if !s.is_empty() {
            store
                .store(&id, &s)
                .await
                .map_err(|e| format!("credential store: {}", e))?;
        }
    }

    sqlx::query_as::<_, SshProfile>("SELECT * FROM ssh_profiles WHERE id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ssh_delete_profile(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    // Best-effort: clear credential first. Failures here shouldn't block row deletion.
    let _ = credential_store().delete(&id).await;
    sqlx::query("DELETE FROM ssh_profiles WHERE id = ?")
        .bind(&id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn ssh_touch_profile(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    // Use ISO-8601 (RFC 3339) so the value parses reliably in WKWebView's Date.
    // SQLite's `datetime('now')` returns "YYYY-MM-DD HH:MM:SS" which can yield
    // Invalid Date in Safari/WKWebView. Match the format used by created_at.
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query("UPDATE ssh_profiles SET last_used_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn ssh_get_credential(id: String) -> Result<Option<String>, String> {
    credential_store().get(&id).await
}

use crate::modes::ssh::models::SshProfile;
use crate::shared::platform::credential_store::{credential_store, CredentialStore};
use crate::shared::repos::ssh_profiles as ssh_profiles_repo;
use sqlx::SqlitePool;
use tauri::State;

#[tauri::command]
pub async fn ssh_list_profiles(pool: State<'_, SqlitePool>) -> Result<Vec<SshProfile>, String> {
    ssh_profiles_repo::list_all(pool.inner())
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
    jump_profile_id: Option<String>,
    proxy_command: Option<String>,
) -> Result<SshProfile, String> {
    if !matches!(auth_type.as_str(), "key" | "password" | "agent" | "interactive") {
        return Err(format!("invalid auth_type: {}", auth_type));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    ssh_profiles_repo::insert(
        pool.inner(),
        &id,
        &name,
        &host,
        port,
        &username,
        &auth_type,
        key_path.as_deref(),
        accent_color.as_deref(),
        &now,
        jump_profile_id.as_deref(),
        proxy_command.as_deref(),
    )
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

    crate::cloud::scheduler::bump("ssh");

    ssh_profiles_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

/// Update profile fields. Each `Option<T>` follows the existing
/// "send-or-leave-alone" convention: `Some(value)` updates, `None`
/// leaves the column untouched.
///
/// For `jump_profile_id` and `proxy_command`, sending an empty string
/// clears the field (the connect path treats empty as "no proxy" — see
/// ssh_session::open_authenticated_ssh_session). Sending the field as
/// `null`/omitted leaves the existing value untouched.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
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
    jump_profile_id: Option<String>,
    proxy_command: Option<String>,
) -> Result<SshProfile, String> {
    if let Some(ref n) = name {
        ssh_profiles_repo::update_name(pool.inner(), &id, n)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref h) = host {
        ssh_profiles_repo::update_host(pool.inner(), &id, h)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(p) = port {
        ssh_profiles_repo::update_port(pool.inner(), &id, p)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref u) = username {
        ssh_profiles_repo::update_username(pool.inner(), &id, u)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref a) = auth_type {
        if !matches!(a.as_str(), "key" | "password" | "agent" | "interactive") {
            return Err(format!("invalid auth_type: {}", a));
        }
        ssh_profiles_repo::update_auth_type(pool.inner(), &id, a)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref kp) = key_path {
        ssh_profiles_repo::update_key_path(pool.inner(), &id, kp)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref ac) = accent_color {
        ssh_profiles_repo::update_accent_color(pool.inner(), &id, ac)
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
    // Some(value) updates the column; None leaves it alone. Empty string is
    // stored as-is — the connect path filters empty strings as "no proxy"
    // (since OpenSSH itself can't have a meaningful empty proxy config).
    if let Some(ref jump) = jump_profile_id {
        // Treat empty string as explicit clear (NULL the column) so the
        // foreign-key SET NULL semantics work cleanly.
        let val = if jump.is_empty() { None } else { Some(jump.as_str()) };
        ssh_profiles_repo::update_jump_profile_id(pool.inner(), &id, val)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref cmd) = proxy_command {
        let val = if cmd.is_empty() { None } else { Some(cmd.as_str()) };
        ssh_profiles_repo::update_proxy_command(pool.inner(), &id, val)
            .await
            .map_err(|e| e.to_string())?;
    }

    crate::cloud::scheduler::bump("ssh");

    ssh_profiles_repo::get_by_id(pool.inner(), &id)
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
    ssh_profiles_repo::delete_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("ssh");
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
    ssh_profiles_repo::touch_last_used(pool.inner(), &id, &now)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ssh_get_credential(id: String) -> Result<Option<String>, String> {
    credential_store().get(&id).await
}

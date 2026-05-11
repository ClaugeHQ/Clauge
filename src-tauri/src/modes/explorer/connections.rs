//! Tauri commands for `explorer_connections` CRUD. Secrets live in
//! `CredentialStore` (service `"Clauge Explorer"`), never in this table.

use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

use crate::modes::explorer::models::ExplorerConnection;
use crate::shared::repos::explorer as repo;

#[tauri::command]
pub async fn explorer_list_connections(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<ExplorerConnection>, String> {
    repo::list(pool.inner())
        .await
        .map_err(|e| format!("list connections: {}", e))
}

#[tauri::command]
pub async fn explorer_get_connection(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<Option<ExplorerConnection>, String> {
    repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| format!("get connection: {}", e))
}

#[tauri::command]
pub async fn explorer_create_connection(
    pool: State<'_, SqlitePool>,
    mut connection: ExplorerConnection,
) -> Result<ExplorerConnection, String> {
    if connection.id.is_empty() {
        connection.id = Uuid::new_v4().to_string();
    }
    if connection.created_at.is_empty() {
        connection.created_at =
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    }
    repo::insert(pool.inner(), &connection)
        .await
        .map_err(|e| format!("create connection: {}", e))?;
    crate::cloud::scheduler::bump("explorer");
    Ok(connection)
}

#[tauri::command]
pub async fn explorer_update_connection(
    pool: State<'_, SqlitePool>,
    connection: ExplorerConnection,
) -> Result<(), String> {
    repo::update(pool.inner(), &connection)
        .await
        .map_err(|e| format!("update connection: {}", e))?;
    crate::cloud::scheduler::bump("explorer");
    Ok(())
}

#[tauri::command]
pub async fn explorer_delete_connection(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    repo::delete_by_id(pool.inner(), &id)
        .await
        .map_err(|e| format!("delete connection: {}", e))?;
    crate::cloud::scheduler::bump("explorer");
    Ok(())
}

/// Store a secret for an explorer connection (S3 access keys, Azure shared
/// keys / SAS tokens / connection strings, FTP / SFTP passwords). Keychain
/// account = `explorer:<connection_id>:<secret_name>` so multiple secrets
/// per connection (e.g. access_key + secret_key for S3) coexist alongside
/// the existing SSH credential entries.
#[tauri::command]
pub async fn explorer_set_secret(
    connection_id: String,
    secret_name: String,
    value: String,
) -> Result<(), String> {
    use crate::shared::platform::credential_store::CredentialStore;
    crate::shared::platform::credential_store::credential_store()
        .store(&explorer_secret_key(&connection_id, &secret_name), &value)
        .await
        .map_err(|e| format!("store secret: {}", e))
}

#[tauri::command]
pub async fn explorer_get_secret(
    connection_id: String,
    secret_name: String,
) -> Result<Option<String>, String> {
    use crate::shared::platform::credential_store::CredentialStore;
    crate::shared::platform::credential_store::credential_store()
        .get(&explorer_secret_key(&connection_id, &secret_name))
        .await
        .map_err(|e| format!("get secret: {}", e))
}

#[tauri::command]
pub async fn explorer_delete_secrets(connection_id: String) -> Result<(), String> {
    use crate::shared::platform::credential_store::CredentialStore;
    let store = crate::shared::platform::credential_store::credential_store();
    // Best-effort: each backend declares its own secret_name conventions.
    // We attempt the common ones; "not found" is fine.
    for name in &[
        "password", "passphrase", "access_key", "secret_key",
        "sas_token", "shared_key", "connection_string",
    ] {
        let _ = store
            .delete(&explorer_secret_key(&connection_id, name))
            .await;
    }
    Ok(())
}

fn explorer_secret_key(connection_id: &str, secret_name: &str) -> String {
    format!("explorer:{}:{}", connection_id, secret_name)
}

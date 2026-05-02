//! Active-session map (tabKey → Box<dyn RemoteFs>) + Tauri commands for
//! opening / closing sessions and operating on remote files.
//!
//! Frontend invokes these by tab-key (each Explorer tab = one session).

use base64::Engine;
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

use crate::modes::explorer::backends::azure_blob::{AzureAuth, AzureBlobBackend};
use crate::modes::explorer::backends::ftp::FtpBackend;
use crate::modes::explorer::backends::s3::S3Backend;
use crate::modes::explorer::backends::sftp::SftpBackend;
use crate::modes::explorer::fs::RemoteFs;
use crate::modes::explorer::models::{DirEntry, ExplorerConnection, FsError, Stat};
use crate::shared::http::build_app_http_client;
use crate::shared::platform::credential_store::{credential_store, CredentialStore};
use crate::shared::repos::explorer as repo;

#[derive(Default)]
pub struct ExplorerSessions {
    map: RwLock<HashMap<String, Arc<dyn RemoteFs>>>,
}

impl ExplorerSessions {
    pub async fn get(&self, tab_key: &str) -> Option<Arc<dyn RemoteFs>> {
        self.map.read().await.get(tab_key).cloned()
    }
    pub async fn set(&self, tab_key: String, fs: Arc<dyn RemoteFs>) {
        self.map.write().await.insert(tab_key, fs);
    }
    pub async fn remove(&self, tab_key: &str) {
        self.map.write().await.remove(tab_key);
    }
}

fn map_fs_err(e: FsError) -> String {
    e.to_string()
}

async fn open_backend(
    pool: &SqlitePool,
    conn: &ExplorerConnection,
) -> Result<Arc<dyn RemoteFs>, String> {
    match conn.kind.as_str() {
        "sftp" => {
            let handle = if let Some(profile_id) = conn.ssh_profile_id.as_ref() {
                crate::modes::ssh::ssh_session::open_authenticated_ssh_session(pool, profile_id)
                    .await?
            } else {
                // Direct credentials: build the SshAuthSpec from the
                // explorer_connections row + secret stored under
                // explorer:<id>:password (auth=password) or
                // explorer:<id>:passphrase (auth=key).
                let host = conn
                    .host
                    .clone()
                    .ok_or("SFTP connection missing host")?;
                let port = conn.port.unwrap_or(22) as u16;
                let username = conn
                    .username
                    .clone()
                    .ok_or("SFTP connection missing username")?;
                let auth_type = conn
                    .auth_type
                    .clone()
                    .unwrap_or_else(|| "password".to_string());
                let secret_name = match auth_type.as_str() {
                    "password" => Some("password"),
                    "key" => Some("passphrase"),
                    _ => None,
                };
                let secret = if let Some(name) = secret_name {
                    credential_store()
                        .get(&format!("explorer:{}:{}", conn.id, name))
                        .await
                        .map_err(|e| format!("ssh secret lookup: {}", e))?
                } else {
                    None
                };
                let spec = crate::modes::ssh::ssh_session::SshAuthSpec {
                    host,
                    port,
                    username,
                    auth_type,
                    key_path: conn.key_path.clone(),
                    secret,
                };
                crate::modes::ssh::ssh_session::open_authenticated_ssh_session_with_spec(spec)
                    .await?
            };
            let backend = SftpBackend::open(handle).await.map_err(map_fs_err)?;
            Ok(Arc::new(backend))
        }
        "ftp" => {
            let host = conn
                .host
                .as_deref()
                .ok_or("FTP connection missing host")?;
            let port = conn.port.unwrap_or(21) as u16;
            let username = conn.username.as_deref().unwrap_or("anonymous");
            let store = credential_store();
            let password = store
                .get(&format!("explorer:{}:password", conn.id))
                .await
                .map_err(|e| format!("password lookup: {}", e))?
                .unwrap_or_default();
            let tls = conn.ftp_tls.as_deref().unwrap_or("none");
            let backend = FtpBackend::connect(host, port, username, &password, tls)
                .await
                .map_err(map_fs_err)?;
            Ok(Arc::new(backend))
        }
        "s3" => {
            let endpoint = conn
                .s3_endpoint
                .as_deref()
                .ok_or("S3 connection missing endpoint")?;
            let region = conn.s3_region.as_deref().unwrap_or("us-east-1");
            let bucket = conn.s3_bucket.as_deref().unwrap_or("");
            let store = credential_store();
            let access_key = store
                .get(&format!("explorer:{}:access_key", conn.id))
                .await
                .map_err(|e| format!("access_key lookup: {}", e))?
                .ok_or("missing S3 access key")?;
            let secret_key = store
                .get(&format!("explorer:{}:secret_key", conn.id))
                .await
                .map_err(|e| format!("secret_key lookup: {}", e))?
                .ok_or("missing S3 secret key")?;
            let http = build_app_http_client(pool).await?;
            let backend = S3Backend::new(
                endpoint,
                region,
                bucket,
                &access_key,
                &secret_key,
                conn.s3_path_style != 0,
                http,
            )
            .map_err(map_fs_err)?;
            Ok(Arc::new(backend))
        }
        "azure_blob" => {
            let account = conn
                .azure_account
                .as_deref()
                .ok_or("Azure connection missing account")?;
            let container = conn
                .azure_container
                .as_deref()
                .ok_or("Azure connection missing container")?;
            let auth_kind = conn.azure_auth_kind.as_deref().unwrap_or("shared_key");
            let store = credential_store();
            let auth = match auth_kind {
                "shared_key" => {
                    let key = store
                        .get(&format!("explorer:{}:shared_key", conn.id))
                        .await
                        .map_err(|e| format!("shared_key lookup: {}", e))?
                        .ok_or("missing Azure shared key")?;
                    AzureAuth::SharedKey {
                        account: account.to_string(),
                        key,
                    }
                }
                "sas" => {
                    let token = store
                        .get(&format!("explorer:{}:sas_token", conn.id))
                        .await
                        .map_err(|e| format!("sas_token lookup: {}", e))?
                        .ok_or("missing Azure SAS token")?;
                    AzureAuth::Sas {
                        account: account.to_string(),
                        token,
                    }
                }
                "connection_string" => {
                    let cs = store
                        .get(&format!("explorer:{}:connection_string", conn.id))
                        .await
                        .map_err(|e| format!("connection_string lookup: {}", e))?
                        .ok_or("missing Azure connection string")?;
                    AzureAuth::ConnectionString(cs)
                }
                other => return Err(format!("unknown Azure auth kind: {}", other)),
            };
            let backend = AzureBlobBackend::new(auth, container).map_err(map_fs_err)?;
            Ok(Arc::new(backend))
        }
        other => Err(format!("unknown explorer kind: {}", other)),
    }
}

#[tauri::command]
pub async fn explorer_open_session(
    pool: State<'_, SqlitePool>,
    sessions: State<'_, ExplorerSessions>,
    connection_id: String,
    tab_key: String,
) -> Result<(), String> {
    let conn = repo::get_by_id(pool.inner(), &connection_id)
        .await
        .map_err(|e| format!("connection lookup: {}", e))?
        .ok_or_else(|| format!("connection {} not found", connection_id))?;
    let fs = open_backend(pool.inner(), &conn).await?;
    sessions.set(tab_key, fs).await;
    let _ = repo::touch_last_used(pool.inner(), &connection_id).await;
    Ok(())
}

#[tauri::command]
pub async fn explorer_close_session(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
) -> Result<(), String> {
    sessions.remove(&tab_key).await;
    Ok(())
}

#[tauri::command]
pub async fn explorer_fs_list(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
    path: String,
) -> Result<Vec<DirEntry>, String> {
    let fs = sessions
        .get(&tab_key)
        .await
        .ok_or("no active session for this tab")?;
    fs.list(&path).await.map_err(map_fs_err)
}

#[tauri::command]
pub async fn explorer_fs_stat(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
    path: String,
) -> Result<Stat, String> {
    let fs = sessions.get(&tab_key).await.ok_or("no active session")?;
    fs.stat(&path).await.map_err(map_fs_err)
}

/// Returns base64-encoded bytes (so binary files round-trip cleanly via JSON).
/// `max_bytes` caps the response — frontend uses the global Max Response Size
/// setting from REST as the default.
#[tauri::command]
pub async fn explorer_fs_read(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
    path: String,
    max_bytes: Option<u64>,
) -> Result<String, String> {
    let fs = sessions.get(&tab_key).await.ok_or("no active session")?;
    let mut s = fs.read(&path, None).await.map_err(map_fs_err)?;
    let cap = max_bytes.unwrap_or(10 * 1024 * 1024); // 10 MB default
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = s.next().await {
        let chunk = chunk.map_err(map_fs_err)?;
        buf.extend_from_slice(&chunk);
        if buf.len() as u64 > cap {
            return Err(format!("file exceeds {} byte cap", cap));
        }
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(&buf))
}

/// `content_b64` is base64-encoded payload bytes from the frontend.
#[tauri::command]
pub async fn explorer_fs_write(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
    path: String,
    content_b64: String,
) -> Result<(), String> {
    let fs = sessions.get(&tab_key).await.ok_or("no active session")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&content_b64)
        .map_err(|e| format!("base64 decode: {}", e))?;
    let size = bytes.len() as u64;
    let body: BoxStream<'static, Result<Bytes, FsError>> =
        Box::pin(stream::iter(vec![Ok(Bytes::from(bytes))]));
    fs.write(&path, body, Some(size)).await.map_err(map_fs_err)
}

#[tauri::command]
pub async fn explorer_fs_delete(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let fs = sessions.get(&tab_key).await.ok_or("no active session")?;
    for p in paths {
        fs.delete(&p).await.map_err(map_fs_err)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn explorer_fs_mkdir(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
    path: String,
) -> Result<(), String> {
    let fs = sessions.get(&tab_key).await.ok_or("no active session")?;
    fs.mkdir(&path).await.map_err(map_fs_err)
}

#[tauri::command]
pub async fn explorer_fs_rename(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
    from: String,
    to: String,
) -> Result<(), String> {
    let fs = sessions.get(&tab_key).await.ok_or("no active session")?;
    fs.rename(&from, &to).await.map_err(map_fs_err)
}

#[tauri::command]
pub async fn explorer_fs_home_dir(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
) -> Result<Option<String>, String> {
    let fs = sessions.get(&tab_key).await.ok_or("no active session")?;
    fs.home_dir().await.map_err(map_fs_err)
}

#[tauri::command]
pub async fn explorer_fs_search(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
    prefix: String,
    glob: String,
) -> Result<Vec<DirEntry>, String> {
    let fs = sessions.get(&tab_key).await.ok_or("no active session")?;
    fs.search(&prefix, &glob).await.map_err(map_fs_err)
}

#[tauri::command]
pub async fn explorer_fs_get_url(
    sessions: State<'_, ExplorerSessions>,
    tab_key: String,
    path: String,
    ttl_secs: Option<u64>,
) -> Result<Option<String>, String> {
    let fs = sessions.get(&tab_key).await.ok_or("no active session")?;
    fs.presigned_url(&path, ttl_secs.unwrap_or(3600))
        .await
        .map_err(map_fs_err)
}

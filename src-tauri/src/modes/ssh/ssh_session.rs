//! Shared SSH connect-and-authenticate flow.
//!
//! Returns a post-auth `Handle` that callers can layer their channel type
//! on top of:
//!  - terminal mode: `request_pty` + `request_shell`
//!  - SFTP backend: `channel_open_session` + `request_subsystem("sftp")`
//!  - tunnel: `channel_open_direct_tcpip(...)`
//!
//! Extracted from `terminal.rs` (gotcha #42 in CLAUGE_ARCHITECTURE: the
//! `connect_and_auth` body was duplicated between terminal.rs and
//! tunnel.rs). Single source of truth eliminates drift.

use russh::client::{self, Handle};
use sqlx::SqlitePool;
use std::sync::Arc;

use crate::modes::ssh::agent::try_agent_auth;
use crate::modes::ssh::models::SshProfile;
use crate::shared::platform::credential_store::{credential_store, CredentialStore};

/// russh client handler — accepts any host key (TOFU phase 1; see the
/// SSH-mode design doc for the planned known-hosts verification).
pub struct ClientHandler;

#[async_trait::async_trait]
impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// Everything needed to dial + authenticate one SSH session. Built either
/// from a stored `ssh_profiles` row or from a "direct credentials" form
/// (Explorer SFTP).
pub struct SshAuthSpec {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// "key" | "password" | "agent".
    pub auth_type: String,
    pub key_path: Option<String>,
    /// Password (auth_type=password) or passphrase (auth_type=key). None
    /// is fine for unencrypted keys / agent auth.
    pub secret: Option<String>,
}

/// Connect to the host described by the SSH profile, perform the chosen
/// auth method, and return the post-auth `Handle`. Touches the profile's
/// `last_used_at` (best-effort — won't fail the connect on bookkeeping
/// errors).
///
/// Callers layer their channel type on top of the returned handle.
pub async fn open_authenticated_ssh_session(
    pool: &SqlitePool,
    profile_id: &str,
) -> Result<Handle<ClientHandler>, String> {
    // 1. Load the profile.
    let profile: SshProfile =
        sqlx::query_as::<_, SshProfile>("SELECT * FROM ssh_profiles WHERE id = ?")
            .bind(profile_id)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("ssh profile lookup: {}", e))?;

    // 2. Bump last_used_at (best-effort). RFC3339 with millisecond precision —
    // WKWebView's Date constructor chokes on the default 6-digit microsecond
    // fractional seconds.
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let _ = sqlx::query("UPDATE ssh_profiles SET last_used_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&profile.id)
        .execute(pool)
        .await;

    // 3. Pull credential (passphrase or password). Missing is fine for
    // unencrypted keys.
    let secret: Option<String> = credential_store()
        .get(&profile.id)
        .await
        .map_err(|e| format!("credential lookup: {}", e))?;

    let spec = SshAuthSpec {
        host: profile.host,
        port: profile.port as u16,
        username: profile.username,
        auth_type: profile.auth_type,
        key_path: profile.key_path,
        secret,
    };
    open_authenticated_ssh_session_with_spec(spec).await
}

/// Same as `open_authenticated_ssh_session`, but takes a pre-built spec
/// rather than loading from `ssh_profiles`. Used by Explorer SFTP when
/// the user picks "New connection details" (no SSH profile).
pub async fn open_authenticated_ssh_session_with_spec(
    spec: SshAuthSpec,
) -> Result<Handle<ClientHandler>, String> {
    // TCP socket. Manual creation so we can disable Nagle's algorithm —
    // critical for interactive SSH (40-200ms keystroke latency without it).
    // russh doesn't enable NODELAY by default and only exposes it via
    // connect_stream (not the higher-level connect()).
    let socket = tokio::net::TcpStream::connect((spec.host.as_str(), spec.port))
        .await
        .map_err(|e| format!("tcp connect: {}", e))?;
    if let Err(e) = socket.set_nodelay(true) {
        eprintln!("[ssh] warning: TCP_NODELAY not set ({})", e);
    }

    // SSH handshake with a hard 15s timeout.
    let config = Arc::new(client::Config::default());
    let connect_fut = client::connect_stream(config, socket, ClientHandler);
    let mut handle: Handle<ClientHandler> = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        connect_fut,
    )
    .await
    {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => return Err(format!("ssh connect: {}", e)),
        Err(_) => return Err("ssh connect: timed out after 15s".to_string()),
    };

    let authed = match spec.auth_type.as_str() {
        "key" => {
            let key_path = spec
                .key_path
                .as_ref()
                .ok_or_else(|| "key auth requires key_path".to_string())?;
            let passphrase = spec.secret.as_deref();
            let keypair = russh_keys::load_secret_key(key_path, passphrase)
                .map_err(|e| format!("load key: {}", e))?;
            handle
                .authenticate_publickey(&spec.username, Arc::new(keypair))
                .await
                .map_err(|e| format!("ssh auth publickey: {}", e))?
        }
        "password" => {
            let password = spec
                .secret
                .ok_or_else(|| "password auth requires a stored secret".to_string())?;
            handle
                .authenticate_password(&spec.username, password)
                .await
                .map_err(|e| format!("ssh auth password: {}", e))?
        }
        "agent" => try_agent_auth(&mut handle, &spec.username).await?,
        other => return Err(format!("unknown auth_type: {}", other)),
    };
    if !authed {
        return Err("authentication failed".to_string());
    }

    Ok(handle)
}

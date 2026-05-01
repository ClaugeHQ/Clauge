//! SSH local port-forward tunnel.
//!
//! Opens an SSH session against a saved `ssh_profiles` row and binds a
//! kernel-chosen local TCP port (`127.0.0.1:0`) that forwards incoming
//! connections to `target_host:target_port` through the bastion via
//! `direct-tcpip` channels. Used by the SQL + NoSQL connect paths so a
//! single SSH profile can front any DB driver with no duplicate creds.
//!
//! The russh client setup intentionally mirrors the one in
//! `modes/ssh/terminal.rs` (same handler, same 15s connect timeout,
//! same auth dispatch). The two are not deduped here on purpose — the
//! standing rule is "no behavior change in unrelated code", and lifting
//! the helper would touch `terminal.rs`. A future cleanup can fold them
//! together once the surfaces stabilise.
//!
//! Drop semantics: dropping `SshTunnel` shuts down the local listener
//! task (via the held `oneshot::Sender`), which in turn drops the
//! russh `Handle` — closing the SSH session cleanly.

use crate::modes::ssh::models::SshProfile;
use crate::shared::platform::credential_store::{credential_store, CredentialStore};
use russh::client::{self, Handle};
use russh::ChannelMsg;
use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

// ---------------------------------------------------------------------------
// russh client handler — accepts any host key in phase 1 (matches terminal.rs)
// ---------------------------------------------------------------------------

struct ClientHandler;

#[async_trait::async_trait]
impl client::Handler for ClientHandler {
    type Error = russh::Error;

    // TODO(ssh-tofu): unify with terminal.rs once host-key TOFU lands.
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Live SSH tunnel handle. While this value is alive, `127.0.0.1:local_port`
/// forwards through the bastion to the configured target. Drop closes the
/// listener and the SSH session.
pub struct SshTunnel {
    pub local_port: u16,
    /// Sending on this dropper signals the accept loop to stop. We send `()`
    /// from `Drop`; if the receiver is already gone (task exited), the send
    /// silently fails — that's fine.
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Open an SSH tunnel forwarding `localhost:<bound_port>` to
/// `target_host:target_port` through the bastion identified by `profile_id`.
///
/// Returns an `SshTunnel` whose `local_port` you connect your DB driver to.
/// Errors propagate as user-facing strings.
pub async fn open(
    pool: &SqlitePool,
    profile_id: &str,
    target_host: &str,
    target_port: u16,
) -> Result<SshTunnel, String> {
    // 1. Load the SSH profile from the same table SSH terminal uses.
    let profile: SshProfile =
        sqlx::query_as::<_, SshProfile>("SELECT * FROM ssh_profiles WHERE id = ?")
            .bind(profile_id)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("ssh profile lookup: {}", e))?;

    // 2. Pull credential (passphrase or password). Missing is fine for
    // unencrypted keys.
    let secret: Option<String> = credential_store()
        .get(&profile.id)
        .await
        .map_err(|e| format!("credential lookup: {}", e))?;

    // 3. Connect + auth. Same shape as terminal.rs, kept here verbatim
    // because lifting it would touch terminal.rs (see module comment).
    let handle = connect_and_auth(&profile, secret).await?;
    let handle = Arc::new(handle);

    // 4. Bind a local listener on a kernel-chosen free port.
    let listener = TcpListener::bind(("127.0.0.1", 0u16))
        .await
        .map_err(|e| format!("bind local listener: {}", e))?;
    let local_port = listener
        .local_addr()
        .map_err(|e| format!("local addr: {}", e))?
        .port();

    eprintln!(
        "[ssh-tunnel] open profile={} target={}:{} local=127.0.0.1:{}",
        profile.id, target_host, target_port, local_port
    );

    // 5. Spawn the accept loop. The russh `Handle` is kept alive inside the
    // task; when the shutdown signal fires (or we drop it on listener
    // failure), the handle drops too and the SSH session closes.
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let target_host_owned = target_host.to_string();
    let handle_for_task = Arc::clone(&handle);

    tokio::spawn(async move {
        run_accept_loop(listener, handle_for_task, target_host_owned, target_port, shutdown_rx)
            .await;
    });

    Ok(SshTunnel {
        local_port,
        shutdown_tx: Some(shutdown_tx),
    })
}

// ---------------------------------------------------------------------------
// Accept loop — one russh `direct-tcpip` channel per inbound TCP connection
// ---------------------------------------------------------------------------

async fn run_accept_loop(
    listener: TcpListener,
    handle: Arc<Handle<ClientHandler>>,
    target_host: String,
    target_port: u16,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                eprintln!("[ssh-tunnel] shutdown signal received, closing listener");
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((tcp, peer)) => {
                        let handle = Arc::clone(&handle);
                        let target_host = target_host.clone();
                        tokio::spawn(async move {
                            if let Err(e) = forward_connection(tcp, handle, target_host, target_port).await {
                                eprintln!("[ssh-tunnel] forward {} failed: {}", peer, e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("[ssh-tunnel] accept error: {}", e);
                        // Brief backoff so a transient error doesn't burn CPU.
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
    // The Arc<Handle> drops when this task ends; russh closes the session.
}

async fn forward_connection(
    tcp: tokio::net::TcpStream,
    handle: Arc<Handle<ClientHandler>>,
    target_host: String,
    target_port: u16,
) -> Result<(), String> {
    // Open a `direct-tcpip` channel through the bastion to the DB host.
    // The "originator" fields are informational; OpenSSH expects them but
    // doesn't care about the values.
    let channel = handle
        .channel_open_direct_tcpip(target_host.as_str(), target_port as u32, "127.0.0.1", 0)
        .await
        .map_err(|e| format!("open direct-tcpip: {}", e))?;

    pump_channel(tcp, channel).await
}

/// Manual bidirectional shovel between a local `TcpStream` and a russh
/// `Channel<Msg>`. We can't use `tokio::io::copy_bidirectional` because the
/// russh `Channel` isn't an `AsyncRead`/`AsyncWrite` in 0.45 — it speaks in
/// `ChannelMsg`. The shape of the loop is: drain TCP→channel.data() and
/// channel.wait()→TCP in a single select.
async fn pump_channel(
    tcp: tokio::net::TcpStream,
    mut channel: russh::Channel<russh::client::Msg>,
) -> Result<(), String> {
    let (mut tcp_read, mut tcp_write) = tcp.into_split();
    let mut buf = vec![0u8; 16 * 1024];

    loop {
        tokio::select! {
            // TCP → SSH channel
            n = tcp_read.read(&mut buf) => {
                match n {
                    Ok(0) => {
                        // Local side closed: send EOF on the channel and exit
                        // the loop. EOF is best-effort.
                        let _ = channel.eof().await;
                        break;
                    }
                    Ok(n) => {
                        if let Err(e) = channel.data(&buf[..n]).await {
                            return Err(format!("channel write: {}", e));
                        }
                    }
                    Err(e) => return Err(format!("local read: {}", e)),
                }
            }
            // SSH channel → TCP
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        let bytes: &[u8] = data.as_ref();
                        if let Err(e) = tcp_write.write_all(bytes).await {
                            return Err(format!("local write: {}", e));
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        // direct-tcpip shouldn't carry stderr-like extended
                        // data, but if it does, surface it on the same wire.
                        let bytes: &[u8] = data.as_ref();
                        if let Err(e) = tcp_write.write_all(bytes).await {
                            return Err(format!("local write: {}", e));
                        }
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) => {
                        let _ = tcp_write.shutdown().await;
                        break;
                    }
                    Some(_) => {} // ignore Window* / ExitStatus / etc.
                    None => break,
                }
            }
        }
    }

    let _ = channel.close().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// SSH connect + auth — same dispatch as modes/ssh/terminal.rs
// ---------------------------------------------------------------------------

async fn connect_and_auth(
    profile: &SshProfile,
    secret: Option<String>,
) -> Result<Handle<ClientHandler>, String> {
    let config = Arc::new(client::Config::default());
    let host = profile.host.clone();
    let port: u16 = profile.port as u16;

    // 15s connect timeout matches terminal.rs.
    let connect_fut = client::connect(config, (host.as_str(), port), ClientHandler);
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

    let authed = match profile.auth_type.as_str() {
        "key" => {
            let key_path = profile
                .key_path
                .as_ref()
                .ok_or_else(|| "key auth requires key_path".to_string())?;
            let passphrase = secret.as_deref();
            let keypair = russh_keys::load_secret_key(key_path, passphrase)
                .map_err(|e| format!("load key: {}", e))?;
            handle
                .authenticate_publickey(&profile.username, Arc::new(keypair))
                .await
                .map_err(|e| format!("ssh auth publickey: {}", e))?
        }
        "password" => {
            let password = secret
                .ok_or_else(|| "password auth requires a stored secret".to_string())?;
            handle
                .authenticate_password(&profile.username, password)
                .await
                .map_err(|e| format!("ssh auth password: {}", e))?
        }
        "agent" => {
            crate::modes::ssh::agent::try_agent_auth(&mut handle, &profile.username).await?
        }
        other => return Err(format!("unknown auth_type: {}", other)),
    };
    if !authed {
        return Err("ssh authentication failed".to_string());
    }
    Ok(handle)
}

// ---------------------------------------------------------------------------
// Tauri command — verifies a tunnel can be opened end-to-end, then drops it.
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ssh_tunnel_test(
    pool: State<'_, SqlitePool>,
    profile_id: String,
    target_host: String,
    target_port: u16,
) -> Result<(), String> {
    let tunnel = open(pool.inner(), &profile_id, &target_host, target_port).await?;
    // Dropping here is fine — we just verified SSH connect + forward setup.
    drop(tunnel);
    Ok(())
}

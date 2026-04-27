use crate::commands::agent_models::TerminalOutputPayload;
use crate::commands::credential_store::{credential_store, CredentialStore};
use crate::commands::ssh_models::{SshCommand, SshProfile, SshTerminalEntry, SshTerminalState};
use base64::Engine;
use russh::client::{self, Handle};
use russh::ChannelMsg;
use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::State;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// russh client handler
// ---------------------------------------------------------------------------

struct ClientHandler;

#[async_trait::async_trait]
impl client::Handler for ClientHandler {
    type Error = russh::Error;

    // TODO(ssh-tofu): record + verify host key fingerprints in the
    // `ssh_known_hosts` table. See docs/superpowers/specs/2026-04-27-ssh-mode-design.md
    // ("Host key (TOFU)"). Phase 1 accepts any key on first connect.
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ssh_spawn_terminal(
    state: State<'_, SshTerminalState>,
    pool: State<'_, SqlitePool>,
    profile_id: String,
    on_output: Channel<TerminalOutputPayload>,
) -> Result<String, String> {
    let terminal_id = Uuid::new_v4().to_string();

    // Fetch profile up-front so we can fail fast with a clean error.
    let profile: SshProfile =
        sqlx::query_as::<_, SshProfile>("SELECT * FROM ssh_profiles WHERE id = ?")
            .bind(&profile_id)
            .fetch_one(pool.inner())
            .await
            .map_err(|e| format!("profile not found: {}", e))?;

    // Atomically bump last_used_at for THIS connection attempt. Doing it here
    // (Rust path) instead of relying on the frontend's opportunistic touch
    // guarantees the stamp is updated for every spawn — no missed updates due
    // to race conditions or unhandled paths. Best-effort: don't fail spawn if
    // the bookkeeping write hiccups.
    // RFC3339 with millisecond precision — WKWebView's Date constructor can
    // choke on the default 6-digit microsecond fractional seconds.
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let _ = sqlx::query("UPDATE ssh_profiles SET last_used_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&profile.id)
        .execute(pool.inner())
        .await;

    // Pull credential (passphrase or password). Missing is fine for unencrypted keys.
    let secret: Option<String> = credential_store()
        .get(&profile.id)
        .await
        .map_err(|e| format!("credential lookup: {}", e))?;

    // Set up the command channel BEFORE spawning so we can return its sender via the
    // state map immediately. The russh task owns the receiver.
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<SshCommand>();

    let tid_for_task = terminal_id.clone();
    let on_output_for_task = on_output.clone();

    eprintln!(
        "[ssh] connect host={} port={} user={} auth={}",
        profile.host, profile.port, profile.username, profile.auth_type
    );

    // Drive the whole russh session inside this task. Any failure → emit
    // exit:true so the frontend can swap to the reconnect banner.
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run_ssh_session(
            profile,
            secret,
            tid_for_task.clone(),
            on_output_for_task.clone(),
            cmd_rx,
        )
        .await
        {
            eprintln!("[ssh] session ended: {}", err);
        }
        // Always signal exit on the way out so the frontend cleans up.
        let _ = on_output_for_task.send(TerminalOutputPayload {
            terminal_id: tid_for_task,
            data: String::new(),
            exit: Some(true),
        });
    });

    state.terminals.lock().insert(
        terminal_id.clone(),
        SshTerminalEntry {
            handle_tx: cmd_tx,
        },
    );

    Ok(terminal_id)
}

#[tauri::command]
pub fn ssh_write_to_terminal(
    state: State<'_, SshTerminalState>,
    terminal_id: String,
    data: String,
) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| format!("base64 decode: {}", e))?;
    let map = state.terminals.lock();
    let entry = map.get(&terminal_id).ok_or("Terminal not found")?;
    entry
        .handle_tx
        .send(SshCommand::Write(bytes))
        .map_err(|e| format!("send write: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn ssh_resize_terminal(
    state: State<'_, SshTerminalState>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let map = state.terminals.lock();
    let entry = map.get(&terminal_id).ok_or("Terminal not found")?;
    entry
        .handle_tx
        .send(SshCommand::Resize { cols, rows })
        .map_err(|e| format!("send resize: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn ssh_kill_terminal(
    state: State<'_, SshTerminalState>,
    terminal_id: String,
) -> Result<(), String> {
    let mut map = state.terminals.lock();
    if let Some(entry) = map.remove(&terminal_id) {
        let _ = entry.handle_tx.send(SshCommand::Kill);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// russh session driver
// ---------------------------------------------------------------------------

async fn run_ssh_session(
    profile: SshProfile,
    secret: Option<String>,
    terminal_id: String,
    on_output: Channel<TerminalOutputPayload>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<SshCommand>,
) -> Result<(), String> {
    let config = Arc::new(client::Config::default());
    let handler = ClientHandler;

    let host = profile.host.clone();
    let port: u16 = profile.port as u16;

    // Hard timeout so a stuck TCP connect / unreachable host doesn't hang the
    // session indefinitely. 15s is generous on cellular/VPN, plenty otherwise.
    let connect_fut = client::connect(config, (host.as_str(), port), handler);
    let mut handle: Handle<ClientHandler> = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        connect_fut,
    )
    .await
    {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => return Err(format!("connect: {}", e)),
        Err(_) => return Err("connect: timed out after 15s".to_string()),
    };

    // Auth
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
                .map_err(|e| format!("auth publickey: {}", e))?
        }
        "password" => {
            let password = secret
                .ok_or_else(|| "password auth requires a stored secret".to_string())?;
            handle
                .authenticate_password(&profile.username, password)
                .await
                .map_err(|e| format!("auth password: {}", e))?
        }
        other => return Err(format!("unknown auth_type: {}", other)),
    };
    if !authed {
        return Err("authentication failed".to_string());
    }

    // Open a session channel and request an interactive shell over a PTY.
    let mut chan = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("open session: {}", e))?;
    chan.request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .map_err(|e| format!("request pty: {}", e))?;
    chan.request_shell(false)
        .await
        .map_err(|e| format!("request shell: {}", e))?;

    // Single-task select loop: forward inbound channel msgs to the frontend
    // and outbound commands to the channel. Exiting either branch tears the
    // session down.
    loop {
        tokio::select! {
            // Inbound from server
            msg = chan.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        let bytes: &[u8] = data.as_ref();
                        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                        if on_output
                            .send(TerminalOutputPayload {
                                terminal_id: terminal_id.clone(),
                                data: encoded,
                                exit: None,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        // Stream stderr into the same xterm.
                        let bytes: &[u8] = data.as_ref();
                        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                        if on_output
                            .send(TerminalOutputPayload {
                                terminal_id: terminal_id.clone(),
                                data: encoded,
                                exit: None,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) => {
                        break;
                    }
                    Some(ChannelMsg::ExitStatus { exit_status: _ }) => {
                        // Remote process exited; wait for the channel close that
                        // typically follows, but don't break here — there may
                        // still be buffered output.
                    }
                    Some(_) => {}
                    None => break,
                }
            }
            // Outbound from frontend
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(SshCommand::Write(bytes)) => {
                        if let Err(e) = chan.data(&bytes[..]).await {
                            eprintln!("[ssh] write error: {}", e);
                            break;
                        }
                    }
                    Some(SshCommand::Resize { cols, rows }) => {
                        if let Err(e) = chan
                            .window_change(cols as u32, rows as u32, 0, 0)
                            .await
                        {
                            eprintln!("[ssh] resize error: {}", e);
                        }
                    }
                    Some(SshCommand::Kill) => {
                        let _ = chan.close().await;
                        break;
                    }
                    None => break, // sender dropped
                }
            }
        }
    }

    let _ = chan.close().await;
    Ok(())
}

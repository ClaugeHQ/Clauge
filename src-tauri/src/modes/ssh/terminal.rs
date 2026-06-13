use crate::companion::fanout;
use crate::modes::agent::models::TerminalOutputPayload;
use crate::modes::ssh::models::{SshCommand, SshTerminalEntry, SshTerminalState};
use crate::modes::ssh::ssh_session::{open_authenticated_ssh_session, ClientHandler};
use base64::Engine;
use russh::client::Handle;
use russh::ChannelMsg;
use sqlx::SqlitePool;
use tauri::ipc::Channel;
use tauri::State;
use uuid::Uuid;

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
    spawn_ssh_terminal_impl(&state, pool.inner(), profile_id, Some(on_output)).await
}

/// Shared spawn path for the Tauri command above and the companion
/// server (POST /v1/sessions/ssh). `on_output: None` = nothing streams
/// to the desktop UI; the russh task still drains channel messages so
/// the session stays healthy until the companion fan-out (D3) taps it.
pub(crate) async fn spawn_ssh_terminal_impl(
    state: &SshTerminalState,
    pool: &SqlitePool,
    profile_id: String,
    on_output: Option<Channel<TerminalOutputPayload>>,
) -> Result<String, String> {
    let terminal_id = Uuid::new_v4().to_string();

    // Set up the command channel BEFORE spawning so we can return its sender via the
    // state map immediately. The russh task owns the receiver.
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<SshCommand>();

    let tid_for_task = terminal_id.clone();
    let on_output_for_task = on_output.clone();

    // Clone the pool handle for the spawned task — SqlitePool is
    // `Clone` (it's an Arc inside).
    let pool_clone: SqlitePool = pool.clone();
    let profile_id_for_task = profile_id.clone();

    log::info!("[ssh] connect profile={}", profile_id);

    // Register with the companion fan-out before the session task runs
    // so the first server bytes land in the mirror scrollback. The
    // profile name becomes the push notification body.
    let fanout_title = crate::shared::repos::ssh_profiles::get_by_id(pool, &profile_id)
        .await
        .map(|p| p.name)
        .unwrap_or_else(|_| "SSH session".to_string());
    fanout::register(&terminal_id, fanout::TermKind::Ssh, &fanout_title);

    // Drive the whole russh session inside this task. Any failure → emit
    // exit:true so the frontend can swap to the reconnect banner.
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run_ssh_session(
            &pool_clone,
            &profile_id_for_task,
            tid_for_task.clone(),
            on_output_for_task.clone(),
            cmd_rx,
        )
        .await
        {
            log::warn!("[ssh] session ended: {}", err);
        }
        // Always signal exit on the way out so mirrors and the frontend
        // both clean up.
        fanout::publish_exit(&tid_for_task);
        if let Some(ch) = &on_output_for_task {
            let _ = ch.send(TerminalOutputPayload {
                terminal_id: tid_for_task,
                data: String::new(),
                exit: Some(true),
            });
        }
    });

    state.terminals.lock().insert(
        terminal_id.clone(),
        SshTerminalEntry {
            handle_tx: cmd_tx,
            profile_id,
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
    drop(map);
    // Desktop keystrokes clear attention from any source (B1).
    fanout::note_input(&terminal_id);
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
    // Mirror-aware sizing: record the desktop viewport and forward the
    // effective (min over clients) size, which equals the desktop size
    // whenever no phone is attached. None = no change → nothing sent.
    if let Some((c, r)) = fanout::set_client_size(&terminal_id, fanout::DESKTOP_CLIENT, cols, rows)
    {
        entry
            .handle_tx
            .send(SshCommand::Resize { cols: c, rows: r })
            .map_err(|e| format!("send resize: {}", e))?;
    }
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
    pool: &SqlitePool,
    profile_id: &str,
    terminal_id: String,
    on_output: Option<Channel<TerminalOutputPayload>>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<SshCommand>,
) -> Result<(), String> {
    // Connect + auth via the shared helper. Returns a post-auth Handle that
    // we layer the PTY + shell on top of.
    let handle: Handle<ClientHandler> = open_authenticated_ssh_session(pool, profile_id).await?;

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
                        fanout::publish(&terminal_id, data.as_ref());
                        if let Some(ch) = &on_output {
                            let bytes: &[u8] = data.as_ref();
                            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                            if ch
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
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        // Stream stderr into the same xterm.
                        fanout::publish(&terminal_id, data.as_ref());
                        if let Some(ch) = &on_output {
                            let bytes: &[u8] = data.as_ref();
                            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                            if ch
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
                            log::warn!("[ssh] write error: {}", e);
                            break;
                        }
                    }
                    Some(SshCommand::Resize { cols, rows }) => {
                        if let Err(e) = chan
                            .window_change(cols as u32, rows as u32, 0, 0)
                            .await
                        {
                            log::warn!("[ssh] resize error: {}", e);
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

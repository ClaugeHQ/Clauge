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
    let terminal_id = Uuid::new_v4().to_string();

    // Set up the command channel BEFORE spawning so we can return its sender via the
    // state map immediately. The russh task owns the receiver.
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<SshCommand>();

    let tid_for_task = terminal_id.clone();
    let on_output_for_task = on_output.clone();

    // Clone the pool handle for the spawned task. State::inner() returns
    // &SqlitePool; SqlitePool is `Clone` (it's an Arc inside).
    let pool_clone: SqlitePool = pool.inner().clone();
    let profile_id_for_task = profile_id.clone();

    eprintln!("[ssh] connect profile={}", profile_id);

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
    pool: &SqlitePool,
    profile_id: &str,
    terminal_id: String,
    on_output: Channel<TerminalOutputPayload>,
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

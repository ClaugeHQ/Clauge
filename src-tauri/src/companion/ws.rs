// Mirror WebSocket: GET /v1/term/{terminalId}/ws (bearer-authed by the
// same /v1 middleware as the REST routes). Wire protocol per the
// mobile spec — server→client {t:"replay"|"out"|"exit"|"size"},
// client→server {t:"in"|"resize"|"ping"} — all byte payloads base64.
//
// Each connection attaches to the fan-out hub (atomic scrollback
// snapshot + broadcast subscription), registers itself as a resize
// client under a per-connection id, and detaches on any exit path so
// the effective PTY size relaxes back to the remaining clients.

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Extension, Router};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use portable_pty::PtySize;
use serde_json::{json, Value};
use std::io::Write;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::broadcast::error::RecvError;

use crate::modes::agent::models::TerminalState;
use crate::modes::ssh::models::{SshCommand, SshTerminalState};

use super::auth::AuthedDevice;
use super::fanout::{self, FanoutEvent, TermKind};
use super::server::CompanionAppState;

/// RFC 6455 "policy violation" — sent when the terminal id is unknown.
const CLOSE_POLICY: u16 = 1008;

pub fn routes() -> Router<Arc<CompanionAppState>> {
    Router::new().route("/term/{terminal_id}/ws", get(ws_upgrade))
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<CompanionAppState>>,
    Extension(device): Extension<AuthedDevice>,
    Path(terminal_id): Path<String>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state, device.0, terminal_id))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<CompanionAppState>,
    device_id: String,
    terminal_id: String,
) {
    let Some(attached) = fanout::attach(&terminal_id) else {
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: CLOSE_POLICY,
                reason: "unknown terminal".into(),
            })))
            .await;
        return;
    };
    let fanout::Attached {
        scrollback,
        mut rx,
        kind,
        effective_size,
    } = attached;

    // Per-connection client id: a fast reconnect from the same device
    // must not have its fresh size clobbered when the stale socket's
    // cleanup runs remove_client.
    let client_id = format!("{}:{}", device_id, uuid::Uuid::new_v4());
    log::info!(
        "[companion] ws attach terminal={} client={}",
        terminal_id,
        client_id
    );
    // Register as a viewer so push gating (phone_attached) sees this
    // phone immediately on attach, not only once it sends a resize.
    fanout::add_viewer(&terminal_id, &client_id);

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Replay first so the phone paints history before any live bytes,
    // then the current effective size (absent until some client has
    // reported one).
    let replay = json!({ "t": "replay", "d": b64(&scrollback) });
    if ws_tx.send(Message::Text(replay.to_string().into())).await.is_err() {
        fanout::remove_viewer(&terminal_id, &client_id);
        return;
    }
    if let Some((cols, rows)) = effective_size {
        let size = json!({ "t": "size", "cols": cols, "rows": rows });
        if ws_tx.send(Message::Text(size.to_string().into())).await.is_err() {
            fanout::remove_viewer(&terminal_id, &client_id);
            return;
        }
    }

    // The listener's graceful shutdown doesn't cover upgraded
    // connections — each socket watches the server's shutdown channel
    // itself so "toggle off" kills all mirrors.
    let mut shutdown = state.shutdown.clone();

    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(FanoutEvent::Out(bytes)) => {
                    let msg = json!({ "t": "out", "d": b64(&bytes) });
                    if ws_tx.send(Message::Text(msg.to_string().into())).await.is_err() {
                        break;
                    }
                }
                Ok(FanoutEvent::Exit) => {
                    let msg = json!({ "t": "exit" });
                    let _ = ws_tx.send(Message::Text(msg.to_string().into())).await;
                    break;
                }
                // Lagged = this receiver missed output. Drop the socket;
                // the phone reconnects and resyncs from scrollback replay.
                Err(RecvError::Lagged(_)) | Err(RecvError::Closed) => break,
            },
            msg = ws_rx.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    handle_client_msg(&state, &terminal_id, &client_id, kind, text.as_str());
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {} // binary/ping/pong — pongs are automatic
            },
            _ = shutdown.changed() => break,
        }
    }

    let _ = ws_tx.send(Message::Close(None)).await;
    fanout::remove_viewer(&terminal_id, &client_id);
    // Detach: if this client was the size constraint, relax the PTY
    // back to the remaining clients' minimum.
    if let Some((cols, rows)) = fanout::remove_client(&terminal_id, &client_id) {
        apply_resize(&state, &terminal_id, kind, cols, rows);
    }
    log::info!(
        "[companion] ws detach terminal={} client={}",
        terminal_id,
        client_id
    );
}

fn handle_client_msg(
    state: &CompanionAppState,
    terminal_id: &str,
    client_id: &str,
    kind: TermKind,
    text: &str,
) {
    let Ok(msg) = serde_json::from_str::<Value>(text) else {
        return;
    };
    match msg.get("t").and_then(Value::as_str) {
        Some("in") => {
            let Some(data) = msg.get("d").and_then(Value::as_str) else {
                return;
            };
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else {
                return;
            };
            write_input(state, terminal_id, kind, &bytes);
        }
        Some("resize") => {
            let cols = msg.get("cols").and_then(Value::as_u64).unwrap_or(0);
            let rows = msg.get("rows").and_then(Value::as_u64).unwrap_or(0);
            if cols == 0 || rows == 0 || cols > u16::MAX as u64 || rows > u16::MAX as u64 {
                return;
            }
            if let Some((c, r)) =
                fanout::set_client_size(terminal_id, client_id, cols as u16, rows as u16)
            {
                apply_resize(state, terminal_id, kind, c, r);
            }
        }
        // ping is just a keepalive; everything else is ignored.
        _ => {}
    }
}

/// Forward phone input through the same internals the desktop write
/// commands use: the PTY writer for agent terminals, the session
/// task's command channel for SSH.
fn write_input(state: &CompanionAppState, terminal_id: &str, kind: TermKind, bytes: &[u8]) {
    match kind {
        TermKind::Agent => {
            let terminal_state = state.app.state::<TerminalState>();
            let mut terminals = terminal_state.terminals.lock();
            if let Some(entry) = terminals.get_mut(terminal_id) {
                let _ = entry
                    .writer
                    .write_all(bytes)
                    .and_then(|_| entry.writer.flush());
            }
        }
        TermKind::Ssh => {
            let ssh_state = state.app.state::<SshTerminalState>();
            let map = ssh_state.terminals.lock();
            if let Some(entry) = map.get(terminal_id) {
                let _ = entry.handle_tx.send(SshCommand::Write(bytes.to_vec()));
            }
        }
    }
    // Answering from the phone clears attention from any source (B1).
    fanout::note_input(terminal_id);
}

/// Apply an already-computed effective size through the same resize
/// internals the desktop commands use.
fn apply_resize(
    state: &CompanionAppState,
    terminal_id: &str,
    kind: TermKind,
    cols: u16,
    rows: u16,
) {
    match kind {
        TermKind::Agent => {
            let terminal_state = state.app.state::<TerminalState>();
            let terminals = terminal_state.terminals.lock();
            if let Some(entry) = terminals.get(terminal_id) {
                let _ = entry.master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
        TermKind::Ssh => {
            let ssh_state = state.app.state::<SshTerminalState>();
            let map = ssh_state.terminals.lock();
            if let Some(entry) = map.get(terminal_id) {
                let _ = entry.handle_tx.send(SshCommand::Resize { cols, rows });
            }
        }
    }
}

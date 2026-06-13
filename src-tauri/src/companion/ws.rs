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
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Extension, Router};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
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

/// Optional phone fit size carried on the WS connect URL
/// (`?cols=NN&rows=MM`). Absent for older apps → desktop-authoritative
/// for that client, exactly as before.
#[derive(Deserialize)]
struct ConnectParams {
    cols: Option<u16>,
    rows: Option<u16>,
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<CompanionAppState>>,
    Extension(device): Extension<AuthedDevice>,
    Path(terminal_id): Path<String>,
    Query(params): Query<ConnectParams>,
) -> Response {
    let fit = match (params.cols, params.rows) {
        (Some(c), Some(r)) if c > 0 && r > 0 => Some((c, r)),
        _ => None,
    };
    ws.on_upgrade(move |socket| handle_socket(socket, state, device.0, terminal_id, fit))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<CompanionAppState>,
    device_id: String,
    terminal_id: String,
    fit: Option<(u16, u16)>,
) {
    // Resize-before-replay: register the viewer + its fit size and
    // reconcile the PTY to the phone size FIRST, so the scrollback replay
    // and the agent's SIGWINCH repaint already reflect the narrow size —
    // the phone never sees the wide→narrow churn. Done before `attach`
    // snapshots scrollback so the replayed frame is the resized one.
    let client_id = format!("{}:{}", device_id, uuid::Uuid::new_v4());
    fanout::add_viewer(&terminal_id, &client_id);
    if let Some((cols, rows)) = fit {
        fanout::set_client_size(&terminal_id, &client_id, cols, rows);
    }
    // Reconcile on attach regardless of fit: a new phone viewer may flip
    // ownership even without a size (it holds last_phone_size in grace).
    fanout::reconcile_now(&terminal_id);

    let Some(attached) = fanout::attach(&terminal_id) else {
        // Unknown terminal: undo the viewer we registered and let the PTY
        // relax back (no-op if the hub is gone).
        fanout::remove_viewer(&terminal_id, &client_id);
        fanout::reconcile_now(&terminal_id);
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

    log::info!(
        "[companion] ws attach terminal={} client={}",
        terminal_id,
        client_id
    );

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Replay first so the phone paints history before any live bytes,
    // then the current effective size (absent until some client has
    // reported one).
    let replay = json!({ "t": "replay", "d": b64(&scrollback) });
    if ws_tx.send(Message::Text(replay.to_string().into())).await.is_err() {
        detach(&terminal_id, &client_id);
        return;
    }
    if let Some((cols, rows)) = effective_size {
        let size = json!({ "t": "size", "cols": cols, "rows": rows });
        if ws_tx.send(Message::Text(size.to_string().into())).await.is_err() {
            detach(&terminal_id, &client_id);
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
                // The reconcile chokepoint resized the PTY — echo the new
                // size so every client renders at it.
                Ok(FanoutEvent::Size(cols, rows)) => {
                    let msg = json!({ "t": "size", "cols": cols, "rows": rows });
                    if ws_tx.send(Message::Text(msg.to_string().into())).await.is_err() {
                        break;
                    }
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
    detach(&terminal_id, &client_id);
    log::info!(
        "[companion] ws detach terminal={} client={}",
        terminal_id,
        client_id
    );
}

/// Forget this client's viewport + viewer, then reconcile so the PTY
/// relaxes back — `remove_viewer` stamps the detach grace when the last
/// phone leaves; reconcile holds the phone size through the grace, then
/// `reconcile_after` restores the desktop size once it expires. Used on
/// every socket exit: clean close, loop break, or a failed initial send.
fn detach(terminal_id: &str, client_id: &str) {
    fanout::remove_client(terminal_id, client_id);
    fanout::remove_viewer(terminal_id, client_id);
    fanout::reconcile_now(terminal_id);
    fanout::reconcile_after(terminal_id, fanout::DETACH_GRACE);
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
            // Phone (or orientation) size → record it and reconcile. While
            // phone-owned this drives the PTY; while desktop-owned the
            // chokepoint keeps the desktop size, so the resize is a no-op.
            fanout::set_client_size(terminal_id, client_id, cols as u16, rows as u16);
            fanout::reconcile_now(terminal_id);
        }
        // ping is just a keepalive; everything else is ignored.
        _ => {}
    }
}

/// Forward phone input through the same internals the desktop write
/// commands use: the PTY writer for agent terminals, the session
/// task's command channel for SSH.
fn write_input(state: &CompanionAppState, terminal_id: &str, kind: TermKind, bytes: &[u8]) {
    let delivered = match kind {
        TermKind::Agent => {
            let terminal_state = state.app.state::<TerminalState>();
            let mut terminals = terminal_state.terminals.lock();
            match terminals.get_mut(terminal_id) {
                Some(entry) => entry
                    .writer
                    .write_all(bytes)
                    .and_then(|_| entry.writer.flush())
                    .is_ok(),
                None => false,
            }
        }
        TermKind::Ssh => {
            let ssh_state = state.app.state::<SshTerminalState>();
            let map = ssh_state.terminals.lock();
            match map.get(terminal_id) {
                Some(entry) => entry.handle_tx.send(SshCommand::Write(bytes.to_vec())).is_ok(),
                None => false,
            }
        }
    };
    // Answering from the phone clears attention from any source (B1) — but
    // only once the keystroke actually reached the PTY/session. A dead
    // terminal or a failed write must not clear a real "needs you" badge.
    if delivered {
        fanout::note_input(terminal_id);
    }
}

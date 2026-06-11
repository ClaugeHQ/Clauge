// Bind + serve loop and lifecycle commands. Mirrors the workspace MCP
// server (modes/workspace/mcp/server.rs) with two deliberate
// differences: it binds 0.0.0.0 (phones connect over LAN/tailnet, not
// loopback) and shutdown is a watch channel instead of a oneshot so
// future WebSocket tasks (D3) can each subscribe and die on stop.

use axum::{middleware, response::Json as JsonResponse, routing::get, Router};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::State as TauriState;
use tokio::sync::watch;

use super::pairing::PairingState;
use super::{auth, pairing, BASE_PORT, PORT_FALLBACK_RANGE};

pub struct ServerHandle {
    pub port: u16,
    pub shutdown: watch::Sender<bool>,
}

#[derive(Clone)]
pub struct CompanionAppState {
    pub pool: SqlitePool,
    /// For emitting `companion:pair-request` to the desktop UI.
    pub app: tauri::AppHandle,
    pub pairing: Arc<PairingState>,
}

/// Bind 0.0.0.0 on the first free port in BASE_PORT..=BASE_PORT+RANGE,
/// spawn the axum server, and return its handle. `shutdown.send(true)`
/// stops the listener gracefully.
pub async fn start(
    pool: SqlitePool,
    app: tauri::AppHandle,
    pairing: Arc<PairingState>,
) -> Result<ServerHandle, String> {
    let state = Arc::new(CompanionAppState { pool, app, pairing });
    let mut last_err: Option<String> = None;
    for offset in 0..=PORT_FALLBACK_RANGE {
        let port = BASE_PORT + offset;
        let addr = format!("0.0.0.0:{}", port);
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                // Everything under /v1 requires a paired device token;
                // /healthz and /pair are the only open endpoints.
                // D2 adds the session/spawn routes to this nest.
                let v1 = Router::new()
                    .route("/server/info", get(server_info))
                    .route_layer(middleware::from_fn_with_state(
                        state.clone(),
                        auth::require_bearer,
                    ));
                let router = Router::new()
                    .route("/healthz", get(|| async { "ok" }))
                    .route("/pair", axum::routing::post(pairing::handle_pair))
                    .nest("/v1", v1)
                    .with_state(state.clone());

                let (tx, mut rx) = watch::channel(false);
                tokio::spawn(async move {
                    let _ = axum::serve(listener, router)
                        .with_graceful_shutdown(async move {
                            let _ = rx.changed().await;
                        })
                        .await;
                });
                log::info!("[companion] server listening on {}", addr);
                return Ok(ServerHandle { port, shutdown: tx });
            }
            Err(e) => {
                last_err = Some(format!("{}: {}", addr, e));
            }
        }
    }
    Err(format!(
        "Failed to bind any port in {}..={}: {}",
        BASE_PORT,
        BASE_PORT + PORT_FALLBACK_RANGE,
        last_err.unwrap_or_default(),
    ))
}

async fn server_info() -> JsonResponse<Value> {
    JsonResponse(json!({
        "serverName": tauri_plugin_os::hostname(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ---------------------------------------------------------------------------
// Lifecycle commands
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CompanionStatus {
    pub running: bool,
    pub port: Option<u16>,
}

#[tauri::command]
pub async fn companion_status(
    state: TauriState<'_, super::CompanionState>,
) -> Result<CompanionStatus, String> {
    let g = state.server.lock().await;
    Ok(match &*g {
        Some(h) => CompanionStatus { running: true, port: Some(h.port) },
        None => CompanionStatus { running: false, port: None },
    })
}

#[tauri::command]
pub async fn companion_start(
    app: tauri::AppHandle,
    pool: TauriState<'_, SqlitePool>,
    state: TauriState<'_, super::CompanionState>,
) -> Result<CompanionStatus, String> {
    let mut g = state.server.lock().await;
    if let Some(h) = &*g {
        return Ok(CompanionStatus { running: true, port: Some(h.port) });
    }
    let handle = start(pool.inner().clone(), app, state.pairing.clone()).await?;
    let port = handle.port;
    *g = Some(handle);
    Ok(CompanionStatus { running: true, port: Some(port) })
}

#[tauri::command]
pub async fn companion_stop(
    state: TauriState<'_, super::CompanionState>,
) -> Result<CompanionStatus, String> {
    let mut g = state.server.lock().await;
    if let Some(h) = g.take() {
        let _ = h.shutdown.send(true);
        log::info!("[companion] server stopped");
    }
    Ok(CompanionStatus { running: false, port: None })
}

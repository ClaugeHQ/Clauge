// Bind + serve loop and lifecycle commands. Mirrors the workspace MCP
// server (modes/workspace/mcp/server.rs) with two deliberate
// differences: it binds 0.0.0.0 (phones connect over LAN/tailnet, not
// loopback) and shutdown is a watch channel instead of a oneshot so
// future WebSocket tasks (D3) can each subscribe and die on stop.

use axum::{middleware, routing::get, Router};
use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::Manager;
use tauri::State as TauriState;
use tokio::sync::watch;

use super::lifecycle::LifecycleState;
use super::pairing::PairingState;
use super::{api, auth, pairing, ws, BASE_PORT, PORT_FALLBACK_RANGE};

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
    /// Shared with the Tauri commands so spawn handlers can park on the
    /// desktop UI opening a real tab and reporting its terminalId.
    pub lifecycle: Arc<LifecycleState>,
    /// Server-stop signal. Upgraded WebSocket connections outlive the
    /// listener's graceful shutdown, so each mirror task watches this
    /// and dies when the server is toggled off.
    pub shutdown: watch::Receiver<bool>,
}

/// Bind 0.0.0.0 on the first free port in BASE_PORT..=BASE_PORT+RANGE,
/// spawn the axum server, and return its handle. `shutdown.send(true)`
/// stops the listener gracefully.
pub async fn start(
    pool: SqlitePool,
    app: tauri::AppHandle,
    pairing: Arc<PairingState>,
    lifecycle: Arc<LifecycleState>,
) -> Result<ServerHandle, String> {
    let (tx, rx) = watch::channel(false);
    let state = Arc::new(CompanionAppState {
        pool,
        app,
        pairing,
        lifecycle,
        shutdown: rx.clone(),
    });
    let mut last_err: Option<String> = None;
    for offset in 0..=PORT_FALLBACK_RANGE {
        let port = BASE_PORT + offset;
        let addr = format!("0.0.0.0:{}", port);
        match bind_reuse(&addr).await {
            Ok(listener) => {
                // Everything under /v1 requires a paired device token;
                // /healthz and /pair are the only open endpoints. The
                // /v1 routes themselves live in api.rs + ws.rs.
                let v1 = api::routes().merge(ws::routes()).route_layer(
                    middleware::from_fn_with_state(state.clone(), auth::require_bearer),
                );
                let router = Router::new()
                    .route("/healthz", get(|| async { "ok" }))
                    .route("/pair", axum::routing::post(pairing::handle_pair))
                    .nest("/v1", v1)
                    .with_state(state.clone());

                let mut rx = rx.clone();
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

/// Bind with SO_REUSEADDR so a socket left in TIME_WAIT by a
/// just-exited instance (e.g. a self-update restart) doesn't force a
/// port walk that would strand the phone's saved port.
async fn bind_reuse(addr: &str) -> std::io::Result<tokio::net::TcpListener> {
    let sa: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{e}")))?;
    let socket = if sa.is_ipv4() {
        tokio::net::TcpSocket::new_v4()?
    } else {
        tokio::net::TcpSocket::new_v6()?
    };
    socket.set_reuseaddr(true)?;
    socket.bind(sa)?;
    socket.listen(1024)
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
    let handle = start(
        pool.inner().clone(),
        app.clone(),
        state.pairing.clone(),
        state.lifecycle.clone(),
    )
    .await?;
    let port = handle.port;
    // Start push dispatch alongside the listener — the drain + attention
    // sweep tasks watch the same shutdown channel and die on stop.
    super::push::start(app, handle.shutdown.subscribe());
    *g = Some(handle);
    // Remember the preference so the server auto-starts on the next launch.
    // Best-effort: the server is already running, so a persist failure must
    // not fail the command — just surface it in the log.
    if let Err(e) = crate::shared::repos::settings::upsert(pool.inner(), "companion_enabled", "true").await {
        log::warn!("[companion] failed to persist enabled=true: {e}");
    }
    Ok(CompanionStatus { running: true, port: Some(port) })
}

#[tauri::command]
pub async fn companion_stop(
    pool: TauriState<'_, SqlitePool>,
    state: TauriState<'_, super::CompanionState>,
) -> Result<CompanionStatus, String> {
    let mut g = state.server.lock().await;
    if let Some(h) = g.take() {
        super::push::stop();
        let _ = h.shutdown.send(true);
        log::info!("[companion] server stopped");
    }
    // Persist so it stays off on the next launch (best-effort; see above).
    if let Err(e) = crate::shared::repos::settings::upsert(pool.inner(), "companion_enabled", "false").await {
        log::warn!("[companion] failed to persist enabled=false: {e}");
    }
    Ok(CompanionStatus { running: false, port: None })
}

/// Auto-start the companion server on launch if the user had it enabled
/// (persisted via `companion_enabled`). Mirrors the workspace MCP autostart.
pub async fn maybe_autostart_companion(app: tauri::AppHandle, pool: SqlitePool) {
    let enabled = matches!(
        crate::shared::repos::settings::get_by_key(&pool, "companion_enabled").await,
        Ok(Some(s)) if s.value.eq_ignore_ascii_case("true")
    );
    if !enabled {
        return;
    }
    let state = app.state::<super::CompanionState>();
    let mut g = state.server.lock().await;
    if g.is_some() {
        return;
    }
    match start(
        pool.clone(),
        app.clone(),
        state.pairing.clone(),
        state.lifecycle.clone(),
    )
    .await
    {
        Ok(handle) => {
            super::push::start(app.clone(), handle.shutdown.subscribe());
            *g = Some(handle);
            log::info!("[companion] autostarted from saved preference");
        }
        Err(e) => log::warn!("[companion] autostart failed: {e}"),
    }
}

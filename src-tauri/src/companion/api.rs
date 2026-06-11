// /v1 REST surface: session lists, spawn/kill, recent projects, FCM
// token registration. Spawns go through the same inner fns as the
// Tauri commands (spawn_*_terminal_impl) with no output channel — the
// PTY runs headless until the D3 fan-out taps it. All shapes are
// camelCase per the mobile spec.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json as JsonResponse, Response},
    routing::{delete, get, post},
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use tauri::Manager;

use crate::modes::agent::models::{AgentSession, TerminalState};
use crate::modes::agent::terminal::spawn_agent_terminal_impl;
use crate::modes::ssh::models::{SshCommand, SshTerminalState};
use crate::modes::ssh::terminal::spawn_ssh_terminal_impl;
use crate::shared::repos::sessions as sessions_repo;
use crate::shared::repos::ssh_profiles as ssh_profiles_repo;

use super::auth::AuthedDevice;
use super::devices;
use super::server::CompanionAppState;

/// Routes nested under /v1 — server.rs wraps them in the bearer
/// middleware, so every handler here runs authenticated.
pub fn routes() -> Router<Arc<CompanionAppState>> {
    Router::new()
        .route("/server/info", get(server_info))
        .route(
            "/sessions/agent",
            get(list_agent_sessions).post(spawn_agent_session),
        )
        .route(
            "/sessions/ssh",
            get(list_ssh_profiles).post(spawn_ssh_session),
        )
        .route("/term/{terminal_id}", delete(kill_terminal))
        .route("/projects/recent", get(recent_projects))
        .route("/device/fcm", post(register_fcm_token))
}

// ---------------------------------------------------------------------------
// Error mapping — log the detail, return a generic message
// ---------------------------------------------------------------------------

fn api_err(status: StatusCode, msg: &str) -> Response {
    (status, JsonResponse(json!({ "error": msg }))).into_response()
}

fn internal(context: &str, e: impl std::fmt::Display) -> Response {
    log::error!("[companion] {}: {}", context, e);
    api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

// ---------------------------------------------------------------------------
// GET /v1/server/info
// ---------------------------------------------------------------------------

async fn server_info() -> JsonResponse<Value> {
    JsonResponse(json!({
        "serverName": tauri_plugin_os::hostname(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/agent
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionInfo {
    pub id: String,
    pub title: String,
    pub provider: String,
    /// "running" | "idle" | "exited"
    pub status: String,
    pub project_path: String,
    pub last_used_at: String,
}

/// Match a live terminal to a session row. Companion spawns stamp the
/// row id on the entry; desktop spawns stamp the claude resume id —
/// check both. Entry present but child reaped = the PTY died this run
/// and nobody has closed the tab yet → "exited". No entry → "idle"
/// (covers desktop-fresh spawns too until the D3 fan-out registers
/// every terminal with the companion).
fn agent_status(terminal_state: &TerminalState, session: &AgentSession) -> &'static str {
    let mut terminals = terminal_state.terminals.lock();
    for entry in terminals.values_mut() {
        let Some(r) = entry.session_ref.as_deref() else {
            continue;
        };
        if r != session.id && Some(r) != session.claude_session_id.as_deref() {
            continue;
        }
        return match entry.child.try_wait() {
            Ok(None) => "running",
            _ => "exited",
        };
    }
    "idle"
}

async fn list_agent_sessions(State(state): State<Arc<CompanionAppState>>) -> Response {
    let rows = match sessions_repo::list_sessions(&state.pool).await {
        Ok(rows) => rows,
        Err(e) => return internal("list agent sessions", e),
    };
    let terminal_state = state.app.state::<TerminalState>();
    let list: Vec<AgentSessionInfo> = rows
        .into_iter()
        .map(|s| {
            let status = agent_status(&terminal_state, &s).to_string();
            AgentSessionInfo {
                id: s.id,
                title: s.title,
                provider: s.provider,
                status,
                project_path: s.project_path,
                last_used_at: s.last_used_at,
            }
        })
        .collect();
    JsonResponse(list).into_response()
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/ssh
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshProfileInfo {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub accent_color: Option<String>,
    pub last_used_at: Option<String>,
    /// Any open tab (desktop or companion) for this profile.
    pub live: bool,
}

async fn list_ssh_profiles(State(state): State<Arc<CompanionAppState>>) -> Response {
    let rows = match ssh_profiles_repo::list_all(&state.pool).await {
        Ok(rows) => rows,
        Err(e) => return internal("list ssh profiles", e),
    };
    let ssh_state = state.app.state::<SshTerminalState>();
    let live: HashSet<String> = ssh_state
        .terminals
        .lock()
        .values()
        .map(|e| e.profile_id.clone())
        .collect();
    let list: Vec<SshProfileInfo> = rows
        .into_iter()
        .map(|p| SshProfileInfo {
            live: live.contains(&p.id),
            id: p.id,
            name: p.name,
            host: p.host,
            port: p.port,
            username: p.username,
            accent_color: p.accent_color,
            last_used_at: p.last_used_at,
        })
        .collect();
    JsonResponse(list).into_response()
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/agent
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpawnAgentBody {
    session_id: Option<String>,
    new_session: Option<NewAgentSession>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewAgentSession {
    project_path: String,
    provider: String,
    title: Option<String>,
}

fn project_name_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

/// Create a manual session row the same way `agent_create_session`
/// does (defaults + lazy provider MCP registration), then return it.
async fn create_session_row(
    state: &CompanionAppState,
    new: NewAgentSession,
) -> Result<AgentSession, Response> {
    if new.project_path.trim().is_empty() {
        return Err(api_err(StatusCode::BAD_REQUEST, "projectPath required"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let project_name = project_name_from_path(&new.project_path);
    let title = new
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| project_name.clone());
    let provider = if new.provider.trim().is_empty() {
        "claude".to_string()
    } else {
        new.provider.trim().to_string()
    };
    sessions_repo::insert_session(
        &state.pool,
        &id,
        &title,
        "",
        &new.project_path,
        &project_name,
        "",
        0,
        None,
        None,
        &now,
        &now,
        &provider,
        None,
    )
    .await
    .map_err(|e| internal("insert session", e))?;
    crate::modes::workspace::commands::ensure_provider_mcp_registered(&state.pool, &provider)
        .await;
    sessions_repo::get_session_by_id(&state.pool, &id)
        .await
        .map_err(|e| internal("reload session", e))
}

async fn spawn_agent_session(
    State(state): State<Arc<CompanionAppState>>,
    JsonResponse(body): JsonResponse<SpawnAgentBody>,
) -> Response {
    let session = match (body.session_id, body.new_session) {
        (Some(id), None) => match sessions_repo::get_session_by_id(&state.pool, &id).await {
            Ok(s) => s,
            Err(sqlx::Error::RowNotFound) => {
                return api_err(StatusCode::NOT_FOUND, "unknown session")
            }
            Err(e) => return internal("load session", e),
        },
        (None, Some(new)) => match create_session_row(&state, new).await {
            Ok(s) => s,
            Err(resp) => return resp,
        },
        _ => {
            return api_err(
                StatusCode::BAD_REQUEST,
                "provide exactly one of sessionId or newSession",
            )
        }
    };

    // Keep list ordering consistent with desktop usage.
    let now = chrono::Utc::now().to_rfc3339();
    let _ = sessions_repo::update_session_last_used(&state.pool, &session.id, &now).await;

    let terminal_state = state.app.state::<TerminalState>();
    let result = spawn_agent_terminal_impl(
        &terminal_state,
        &state.pool,
        Some(session.id.clone()),
        session.claude_session_id.clone(),
        session.project_path.clone(),
        Some(session.context_prompt.clone()).filter(|p| !p.is_empty()),
        Some(session.skip_permissions == 1),
        session.git_name.clone(),
        session.git_email.clone(),
        Some(session.provider.clone()),
        session.binary_path.clone(),
        None,
        None, // no output channel — D3 fan-out taps the PTY
    )
    .await;
    match result {
        Ok(terminal_id) => {
            log::info!(
                "[companion] spawned agent terminal {} for session {}",
                terminal_id,
                session.id
            );
            JsonResponse(json!({ "terminalId": terminal_id })).into_response()
        }
        Err(e) => internal("spawn agent terminal", e),
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/ssh
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpawnSshBody {
    profile_id: String,
}

async fn spawn_ssh_session(
    State(state): State<Arc<CompanionAppState>>,
    JsonResponse(body): JsonResponse<SpawnSshBody>,
) -> Response {
    match ssh_profiles_repo::get_by_id(&state.pool, &body.profile_id).await {
        Ok(_) => {}
        Err(sqlx::Error::RowNotFound) => return api_err(StatusCode::NOT_FOUND, "unknown profile"),
        Err(e) => return internal("load ssh profile", e),
    }
    let now = chrono::Utc::now().to_rfc3339();
    let _ = ssh_profiles_repo::touch_last_used(&state.pool, &body.profile_id, &now).await;

    let ssh_state = state.app.state::<SshTerminalState>();
    match spawn_ssh_terminal_impl(&ssh_state, &state.pool, body.profile_id.clone(), None).await {
        Ok(terminal_id) => {
            log::info!(
                "[companion] spawned ssh terminal {} for profile {}",
                terminal_id,
                body.profile_id
            );
            JsonResponse(json!({ "terminalId": terminal_id })).into_response()
        }
        Err(e) => internal("spawn ssh terminal", e),
    }
}

// ---------------------------------------------------------------------------
// DELETE /v1/term/{terminalId}
// ---------------------------------------------------------------------------

async fn kill_terminal(
    State(state): State<Arc<CompanionAppState>>,
    Path(terminal_id): Path<String>,
) -> Response {
    // Same internals as agent_kill_terminal / ssh_kill_terminal: remove
    // the registry entry and kill/close. Try agent first, then ssh.
    {
        let terminal_state = state.app.state::<TerminalState>();
        let removed = terminal_state.terminals.lock().remove(&terminal_id);
        if let Some(mut entry) = removed {
            let _ = entry.child.kill();
            log::info!("[companion] killed agent terminal {}", terminal_id);
            return StatusCode::NO_CONTENT.into_response();
        }
    }
    {
        let ssh_state = state.app.state::<SshTerminalState>();
        let removed = ssh_state.terminals.lock().remove(&terminal_id);
        if let Some(entry) = removed {
            let _ = entry.handle_tx.send(SshCommand::Kill);
            log::info!("[companion] killed ssh terminal {}", terminal_id);
            return StatusCode::NO_CONTENT.into_response();
        }
    }
    api_err(StatusCode::NOT_FOUND, "unknown terminal")
}

// ---------------------------------------------------------------------------
// GET /v1/projects/recent
// ---------------------------------------------------------------------------

async fn recent_projects(State(state): State<Arc<CompanionAppState>>) -> Response {
    // No repo fn exists for this aggregate — single read-only query.
    let rows: Result<Vec<(String,)>, sqlx::Error> = sqlx::query_as(
        "SELECT project_path FROM agent_sessions \
         WHERE origin = 'manual' \
         GROUP BY project_path \
         ORDER BY MAX(last_used_at) DESC \
         LIMIT 20",
    )
    .fetch_all(&state.pool)
    .await;
    match rows {
        Ok(rows) => {
            let paths: Vec<String> = rows.into_iter().map(|r| r.0).collect();
            JsonResponse(paths).into_response()
        }
        Err(e) => internal("recent projects", e),
    }
}

// ---------------------------------------------------------------------------
// POST /v1/device/fcm
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FcmBody {
    token: String,
}

async fn register_fcm_token(
    State(state): State<Arc<CompanionAppState>>,
    Extension(device): Extension<AuthedDevice>,
    JsonResponse(body): JsonResponse<FcmBody>,
) -> Response {
    if body.token.trim().is_empty() {
        return api_err(StatusCode::BAD_REQUEST, "token required");
    }
    match devices::set_fcm_token(&state.pool, &device.0, &body.token).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal("store fcm token", e),
    }
}

// ---------------------------------------------------------------------------
// Tests — golden JSON for the list response shapes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_session_info_golden_json() {
        let info = AgentSessionInfo {
            id: "sess-1".into(),
            title: "Fix the parser".into(),
            provider: "claude".into(),
            status: "running".into(),
            project_path: "/Users/me/proj".into(),
            last_used_at: "2026-06-11T10:00:00Z".into(),
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({
                "id": "sess-1",
                "title": "Fix the parser",
                "provider": "claude",
                "status": "running",
                "projectPath": "/Users/me/proj",
                "lastUsedAt": "2026-06-11T10:00:00Z",
            })
        );
    }

    #[test]
    fn ssh_profile_info_golden_json() {
        let info = SshProfileInfo {
            id: "prof-1".into(),
            name: "prod box".into(),
            host: "10.0.0.5".into(),
            port: 22,
            username: "root".into(),
            accent_color: None,
            last_used_at: Some("2026-06-10T08:30:00Z".into()),
            live: true,
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({
                "id": "prof-1",
                "name": "prod box",
                "host": "10.0.0.5",
                "port": 22,
                "username": "root",
                "accentColor": null,
                "lastUsedAt": "2026-06-10T08:30:00Z",
                "live": true,
            })
        );
    }

    #[test]
    fn spawn_agent_body_accepts_both_shapes() {
        let attach: SpawnAgentBody =
            serde_json::from_value(json!({ "sessionId": "sess-1" })).unwrap();
        assert_eq!(attach.session_id.as_deref(), Some("sess-1"));
        assert!(attach.new_session.is_none());

        let fresh: SpawnAgentBody = serde_json::from_value(json!({
            "newSession": { "projectPath": "/tmp/p", "provider": "codex", "title": "T" }
        }))
        .unwrap();
        assert!(fresh.session_id.is_none());
        let new = fresh.new_session.unwrap();
        assert_eq!(new.project_path, "/tmp/p");
        assert_eq!(new.provider, "codex");
        assert_eq!(new.title.as_deref(), Some("T"));
    }
}

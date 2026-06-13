// Companion server — the phone-facing HTTP surface for Clauge Mobile.
// Pairing, device tokens, and (in later tasks) session lists, spawn,
// and PTY mirroring over WebSocket. OFF by default: the server only
// runs after an explicit `companion_start` from Settings → Mobile.

pub mod api;
pub mod auth;
pub mod devices;
pub mod fanout;
pub mod lifecycle;
pub mod pairing;
pub mod push;
pub mod server;
pub mod ws;

use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;

/// First port tried by `server::start`. Sits just above the workspace
/// MCP range so both servers can coexist with their fallback walks.
pub const BASE_PORT: u16 = 7431;

/// How many sequential ports `start` walks past BASE_PORT when the
/// bind fails (7431..=7436). Same rationale as the MCP server: covers
/// the "something else grabbed the port" case without a long stall.
pub const PORT_FALLBACK_RANGE: u16 = 5;

/// Tauri event fired when a phone POSTs /pair with a valid code. The
/// frontend shows an approval dialog and answers via
/// `companion_approve_pair` / `companion_deny_pair`.
pub const EVT_PAIR_REQUEST: &str = "companion:pair-request";

/// Tauri event fired when a phone asks to open an Agent/SSH session.
/// The frontend opens a real desktop tab and answers via
/// `companion_report_opened` / `companion_report_open_failed`.
pub const EVT_OPEN_SESSION: &str = "companion:open-session";

/// Tauri event fired when a phone closes a session. The frontend
/// closes the matching tab the normal way (no confirm prompt).
pub const EVT_CLOSE_SESSION: &str = "companion:close-session";

pub struct CompanionState {
    /// Single-instance server handle, MCP-style: Some = running.
    pub server: AsyncMutex<Option<server::ServerHandle>>,
    /// Shared with the axum side so /pair can validate codes issued by
    /// the `companion_new_pair_code` command and park on approvals.
    pub pairing: Arc<pairing::PairingState>,
    /// Shared with the axum side so spawn handlers can park on the
    /// desktop UI opening a real tab and reporting its terminalId.
    pub lifecycle: Arc<lifecycle::LifecycleState>,
}

impl Default for CompanionState {
    fn default() -> Self {
        Self {
            server: AsyncMutex::new(None),
            pairing: Arc::new(pairing::PairingState::default()),
            lifecycle: Arc::new(lifecycle::LifecycleState::default()),
        }
    }
}

// ---------------------------------------------------------------------------
// Lifecycle report commands — the desktop UI's answer to a parked
// `companion:open-session` request.
// ---------------------------------------------------------------------------

/// The frontend opened a real tab and its PTY registered a terminalId.
/// Unblocks the parked /v1/sessions handler so it returns `{terminalId}`.
#[tauri::command]
pub fn companion_report_opened(
    request_id: String,
    terminal_id: String,
    state: tauri::State<'_, CompanionState>,
) -> Result<(), String> {
    state.lifecycle.resolve(&request_id, Ok(terminal_id))
}

/// The frontend failed to open the tab. Unblocks the parked handler so
/// it returns a 500 with the reported reason.
#[tauri::command]
pub fn companion_report_open_failed(
    request_id: String,
    error: String,
    state: tauri::State<'_, CompanionState>,
) -> Result<(), String> {
    state.lifecycle.resolve(&request_id, Err(error))
}

/// Desktop focus signal — retained so the frontend/IPC contract doesn't
/// break, but the sizing engine is now phone-always-wins-while-attached:
/// desktop focus no longer influences the PTY size. This is a harmless
/// no-op (it records the flag, which `desired_size` ignores).
#[tauri::command]
pub fn companion_set_terminal_focus(terminal_id: String, focused: bool) {
    if !fanout::hub_exists(&terminal_id) {
        return;
    }
    fanout::set_desktop_focused(&terminal_id, focused);
}

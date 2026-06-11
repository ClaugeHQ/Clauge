// Companion server — the phone-facing HTTP surface for Clauge Mobile.
// Pairing, device tokens, and (in later tasks) session lists, spawn,
// and PTY mirroring over WebSocket. OFF by default: the server only
// runs after an explicit `companion_start` from Settings → Mobile.

pub mod api;
pub mod auth;
pub mod devices;
pub mod pairing;
pub mod server;

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

pub struct CompanionState {
    /// Single-instance server handle, MCP-style: Some = running.
    pub server: AsyncMutex<Option<server::ServerHandle>>,
    /// Shared with the axum side so /pair can validate codes issued by
    /// the `companion_new_pair_code` command and park on approvals.
    pub pairing: Arc<pairing::PairingState>,
}

impl Default for CompanionState {
    fn default() -> Self {
        Self {
            server: AsyncMutex::new(None),
            pairing: Arc::new(pairing::PairingState::default()),
        }
    }
}

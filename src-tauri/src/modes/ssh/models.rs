use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;

// ---------------------------------------------------------------------------
// SSH profile / known hosts (DB-backed)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SshProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String, // "key" | "password" | "agent"
    pub key_path: Option<String>,
    pub accent_color: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,
    /// Optional self-FK: ID of another `ssh_profiles` row to use as a jump
    /// host (OpenSSH `ProxyJump`). The connection traverses jumps recursively
    /// at connect time. NULL = direct connect.
    pub jump_profile_id: Option<String>,
    /// Optional OpenSSH `ProxyCommand` template (with %h/%p/%r placeholders).
    /// Tokenized via shell-words and spawned as a subprocess at connect
    /// time; argv pipes become the underlying transport. NULL = no proxy
    /// command. If both this and `jump_profile_id` are set, ProxyCommand
    /// wins (matches OpenSSH spec).
    pub proxy_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SshKnownHost {
    pub profile_id: String,
    pub host: String,
    pub port: i64,
    pub fingerprint_sha256: String,
    pub accepted_at: String,
}

// ---------------------------------------------------------------------------
// In-process SSH terminal session state
//
// We never store russh's `Channel<Msg>` directly; the whole russh session
// lives inside a dedicated tokio task. Other parts of the app talk to that
// task through an mpsc command channel, which keeps the state map cheaply
// `Send + Sync` and avoids leaking russh internals.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SshCommand {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Kill,
}

pub(crate) struct SshTerminalEntry {
    pub(crate) handle_tx: tokio::sync::mpsc::UnboundedSender<SshCommand>,
    /// Profile this tab was spawned from — lets the companion server
    /// mark a profile `live` when any tab for it is open.
    pub(crate) profile_id: String,
}

pub struct SshTerminalState {
    pub(crate) terminals: Arc<Mutex<HashMap<String, SshTerminalEntry>>>,
}

impl Default for SshTerminalState {
    fn default() -> Self {
        Self {
            terminals: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// ---------------------------------------------------------------------------
// Pending keyboard-interactive auth prompts.
//
// When the SSH server sends an InfoRequest during keyboard-interactive auth,
// the connect path emits a Tauri event with a fresh `request_id` and stashes
// a oneshot Sender keyed by that id in this map. The frontend's auth-prompts
// modal collects user answers and calls `ssh_submit_auth_prompts` which
// looks up the Sender and forwards the answers — unblocking the parked
// auth flow.
//
// Cleanup: the Sender is removed (taken) when answers arrive, OR when the
// auth flow's timeout fires and the receiver drops (the Sender is then
// orphaned in the map but the next take/cleanup pass clears it; in practice
// the auth path also removes it from the map on cancel).
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct PendingAuthPrompts {
    pub(crate) inner: Arc<Mutex<HashMap<String, oneshot::Sender<Vec<String>>>>>,
}

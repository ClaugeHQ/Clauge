use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

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
    pub auth_type: String, // "key" | "password"
    pub key_path: Option<String>,
    pub accent_color: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,
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

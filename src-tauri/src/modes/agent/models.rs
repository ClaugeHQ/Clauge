use portable_pty::MasterPty;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use parking_lot::Mutex;

// ---------------------------------------------------------------------------
// Session data
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub project_path: String,
    pub project_name: String,
    pub claude_session_id: Option<String>,
    pub context_prompt: String,
    pub worktree_path: Option<String>,
    pub worktree_branch: Option<String>,
    pub skip_permissions: i32,
    pub git_name: Option<String>,
    pub git_email: Option<String>,
    pub created_at: String,
    pub last_used_at: String,
    /// `'manual'` = user-spawned terminal session (shown in Agent panel).
    /// `'card'`   = drawer-spawned hidden session for a workspace card
    /// (filtered out of the Agent panel; surfaced inside its card drawer).
    pub origin: String,
    /// Backref to the workspace card that owns this hidden session.
    /// `None` for manual sessions and for released hidden sessions.
    pub card_id: Option<String>,
    /// Which coding-assistant CLI backs this session. Maps to
    /// `CliRunner::id()` — `"claude"` | `"codex"` | `"gemini"` |
    /// `"opencode"`. Migration 13 added the column with a default of
    /// `"claude"`, so every pre-multi-CLI row still resolves correctly.
    pub provider: String,
    /// Per-session override for the CLI binary location. `None`/empty
    /// = fall back to standard $PATH lookup. Set when the user has
    /// the agent installed somewhere `find_binary` can't reach — a
    /// non-standard prefix, an asdf/nvm shim path, or a per-project
    /// pinned version. Migration 17 added the column.
    pub binary_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentContext {
    pub id: String,
    pub name: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Session discovery & usage (from Claude Code .jsonl files)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredSession {
    pub session_id: String,
    pub modified_at: String,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_context_tokens: u64,
    pub context_window: u64,
    pub fill_percent: f64,
    pub model: String,
    pub compacted: bool,
}

// ---------------------------------------------------------------------------
// Plugin management
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePlugin {
    pub name: String,
    pub marketplace: String,
    pub enabled: bool,
    pub version: Option<String>,
    pub install_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePlugin {
    pub name: String,
    pub description: String,
    pub marketplace: String,
    pub category: Option<String>,
    pub installed: bool,
    pub installs: Option<u64>,
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileChange {
    pub path: String,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Usage analytics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAnalytics {
    pub total_cost: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_write_tokens: u64,
    pub total_sessions: u32,
    pub total_api_calls: u32,
    pub cache_hit_percent: f64,
    pub daily: Vec<DailyUsage>,
    pub by_model: Vec<ModelUsage>,
    pub by_project: Vec<ProjectUsage>,
    pub top_sessions: Vec<SessionCost>,
    pub tools: Vec<ToolCount>,
    pub shell_commands: Vec<ToolCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsage {
    pub date: String,
    pub cost: f64,
    pub calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub cost: f64,
    pub calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_hit_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUsage {
    pub project: String,
    pub cost: f64,
    pub sessions: u32,
    pub calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCost {
    pub session_id: String,
    pub project: String,
    pub cost: f64,
    pub calls: u32,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCount {
    pub name: String,
    pub count: u32,
}

// ---------------------------------------------------------------------------
// Terminal state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputPayload {
    pub terminal_id: String,
    pub data: String, // base64 encoded
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<bool>, // true when PTY closed (EOF or read error)
}

pub(crate) struct TerminalEntry {
    pub(crate) master: Box<dyn MasterPty + Send>,
    pub(crate) writer: Box<dyn Write + Send>,
    #[allow(dead_code)]
    pub(crate) child: Box<dyn portable_pty::Child + Send>,
    /// Which agent session this PTY belongs to, for the companion
    /// server's status derivation. Companion spawns stamp the session
    /// row id; desktop spawns stamp the claude resume id they were
    /// given (the frontend keeps its own session→terminal map and
    /// never reads this). `None` for plain shell terminals.
    pub(crate) session_ref: Option<String>,
}

pub struct TerminalState {
    pub(crate) terminals: Arc<Mutex<HashMap<String, TerminalEntry>>>,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self {
            terminals: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

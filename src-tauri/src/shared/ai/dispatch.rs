// Generic AI tool registry + dispatch loop.
//
// Each mode (rest, sql, nosql, ssh, agent) registers its tools at app
// startup via `register()`. The chat streamers (anthropic.rs / openai.rs)
// invoke `execute()` keyed by the tool name the model emitted; the
// registry looks up the matching descriptor and runs its executor.
//
// The runtime contract preserved from the legacy `commands::ai::tools`:
//   - Tool executor returns `String` (already serialised result body)
//   - Logging shape is identical: `[AI Tool] name=… id=… params=[…]`
//     before, `result_len=… result_preview=…` after.
//   - Frontend-supplied tool JSON schemas continue to flow through
//     `ai_chat(tools: Vec<serde_json::Value>)` byte-identically. The
//     `schema` field on `ToolDescriptor` is informational only.
//
// Adding a new tool to a mode = one new function + one `register()` entry,
// zero edits to this file.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, RwLock};

use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::shared::ai::types::ChatContext;
use crate::modes::nosql::client::NoSqlConnections;
use crate::modes::sql::client::SqlConnectionManager;

/// Bag of state every tool executor receives. Mirrors the params the legacy
/// `execute_tool` accepted so registered handlers can be straight rehoming
/// of the old match arms.
pub struct ToolContext<'a> {
    pub tool_use_id: &'a str,
    pub input: &'a serde_json::Value,
    pub context: &'a ChatContext,
    pub pool: &'a SqlitePool,
    pub app: &'a AppHandle,
    pub session_id: &'a str,
    pub sql_manager: &'a Arc<SqlConnectionManager>,
    pub nosql_conns: &'a NoSqlConnections,
}

pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = String> + Send + 'a>>;
pub type ToolExecutor = for<'a> fn(ctx: &'a ToolContext<'a>) -> ToolFuture<'a>;

#[derive(Clone)]
pub struct ToolDescriptor {
    /// Tool name as the model sees it (e.g. `execute_request`, `list_tables`).
    pub name: &'static str,
    /// Owning mode slug: `agent` | `rest` | `sql` | `nosql` | `ssh`.
    pub mode: &'static str,
    /// Short description (informational; outbound schema is sent by frontend).
    #[allow(dead_code)]
    pub description: &'static str,
    /// JSONSchema for the input shape (informational; outbound schema is
    /// sent by frontend so the model-facing JSON is byte-identical to today).
    #[allow(dead_code)]
    pub schema: serde_json::Value,
    /// Async executor.
    pub executor: ToolExecutor,
}

fn registry() -> &'static RwLock<Vec<ToolDescriptor>> {
    static REGISTRY: OnceLock<RwLock<Vec<ToolDescriptor>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Modes call this at app startup to record their tools.
/// Idempotent: re-registering the same `name` replaces the prior descriptor
/// so a hot-reload loop in dev does not stack duplicates.
pub fn register(descriptor: ToolDescriptor) {
    let mut guard = registry().write().expect("tool registry poisoned");
    if let Some(slot) = guard.iter_mut().find(|d| d.name == descriptor.name) {
        *slot = descriptor;
    } else {
        guard.push(descriptor);
    }
}

/// Returns a clone of the descriptor matching `name`, or `None`.
#[allow(dead_code)]
pub fn lookup(name: &str) -> Option<ToolDescriptor> {
    registry()
        .read()
        .expect("tool registry poisoned")
        .iter()
        .find(|d| d.name == name)
        .cloned()
}

/// Cheap, name-only heuristic: is this tool an "introspection" call
/// (read-only schema / list / sample) vs an "action" call (executes a
/// query, writes a file, applies a change)?
///
/// Used by the streaming loop to detect "N rounds of inspecting without
/// committing to an answer" — the classic failure mode that burns paid
/// credits while the model gets nowhere.
///
/// Defaults to introspection on unknown names so a misclassification only
/// fires the steering message *earlier*, never later.
pub fn is_introspection_tool(name: &str) -> bool {
    const ACTION_NEEDLES: &[&str] = &[
        "execute", "apply", "find", "aggregate", "count", "insert",
        "create", "update", "delete", "write", "mkdir", "rename",
        "upload", "download", "run", "submit", "redis_execute",
    ];
    let lower = name.to_lowercase();
    !ACTION_NEEDLES.iter().any(|n| lower.contains(n))
}

/// Stable hash of a tool-call's arguments. Used by the streaming loop's
/// dedup detector — if the same (tool_name, args_hash) repeats across
/// adjacent rounds, the model is stuck and should be nudged.
pub fn hash_tool_args(args: &serde_json::Value) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let canonical = serde_json::to_string(args).unwrap_or_default();
    let mut h = DefaultHasher::new();
    canonical.hash(&mut h);
    h.finish()
}

/// Returns descriptors for all tools owned by the given mode.
#[allow(dead_code)]
pub fn tools_for_mode(mode: &str) -> Vec<ToolDescriptor> {
    registry()
        .read()
        .expect("tool registry poisoned")
        .iter()
        .filter(|d| d.mode == mode)
        .cloned()
        .collect()
}

/// Replacement for the legacy `commands::ai::tools::execute_tool`.
/// Same logging shape; same `String` return body. If no mode has registered
/// the requested tool, returns `"Unknown tool: <name>"` to match historical
/// behaviour exactly.
pub async fn execute<'a>(tool_name: &str, ctx: ToolContext<'a>) -> String {
    let safe_keys: Vec<String> = ctx
        .input
        .as_object()
        .map(|o| o.keys().map(|k| k.to_string()).collect())
        .unwrap_or_default();
    log::info!(
        "[AI Tool] name={} id={} params=[{}]",
        tool_name,
        ctx.tool_use_id,
        safe_keys.join(", ")
    );

    let descriptor = registry()
        .read()
        .expect("tool registry poisoned")
        .iter()
        .find(|d| d.name == tool_name)
        .cloned();

    let result = match descriptor {
        Some(d) => (d.executor)(&ctx).await,
        None => format!("Unknown tool: {}", tool_name),
    };

    log::info!(
        "[AI Tool] name={} result_len={} result_preview={}",
        tool_name,
        result.len(),
        crate::shared::ai::context::truncate_str(&result, 300)
    );
    result
}

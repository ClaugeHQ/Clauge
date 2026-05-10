// MCP server for the Workspace mode. Lets external agents (Claude
// Code today; Codex / Gemini later) drive Notes + Boards via the
// Model Context Protocol over local HTTP.
//
// Transport: streamable HTTP (POST /mcp returns a JSON-RPC response).
// We don't yet support the SSE side of streamable-http — Claude Code's
// tool-call path doesn't require it, and a single-response POST is
// strictly simpler. Bearer-token auth keeps random local processes
// out; the token is auto-generated on first start and persisted in
// settings (rotatable from the UI).
//
// Tool surface:
//   workspaces.list, workspaces.read
//   notes.list, notes.read, notes.create, notes.update, notes.delete
//   boards.list, boards.read
//   cards.list, cards.create, cards.update, cards.move
// Every mutation runs with actor="claude" so the existing attribution
// + Inbox machinery surfaces agent activity automatically.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json as JsonResponse},
    routing::post,
    Json,
    Router,
};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::shared::repos::{sessions as session_repo, workspaces as repo};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Map an incoming request to an actor string used for attribution.
///
/// Order of precedence:
///   1. `User-Agent` header — every CLI worth its salt sets one. We
///      lower-case it and look for a known agent slug. This handles
///      Claude Code, Codex, Gemini, OpenCode without per-agent setup.
///   2. `clientInfo.name` from the JSON-RPC `initialize` params (when
///      we can read it from the current request body) — second-best
///      because it's only present on init, but we accept any request
///      that happens to include it.
///   3. Default `'agent'` — generic fallback so attribution never
///      shows the literal string `'user'`.
///
/// Adding a new agent is one entry in `KNOWN_AGENTS`. The actor string
/// flows straight into `updated_by`, so the existing Inbox / attribution
/// pipeline picks it up with zero extra wiring.
fn actor_from_request(headers: &axum::http::HeaderMap, body: &Value) -> String {
    let known: &[(&str, &str)] = &[
        ("claude", "claude"),       // claude-code/<v>, anthropic-claude/<v>
        ("codex", "codex"),         // openai-codex / codex-cli
        ("gemini", "gemini"),       // google-gemini-cli
        ("opencode", "opencode"),   // open-code project
        ("aider", "aider"),         // aider.chat
    ];
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if !ua.is_empty() {
        if let Some((_, actor)) = known.iter().find(|(slug, _)| ua.contains(slug)) {
            return (*actor).to_string();
        }
    }
    if let Some(name) = body
        .get("params")
        .and_then(|p| p.get("clientInfo"))
        .and_then(|c| c.get("name"))
        .and_then(|v| v.as_str())
    {
        let lower = name.to_lowercase();
        if let Some((_, actor)) = known.iter().find(|(slug, _)| lower.contains(slug)) {
            return (*actor).to_string();
        }
        // Sanitise unknown client name so it can't inject odd chars
        // into the DB. Keep alphanum + dashes.
        let cleaned: String = lower
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(32)
            .collect();
        if !cleaned.is_empty() {
            return cleaned;
        }
    }
    "agent".to_string()
}

pub struct McpHandle {
    pub port: u16,
    pub shutdown: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
struct McpAppState {
    pool: SqlitePool,
    token: String,
    /// Optional Tauri handle so MCP-side mutations can emit live
    /// events (e.g. `workspace:card-updated`). None when MCP runs
    /// before the Tauri app is fully bootstrapped — emits become
    /// no-ops in that window.
    app: Option<tauri::AppHandle>,
}

/// Bind a listener on 127.0.0.1:`port`, spawn the axum server, and
/// return a handle whose `shutdown` sender stops the server.
pub async fn start(
    pool: SqlitePool,
    port: u16,
    token: String,
    app: Option<tauri::AppHandle>,
) -> Result<McpHandle, String> {
    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Failed to bind {}: {}", addr, e))?;

    let state = McpAppState { pool, token, app };
    let app = Router::new()
        .route("/mcp", post(handle_mcp))
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .with_state(Arc::new(state));

    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });
    Ok(McpHandle { port, shutdown: Some(tx) })
}

/// Single JSON-RPC POST handler. We dispatch on `method` and respond
/// with either `result` or `error` — never both.
async fn handle_mcp(
    State(state): State<Arc<McpAppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    // Bearer auth — strict comparison, no fallthrough.
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if token != state.token {
        return (
            StatusCode::UNAUTHORIZED,
            JsonResponse(json!({
                "jsonrpc": "2.0",
                "error": { "code": -32001, "message": "Unauthorized" }
            })),
        )
            .into_response();
    }

    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let params = body.get("params").cloned().unwrap_or(json!({}));

    // Resolve the actor for THIS request. Mutating tools route this
    // string straight into `updated_by`, so attribution + Inbox work
    // identically across Claude / Codex / Gemini / etc.
    let actor = actor_from_request(&headers, &body);

    let result: Result<Value, (i32, String)> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "clauge-workspace", "version": "1.0.0" }
        })),
        "notifications/initialized" => Ok(Value::Null),
        "tools/list" => Ok(json!({ "tools": tool_descriptors() })),
        "tools/call" => dispatch_tool(&state.pool, state.app.as_ref(), params, &actor).await,
        "ping" => Ok(json!({})),
        _ => Err((-32601, format!("Method not found: {}", method))),
    };

    let response = match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, msg)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } })
        }
    };
    (StatusCode::OK, JsonResponse(response)).into_response()
}

fn tool_descriptors() -> Value {
    json!([
        {
            "name": "workspaces_list",
            "description": "List all workspaces (containers of notes and boards).",
            "inputSchema": { "type": "object", "properties": {}, "required": [] }
        },
        {
            "name": "workspaces_upsert_for_project",
            "description": "Find a workspace bound to the given project path. If none exists, create one named after the folder with a default 5-column board, and return it. Use this whenever you have a project path (e.g. cwd) and want a workspace to put notes or cards in — it's the canonical way to resolve 'this project' to a workspace id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectPath": { "type": "string", "description": "Absolute path to the project's root folder." }
                },
                "required": ["projectPath"]
            }
        },
        {
            "name": "notes_list",
            "description": "List notes inside a workspace.",
            "inputSchema": {
                "type": "object",
                "properties": { "workspaceId": { "type": "string" } },
                "required": ["workspaceId"]
            }
        },
        {
            "name": "notes_read",
            "description": "Read a note by id (returns title, content, tags, linked session).",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "notes_create",
            "description": "Create a new note inside an existing workspace. Returns the new note. Use this when you already know the workspaceId; otherwise prefer notes_create_for_project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["workspaceId", "title"]
            }
        },
        {
            "name": "notes_create_for_project",
            "description": "Create a NEW note for the given project (always creates, never replaces). If no workspace exists for the project path, one is auto-created (named after the folder, with a default board) before the note is added. Returns { workspace, note }. Prefer notes_upsert_for_project when the user is asking to record/refresh information on a topic — it'll update an existing same-titled note instead of stacking duplicates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectPath": { "type": "string", "description": "Absolute path to the project's root folder." },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["projectPath", "title"]
            }
        },
        {
            "name": "notes_upsert_for_project",
            "description": "Find-or-create a note in the project's workspace (workspace itself is auto-created if missing). Match is by case-insensitive title within that one workspace. If the note exists, content/tags are UPDATED (replace by default; pass mode='append' to add to the bottom). Returns { workspace, note, created: bool }. This is the right tool for evolving topical docs ('Overview', 'Architecture', 'TODO', etc.) — calling it twice with the same title edits the same note instead of duplicating.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectPath": { "type": "string", "description": "Absolute path to the project's root folder." },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "mode": {
                        "type": "string",
                        "enum": ["replace", "append"],
                        "description": "How to apply content when the note already exists. 'replace' (default) overwrites; 'append' adds the new content as a new section at the bottom separated by ---."
                    }
                },
                "required": ["projectPath", "title"]
            }
        },
        {
            "name": "notes_update",
            "description": "Update an existing note. Pass any of title, content, tags.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["id"]
            }
        },
        {
            "name": "boards_list",
            "description": "List boards inside a workspace.",
            "inputSchema": {
                "type": "object",
                "properties": { "workspaceId": { "type": "string" } },
                "required": ["workspaceId"]
            }
        },
        {
            "name": "boards_read",
            "description": "Read a board's columns and cards. Returns { columns, cards } in one payload.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "cards_create",
            "description": "Create a new card inside a column. Pass `coworkerId` to attribute the card to a persona (the persona's avatar/name will appear on the card foot). Pass `parentCardId` to record lineage when this card is spawned from another card's discussion — the UI shows a 'From card X' breadcrumb.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "columnId":     { "type": "string" },
                    "title":        { "type": "string" },
                    "description":  { "type": "string" },
                    "priority":     { "type": "string", "enum": ["P0", "P1", "P2", "P3"] },
                    "tags":         { "type": "array", "items": { "type": "string" } },
                    "coworkerId":   { "type": "string", "description": "Persona that's creating this card. Use your declared coworker_id when acting as a persona." },
                    "parentCardId": { "type": "string", "description": "Card this one was spawned from, for lineage." }
                },
                "required": ["columnId", "title"]
            }
        },
        {
            "name": "cards_update",
            "description": "Update a card's title, description, priority, tags, or review checklist. Pass `coworkerId` to record which persona made the change.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id":              { "type": "string" },
                    "title":           { "type": "string" },
                    "description":     { "type": "string" },
                    "priority":        { "type": "string" },
                    "tags":            { "type": "array", "items": { "type": "string" } },
                    "reviewChecklist": { "type": "string" },
                    "coworkerId":      { "type": "string" }
                },
                "required": ["id"]
            }
        },
        {
            "name": "cards_move",
            "description": "Move a card to a column / position. Moving an agent's card to a Review-class column auto-flags it as Pending review.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "columnId": { "type": "string" },
                    "position": { "type": "integer" }
                },
                "required": ["id", "columnId"]
            }
        },
        {
            "name": "notes_search",
            "description": "Full-text search notes by title and content. Optional workspaceId scopes the search; otherwise searches across all workspaces. Returns ranked notes (best match first). Use this before notes_upsert_for_project to check whether a topic already has a note under a different title.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "FTS5 query string. Plain words match prefix-tokens; quote phrases for exact matches." },
                    "workspaceId": { "type": "string" },
                    "limit": { "type": "integer", "description": "Max rows to return (default 20)." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "cards_search",
            "description": "Full-text search board cards by title and description. Returns ranked cards (best match first). Useful for de-duplicating: search before cards_create.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "workspaceId": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "cards_approve",
            "description": "Clear the Pending-review flag on a card (optionally appending a short approval comment to the description). Use when the human work the agent submitted has been validated and the card should leave the review queue.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "comment": { "type": "string", "description": "Optional approval note appended to the card description." }
                },
                "required": ["id"]
            }
        },
        {
            "name": "cards_request_changes",
            "description": "Send a card back from Review with structured feedback. Appends the feedback to the description (under a 'Review feedback' marker), clears the Pending-review flag, and (if columnId is provided) moves the card there — typically the 'In Review' column.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "feedback": { "type": "string" },
                    "columnId": { "type": "string", "description": "Optional column to move the card to (e.g. the In Review column)." }
                },
                "required": ["id", "feedback"]
            }
        },
        {
            "name": "cards_list_pending_review",
            "description": "List cards currently flagged as Pending review (i.e. an agent moved them into a Review column). Optionally scope to one workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" }
                },
                "required": []
            }
        },
        {
            "name": "boards_summary",
            "description": "One-shot board health snapshot: per-column card counts, total cards, review backlog. Cheaper than boards_read when you only need numbers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "boardId": { "type": "string" }
                },
                "required": ["boardId"]
            }
        },
        {
            "name": "workspace_summary",
            "description": "Workspace-level snapshot: note count, card count, board count, review backlog, and a per-actor edit count breakdown (helps see who is contributing — user vs agent).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" }
                },
                "required": ["workspaceId"]
            }
        },
        {
            "name": "notes_append_section",
            "description": "Append a new markdown section (heading + body) to the bottom of an existing note. Less destructive than notes_update — preserves all prior content. Use this for incremental log/journal style notes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "heading": { "type": "string", "description": "Section heading text. A '## ' prefix is added automatically." },
                    "content": { "type": "string", "description": "Body of the new section." }
                },
                "required": ["id", "heading", "content"]
            }
        },
        {
            "name": "notes_apply_diff",
            "description": "Surgical find/replace on a note's content (literal string match — not a regex). The `find` text must appear exactly once or the call errors. Prefer this over notes_update for small edits where you don't want to risk overwriting the rest of the note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "find": { "type": "string" },
                    "replace": { "type": "string" }
                },
                "required": ["id", "find", "replace"]
            }
        },
        {
            "name": "cards_create_from_branch",
            "description": "Convenience: create a card titled after a git branch name (e.g. 'feature/add-login' → 'Add login'). The branch is stored as `externalId` so future tooling can link card ↔ branch. If projectPath is given, the card lands in the project's workspace's first board (Todo column when present, else first column). Otherwise pass columnId explicitly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "branchName": { "type": "string" },
                    "projectPath": { "type": "string" },
                    "columnId": { "type": "string", "description": "Override target column. Wins over projectPath inference." },
                    "description": { "type": "string" }
                },
                "required": ["branchName"]
            }
        },
        {
            "name": "notes_link_to_session",
            "description": "Attach an agent session id to a note. Lets the UI jump from a note straight back to the conversation that produced it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "sessionId": { "type": ["string", "null"] }
                },
                "required": ["id"]
            }
        },
        {
            "name": "cards_link_to_session",
            "description": "Attach an agent session id to a card. Same purpose as notes_link_to_session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "sessionId": { "type": ["string", "null"] }
                },
                "required": ["id"]
            }
        },
        {
            "name": "activity_feed",
            "description": "Recent agent activity — notes and cards mutated by non-user actors. Filter by actor and a since-timestamp. Same data the Inbox UI surfaces; useful for an agent to catch up on what other agents have been doing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "since": { "type": "string", "description": "RFC3339 timestamp; only entries newer than this are returned." },
                    "actor": { "type": "string", "description": "Filter to one actor slug (e.g. 'claude', 'codex')." },
                    "limit": { "type": "integer", "description": "Default 50." }
                },
                "required": []
            }
        },
        {
            "name": "notes_freeze",
            "description": "Mark a note as frozen — agents (including this one) can no longer mutate it via MCP until unfrozen. UI edits are unaffected. Use to lock down canonical docs (e.g. 'Architecture Overview').",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "notes_unfreeze",
            "description": "Reverse of notes_freeze.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "cards_freeze",
            "description": "Mark a card as frozen — blocks agent updates/moves. UI edits unaffected.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "cards_unfreeze",
            "description": "Reverse of cards_freeze.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "workspace_link_to_repo",
            "description": "Set or clear the workspace's GitHub/GitLab URL. Used as the default remote when a board has no per-board override. Pass `null` (or omit `repoUrl`) to clear.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" },
                    "repoUrl": { "type": ["string", "null"] }
                },
                "required": ["workspaceId"]
            }
        },
        {
            "name": "cards_add_comment",
            "description": "Post a comment on a card. Comments live in the card's thread (visible in the drawer). Pass `coworkerId` when you're acting as a persona — your comment renders with that persona's avatar + name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id":         { "type": "string" },
                    "body":       { "type": "string" },
                    "coworkerId": { "type": "string", "description": "Persona authoring the comment, when applicable." }
                },
                "required": ["id", "body"]
            }
        },
        {
            "name": "cards_start_work",
            "description": "Create an isolated git worktree + branch for this card and attach it to the active hidden session. Call this BEFORE you make file edits in a card-driven chat — your subsequent edits go into the worktree, keeping the user's main checkout clean. No-op if the card already has a worktree. Errors when the card is owned by a manual terminal session (those manage their own worktrees).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Card id." }
                },
                "required": ["id"]
            }
        },
        {
            "name": "cards_call_coworker",
            "description": "Invoke a specific coworker (persona) to chat on a card. The system: (1) posts your message as a comment from the calling agent, (2) creates-or-reuses the card's hidden session for that coworker, (3) runs the coworker as an agent with their persona prompt, (4) posts the response as a comment from the coworker, and (5) returns the response text so you can summarise back to the user. Use this when the user asks you to involve a specific coworker on a card without leaving your own session. The card becomes claimed by the coworker after this call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id":         { "type": "string", "description": "Card id." },
                    "coworkerId": { "type": "string", "description": "Coworker to invoke." },
                    "message":    { "type": "string", "description": "What to ask them — e.g. 'Brainstorm OAuth approaches for this card'." }
                },
                "required": ["id", "coworkerId", "message"]
            }
        },
        {
            "name": "coworkers_list",
            "description": "List all coworkers (personas) the user has set up. Each coworker has a name, role, system_prompt that's appended at agent spawn, avatar, and underlying provider. Use this to know who's on the team — e.g. when the user asks 'who's working with me?'.",
            "inputSchema": { "type": "object", "properties": {}, "required": [] }
        },
        {
            "name": "cards_claim",
            "description": "Claim ownership of a card for this agent's calling session — the agent becomes the single work-stream allowed to drive the card going forward. The drawer in the UI will show 'Active in <session-title>' and disable its in-drawer chat. Use this when the user tells you to 'work on card X' from the terminal: claim, then proceed to add comments / move columns / write code as normal. Errors when the card is already claimed by a different session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Card id." }
                },
                "required": ["id"]
            }
        },
        {
            "name": "cards_release",
            "description": "Release the claim this session holds on a card. Use when finished working — the card unlocks and the drawer can host new chats again. No-op when the card isn't claimed by this session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Card id." }
                },
                "required": ["id"]
            }
        },
        {
            "name": "cards_push_to_repo",
            "description": "Create a real GitHub/GitLab issue from a local card. Requires the workspace to have a repo URL set (workspace_link_to_repo) — otherwise the call errors. The card must currently be local (no externalId). On success the card's externalId/externalUrl are populated; the local→repo badge updates automatically. This call SHELLS OUT to `gh` or `glab` and so requires the user to have those CLIs installed and authenticated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" }
                },
                "required": ["id"]
            }
        },
        {
            "name": "cards_commit",
            "description": "Stage and commit any pending changes in the card's worktree with the given message. Requires an active claim + worktree (call cards_start_work first if needed). Errors with 'no changes' when the worktree is clean. ONLY call this when the user explicitly asks to commit — never on your own initiative. Drops a 'Commit on <branch>' bubble on the card thread so the user sees the activity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cardId":  { "type": "string", "description": "Card id." },
                    "message": { "type": "string", "description": "Commit message. Should describe the change in present tense — e.g. 'Add login rate-limit guard'." }
                },
                "required": ["cardId", "message"]
            }
        },
        {
            "name": "cards_raise_pr",
            "description": "Push the card's branch and (if no PR exists yet) open a GitHub PR / GitLab MR for it. Idempotent — when the card already has a pr_url, this just pushes new commits to the existing PR's branch (no second PR is opened). Requires worktree + branch + workspace repo URL. ONLY call when the user explicitly asks ('raise a PR', 'ship it', 'push it') — never autonomously. Returns { prUrl, alreadyExisted, branch }.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cardId": { "type": "string", "description": "Card id." },
                    "title":  { "type": "string", "description": "Optional PR title. Defaults to the card title." },
                    "body":   { "type": "string", "description": "Optional PR body. Defaults to a one-line reference to the card thread." }
                },
                "required": ["cardId"]
            }
        },
        {
            "name": "cards_link_pr",
            "description": "Stamp a PR / MR URL onto a card without running any CLI. Use when you raised a PR via raw bash (legacy path) and want the card's UI to show the link. The preferred path is cards_raise_pr — it does the push + open + link in one step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cardId": { "type": "string", "description": "Card id." },
                    "prUrl":  { "type": "string", "description": "Full PR / MR URL." }
                },
                "required": ["cardId", "prUrl"]
            }
        }
    ])
}

/// Find-or-create a workspace bound to `project_path`. Match is by
/// exact absolute path; the new workspace's name is the last segment.
/// Used by the `*_for_project` convenience tools so an agent can drop
/// notes/boards into "the current project" without first listing
/// workspaces. The default board layout (5 columns) is created so the
/// workspace is immediately useful — matches the UI's create flow.
async fn upsert_workspace_for_project(
    pool: &SqlitePool,
    project_path: &str,
    actor: &str,
) -> Result<crate::modes::workspace::models::Workspace, (i32, String)> {
    let map_db = |e: sqlx::Error| -> (i32, String) { (-32603, format!("DB error: {}", e)) };
    if let Some(ws) = repo::find_workspace_by_project_path(pool, project_path)
        .await
        .map_err(map_db)?
    {
        return Ok(ws);
    }
    // Insert new workspace named after the last path segment.
    let name = std::path::Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();
    let project_name = name.clone();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    repo::insert_workspace(
        pool,
        &id,
        &name,
        Some(project_path),
        Some(&project_name),
        None,
        actor,
        &now,
    )
    .await
    .map_err(map_db)?;

    // Default board seeded from the shared `repo::DEFAULT_BOARD_COLUMNS`
    // constant — keeps the agent-spawned board layout in lockstep with
    // the UI-created one. See `commands::create_default_board`.
    let board_id = uuid::Uuid::new_v4().to_string();
    repo::insert_board(pool, &board_id, &id, "Tasks", "manual", None, 0, &now)
        .await
        .map_err(map_db)?;
    for (idx, (col_name, col_color)) in repo::DEFAULT_BOARD_COLUMNS.iter().enumerate() {
        repo::insert_column(
            pool,
            &uuid::Uuid::new_v4().to_string(),
            &board_id,
            col_name,
            Some(col_color),
            idx as i32,
            &now,
        )
        .await
        .map_err(map_db)?;
    }

    repo::get_workspace_by_id(pool, &id).await.map_err(map_db)
}

/// Auto-link a card to the agent session that "owns" it whenever an
/// agent mutates it via MCP. The MCP server doesn't natively know the
/// caller's session id (one shared HTTP endpoint, multiple potential
/// sessions per provider), so we use a heuristic: the most recently-
/// used session whose `project_path` matches the workspace's. In the
/// solo-dev / single-session-per-project case this is exact; with
/// multiple parallel sessions per project, "most recent" tracks
/// whichever session the user last activated, which is virtually
/// always the one currently making MCP calls.
///
/// Skipped when:
///   • Card's workspace has no project_path bound (nothing to match).
///   • No agent_session exists for that project_path yet.
///   • Caller is a user actor (UI edits — those don't represent
///     "session activity"). MCP actors are always agents in practice,
///     but we guard explicitly so callers can pass any actor.
///   • The card is already linked to that exact session — saves a
///     no-op write.
async fn auto_link_card_to_recent_session(
    pool: &SqlitePool,
    card_id: &str,
    actor: &str,
    now: &str,
) {
    if actor == "user" || actor.starts_with("user:") {
        return;
    }
    // Walk card → column → board → workspace.project_path. Both columns
    // are nullable so the tuple is (Option, Option).
    let row: Result<Option<(Option<String>, Option<String>)>, _> = sqlx::query_as(
        "SELECT c.linked_session_id, w.project_path \
         FROM workspace_board_cards c \
         JOIN workspace_board_columns col ON col.id = c.column_id \
         JOIN workspace_boards b ON b.id = col.board_id \
         JOIN workspaces w ON w.id = b.workspace_id \
         WHERE c.id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await;
    let (current_link, project_path) = match row {
        Ok(Some(r)) => r,
        _ => return,
    };
    let project_path = match project_path.filter(|p| !p.trim().is_empty()) {
        Some(p) => p,
        None => return,
    };
    let session = match session_repo::find_recent_session_for_project(pool, &project_path).await {
        Ok(Some(s)) => s,
        _ => return,
    };
    if current_link.as_deref() == Some(session.id.as_str()) {
        return;
    }
    // Best-effort write — if it fails (deleted card race, etc.) we
    // silently skip; the original mutation already succeeded.
    let _ = repo::update_card_linked_session(pool, card_id, Some(&session.id), actor, now).await;
}

/// Sibling of `auto_link_card_to_recent_session`, scoped to notes.
/// Same heuristic, same skip rules — the join walk is shorter because
/// notes belong to the workspace directly (no column/board hops).
async fn auto_link_note_to_recent_session(
    pool: &SqlitePool,
    note_id: &str,
    actor: &str,
    now: &str,
) {
    if actor == "user" || actor.starts_with("user:") {
        return;
    }
    let row: Result<Option<(Option<String>, Option<String>)>, _> = sqlx::query_as(
        "SELECT n.linked_session_id, w.project_path \
         FROM workspace_notes n \
         JOIN workspaces w ON w.id = n.workspace_id \
         WHERE n.id = ?",
    )
    .bind(note_id)
    .fetch_optional(pool)
    .await;
    let (current_link, project_path) = match row {
        Ok(Some(r)) => r,
        _ => return,
    };
    let project_path = match project_path.filter(|p| !p.trim().is_empty()) {
        Some(p) => p,
        None => return,
    };
    let session = match session_repo::find_recent_session_for_project(pool, &project_path).await {
        Ok(Some(s)) => s,
        _ => return,
    };
    if current_link.as_deref() == Some(session.id.as_str()) {
        return;
    }
    let _ = repo::update_note_linked_session(pool, note_id, Some(&session.id), actor, now).await;
}

/// Wrap a value as MCP tool-call content. Single text item — clients
/// re-parse the JSON; that's the simplest, most compatible shape.
fn ok_text(value: Value) -> Value {
    json!({
        "content": [
            { "type": "text", "text": value.to_string() }
        ],
        "isError": false
    })
}

async fn dispatch_tool(
    pool: &SqlitePool,
    app: Option<&tauri::AppHandle>,
    params: Value,
    actor: &str,
) -> Result<Value, (i32, String)> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((-32602, "Missing tool name".into()))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let now = chrono::Utc::now().to_rfc3339();
    let new_id = || uuid::Uuid::new_v4().to_string();
    let str_arg = |key: &str| -> Option<String> {
        args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
    };
    let req_str = |key: &str| -> Result<String, (i32, String)> {
        str_arg(key).ok_or((-32602, format!("{} required", key)))
    };
    let str_array = |key: &str| -> Vec<String> {
        args.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };

    let map_db = |e: sqlx::Error| -> (i32, String) { (-32603, format!("DB error: {}", e)) };

    match name {
        "workspaces_list" => {
            let v = repo::list_workspaces(pool).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "workspaces_upsert_for_project" => {
            let project_path = req_str("projectPath")?;
            let ws = upsert_workspace_for_project(pool, &project_path, actor).await?;
            Ok(ok_text(serde_json::to_value(ws).unwrap_or(Value::Null)))
        }
        "notes_create_for_project" => {
            let project_path = req_str("projectPath")?;
            let title = req_str("title")?;
            let content = str_arg("content").unwrap_or_default();
            let tags = str_array("tags");
            let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into());
            let ws = upsert_workspace_for_project(pool, &project_path, actor).await?;
            let id = new_id();
            repo::insert_note(
                pool, &id, &ws.id, &title, &content, &tags_json, None, actor, &now,
            )
            .await
            .map_err(map_db)?;
            auto_link_note_to_recent_session(pool, &id, actor, &now).await;
            let note = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(json!({
                "workspace": ws,
                "note": note
            })))
        }
        "notes_upsert_for_project" => {
            let project_path = req_str("projectPath")?;
            let title = req_str("title")?;
            let new_content = str_arg("content").unwrap_or_default();
            let mode = str_arg("mode").unwrap_or_else(|| "replace".to_string());
            let tags_provided = args.get("tags").is_some();
            let new_tags = str_array("tags");

            let ws = upsert_workspace_for_project(pool, &project_path, actor).await?;
            let existing = repo::find_note_by_title_in_workspace(pool, &ws.id, &title)
                .await
                .map_err(map_db)?;

            let note = match existing {
                None => {
                    // Create — same path as notes_create_for_project.
                    let tags_json = serde_json::to_string(&new_tags)
                        .unwrap_or_else(|_| "[]".into());
                    let id = new_id();
                    repo::insert_note(
                        pool, &id, &ws.id, &title, &new_content, &tags_json,
                        None, actor, &now,
                    )
                    .await
                    .map_err(map_db)?;
                    let n = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
                    Ok::<_, (i32, String)>((n, true))
                }
                Some(cur) => {
                    // Frozen notes are off-limits to agents — caller must
                    // unfreeze (or change via the UI) before this call works.
                    if repo::is_note_frozen(pool, &cur.id).await.map_err(map_db)? {
                        return Err((-32000, format!("Note '{}' is frozen", cur.title)));
                    }
                    // Update — apply mode, decide tags policy.
                    let merged_content = match mode.as_str() {
                        "append" => {
                            if cur.content.trim().is_empty() {
                                new_content.clone()
                            } else if new_content.trim().is_empty() {
                                cur.content.clone()
                            } else {
                                format!("{}\n\n---\n\n{}", cur.content.trim_end(), new_content)
                            }
                        }
                        // "replace" + any other value falls through to overwrite.
                        _ => new_content.clone(),
                    };
                    let tags_json = if tags_provided {
                        serde_json::to_string(&new_tags).unwrap_or_else(|_| "[]".into())
                    } else {
                        cur.tags.clone()
                    };
                    repo::update_note(
                        pool, &cur.id, &title, &merged_content, &tags_json,
                        cur.linked_session_id.as_deref(), actor, &now,
                    )
                    .await
                    .map_err(map_db)?;
                    let n = repo::get_note_by_id(pool, &cur.id).await.map_err(map_db)?;
                    Ok::<_, (i32, String)>((n, false))
                }
            }?;

            auto_link_note_to_recent_session(pool, &note.0.id, actor, &now).await;
            Ok(ok_text(json!({
                "workspace": ws,
                "note": note.0,
                "created": note.1
            })))
        }
        "notes_list" => {
            let workspace_id = req_str("workspaceId")?;
            let v = repo::list_notes_in_workspace(pool, &workspace_id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "notes_read" => {
            let id = req_str("id")?;
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "notes_create" => {
            let workspace_id = req_str("workspaceId")?;
            let title = req_str("title")?;
            let content = str_arg("content").unwrap_or_default();
            let tags = str_array("tags");
            let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into());
            let id = new_id();
            repo::insert_note(pool, &id, &workspace_id, &title, &content, &tags_json, None, actor, &now)
                .await.map_err(map_db)?;
            auto_link_note_to_recent_session(pool, &id, actor, &now).await;
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "notes_update" => {
            let id = req_str("id")?;
            // Frozen → reject. UI is the only path to edit a frozen note.
            if repo::is_note_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Note is frozen".into()));
            }
            // Read existing so we can patch only the fields the agent passed.
            let cur = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            let title = str_arg("title").unwrap_or(cur.title);
            let content = str_arg("content").unwrap_or(cur.content);
            let tags_json = if args.get("tags").is_some() {
                serde_json::to_string(&str_array("tags")).unwrap_or("[]".into())
            } else {
                cur.tags
            };
            repo::update_note(pool, &id, &title, &content, &tags_json,
                cur.linked_session_id.as_deref(), actor, &now)
                .await.map_err(map_db)?;
            auto_link_note_to_recent_session(pool, &id, actor, &now).await;
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "boards_list" => {
            let workspace_id = req_str("workspaceId")?;
            let v = repo::list_boards_in_workspace(pool, &workspace_id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "boards_read" => {
            let id = req_str("id")?;
            let cols = repo::list_columns_in_board(pool, &id).await.map_err(map_db)?;
            let cards = repo::list_cards_in_board(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(json!({ "columns": cols, "cards": cards })))
        }
        "cards_create" => {
            let column_id = req_str("columnId")?;
            let title = req_str("title")?;
            let description = str_arg("description").unwrap_or_default();
            let priority = str_arg("priority");
            let tags_json = serde_json::to_string(&str_array("tags")).unwrap_or("[]".into());
            let coworker_id = str_arg("coworkerId");
            let parent_card_id = str_arg("parentCardId");
            // Place at end of the destination column.
            let existing_in_col: Vec<_> = repo::list_cards_in_board(pool, &column_id)
                .await.unwrap_or_default()
                .into_iter().filter(|c| c.column_id == column_id).collect();
            let pos = existing_in_col.len() as i32;
            let id = new_id();
            repo::insert_card(
                pool, &id, &column_id, &title, &description,
                priority.as_deref(), &tags_json, pos,
                None, None, None,
                parent_card_id.as_deref(),
                coworker_id.as_deref(),
                actor, &now,
            )
            .await.map_err(map_db)?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id })))
        }
        "cards_update" => {
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let row: Option<(String, String, Option<String>, String, Option<String>)> =
                sqlx::query_as(
                    "SELECT title, description, priority, tags, review_checklist \
                     FROM workspace_board_cards WHERE id = ?",
                )
                .bind(&id)
                .fetch_optional(pool)
                .await
                .map_err(map_db)?;
            let (cur_title, cur_desc, cur_prio, cur_tags, cur_check) =
                row.ok_or((-32602, "card not found".into()))?;
            let title = str_arg("title").unwrap_or(cur_title);
            let description = str_arg("description").unwrap_or(cur_desc);
            let priority = if args.get("priority").is_some() { str_arg("priority") } else { cur_prio };
            let tags_json = if args.get("tags").is_some() {
                serde_json::to_string(&str_array("tags")).unwrap_or("[]".into())
            } else {
                cur_tags
            };
            let review_checklist = if args.get("reviewChecklist").is_some() {
                str_arg("reviewChecklist")
            } else {
                cur_check
            };
            let coworker_id = str_arg("coworkerId");
            repo::update_card(
                pool, &id, &title, &description, priority.as_deref(),
                &tags_json, review_checklist.as_deref(),
                coworker_id.as_deref(),
                actor, &now,
            )
            .await.map_err(map_db)?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id, "ok": true })))
        }
        "cards_move" => {
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let column_id = req_str("columnId")?;
            let position = args.get("position").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            // Reuse the same review-flag rule the Tauri command applies:
            // an agent moving to a "Review" column triggers pending review.
            let review_pending = {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT name FROM workspace_board_columns WHERE id = ?",
                )
                .bind(&column_id)
                .fetch_optional(pool)
                .await
                .map_err(map_db)?;
                match row {
                    // Same matcher as commands.rs — strict "review"
                    // column (not "in review", which is the active-
                    // work column).
                    Some((name,)) if super::commands::is_review_only_column(&name) => 1,
                    _ => 0,
                }
            };
            repo::move_card(pool, &id, &column_id, position, review_pending, actor, &now)
                .await.map_err(map_db)?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id, "ok": true, "reviewPending": review_pending == 1 })))
        }

        // ---------------------------------------------------------------
        // v1.1 — search, review workflow, summaries, surgical edits,
        // freeze + linking, activity feed.
        // ---------------------------------------------------------------

        "notes_search" => {
            let query = req_str("query")?;
            let workspace_id = str_arg("workspaceId");
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20) as i32;
            let v = repo::search_notes(pool, &query, workspace_id.as_deref(), limit)
                .await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "cards_search" => {
            let query = req_str("query")?;
            let workspace_id = str_arg("workspaceId");
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20) as i32;
            let v = repo::search_cards(pool, &query, workspace_id.as_deref(), limit)
                .await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }

        "cards_approve" => {
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            // Approval comment lands as a regular comment row. The
            // semantics ("this was an approval") are conveyed by the
            // tool name + the review_pending=0 transition, not by
            // markup in the body.
            if let Some(comment) = str_arg("comment") {
                if !comment.trim().is_empty() {
                    let cid = new_id();
                    repo::insert_card_comment(pool, &cid, &id, actor, None, comment.trim(), None, &now)
                        .await.map_err(map_db)?;
                }
            }
            repo::clear_review_pending(pool, &id, actor, &now).await.map_err(map_db)?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id, "ok": true, "reviewPending": false })))
        }
        "cards_request_changes" => {
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let feedback = req_str("feedback")?;
            let trimmed = feedback.trim();
            if !trimmed.is_empty() {
                let cid = new_id();
                repo::insert_card_comment(pool, &cid, &id, actor, None, trimmed, None, &now)
                    .await.map_err(map_db)?;
            }
            // Optional move first (so the column-name review check sees
            // the new column), then clear the pending flag explicitly.
            if let Some(column_id) = str_arg("columnId") {
                let position = args.get("position")
                    .and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                // Move with review_pending=0 — we're explicitly leaving review.
                repo::move_card(pool, &id, &column_id, position, 0, actor, &now)
                    .await.map_err(map_db)?;
            } else {
                repo::clear_review_pending(pool, &id, actor, &now)
                    .await.map_err(map_db)?;
            }
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({ "id": id, "ok": true })))
        }
        "cards_list_pending_review" => {
            let workspace_id = str_arg("workspaceId");
            let v = repo::list_review_pending_cards(pool, workspace_id.as_deref())
                .await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }

        "boards_summary" => {
            let board_id = req_str("boardId")?;
            let columns = repo::count_cards_per_column(pool, &board_id)
                .await.map_err(map_db)?;
            let total: i64 = columns.iter().map(|c| c.count).sum();
            let review_pending = repo::count_review_pending_in_board(pool, &board_id)
                .await.map_err(map_db)?;
            Ok(ok_text(json!({
                "boardId": board_id,
                "totalCards": total,
                "reviewPending": review_pending,
                "columns": columns,
            })))
        }
        "workspace_summary" => {
            let workspace_id = req_str("workspaceId")?;
            let workspace = repo::get_workspace_by_id(pool, &workspace_id)
                .await.map_err(map_db)?;
            let boards = repo::list_boards_in_workspace(pool, &workspace_id)
                .await.map_err(map_db)?;
            let notes = repo::count_notes_in_workspace(pool, &workspace_id)
                .await.map_err(map_db)?;
            let cards = repo::count_cards_in_workspace(pool, &workspace_id)
                .await.map_err(map_db)?;
            let actors = repo::count_recent_edits_by_actor(pool, &workspace_id)
                .await.map_err(map_db)?;
            let mut review_pending = 0i64;
            for b in &boards {
                review_pending += repo::count_review_pending_in_board(pool, &b.id)
                    .await.map_err(map_db)?;
            }
            Ok(ok_text(json!({
                "workspace": workspace,
                "noteCount": notes,
                "cardCount": cards,
                "boardCount": boards.len(),
                "reviewPending": review_pending,
                "actors": actors,
                "boards": boards,
            })))
        }

        "notes_append_section" => {
            let id = req_str("id")?;
            if repo::is_note_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Note is frozen".into()));
            }
            let heading = req_str("heading")?;
            let body = req_str("content")?;
            let cur = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            let section = format!("## {}\n\n{}", heading.trim(), body);
            let merged = if cur.content.trim().is_empty() {
                section
            } else {
                format!("{}\n\n{}", cur.content.trim_end(), section)
            };
            repo::update_note(pool, &id, &cur.title, &merged, &cur.tags,
                cur.linked_session_id.as_deref(), actor, &now)
                .await.map_err(map_db)?;
            auto_link_note_to_recent_session(pool, &id, actor, &now).await;
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "notes_apply_diff" => {
            let id = req_str("id")?;
            if repo::is_note_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Note is frozen".into()));
            }
            let find = req_str("find")?;
            let replace = req_str("replace")?;
            let cur = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            // Enforce uniqueness — same rule as Edit tool — so the agent
            // can't silently rewrite the wrong section.
            let occurrences = cur.content.matches(&find).count();
            if occurrences == 0 {
                return Err((-32000, "find string not present in note".into()));
            }
            if occurrences > 1 {
                return Err((-32000, format!(
                    "find string is not unique ({} matches) — pass more surrounding context",
                    occurrences,
                )));
            }
            let next = cur.content.replacen(&find, &replace, 1);
            repo::update_note(pool, &id, &cur.title, &next, &cur.tags,
                cur.linked_session_id.as_deref(), actor, &now)
                .await.map_err(map_db)?;
            auto_link_note_to_recent_session(pool, &id, actor, &now).await;
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }

        "cards_create_from_branch" => {
            let branch = req_str("branchName")?;
            // Pull the leaf segment and humanise: "feature/add-user-login"
            // → "Add user login". The full branch goes into externalId so
            // future tooling can correlate.
            let leaf = branch.rsplit('/').next().unwrap_or(&branch);
            let title = {
                let words: Vec<String> = leaf
                    .split(|c: char| c == '-' || c == '_')
                    .filter(|s| !s.is_empty())
                    .enumerate()
                    .map(|(i, w)| {
                        if i == 0 {
                            let mut chars = w.chars();
                            match chars.next() {
                                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                                None => String::new(),
                            }
                        } else {
                            w.to_string()
                        }
                    })
                    .collect();
                let joined = words.join(" ");
                if joined.is_empty() { branch.clone() } else { joined }
            };
            let description = str_arg("description").unwrap_or_default();
            let column_id = if let Some(col) = str_arg("columnId") {
                col
            } else {
                // Need a project path to find a default column.
                let project_path = str_arg("projectPath").ok_or((
                    -32602,
                    "columnId or projectPath required".into(),
                ))?;
                let ws = upsert_workspace_for_project(pool, &project_path, actor).await?;
                let boards = repo::list_boards_in_workspace(pool, &ws.id)
                    .await.map_err(map_db)?;
                let board = boards.into_iter().next().ok_or((
                    -32603,
                    "workspace has no boards".into(),
                ))?;
                let cols = repo::list_columns_in_board(pool, &board.id)
                    .await.map_err(map_db)?;
                // Prefer a "Todo" column, fall back to the first column.
                let chosen = cols.iter()
                    .find(|c| c.name.eq_ignore_ascii_case("todo"))
                    .or_else(|| cols.first())
                    .ok_or((-32603, "board has no columns".into()))?;
                chosen.id.clone()
            };
            let id = new_id();
            // Position at end of column.
            let existing: Vec<_> = sqlx::query_as::<_, (i64,)>(
                "SELECT COUNT(*) FROM workspace_board_cards WHERE column_id = ?",
            )
            .bind(&column_id)
            .fetch_all(pool)
            .await
            .map_err(map_db)?;
            let pos = existing.first().map(|r| r.0).unwrap_or(0) as i32;
            repo::insert_card(
                pool, &id, &column_id, &title, &description,
                None, "[]", pos,
                Some(&branch), None, None,
                None, // parent_card_id — branch-created cards are top-level
                None, // coworker_id — branches don't carry persona context
                actor, &now,
            )
            .await.map_err(map_db)?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({
                "id": id,
                "title": title,
                "branch": branch,
                "columnId": column_id,
            })))
        }

        "notes_link_to_session" => {
            let id = req_str("id")?;
            if repo::is_note_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Note is frozen".into()));
            }
            let session_id = str_arg("sessionId");
            repo::update_note_linked_session(pool, &id, session_id.as_deref(), actor, &now)
                .await.map_err(map_db)?;
            let v = repo::get_note_by_id(pool, &id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }
        "cards_link_to_session" => {
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let session_id = str_arg("sessionId");
            repo::update_card_linked_session(pool, &id, session_id.as_deref(), actor, &now)
                .await.map_err(map_db)?;
            Ok(ok_text(json!({ "id": id, "ok": true })))
        }

        "activity_feed" => {
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(50) as i32;
            // Pull a generous window from the DB then filter in-memory —
            // keeps the SQL trivial; the result set is small (Inbox-class).
            let raw_limit = (limit as usize).saturating_mul(4).max(50) as i32;
            let mut rows = repo::list_inbox(pool, raw_limit).await.map_err(map_db)?;
            if let Some(since) = str_arg("since") {
                rows.retain(|r| r.updated_at.as_str() > since.as_str());
            }
            if let Some(want) = str_arg("actor") {
                rows.retain(|r| r.updated_by == want);
            }
            rows.truncate(limit as usize);
            Ok(ok_text(serde_json::to_value(rows).unwrap_or(Value::Null)))
        }

        "notes_freeze" => {
            let id = req_str("id")?;
            repo::set_note_frozen(pool, &id, 1).await.map_err(map_db)?;
            Ok(ok_text(json!({ "id": id, "frozen": true })))
        }
        "notes_unfreeze" => {
            let id = req_str("id")?;
            repo::set_note_frozen(pool, &id, 0).await.map_err(map_db)?;
            Ok(ok_text(json!({ "id": id, "frozen": false })))
        }
        "cards_freeze" => {
            let id = req_str("id")?;
            repo::set_card_frozen(pool, &id, 1).await.map_err(map_db)?;
            Ok(ok_text(json!({ "id": id, "frozen": true })))
        }
        "cards_unfreeze" => {
            let id = req_str("id")?;
            repo::set_card_frozen(pool, &id, 0).await.map_err(map_db)?;
            Ok(ok_text(json!({ "id": id, "frozen": false })))
        }

        "workspace_link_to_repo" => {
            let workspace_id = req_str("workspaceId")?;
            let repo_url = str_arg("repoUrl");
            repo::set_workspace_repo_url(pool, &workspace_id, repo_url.as_deref(), actor, &now)
                .await.map_err(map_db)?;
            let ws = repo::get_workspace_by_id(pool, &workspace_id).await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(ws).unwrap_or(Value::Null)))
        }

        "cards_add_comment" => {
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let body = req_str("body")?;
            let trimmed = body.trim();
            if trimmed.is_empty() {
                return Err((-32602, "body must not be empty".into()));
            }
            let coworker_id = str_arg("coworkerId");
            let comment_id = new_id();
            repo::insert_card_comment(
                pool, &comment_id, &id, actor,
                coworker_id.as_deref(),
                trimmed, None, &now,
            )
            .await.map_err(map_db)?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(json!({
                "id": comment_id,
                "cardId": id,
                "actor": actor,
                "coworkerId": coworker_id,
                "body": trimmed,
                "createdAt": now,
            })))
        }

        "cards_push_to_repo" => {
            let id = req_str("id")?;
            // Frozen → no push (would mutate externalId/url).
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let result = crate::modes::workspace::push::push_card_to_repo(pool, &id, actor)
                .await
                .map_err(|e| (-32603, e))?;
            auto_link_card_to_recent_session(pool, &id, actor, &now).await;
            Ok(ok_text(result))
        }

        "cards_commit" => {
            let id = req_str("cardId")?;
            let message = req_str("message")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let result = crate::modes::workspace::pr::commit_card(pool, app, &id, &message, actor)
                .await
                .map_err(|e| (-32603, e.message()))?;
            Ok(ok_text(serde_json::to_value(result).unwrap_or(Value::Null)))
        }

        "cards_raise_pr" => {
            let id = req_str("cardId")?;
            let title = str_arg("title");
            let body = str_arg("body");
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let result = crate::modes::workspace::pr::raise_or_update_pr(
                pool, app, &id, title.as_deref(), body.as_deref(), actor,
            )
            .await
            .map_err(|e| (-32603, e.message()))?;
            Ok(ok_text(serde_json::to_value(result).unwrap_or(Value::Null)))
        }

        "cards_link_pr" => {
            let id = req_str("cardId")?;
            let pr_url = req_str("prUrl")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            crate::modes::workspace::pr::link_pr_url(pool, app, &id, &pr_url, actor)
                .await
                .map_err(|e| (-32603, e.message()))?;
            Ok(ok_text(json!({ "cardId": id, "prUrl": pr_url })))
        }

        "coworkers_list" => {
            let v = crate::shared::repos::coworkers::list_coworkers(pool)
                .await.map_err(map_db)?;
            Ok(ok_text(serde_json::to_value(v).unwrap_or(Value::Null)))
        }

        "cards_claim" => {
            // Terminal-side claim. We don't get the agent's session id
            // over the wire, so we use the same auto-link heuristic to
            // resolve "the most recent session for this card's project"
            // and stamp it as the claimer. Errors when:
            //   • Card not found / no project_path bound
            //   • Card already claimed by a *different* session
            //   • No matching agent session exists yet (agent must
            //     have been launched in Agent mode at least once)
            let id = req_str("id")?;
            let row = repo::get_card_claim_and_project(pool, &id)
                .await.map_err(map_db)?
                .ok_or((-32602, "Card not found".into()))?;
            let (existing_session, _existing_coworker, project_path_opt) = row;
            let project_path = project_path_opt
                .filter(|p| !p.trim().is_empty())
                .ok_or((-32602, "Workspace has no project_path bound".into()))?;
            let calling_session = session_repo::find_recent_session_for_project(pool, &project_path)
                .await.map_err(map_db)?
                .ok_or((-32603, "No agent session found for this project — start one in Agent mode first".into()))?;
            if let Some(existing) = existing_session.as_deref() {
                if existing != calling_session.id {
                    return Err((-32000, format!(
                        "Card is already claimed by session '{}'. Release it first.",
                        calling_session.title
                    )));
                }
            } else {
                // Terminal claims have no persona — pass None for coworker_id.
                repo::claim_card(pool, &id, &calling_session.id, None, actor, &now)
                    .await.map_err(map_db)?;
            }
            Ok(ok_text(json!({
                "id": id,
                "claimedSessionId": calling_session.id,
                "sessionTitle": calling_session.title,
            })))
        }

        "cards_release" => {
            let id = req_str("id")?;
            let row = repo::get_card_claim_and_project(pool, &id)
                .await.map_err(map_db)?
                .ok_or((-32602, "Card not found".into()))?;
            let (existing_session, _existing_coworker, project_path_opt) = row;
            let project_path = project_path_opt.unwrap_or_default();
            let calling_session = session_repo::find_recent_session_for_project(pool, &project_path)
                .await.map_err(map_db)?;
            // Only release if the caller is the holder — otherwise it's a no-op
            // so a misfire from another session can't inadvertently unlock the card.
            if let (Some(holder), Some(caller)) = (existing_session.as_deref(), calling_session.as_ref()) {
                if holder == caller.id {
                    repo::release_card(pool, &id, actor, &now)
                        .await.map_err(map_db)?;
                }
            }
            Ok(ok_text(json!({ "id": id, "ok": true })))
        }

        "cards_start_work" => {
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let r = crate::modes::workspace::agent_spawn::start_work(pool, &id, actor)
                .await
                .map_err(|e| (-32603, e))?;
            Ok(ok_text(json!({
                "ok": true,
                "worktreePath": r.worktree_path,
                "worktreeBranch": r.worktree_branch,
            })))
        }

        "cards_call_coworker" => {
            // Terminal-callable wrapper around drawer_chat_turn — lets the
            // calling agent (e.g. Claude in the user's terminal) invoke a
            // specific persona on a card. The persona's reply is persisted
            // as a comment AND returned in the result so the caller can
            // summarise back to the user.
            let id = req_str("id")?;
            if repo::is_card_frozen(pool, &id).await.map_err(map_db)? {
                return Err((-32000, "Card is frozen".into()));
            }
            let coworker_id = req_str("coworkerId")?;
            let message = req_str("message")?;
            // Pass the Tauri AppHandle through (when available) so any
            // open drawer for this card refreshes live via the
            // `workspace:card-updated` event. Falls back to next-poll
            // refresh when MCP started before Tauri was ready.
            let result = crate::modes::workspace::agent_spawn::drawer_chat_turn(
                pool, app, &id, &coworker_id, &message, actor,
            )
            .await
            .map_err(|e| (-32603, e))?;
            Ok(ok_text(json!({
                "ok": true,
                "sessionId": result.session_id,
                "userCommentId": result.user_comment.id,
                "replyCommentId": result.reply_comment.as_ref().map(|c| c.id.clone()),
                "response": result.reply_comment.as_ref().map(|c| c.body.clone()),
                "agentError": result.agent_error,
            })))
        }

        _ => Err((-32601, format!("Tool not found: {}", name))),
    }
}

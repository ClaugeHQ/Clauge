// Workspace mode — owns CRUD for Workspaces (containers), Notes
// (markdown pages), and Boards (Kanban with columns + cards).
//
// `commands` hosts `#[tauri::command]` handlers; `models` carries the
// shared data types. All persistence funnels through
// `crate::shared::repos::workspaces`. Agent integration is the MCP
// server in `mcp/` — workspace mode does NOT register tools into the
// shared in-app AI dispatch (the AIPanel is gated out for this mode
// in the frontend).

pub mod agent_spawn;
pub mod cli_errors;
pub mod commands;
pub mod mcp;
pub mod meetings;
pub mod models;
pub mod pr;
pub mod push;

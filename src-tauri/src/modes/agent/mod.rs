// Agent mode — owns Claude Code session CRUD, the `claude` CLI PTY runner,
// agent-side git/worktree helpers, plugin management, and usage analytics.
//
// `commands`, `terminal`, `git`, `worktree`, `plugins`, and `usage` host
// `#[tauri::command]` handlers; `models` carries the shared session,
// terminal, and usage structs. lib.rs references handlers as
// `crate::modes::agent::<file>::*` and the terminal state as
// `crate::modes::agent::models::TerminalState`.
//
// The hardcoded `claude` CLI references in `terminal`, `plugins`, and
// `usage` stay intact for now; a later wave will introduce a `CliRunner`
// trait that abstracts the binary path and arguments.

pub mod commands;
pub mod git;
pub mod hooks;
pub mod models;
pub mod plugins;
pub mod terminal;
pub mod usage;
pub mod worktree;

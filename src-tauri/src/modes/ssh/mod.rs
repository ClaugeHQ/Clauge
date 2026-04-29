// SSH mode — owns the russh-backed terminal session, profile CRUD, and
// the Keychain-stored credential lookups exposed to the frontend.
//
// `models` is the shared in-process state + serde structs.
// `profiles` and `terminal` host the `#[tauri::command]` handlers; lib.rs
// references them as `crate::modes::ssh::profiles::*` / `::terminal::*`.
// `ai_tools` registers SSH's `execute_shell` AI tool with the shared
// `crate::shared::ai::dispatch` registry at startup.

pub mod ai_tools;
pub mod models;
pub mod profiles;
pub mod terminal;
pub mod tunnel;

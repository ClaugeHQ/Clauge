// SSH mode — owns the russh-backed terminal session, profile CRUD, and
// the Keychain-stored credential lookups exposed to the frontend.
//
// `models` is the shared in-process state + serde structs.
// `profiles` and `terminal` host the `#[tauri::command]` handlers; lib.rs
// references them as `crate::modes::ssh::profiles::*` / `::terminal::*`.

pub mod models;
pub mod profiles;
pub mod terminal;

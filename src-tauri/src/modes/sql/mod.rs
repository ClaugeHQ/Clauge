// SQL mode — owns the runtime user-DB connection pools (Postgres / MySQL /
// SQLite) for queries against user-supplied databases, the saved-connection
// and SQL-script CRUD that targets app SQLite, and the AI tool
// implementations that drive query execution from the chat surface.
//
// `client` hosts the `#[tauri::command]` handlers and the shared
// `SqlConnectionManager` state; lib.rs references them as
// `crate::modes::sql::client::*`.
// `ai_tools` mixes app-SQLite reads with sqlx driver calls (Postgres / MySQL
// / SQLite) against the active user connection, and is invoked from
// `commands::ai::tools` via `crate::modes::sql::ai_tools::*`.

pub mod client;
pub mod ai_tools;
pub mod dialects;

//! SQLite connection pool initialization for the Clauge app database.
//!
//! Lives in `app_data_dir/clauge.db`, with `foreign_keys = ON` enforced
//! per-connection (SQLite default is OFF). The pool is created once during
//! `setup()` and managed via Tauri state for `#[tauri::command]` access.

use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

/// Maximum concurrent connections to the SQLite database.
///
/// SQLite serialises writes regardless of pool size, but multiple read
/// connections can run concurrently in WAL mode. 5 covers the typical
/// frontend-driven query bursts (multiple list_* commands firing at once
/// from `loadX()` calls in the Svelte stores).
const MAX_CONNECTIONS: u32 = 5;

/// Open the Clauge SQLite pool, creating the file if missing.
///
/// `app_data_dir` is `~/Library/Application Support/com.clauge.desktop/`
/// on macOS and the per-platform equivalent elsewhere — Tauri provides it
/// via `app.path().app_data_dir()`.
pub async fn init(app_data_dir: &Path) -> Result<SqlitePool, String> {
    std::fs::create_dir_all(app_data_dir)
        .map_err(|e| format!("create app data dir: {}", e))?;

    let db_path = app_data_dir.join("clauge.db");
    let url = format!("sqlite:{}?mode=rwc", db_path.display());

    let opts = SqliteConnectOptions::from_str(&url)
        .map_err(|e| format!("invalid db url {}: {}", url, e))?
        .pragma("foreign_keys", "ON")
        .create_if_missing(true);

    SqlitePoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .connect_with(opts)
        .await
        .map_err(|e| format!("connect to {}: {}", db_path.display(), e))
}

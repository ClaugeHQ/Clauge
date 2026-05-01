//! One-time import of pre-database Clauge data into the SQLite store.
//!
//! Older Clauge builds stored agent sessions and contexts as JSON / Markdown
//! files under `~/.clauge/`:
//!   - `~/.clauge/sessions.json`     — array of agent session profiles
//!   - `~/.clauge/contexts/*.md`     — context snippets, filename = name
//!   - `~/.clauge/session_key`       — Anthropic session key for usage stats
//!
//! After the SQLite store landed (migration v4), this module bridges the
//! gap: on first launch of the new code, it imports the JSON/MD files into
//! `agent_sessions` / `agent_contexts` / `agent_session_contexts`, copies
//! the session key into `settings`, and archives the originals to
//! `~/.clauge/backup/` so a re-run can't double-import.
//!
//! Idempotent via the `clauge_migration_done` row in `settings`. Safe to
//! call on every boot; runs the actual work at most once per machine.

use std::collections::HashMap;
use std::path::Path;

use sqlx::sqlite::SqlitePool;

const DONE_KEY: &str = "clauge_migration_done";

/// Run the import if a `~/.clauge/sessions.json` exists and we haven't
/// already imported it on this database. Failures inside any step are
/// best-effort — partial imports are preferable to refusing to launch.
pub async fn run_if_needed(pool: &SqlitePool) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let clauge_dir = home.join(".clauge");
    let sessions_json = clauge_dir.join("sessions.json");

    if !sessions_json.exists() {
        return;
    }
    if already_done(pool).await {
        return;
    }

    // Step 1: contexts first so we have IDs to link sessions against.
    let contexts_dir = clauge_dir.join("contexts");
    let context_name_to_id = import_contexts(pool, &contexts_dir).await;

    // Step 2: sessions + their context attachments.
    import_sessions(pool, &sessions_json, &context_name_to_id).await;

    // Step 3: session key.
    let key_path = clauge_dir.join("session_key");
    import_session_key(pool, &key_path).await;

    // Step 4: mark done so subsequent boots short-circuit.
    let _ = sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, 'true')")
        .bind(DONE_KEY)
        .execute(pool)
        .await;

    // Step 5: archive originals.
    archive_legacy_files(&clauge_dir, &sessions_json, &key_path, &contexts_dir);
}

async fn already_done(pool: &SqlitePool) -> bool {
    let val: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(DONE_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    val.is_some()
}

/// Returns `name → context_id` for the imported context snippets.
async fn import_contexts(pool: &SqlitePool, contexts_dir: &Path) -> HashMap<String, String> {
    let mut name_to_id: HashMap<String, String> = HashMap::new();
    if !contexts_dir.exists() {
        return name_to_id;
    }
    let Ok(entries) = std::fs::read_dir(contexts_dir) else {
        return name_to_id;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        if name.is_empty() || content.is_empty() {
            continue;
        }
        let ctx_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO agent_contexts (id, name, content, created_at, updated_at) \
             VALUES (?,?,?,?,?)",
        )
        .bind(&ctx_id)
        .bind(&name)
        .bind(&content)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .is_ok();
        if inserted {
            name_to_id.insert(name, ctx_id);
        }
    }
    name_to_id
}

async fn import_sessions(
    pool: &SqlitePool,
    sessions_json: &Path,
    context_name_to_id: &HashMap<String, String>,
) {
    let Ok(content) = std::fs::read_to_string(sessions_json) else {
        return;
    };
    let Ok(store) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    let Some(profiles) = store.get("profiles").and_then(|v| v.as_array()) else {
        return;
    };

    for p in profiles {
        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO agent_sessions \
                (id, title, purpose, project_path, project_name, claude_session_id, \
                 context_prompt, worktree_path, worktree_branch, skip_permissions, \
                 git_name, git_email, created_at, last_used_at) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(p.get("title").and_then(|v| v.as_str()).unwrap_or(""))
        .bind(p.get("purpose").and_then(|v| v.as_str()).unwrap_or("Custom"))
        .bind(p.get("projectPath").and_then(|v| v.as_str()).unwrap_or(""))
        .bind(p.get("projectName").and_then(|v| v.as_str()).unwrap_or(""))
        .bind(p.get("claudeSessionId").and_then(|v| v.as_str()))
        .bind(p.get("contextPrompt").and_then(|v| v.as_str()).unwrap_or(""))
        .bind(p.get("worktreePath").and_then(|v| v.as_str()))
        .bind(p.get("worktreeBranch").and_then(|v| v.as_str()))
        .bind(
            if p.get("skipPermissions").and_then(|v| v.as_bool()).unwrap_or(false) { 1 } else { 0 },
        )
        .bind(p.get("gitName").and_then(|v| v.as_str()))
        .bind(p.get("gitEmail").and_then(|v| v.as_str()))
        .bind(p.get("createdAt").and_then(|v| v.as_str()).unwrap_or(""))
        .bind(p.get("lastUsedAt").and_then(|v| v.as_str()).unwrap_or(""))
        .execute(pool)
        .await;

        if inserted.is_err() {
            continue;
        }

        // Link attached contexts via junction table.
        if let Some(ctx_names) = p.get("contexts").and_then(|v| v.as_array()) {
            for ctx_name in ctx_names {
                if let Some(name_str) = ctx_name.as_str() {
                    if let Some(ctx_id) = context_name_to_id.get(name_str) {
                        let _ = sqlx::query(
                            "INSERT OR IGNORE INTO agent_session_contexts \
                             (session_id, context_id) VALUES (?,?)",
                        )
                        .bind(id)
                        .bind(ctx_id)
                        .execute(pool)
                        .await;
                    }
                }
            }
        }
    }
}

async fn import_session_key(pool: &SqlitePool, key_path: &Path) {
    if !key_path.exists() {
        return;
    }
    let Ok(key) = std::fs::read_to_string(key_path) else {
        return;
    };
    let key = key.trim();
    if key.is_empty() {
        return;
    }
    let _ = sqlx::query(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('agent_session_key', ?)",
    )
    .bind(key)
    .execute(pool)
    .await;
}

fn archive_legacy_files(
    clauge_dir: &Path,
    sessions_json: &Path,
    key_path: &Path,
    contexts_dir: &Path,
) {
    let backup = clauge_dir.join("backup");
    let _ = std::fs::create_dir_all(&backup);
    let _ = std::fs::rename(sessions_json, backup.join("sessions.json"));
    if key_path.exists() {
        let _ = std::fs::rename(key_path, backup.join("session_key"));
    }
    if contexts_dir.exists() {
        let _ = std::fs::rename(contexts_dir, backup.join("contexts"));
    }
}

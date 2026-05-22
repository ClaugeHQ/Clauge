use std::sync::Arc;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::shared::ai::types::ChatContext;
use crate::modes::sql::client::SqlConnectionManager;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SqlErrKind {
    Transient,
    SchemaColumn,
    SchemaTable,
    Permission,
    Syntax,
    ConnectionDropped,
    Other,
}

fn classify_sql_error(raw: &str) -> SqlErrKind {
    let lower = raw.to_lowercase();
    let transient_signals = [
        "retry in a moment",
        "try again",
        "internal error",
        "timeout",
        "timed out",
        "deadlock detected",
        "temporary failure",
        "503",
        "502",
        "504",
        "too many connections",
        "service unavailable",
    ];
    if transient_signals.iter().any(|s| lower.contains(s)) {
        return SqlErrKind::Transient;
    }
    let dne = lower.contains("does not exist") || lower.contains("doesn't exist") || lower.contains("no such");
    if dne && lower.contains("column") { return SqlErrKind::SchemaColumn; }
    if dne && (lower.contains("relation") || lower.contains("table")) { return SqlErrKind::SchemaTable; }
    if lower.contains("permission denied") || lower.contains("not authorized") || lower.contains("access denied") {
        return SqlErrKind::Permission;
    }
    if lower.contains("syntax error") { return SqlErrKind::Syntax; }
    if lower.contains("connection")
        && (lower.contains("refused") || lower.contains("closed") || lower.contains("reset") || lower.contains("broken pipe"))
    {
        return SqlErrKind::ConnectionDropped;
    }
    SqlErrKind::Other
}

/// Best-effort fuzzy match: returns at most 5 candidates ordered by a
/// crude similarity score. Used to suggest "did you mean" alternatives
/// without pulling in a Levenshtein crate.
fn similar_names(target: &str, candidates: &[String], limit: usize) -> Vec<String> {
    let t = target.to_lowercase();
    let mut scored: Vec<(i32, &String)> = candidates.iter().map(|c| {
        let cl = c.to_lowercase();
        let mut score = 0;
        if cl == t { score += 1000; }
        if cl.contains(&t) || t.contains(&cl) { score += 200; }
        let common: usize = t.chars().filter(|ch| cl.contains(*ch)).count();
        score += (common as i32) * 5;
        if cl.starts_with(t.chars().next().unwrap_or('_')) { score += 30; }
        (score, c)
    }).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().filter(|(s, _)| *s > 0).take(limit).map(|(_, c)| c.clone()).collect()
}

/// Extract identifier targets following FROM/JOIN/INTO/UPDATE in a SQL string.
/// Strips quoting and schema prefix. Imperfect but covers the common shapes.
fn extract_query_tables(query: &str) -> Vec<String> {
    let bytes = query.as_bytes();
    let upper: Vec<u8> = bytes.iter().map(|b| b.to_ascii_uppercase()).collect();
    let keywords: &[&[u8]] = &[b"FROM ", b"JOIN ", b"INTO ", b"UPDATE "];
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < upper.len() {
        let mut matched: Option<usize> = None;
        for kw in keywords {
            if upper[i..].starts_with(kw) { matched = Some(kw.len()); break; }
        }
        if let Some(kw_len) = matched {
            let mut j = i + kw_len;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n') { j += 1; }
            while j < bytes.len() && matches!(bytes[j], b'"' | b'`' | b'\'' | b'[') { j += 1; }
            let start = j;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'.') {
                j += 1;
            }
            if j > start {
                let raw_name = std::str::from_utf8(&bytes[start..j]).unwrap_or("");
                let name = raw_name.rsplit_once('.').map(|(_, n)| n.to_string()).unwrap_or_else(|| raw_name.to_string());
                if !name.is_empty() && !out.contains(&name) { out.push(name); }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

async fn fetch_table_list(
    pool_entry: &crate::modes::sql::client::DatabasePool,
    database: &str,
    app_pool: &SqlitePool,
) -> Vec<String> {
    use crate::modes::sql::client::DatabasePool;
    let limit = crate::shared::repos::settings::get_u64_or(app_pool, "sql_table_list_limit", 200).await;
    match pool_entry {
        DatabasePool::Postgres(p) => {
            let q = format!("SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY table_name LIMIT {}", limit);
            sqlx::query_scalar::<_, String>(&q).fetch_all(p).await.unwrap_or_default()
        }
        DatabasePool::MySql(p) => {
            if database.is_empty() {
                let q = format!("SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name LIMIT {}", limit);
                sqlx::query_scalar::<_, String>(&q).fetch_all(p).await.unwrap_or_default()
            } else {
                let q = format!("SELECT table_name FROM information_schema.tables WHERE table_schema = ? ORDER BY table_name LIMIT {}", limit);
                sqlx::query_scalar::<_, String>(&q).bind(database).fetch_all(p).await.unwrap_or_default()
            }
        }
        DatabasePool::Sqlite(p) => {
            let q = format!("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name LIMIT {}", limit);
            sqlx::query_scalar::<_, String>(&q).fetch_all(p).await.unwrap_or_default()
        }
        DatabasePool::Clickhouse(c) => {
            let q = format!("SELECT name FROM system.tables WHERE database = currentDatabase() ORDER BY name LIMIT {}", limit);
            match c.query(&q).await {
                Ok(r) => r.rows.into_iter().filter_map(|row| row.first().and_then(|v| v.as_str().map(|s| s.to_string()))).collect(),
                Err(_) => Vec::new(),
            }
        }
        DatabasePool::D1(c) => {
            let q = format!("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name LIMIT {}", limit);
            match c.query(&q).await {
                Ok(r) => r.rows.into_iter().filter_map(|row| row.first().and_then(|v| v.as_str().map(|s| s.to_string()))).collect(),
                Err(_) => Vec::new(),
            }
        }
    }
}

async fn fetch_columns_for_table(
    pool_entry: &crate::modes::sql::client::DatabasePool,
    database: &str,
    table: &str,
) -> Vec<String> {
    use crate::modes::sql::client::DatabasePool;
    match pool_entry {
        DatabasePool::Postgres(p) => {
            sqlx::query_scalar::<_, String>(
                "SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1 ORDER BY ordinal_position"
            ).bind(table).fetch_all(p).await.unwrap_or_default()
        }
        DatabasePool::MySql(p) => {
            if database.is_empty() {
                sqlx::query_scalar::<_, String>(
                    "SELECT column_name FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ? ORDER BY ordinal_position"
                ).bind(table).fetch_all(p).await.unwrap_or_default()
            } else {
                sqlx::query_scalar::<_, String>(
                    "SELECT column_name FROM information_schema.columns WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position"
                ).bind(database).bind(table).fetch_all(p).await.unwrap_or_default()
            }
        }
        DatabasePool::Sqlite(p) => {
            use sqlx::Row;
            sqlx::query(&format!("PRAGMA table_info({})", table))
                .fetch_all(p).await
                .map(|rows| rows.iter().filter_map(|r| r.try_get::<String, _>("name").ok()).collect())
                .unwrap_or_default()
        }
        DatabasePool::Clickhouse(c) => {
            let q = format!("SELECT name FROM system.columns WHERE database = currentDatabase() AND table = '{}' ORDER BY position", table.replace('\'', "''"));
            match c.query(&q).await {
                Ok(r) => r.rows.into_iter().filter_map(|row| row.first().and_then(|v| v.as_str().map(|s| s.to_string()))).collect(),
                Err(_) => Vec::new(),
            }
        }
        DatabasePool::D1(c) => {
            let q = format!("PRAGMA table_info({})", table);
            match c.query(&q).await {
                Ok(r) => {
                    let name_idx = r.columns.iter().position(|n| n.eq_ignore_ascii_case("name"));
                    if let Some(idx) = name_idx {
                        r.rows.into_iter().filter_map(|row| row.get(idx).and_then(|v| v.as_str().map(|s| s.to_string()))).collect()
                    } else { Vec::new() }
                }
                Err(_) => Vec::new(),
            }
        }
    }
}

/// Pull the offending identifier ("name", "users") out of a database error,
/// best effort across common driver formats.
fn extract_offending_name(raw: &str) -> Option<String> {
    let pats: &[&str] = &[
        "column \"", "column '", "column `",
        "relation \"", "relation '",
        "table \"", "table '",
        "no such column: ", "no such table: ",
        "Unknown column '", "Unknown table '",
        "Missing columns: '",
    ];
    for p in pats {
        if let Some(start) = raw.find(p) {
            let after = &raw[start + p.len()..];
            let end = after.find(|c: char| c == '"' || c == '\'' || c == '`' || c == ' ' || c == ',' || c == ')').unwrap_or(after.len());
            let name = after[..end].trim().to_string();
            if !name.is_empty() { return Some(name); }
        }
    }
    None
}

/// Run schema discovery on a column/table error and produce a hint block
/// the model can use to either fix the query or ask a clarifying question.
async fn build_schema_hint(
    raw_err: &str,
    kind: SqlErrKind,
    pool_entry: &crate::modes::sql::client::DatabasePool,
    database: &str,
    query: &str,
    app_pool: &SqlitePool,
) -> String {
    let mut out = String::new();
    let offender = extract_offending_name(raw_err);
    let query_tables = extract_query_tables(query);
    let all_tables = fetch_table_list(pool_entry, database, app_pool).await;

    if kind == SqlErrKind::SchemaTable {
        if !all_tables.is_empty() {
            out.push_str("\n\nAvailable tables in this database: ");
            out.push_str(&all_tables.join(", "));
        }
        if let Some(name) = &offender {
            let suggestions = similar_names(name, &all_tables, 3);
            if !suggestions.is_empty() {
                out.push_str(&format!("\nClosest matches to '{}': {}", name, suggestions.join(", ")));
            }
        }
    } else if kind == SqlErrKind::SchemaColumn {
        let candidate_tables: Vec<String> = if !query_tables.is_empty() {
            query_tables.iter().filter(|t| all_tables.iter().any(|a| a.eq_ignore_ascii_case(t))).cloned().collect()
        } else { Vec::new() };
        for t in &candidate_tables {
            let cols = fetch_columns_for_table(pool_entry, database, t).await;
            if !cols.is_empty() {
                out.push_str(&format!("\n\nColumns of '{}': {}", t, cols.join(", ")));
                if let Some(name) = &offender {
                    let suggestions = similar_names(name, &cols, 3);
                    if !suggestions.is_empty() {
                        out.push_str(&format!("\nClosest matches to '{}' in '{}': {}", name, t, suggestions.join(", ")));
                    }
                }
            }
        }
        if candidate_tables.is_empty() && !all_tables.is_empty() {
            out.push_str("\n\nAvailable tables in this database: ");
            out.push_str(&all_tables.join(", "));
        }
    }
    out
}

/// Rewrites a sqlx/ClickHouse/D1 error into a message that tells the model
/// exactly what to do next, by category. Transient errors are not reached
/// here unless retries have already been exhausted in execute_query.
fn diagnose_query_error(raw: &str) -> String {
    match classify_sql_error(raw) {
        SqlErrKind::Transient => format!(
            "Database still unreachable after 4 automatic retries (~9s of waiting with exponential backoff). Driver said:\n```\n{}\n```\nThis pattern usually means a Cloudflare D1 cold-start or a brief regional outage — the database itself isn't broken. Tell the user to retry in 15–30 seconds. Show the raw error verbatim. Do not call execute_query again immediately.",
            raw
        ),
        SqlErrKind::SchemaColumn => format!(
            "Query failed. Database said:\n```\n{}\n```\nNEXT ACTION (required): call describe_table on the referenced table, find the real column name, and rewrite the query. Do not retry the same query. Quote the raw error above to the user verbatim.",
            raw
        ),
        SqlErrKind::SchemaTable => format!(
            "Query failed. Database said:\n```\n{}\n```\nNEXT ACTION (required): call list_tables, find the closest real table name, and rewrite the query. Do not retry the same query. Quote the raw error above to the user verbatim.",
            raw
        ),
        SqlErrKind::Permission => format!(
            "Query failed. Database said:\n```\n{}\n```\nTell the user their database role lacks the required permission. Do not retry. Quote the raw error verbatim.",
            raw
        ),
        SqlErrKind::Syntax => format!(
            "Query failed. Database said:\n```\n{}\n```\nRewrite the query fixing the syntax issue highlighted above. Quote the raw error to the user verbatim before retrying.",
            raw
        ),
        SqlErrKind::ConnectionDropped => format!(
            "Connection failed mid-query. Driver said:\n```\n{}\n```\nTell the user the connection dropped and to reconnect via the SQL Connections panel. Do not retry from your side — the pool needs to be reopened.",
            raw
        ),
        SqlErrKind::Other => format!(
            "Query failed. Database said:\n```\n{}\n```\nQuote the raw error above to the user verbatim. Do not paraphrase as 'internal error'.",
            raw
        ),
    }
}

/// Ensure a database pool exists for the given connection_id + database combo.
/// Extracts saved_connection_id from context env_vars to build stable cache keys.
/// Cache key format: "savedId:dbName" — same as frontend's connectToDatabase.
async fn ensure_pool(
    connection_id: &str,
    database: Option<&str>,
    context: &ChatContext,
    pool: &SqlitePool,
    sql_manager: &Arc<SqlConnectionManager>,
) -> Result<String, String> {
    let saved_from_ctx = context.env_vars.iter()
        .find(|v| v.key == "saved_connection_id")
        .map(|v| v.value.to_string());

    // The AI may send the saved row id ("abc"), a pool key ("abc:mydb"), or
    // something stale. Derive a real saved_id from whichever shape we got,
    // preferring context when present.
    let (input_saved_id, input_pool_key) = match connection_id.split_once(':') {
        Some((s, _)) => (s.to_string(), Some(connection_id.to_string())),
        None => (connection_id.to_string(), None),
    };
    let saved_id = saved_from_ctx.unwrap_or(input_saved_id);

    let db_str = database.map(|s| s.to_string()).filter(|s| !s.is_empty());
    let cache_key_with_db = db_str.as_ref().map(|db| format!("{}:{}", saved_id, db));

    log::info!(
        "[AI SQL] ensure_pool input: connection_id={} database={:?} → saved_id={} cache_key={:?} pool_key_hint={:?}",
        connection_id, database, saved_id, cache_key_with_db, input_pool_key
    );

    // Try every reasonable key against the live pool map BEFORE deciding to
    // auto-connect. Order: exact `saved_id:db`, then `connection_id` verbatim
    // (in case the AI passed an already-built pool key), then any pool whose
    // key starts with `saved_id:` (matches any open db for this connection).
    {
        let connections = sql_manager.connections.lock().await;
        let keys: Vec<&String> = connections.keys().collect();
        log::info!("[AI SQL] ensure_pool pool map keys ({}): {:?}", keys.len(), keys);

        if let Some(ck) = &cache_key_with_db {
            if connections.contains_key(ck) {
                return Ok(ck.clone());
            }
        }
        if connections.contains_key(connection_id) {
            return Ok(connection_id.to_string());
        }
        let prefix = format!("{}:", saved_id);
        if let Some(any) = keys.iter().find(|k| k.starts_with(&prefix)) {
            let chosen = (*any).clone();
            log::info!("[AI SQL] ensure_pool prefix-match: {} from {}", chosen, prefix);
            return Ok(chosen);
        }
    }

    // No live pool found — try to auto-connect from saved record.
    let saved = crate::shared::repos::sql_connections::get_by_id_optional(pool, &saved_id)
        .await
        .map_err(|e| format!("DB error looking up saved connection '{}': {}", saved_id, e))?;

    if let Some(saved) = saved {
        let target_db = db_str.clone().unwrap_or_else(|| saved.database_name.clone());
        let final_key = format!("{}:{}", saved.id, target_db);
        let host = saved.host.clone();
        let port = saved.port;
        let config = crate::modes::sql::client::SqlConnectionConfig {
            name: String::new(),
            driver: saved.driver,
            host: saved.host,
            port: saved.port as u16,
            database: target_db.clone(),
            username: saved.username,
            password: saved.password,
            ssl: saved.ssl == 1,
            ssh_profile_id: saved.ssh_profile_id,
        };

        let (new_pool, tunnel) = crate::modes::sql::client::create_pool_with_tunnel(&config, Some(pool)).await
            .map_err(|e| format!(
                "Auto-connect failed for {}:{}/{}. Driver said:\n```\n{}\n```\nNEXT ACTION: Tell the user the database is not reachable (server may be down, network blocked, or credentials invalid). Do not retry. Show the raw error verbatim.",
                host, port, target_db, e
            ))?;

        let mut conns = sql_manager.connections.lock().await;
        if conns.contains_key(&final_key) {
            drop(tunnel);
            return Ok(final_key);
        }
        conns.insert(final_key.clone(), new_pool);
        if let Some(t) = tunnel {
            sql_manager.tunnels.lock().await.insert(final_key.clone(), t);
        }
        log::info!("[AI SQL] Auto-connected to {}:{}/{} as pool {}", host, port, target_db, final_key);
        Ok(final_key)
    } else {
        let keys_snapshot: Vec<String> = {
            let conns = sql_manager.connections.lock().await;
            conns.keys().cloned().collect()
        };
        Err(format!(
            "No saved connection found for id '{}' (resolved saved_id='{}', database={:?}). Pool map currently has: {:?}.\nNEXT ACTION: Tell the user that the connection_id sent by the AI does not correspond to any saved record. Likely causes: (a) the saved connection was deleted, (b) chat context has a stale connection_id from a prior session, (c) the active connection has not been used yet (try one manual query first to warm the pool). Do not retry.",
            connection_id, saved_id, database, keys_snapshot
        ))
    }
}

pub async fn execute_sql_tool(
    tool_name: &str,
    input: &serde_json::Value,
    _context: &ChatContext,
    pool: &SqlitePool,
    app: &AppHandle,
    session_id: &str,
    sql_manager: &Arc<SqlConnectionManager>,
) -> String {
    match tool_name {
        "list_connections" => {
            let conns = crate::shared::repos::sql_connections::list_all(pool).await;

            match conns {
                Ok(rows) => {
                    let result: Vec<serde_json::Value> = rows
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "id": c.id,
                                "name": c.name,
                                "driver": c.driver,
                                "host": c.host,
                                "port": c.port,
                                "database": c.database_name,
                                "note": "Use the connection_id from <context> envVars for tool calls, not this id. This is the saved config ID.",
                            })
                        })
                        .collect();
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| "[]".to_string())
                }
                Err(e) => format!("Error: {}", e),
            }
        }
        "list_databases" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            if connection_id.is_empty() {
                return "Error: connection_id is required".to_string();
            }

            let pool_id = match ensure_pool(connection_id, input["database"].as_str(), _context, pool, sql_manager).await {
                Ok(id) => id,
                Err(e) => return e,
            };
            let connections = sql_manager.connections.lock().await;
            let pool_entry = match connections.get(&pool_id) {
                Some(p) => p,
                None => return format!("Error: Pool '{}' not found after auto-connect.", pool_id),
            };

            use crate::modes::sql::client::DatabasePool;
            let result = match pool_entry {
                DatabasePool::Postgres(p) => {
                    sqlx::query_as::<_, (String,)>(
                        "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
                    )
                    .fetch_all(p)
                    .await
                }
                DatabasePool::MySql(p) => {
                    // VARCHAR vs VARBINARY return type differs per MySQL
                    // server charset — use the helper that handles both.
                    match sqlx::query("SHOW DATABASES").fetch_all(p).await {
                        Ok(rows) => Ok(rows
                            .iter()
                            .map(|row| (crate::modes::sql::client::mysql_decode_string(row, 0),))
                            .filter(|(s,)| !s.is_empty())
                            .collect()),
                        Err(e) => Err(e),
                    }
                }
                DatabasePool::Sqlite(_) => {
                    Ok(vec![("main".to_string(),)])
                }
                DatabasePool::Clickhouse(c) => {
                    match c.query("SELECT name FROM system.databases ORDER BY name").await {
                        Ok(r) => {
                            let dbs: Vec<(String,)> = r
                                .rows
                                .into_iter()
                                .filter_map(|row| {
                                    row.into_iter().next().and_then(|v| v.as_str().map(|s| (s.to_string(),)))
                                })
                                .collect();
                            Ok(dbs)
                        }
                        Err(e) => Err(sqlx::Error::Protocol(e)),
                    }
                }
                DatabasePool::D1(c) => Ok(vec![(c.database.clone(),)]),
            };

            match result {
                Ok(rows) => {
                    let dbs: Vec<String> = rows.into_iter().map(|r| r.0).collect();
                    serde_json::to_string_pretty(&dbs).unwrap_or_else(|_| "[]".to_string())
                }
                Err(e) => format!("Error listing databases: {}", e),
            }
        }
        "list_schemas" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            if connection_id.is_empty() {
                return "Error: connection_id is required".to_string();
            }

            let pool_id = match ensure_pool(connection_id, input["database"].as_str(), _context, pool, sql_manager).await {
                Ok(id) => id,
                Err(e) => return e,
            };
            let connections = sql_manager.connections.lock().await;
            let pool_entry = match connections.get(&pool_id) {
                Some(p) => p,
                None => return format!("Error: Pool '{}' not found after auto-connect.", pool_id),
            };

            use crate::modes::sql::client::DatabasePool;
            let result = match pool_entry {
                DatabasePool::Postgres(p) => {
                    sqlx::query_as::<_, (String,)>(
                        "SELECT schema_name FROM information_schema.schemata WHERE schema_name NOT IN ('pg_catalog', 'information_schema', 'pg_toast') ORDER BY schema_name",
                    )
                    .fetch_all(p)
                    .await
                }
                DatabasePool::MySql(_) => Ok(vec![("default".to_string(),)]),
                DatabasePool::Sqlite(_) => Ok(vec![("main".to_string(),)]),
                // ClickHouse has no separate schema concept; surface the
                // active database name as the only schema.
                DatabasePool::Clickhouse(c) => Ok(vec![(c.database.clone(),)]),
                // D1 is SQLite under the hood — single "main" schema.
                DatabasePool::D1(_) => Ok(vec![("main".to_string(),)]),
            };

            match result {
                Ok(rows) => {
                    let schemas: Vec<String> = rows.into_iter().map(|r| r.0).collect();
                    serde_json::to_string_pretty(&schemas).unwrap_or_else(|_| "[]".to_string())
                }
                Err(e) => format!("Error listing schemas: {}", e),
            }
        }
        "list_tables" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            if connection_id.is_empty() {
                return "Error: connection_id is required".to_string();
            }
            let database = input["database"].as_str().map(|s| s.to_string());
            let schema = input["schema"].as_str().map(|s| s.to_string());

            let pool_id = match ensure_pool(connection_id, input["database"].as_str(), _context, pool, sql_manager).await {
                Ok(id) => id,
                Err(e) => return e,
            };
            let connections = sql_manager.connections.lock().await;
            let pool_entry = match connections.get(&pool_id) {
                Some(p) => p,
                None => return format!("Error: Pool '{}' not found after auto-connect.", pool_id),
            };

            use crate::modes::sql::client::DatabasePool;
            let result: Result<Vec<(String, String)>, String> = match pool_entry {
                DatabasePool::Postgres(p) => {
                    let schema_name = schema.unwrap_or_else(|| "public".to_string());
                    sqlx::query_as::<_, (String, String)>(
                        "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name",
                    )
                    .bind(&schema_name)
                    .fetch_all(p)
                    .await
                    .map_err(|e| e.to_string())
                }
                DatabasePool::MySql(p) => {
                    let db = database.unwrap_or_default();
                    let query = if db.is_empty() {
                        "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name".to_string()
                    } else {
                        format!(
                            "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = '{}' ORDER BY table_name",
                            db.replace('\'', "''")
                        )
                    };
                    sqlx::query_as::<_, (String, String)>(&query)
                        .fetch_all(p)
                        .await
                        .map_err(|e| e.to_string())
                }
                DatabasePool::Sqlite(p) => {
                    sqlx::query_as::<_, (String, String)>(
                        "SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
                    )
                    .fetch_all(p)
                    .await
                    .map_err(|e| e.to_string())
                }
                DatabasePool::Clickhouse(c) => {
                    let db_name = database.clone().unwrap_or_else(|| c.database.clone());
                    let safe_db = db_name.replace('\'', "''");
                    let stmt = format!(
                        "SELECT name, engine FROM system.tables WHERE database = '{}' ORDER BY name",
                        safe_db
                    );
                    match c.query(&stmt).await {
                        Ok(r) => Ok(r
                            .rows
                            .into_iter()
                            .filter_map(|row| {
                                let mut it = row.into_iter();
                                let name = it.next()?.as_str().map(|s| s.to_string())?;
                                let engine = it
                                    .next()
                                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                                    .unwrap_or_default();
                                let tt = if engine.to_lowercase().contains("view") {
                                    "VIEW".to_string()
                                } else {
                                    "TABLE".to_string()
                                };
                                Some((name, tt))
                            })
                            .collect()),
                        Err(e) => Err(e),
                    }
                }
                DatabasePool::D1(c) => {
                    // Same sqlite_master query the SQLite branch above
                    // would use; D1 hides its own bookkeeping in `_cf_*`.
                    match c
                        .query(
                            "SELECT name, type FROM sqlite_master \
                             WHERE type IN ('table', 'view') \
                               AND name NOT LIKE 'sqlite_%' \
                               AND name NOT LIKE '_cf_%' \
                             ORDER BY name",
                        )
                        .await
                    {
                        Ok(r) => Ok(r
                            .rows
                            .into_iter()
                            .filter_map(|row| {
                                let mut it = row.into_iter();
                                let name = it.next()?.as_str().map(|s| s.to_string())?;
                                let tt = it
                                    .next()
                                    .and_then(|v| v.as_str().map(|s| s.to_uppercase()))
                                    .unwrap_or_else(|| "TABLE".to_string());
                                Some((name, tt))
                            })
                            .collect()),
                        Err(e) => Err(e),
                    }
                }
            };

            match result {
                Ok(rows) => {
                    let tables: Vec<serde_json::Value> = rows
                        .into_iter()
                        .map(|(name, table_type)| {
                            let tt = if table_type == "BASE TABLE" { "TABLE" } else { &table_type };
                            serde_json::json!({"name": name, "type": tt})
                        })
                        .collect();
                    serde_json::to_string_pretty(&tables).unwrap_or_else(|_| "[]".to_string())
                }
                Err(e) => format!("Error listing tables: {}", e),
            }
        }
        "describe_table" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let table = input["table"].as_str().unwrap_or("");
            if connection_id.is_empty() || table.is_empty() {
                return "Error: connection_id and table are required".to_string();
            }
            let schema = input["schema"].as_str().map(|s| s.to_string());

            let pool_id = match ensure_pool(connection_id, input["database"].as_str(), _context, pool, sql_manager).await {
                Ok(id) => id,
                Err(e) => return e,
            };
            let connections = sql_manager.connections.lock().await;
            let pool_entry = match connections.get(&pool_id) {
                Some(p) => p,
                None => return format!("Error: Pool '{}' not found after auto-connect.", pool_id),
            };

            use crate::modes::sql::client::DatabasePool;
            let result: Result<Vec<serde_json::Value>, String> = match pool_entry {
                DatabasePool::Postgres(p) => {
                    let schema_name = schema.unwrap_or_else(|| "public".to_string());

                    #[derive(sqlx::FromRow)]
                    struct PgCol {
                        column_name: String,
                        data_type: String,
                        is_nullable: String,
                        column_default: Option<String>,
                        is_pk: Option<bool>,
                    }

                    sqlx::query_as::<_, PgCol>(
                        "SELECT c.column_name, c.data_type, c.is_nullable, c.column_default,
                            EXISTS (
                                SELECT 1 FROM information_schema.table_constraints tc
                                JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name
                                WHERE tc.table_name = c.table_name AND tc.table_schema = c.table_schema AND tc.constraint_type = 'PRIMARY KEY' AND kcu.column_name = c.column_name
                            ) as is_pk
                        FROM information_schema.columns c
                        WHERE c.table_name = $1 AND c.table_schema = $2
                        ORDER BY c.ordinal_position",
                    )
                    .bind(table)
                    .bind(&schema_name)
                    .fetch_all(p)
                    .await
                    .map(|rows| {
                        rows.into_iter()
                            .map(|r| serde_json::json!({
                                "name": r.column_name,
                                "type": r.data_type,
                                "nullable": r.is_nullable == "YES",
                                "primaryKey": r.is_pk.unwrap_or(false),
                                "default": r.column_default,
                            }))
                            .collect()
                    })
                    .map_err(|e| e.to_string())
                }
                DatabasePool::MySql(p) => {
                    #[derive(sqlx::FromRow)]
                    struct MysqlCol {
                        #[sqlx(rename = "Field")]
                        field: String,
                        #[sqlx(rename = "Type")]
                        col_type: String,
                        #[sqlx(rename = "Null")]
                        nullable: String,
                        #[sqlx(rename = "Key")]
                        key: String,
                        #[sqlx(rename = "Default")]
                        default: Option<String>,
                    }

                    sqlx::query_as::<_, MysqlCol>(&format!(
                        "DESCRIBE `{}`",
                        table.replace('`', "``")
                    ))
                    .fetch_all(p)
                    .await
                    .map(|rows| {
                        rows.into_iter()
                            .map(|r| serde_json::json!({
                                "name": r.field,
                                "type": r.col_type,
                                "nullable": r.nullable == "YES",
                                "primaryKey": r.key == "PRI",
                                "default": r.default,
                            }))
                            .collect()
                    })
                    .map_err(|e| e.to_string())
                }
                DatabasePool::Clickhouse(c) => {
                    let db_name = schema.clone().unwrap_or_else(|| c.database.clone());
                    let safe_db = db_name.replace('\'', "''");
                    let safe_table = table.replace('\'', "''");
                    let stmt = format!(
                        "SELECT name, type, default_expression, is_in_primary_key \
                         FROM system.columns \
                         WHERE database = '{}' AND table = '{}' \
                         ORDER BY position",
                        safe_db, safe_table
                    );
                    match c.query(&stmt).await {
                        Ok(r) => {
                            let cols: Vec<serde_json::Value> = r
                                .rows
                                .into_iter()
                                .filter_map(|row| {
                                    let mut it = row.into_iter();
                                    let name = it.next()?.as_str().map(|s| s.to_string())?;
                                    let dtype = it
                                        .next()
                                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                                        .unwrap_or_default();
                                    let default = it
                                        .next()
                                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                                        .filter(|s| !s.is_empty());
                                    let is_pk = it
                                        .next()
                                        .map(|v| match v {
                                            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) > 0,
                                            serde_json::Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
                                            serde_json::Value::Bool(b) => b,
                                            _ => false,
                                        })
                                        .unwrap_or(false);
                                    let nullable = dtype.starts_with("Nullable(");
                                    Some(serde_json::json!({
                                        "name": name,
                                        "type": dtype,
                                        "nullable": nullable,
                                        "primaryKey": is_pk,
                                        "default": default,
                                    }))
                                })
                                .collect();
                            Ok(cols)
                        }
                        Err(e) => Err(e),
                    }
                }
                DatabasePool::D1(c) => {
                    // D1 supports PRAGMA. Mirror the SQLite shape via raw
                    // JSON values since D1Client doesn't dispatch through
                    // sqlx's typed row parser.
                    let stmt = format!(
                        "PRAGMA table_info(\"{}\")",
                        table.replace('"', "\"\"")
                    );
                    match c.query(&stmt).await {
                        Ok(r) => {
                            let cols: Vec<serde_json::Value> = r
                                .rows
                                .into_iter()
                                .filter_map(|row| {
                                    // cid, name, type, notnull, dflt_value, pk
                                    let mut it = row.into_iter();
                                    let _cid = it.next();
                                    let name = it.next()?.as_str().map(|s| s.to_string())?;
                                    let dtype = it
                                        .next()
                                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                                        .unwrap_or_default();
                                    let notnull = it
                                        .next()
                                        .map(|v| match v {
                                            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) > 0,
                                            serde_json::Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
                                            serde_json::Value::Bool(b) => b,
                                            _ => false,
                                        })
                                        .unwrap_or(false);
                                    let dflt = it
                                        .next()
                                        .and_then(|v| match v {
                                            serde_json::Value::Null => None,
                                            serde_json::Value::String(s) => Some(s),
                                            other => Some(other.to_string()),
                                        });
                                    let is_pk = it
                                        .next()
                                        .map(|v| match v {
                                            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) > 0,
                                            serde_json::Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
                                            serde_json::Value::Bool(b) => b,
                                            _ => false,
                                        })
                                        .unwrap_or(false);
                                    Some(serde_json::json!({
                                        "name": name,
                                        "type": dtype,
                                        "nullable": !notnull,
                                        "primaryKey": is_pk,
                                        "default": dflt,
                                    }))
                                })
                                .collect();
                            Ok(cols)
                        }
                        Err(e) => Err(e),
                    }
                }
                DatabasePool::Sqlite(p) => {
                    #[derive(sqlx::FromRow)]
                    struct SqliteCol {
                        name: String,
                        #[sqlx(rename = "type")]
                        col_type: String,
                        notnull: i32,
                        dflt_value: Option<String>,
                        pk: i32,
                    }

                    sqlx::query_as::<_, SqliteCol>(&format!(
                        "PRAGMA table_info(\"{}\")",
                        table.replace('"', "\"\"")
                    ))
                    .fetch_all(p)
                    .await
                    .map(|rows| {
                        rows.into_iter()
                            .map(|r| serde_json::json!({
                                "name": r.name,
                                "type": r.col_type,
                                "nullable": r.notnull == 0,
                                "primaryKey": r.pk > 0,
                                "default": r.dflt_value,
                            }))
                            .collect()
                    })
                    .map_err(|e| e.to_string())
                }
            };

            match result {
                Ok(columns) => serde_json::to_string_pretty(&columns).unwrap_or_else(|_| "[]".to_string()),
                Err(e) => format!("Error describing table: {}", e),
            }
        }
        "execute_query" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let query = input["query"].as_str().unwrap_or("");
            if connection_id.is_empty() || query.is_empty() {
                return "Error: connection_id and query are required".to_string();
            }

            let pool_id = match ensure_pool(connection_id, input["database"].as_str(), _context, pool, sql_manager).await {
                Ok(id) => id,
                Err(e) => return e,
            };
            let connections = sql_manager.connections.lock().await;
            let pool_entry = match connections.get(&pool_id) {
                Some(p) => p,
                None => return format!("Error: Pool '{}' not found after auto-connect.", pool_id),
            };

            use crate::modes::sql::client::DatabasePool;
            use sqlx::{Column, Row};

            let is_clickhouse = matches!(pool_entry, DatabasePool::Clickhouse(_));

            let start = std::time::Instant::now();
            let mut attempt: u32 = 0;
            let result: Result<(Vec<String>, Vec<Vec<serde_json::Value>>), String> = loop {
                attempt += 1;
                // Use the same per-driver row decoders the SQL panel uses
                // (client.rs::{pg,mysql,sqlite}_row_to_json). Prior to this
                // refactor the AI path had its own type-probing duplicates
                // which fell out of date — most visibly, MySQL UNSIGNED and
                // information_schema VARBINARY columns surfaced as NULL.
                // Single source of truth: when a new MySQL/Postgres type
                // needs decoding it gets added to client.rs and both UIs
                // pick it up.
                let outcome: Result<(Vec<String>, Vec<Vec<serde_json::Value>>), String> = match pool_entry {
                DatabasePool::Postgres(p) => {
                    sqlx::query(query)
                        .fetch_all(p)
                        .await
                        .map(|rows| {
                            let columns: Vec<String> = if rows.is_empty() {
                                vec![]
                            } else {
                                rows[0].columns().iter().map(|c| c.name().to_string()).collect()
                            };
                            let json_rows: Vec<Vec<serde_json::Value>> = rows
                                .iter()
                                .map(crate::modes::sql::client::pg_row_to_json)
                                .collect();
                            (columns, json_rows)
                        })
                        .map_err(|e| e.to_string())
                }
                DatabasePool::MySql(p) => {
                    sqlx::query(query)
                        .fetch_all(p)
                        .await
                        .map(|rows| {
                            let columns: Vec<String> = if rows.is_empty() {
                                vec![]
                            } else {
                                rows[0].columns().iter().map(|c| c.name().to_string()).collect()
                            };
                            let json_rows: Vec<Vec<serde_json::Value>> = rows
                                .iter()
                                .map(crate::modes::sql::client::mysql_row_to_json)
                                .collect();
                            (columns, json_rows)
                        })
                        .map_err(|e| e.to_string())
                }
                DatabasePool::Sqlite(p) => {
                    sqlx::query(query)
                        .fetch_all(p)
                        .await
                        .map(|rows| {
                            let columns: Vec<String> = if rows.is_empty() {
                                vec![]
                            } else {
                                rows[0].columns().iter().map(|c| c.name().to_string()).collect()
                            };
                            let json_rows: Vec<Vec<serde_json::Value>> = rows
                                .iter()
                                .map(crate::modes::sql::client::sqlite_row_to_json)
                                .collect();
                            (columns, json_rows)
                        })
                        .map_err(|e| e.to_string())
                }
                DatabasePool::Clickhouse(c) => {
                    c.query(query).await.map(|r| (r.columns, r.rows))
                }
                DatabasePool::D1(c) => {
                    c.query(query).await.map(|r| (r.columns, r.rows))
                }
                };
                match &outcome {
                    Err(e) if attempt < 5 && classify_sql_error(e) == SqlErrKind::Transient => {
                        let backoff_ms = 600u64 * (1u64 << (attempt - 1).min(4));
                        log::info!("[AI SQL] transient error on attempt {} (sleeping {}ms): {}", attempt, backoff_ms, e);
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        continue;
                    }
                    _ => break outcome,
                }
            };
            let duration_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok((columns, rows)) => {
                    let row_count = rows.len();

                    let (saved_conn_id, derived_db) = pool_id
                        .split_once(':')
                        .map(|(s, d)| (s.to_string(), d.to_string()))
                        .unwrap_or((pool_id.clone(), String::new()));
                    let database_out = input["database"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .unwrap_or(derived_db);

                    let _ = app.emit(
                        &format!("ai:action:{}", session_id),
                        serde_json::json!({
                            "action": "ai_execute_sql",
                            "data": {
                                "query": query,
                                "connectionId": saved_conn_id,
                                "database": database_out,
                                "rowCount": row_count,
                                "durationMs": duration_ms,
                                "columns": columns,
                            },
                        }),
                    );

                    let ch_note = if is_clickhouse {
                        " ClickHouse note: Int64/UInt64/Decimal columns arrive as JSON strings (precision-preserving)."
                    } else {
                        ""
                    };
                    if row_count == 0 {
                        format!("Query returned 0 rows in {}ms.{}", duration_ms, ch_note)
                    } else {
                        const SAMPLE_MAX_ROWS: usize = 10;
                        const SAMPLE_MAX_BYTES: usize = 4096;
                        let sample = crate::shared::ai::sample::format_row_sample(
                            &columns, &rows, SAMPLE_MAX_ROWS, SAMPLE_MAX_BYTES,
                        );
                        let shown = row_count.min(SAMPLE_MAX_ROWS);
                        let header = if row_count > shown {
                            format!(
                                "Query returned {} row(s) in {}ms.{} Showing first {} for your reasoning — full result is already in the SQL panel, do NOT re-print it to the user.",
                                row_count, duration_ms, ch_note, shown
                            )
                        } else {
                            format!(
                                "Query returned {} row(s) in {}ms.{} Full result is already in the SQL panel, do NOT re-print it to the user — use the sample below only for your reasoning.",
                                row_count, duration_ms, ch_note
                            )
                        };
                        format!("{}\n```json\n{}\n```", header, sample)
                    }
                }
                Err(e) => {
                    let kind = classify_sql_error(&e);
                    let base = diagnose_query_error(&e);
                    if matches!(kind, SqlErrKind::SchemaColumn | SqlErrKind::SchemaTable) {
                        let db = input["database"].as_str().unwrap_or("");
                        let hint = build_schema_hint(&e, kind, pool_entry, db, query, pool).await;
                        format!("{}{}", base, hint)
                    } else {
                        base
                    }
                }
            }
        }
        "apply_query" => {
            let query = input["query"].as_str().unwrap_or("");
            if query.is_empty() {
                return "Error: query is required".to_string();
            }
            let _ = app.emit(
                &format!("ai:action:{}", session_id),
                serde_json::json!({
                    "action": "apply_query",
                    "data": { "query": query },
                }),
            );
            "Query written to the user's editor.".to_string()
        }
        "get_schema" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            if connection_id.is_empty() {
                return "Error: connection_id is required".to_string();
            }

            let pool_id = match ensure_pool(connection_id, input["database"].as_str(), _context, pool, sql_manager).await {
                Ok(id) => id,
                Err(e) => return e,
            };
            let connections = sql_manager.connections.lock().await;
            let pool_entry = match connections.get(&pool_id) {
                Some(p) => p,
                None => return format!("Error: Pool '{}' not found after auto-connect.", pool_id),
            };

            let database = input["database"].as_str().unwrap_or("");
            let schema = input["schema"].as_str().unwrap_or("");

            // Get tables
            let tables_result = match pool_entry {
                crate::modes::sql::client::DatabasePool::Postgres(p) => {
                    let schema_filter = if schema.is_empty() { "public" } else { schema };
                    sqlx::query_as::<_, (String, String)>(
                        "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = $1 ORDER BY table_name"
                    )
                    .bind(schema_filter)
                    .fetch_all(p)
                    .await
                }
                crate::modes::sql::client::DatabasePool::MySql(p) => {
                    if database.is_empty() {
                        sqlx::query_as::<_, (String, String)>(
                            "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name"
                        )
                        .fetch_all(p)
                        .await
                    } else {
                        sqlx::query_as::<_, (String, String)>(
                            "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = ? ORDER BY table_name"
                        )
                        .bind(database)
                        .fetch_all(p)
                        .await
                    }
                }
                crate::modes::sql::client::DatabasePool::Sqlite(p) => {
                    sqlx::query_as::<_, (String, String)>(
                        "SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name"
                    )
                    .fetch_all(p)
                    .await
                }
                crate::modes::sql::client::DatabasePool::Clickhouse(c) => {
                    let db_name = if database.is_empty() { c.database.clone() } else { database.to_string() };
                    let safe_db = db_name.replace('\'', "''");
                    let stmt = format!(
                        "SELECT name, engine FROM system.tables WHERE database = '{}' ORDER BY name",
                        safe_db
                    );
                    match c.query(&stmt).await {
                        Ok(r) => Ok(r
                            .rows
                            .into_iter()
                            .filter_map(|row| {
                                let mut it = row.into_iter();
                                let name = it.next()?.as_str().map(|s| s.to_string())?;
                                let engine = it
                                    .next()
                                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                                    .unwrap_or_default();
                                let tt = if engine.to_lowercase().contains("view") {
                                    "VIEW".to_string()
                                } else {
                                    "TABLE".to_string()
                                };
                                Some((name, tt))
                            })
                            .collect()),
                        Err(e) => Err(sqlx::Error::Protocol(e)),
                    }
                }
                crate::modes::sql::client::DatabasePool::D1(c) => {
                    match c
                        .query(
                            "SELECT name, type FROM sqlite_master \
                             WHERE type IN ('table', 'view') \
                               AND name NOT LIKE 'sqlite_%' \
                               AND name NOT LIKE '_cf_%' \
                             ORDER BY name",
                        )
                        .await
                    {
                        Ok(r) => Ok(r
                            .rows
                            .into_iter()
                            .filter_map(|row| {
                                let mut it = row.into_iter();
                                let name = it.next()?.as_str().map(|s| s.to_string())?;
                                let tt = it
                                    .next()
                                    .and_then(|v| v.as_str().map(|s| s.to_uppercase()))
                                    .unwrap_or_else(|| "TABLE".to_string());
                                Some((name, tt))
                            })
                            .collect()),
                        Err(e) => Err(sqlx::Error::Protocol(e)),
                    }
                }
            };

            match tables_result {
                Ok(tables) => {
                    let mut schema_output = Vec::new();
                    for (table_name, table_type) in &tables {
                        // Get columns for each table
                        let cols = match pool_entry {
                            crate::modes::sql::client::DatabasePool::Postgres(p) => {
                                let sf = if schema.is_empty() { "public" } else { schema };
                                sqlx::query_as::<_, (String, String, String)>(
                                    "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position"
                                )
                                .bind(sf)
                                .bind(table_name)
                                .fetch_all(p)
                                .await
                            }
                            crate::modes::sql::client::DatabasePool::MySql(p) => {
                                if database.is_empty() {
                                    sqlx::query_as::<_, (String, String, String)>(
                                        "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ? ORDER BY ordinal_position"
                                    )
                                    .bind(table_name)
                                    .fetch_all(p)
                                    .await
                                } else {
                                    sqlx::query_as::<_, (String, String, String)>(
                                        "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position"
                                    )
                                    .bind(database)
                                    .bind(table_name)
                                    .fetch_all(p)
                                    .await
                                }
                            }
                            crate::modes::sql::client::DatabasePool::Sqlite(p) => {
                                // PRAGMA doesn't fit the 3-column shape; format manually
                                let pragma: Vec<(i32, String, String, i32, Option<String>, i32)> = sqlx::query_as(
                                    &format!("PRAGMA table_info(\"{}\")", table_name.replace('"', "\"\""))
                                )
                                .fetch_all(p)
                                .await
                                .unwrap_or_default();
                                Ok(pragma.iter().map(|(_cid, name, dtype, notnull, _dflt, _pk)| {
                                    (name.clone(), dtype.clone(), if *notnull == 1 { "NO".to_string() } else { "YES".to_string() })
                                }).collect())
                            }
                            crate::modes::sql::client::DatabasePool::Clickhouse(c) => {
                                let db_name = if database.is_empty() { c.database.clone() } else { database.to_string() };
                                let safe_db = db_name.replace('\'', "''");
                                let safe_table = table_name.replace('\'', "''");
                                let stmt = format!(
                                    "SELECT name, type FROM system.columns \
                                     WHERE database = '{}' AND table = '{}' \
                                     ORDER BY position",
                                    safe_db, safe_table
                                );
                                match c.query(&stmt).await {
                                    Ok(r) => {
                                        let cols: Vec<(String, String, String)> = r
                                            .rows
                                            .into_iter()
                                            .filter_map(|row| {
                                                let mut it = row.into_iter();
                                                let name = it.next()?.as_str().map(|s| s.to_string())?;
                                                let dtype = it
                                                    .next()
                                                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                                                    .unwrap_or_default();
                                                let nullable = if dtype.starts_with("Nullable(") {
                                                    "YES".to_string()
                                                } else {
                                                    "NO".to_string()
                                                };
                                                Some((name, dtype, nullable))
                                            })
                                            .collect();
                                        Ok(cols)
                                    }
                                    Err(e) => Err(sqlx::Error::Protocol(e)),
                                }
                            }
                            crate::modes::sql::client::DatabasePool::D1(c) => {
                                let stmt = format!(
                                    "PRAGMA table_info(\"{}\")",
                                    table_name.replace('"', "\"\"")
                                );
                                match c.query(&stmt).await {
                                    Ok(r) => {
                                        let cols: Vec<(String, String, String)> = r
                                            .rows
                                            .into_iter()
                                            .filter_map(|row| {
                                                let mut it = row.into_iter();
                                                let _cid = it.next();
                                                let name = it.next()?.as_str().map(|s| s.to_string())?;
                                                let dtype = it
                                                    .next()
                                                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                                                    .unwrap_or_default();
                                                let notnull = it
                                                    .next()
                                                    .map(|v| matches!(v,
                                                        serde_json::Value::Number(n) if n.as_u64().unwrap_or(0) > 0
                                                    ))
                                                    .unwrap_or(false);
                                                let nullable = if notnull { "NO".to_string() } else { "YES".to_string() };
                                                Some((name, dtype, nullable))
                                            })
                                            .collect();
                                        Ok(cols)
                                    }
                                    Err(e) => Err(sqlx::Error::Protocol(e)),
                                }
                            }
                        };
                        let cols_str = match cols {
                            Ok(c) => c.iter().map(|(name, dtype, nullable)| {
                                if nullable == "NO" { format!("{} {} NOT NULL", name, dtype) }
                                else { format!("{} {}", name, dtype) }
                            }).collect::<Vec<_>>().join(", "),
                            Err(_) => "?".to_string(),
                        };
                        let prefix = if table_type.contains("VIEW") { "VIEW" } else { "TABLE" };
                        schema_output.push(format!("{} {}({})", prefix, table_name, cols_str));
                    }
                    schema_output.join("\n")
                }
                Err(e) => format!("Error listing tables: {}", e),
            }
        }
        "explain_query" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let query = input["query"].as_str().unwrap_or("");
            if connection_id.is_empty() || query.is_empty() {
                return "Error: connection_id and query are required".to_string();
            }

            let pool_id = match ensure_pool(connection_id, input["database"].as_str(), _context, pool, sql_manager).await {
                Ok(id) => id,
                Err(e) => return e,
            };
            let connections = sql_manager.connections.lock().await;
            let pool_entry = match connections.get(&pool_id) {
                Some(p) => p,
                None => return format!("Error: Pool '{}' not found after auto-connect.", pool_id),
            };

            let explain_sql = match pool_entry {
                crate::modes::sql::client::DatabasePool::Postgres(_) => format!("EXPLAIN ANALYZE {}", query),
                crate::modes::sql::client::DatabasePool::MySql(_) => format!("EXPLAIN {}", query),
                crate::modes::sql::client::DatabasePool::Sqlite(_) => format!("EXPLAIN QUERY PLAN {}", query),
                // ClickHouse uses plain `EXPLAIN <query>` (defaults to
                // EXPLAIN PLAN). Older versions accept the same syntax.
                crate::modes::sql::client::DatabasePool::Clickhouse(_) => format!("EXPLAIN {}", query),
                // D1 is SQLite — same EXPLAIN QUERY PLAN syntax.
                crate::modes::sql::client::DatabasePool::D1(_) => format!("EXPLAIN QUERY PLAN {}", query),
            };

            let result = match pool_entry {
                crate::modes::sql::client::DatabasePool::Postgres(p) => {
                    sqlx::query_scalar::<_, String>(&explain_sql)
                        .fetch_all(p)
                        .await
                        .map(|rows| rows.join("\n"))
                }
                crate::modes::sql::client::DatabasePool::MySql(p) => {
                    use sqlx::Row;
                    sqlx::query(&explain_sql)
                        .fetch_all(p)
                        .await
                        .map(|rows| {
                            rows.iter().map(|r| {
                                let ncols = r.columns().len();
                                (0..ncols).map(|i| r.try_get::<String, _>(i).unwrap_or_default()).collect::<Vec<_>>().join(" | ")
                            }).collect::<Vec<_>>().join("\n")
                        })
                }
                crate::modes::sql::client::DatabasePool::Sqlite(p) => {
                    use sqlx::Row;
                    sqlx::query(&explain_sql)
                        .fetch_all(p)
                        .await
                        .map(|rows| {
                            rows.iter().map(|r| {
                                let ncols = r.columns().len();
                                (0..ncols).map(|i| r.try_get::<String, _>(i).unwrap_or_default()).collect::<Vec<_>>().join(" | ")
                            }).collect::<Vec<_>>().join("\n")
                        })
                }
                crate::modes::sql::client::DatabasePool::Clickhouse(c) => {
                    match c.query(&explain_sql).await {
                        Ok(r) => Ok(r
                            .rows
                            .iter()
                            .map(|row| {
                                row.iter()
                                    .map(|v| match v {
                                        serde_json::Value::String(s) => s.clone(),
                                        serde_json::Value::Null => String::new(),
                                        other => other.to_string(),
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" | ")
                            })
                            .collect::<Vec<_>>()
                            .join("\n")),
                        Err(e) => Err(sqlx::Error::Protocol(e)),
                    }
                }
                crate::modes::sql::client::DatabasePool::D1(c) => {
                    match c.query(&explain_sql).await {
                        Ok(r) => Ok(r
                            .rows
                            .iter()
                            .map(|row| {
                                row.iter()
                                    .map(|v| match v {
                                        serde_json::Value::String(s) => s.clone(),
                                        serde_json::Value::Null => String::new(),
                                        other => other.to_string(),
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" | ")
                            })
                            .collect::<Vec<_>>()
                            .join("\n")),
                        Err(e) => Err(sqlx::Error::Protocol(e)),
                    }
                }
            };

            match result {
                Ok(output) => output,
                Err(e) => format!("Error: {}", e),
            }
        }
        _ => format!("Unknown SQL tool: {}", tool_name),
    }
}

// --- Dispatch registry integration -----------------------------------------
//
// Every SQL tool is dispatched through the same `execute_sql_tool` match,
// so each registered descriptor's executor is a thin adapter that captures
// the tool name and forwards the rest of `ToolContext` into `execute_sql_tool`.

use crate::shared::ai::dispatch::{register, ToolContext, ToolDescriptor, ToolFuture};

macro_rules! sql_tool_executor {
    ($name:literal) => {{
        fn exec<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
            Box::pin(async move {
                execute_sql_tool(
                    $name,
                    ctx.input,
                    ctx.context,
                    ctx.pool,
                    ctx.app,
                    ctx.session_id,
                    ctx.sql_manager,
                )
                .await
            })
        }
        exec as crate::shared::ai::dispatch::ToolExecutor
    }};
}

/// Register every SQL-mode AI tool with the shared dispatch registry.
pub fn register_tools() {
    let entries: &[(&'static str, &'static str, crate::shared::ai::dispatch::ToolExecutor)] = &[
        ("list_connections", "List saved SQL connections", sql_tool_executor!("list_connections")),
        ("list_databases", "List databases on a SQL connection", sql_tool_executor!("list_databases")),
        ("list_tables", "List tables in a SQL database/schema", sql_tool_executor!("list_tables")),
        ("describe_table", "Describe columns of a SQL table", sql_tool_executor!("describe_table")),
        ("execute_query", "Execute a SQL query against the active connection", sql_tool_executor!("execute_query")),
        ("apply_query", "Send a SQL query suggestion to the user for approval", sql_tool_executor!("apply_query")),
        ("list_schemas", "List schemas in a SQL database", sql_tool_executor!("list_schemas")),
        ("get_schema", "Fetch the full schema (tables + columns) for a SQL database", sql_tool_executor!("get_schema")),
        ("explain_query", "Run EXPLAIN on a SQL query", sql_tool_executor!("explain_query")),
    ];

    for (name, description, executor) in entries {
        register(ToolDescriptor {
            name,
            mode: "sql",
            description,
            schema: serde_json::json!({}),
            executor: *executor,
        });
    }
}

use std::sync::Arc;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::shared::ai::types::ChatContext;
use crate::modes::sql::client::SqlConnectionManager;

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
    // Extract saved_connection_id from context — needed to build stable cache keys
    let saved_from_ctx = context.env_vars.iter()
        .find(|v| v.key == "saved_connection_id")
        .map(|v| v.value.as_str());
    let saved_id = saved_from_ctx.unwrap_or(connection_id);

    // Build a stable cache key: "savedId:dbName"
    let cache_key = match database {
        Some(db) if !db.is_empty() => format!("{}:{}", saved_id, db),
        _ => saved_id.to_string(),
    };

    // Check if pool already exists under the stable key
    {
        let connections = sql_manager.connections.lock().await;
        if connections.contains_key(&cache_key) {
            return Ok(cache_key);
        }
        // Also check bare connection_id (instance pool created by frontend sql_connect)
        // BUT only if no specific database was requested — otherwise we'd return
        // the wrong-DB pool
        if database.map_or(true, |db| db.is_empty()) && connections.contains_key(connection_id) {
            return Ok(connection_id.to_string());
        }
    }

    // Pool not found — try to auto-connect using saved connection config
    let saved = crate::shared::repos::sql_connections::get_by_id_optional(pool, saved_id)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    if let Some(saved) = saved {
        let target_db = database.unwrap_or(&saved.database_name);
        let host = saved.host.clone();
        let port = saved.port;
        let config = crate::modes::sql::client::SqlConnectionConfig {
            name: String::new(),
            driver: saved.driver,
            host: saved.host,
            port: saved.port as u16,
            database: target_db.to_string(),
            username: saved.username,
            password: saved.password,
            ssl: saved.ssl == 1,
            // Forward the tunnel selection so AI auto-connect honours it.
            ssh_profile_id: saved.ssh_profile_id,
        };

        // Pass the app pool so saved connections with `ssh_profile_id` can
        // open their tunnel. Connections without a tunnel take the same
        // legacy path as before — same URLs, same drivers.
        let (new_pool, tunnel) = crate::modes::sql::client::create_pool_with_tunnel(&config, Some(pool)).await
            .map_err(|e| format!("Auto-connect failed for {}:{}/{}: {}", host, port, target_db, e))?;

        // Store under stable key so subsequent calls reuse the same pool
        let mut conns = sql_manager.connections.lock().await;
        if conns.contains_key(&cache_key) {
            // Race: another caller beat us to it. Drop our extra pool and
            // tunnel; the existing entry wins.
            drop(tunnel);
            return Ok(cache_key);
        }
        conns.insert(cache_key.clone(), new_pool);
        if let Some(t) = tunnel {
            sql_manager.tunnels.lock().await.insert(cache_key.clone(), t);
        }
        log::info!("[AI SQL] Auto-connected to {}:{}/{} as pool {}", host, port, target_db, cache_key);
        Ok(cache_key)
    } else {
        Err(format!("Connection '{}' not found in active pools or saved connections.", connection_id))
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
                    sqlx::query_as::<_, (String,)>("SHOW DATABASES")
                        .fetch_all(p)
                        .await
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

            let start = std::time::Instant::now();
            let result: Result<(Vec<String>, Vec<Vec<serde_json::Value>>), String> = match pool_entry {
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
                            let json_rows: Vec<Vec<serde_json::Value>> = rows.iter().map(|row| {
                                row.columns().iter().map(|col| {
                                    let idx = col.ordinal();
                                    // Try numeric types BEFORE string — sqlx won't coerce int to string
                                    if let Ok(Some(v)) = row.try_get::<Option<bool>, _>(idx) {
                                        return serde_json::Value::Bool(v);
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<i32>, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<i64>, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<f64>, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<f32>, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<rust_decimal::Decimal>, _>(idx) {
                                        return serde_json::json!(v.to_string());
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<uuid::Uuid>, _>(idx) {
                                        return serde_json::Value::String(v.to_string());
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<chrono::NaiveDateTime>, _>(idx) {
                                        return serde_json::Value::String(v.to_string());
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(idx) {
                                        return serde_json::Value::String(v.to_rfc3339());
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<chrono::NaiveDate>, _>(idx) {
                                        return serde_json::Value::String(v.to_string());
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<chrono::NaiveTime>, _>(idx) {
                                        return serde_json::Value::String(v.to_string());
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<serde_json::Value>, _>(idx) {
                                        return v;
                                    }
                                    if let Ok(Some(v)) = row.try_get::<Option<String>, _>(idx) {
                                        return serde_json::Value::String(v);
                                    }
                                    serde_json::Value::Null
                                }).collect()
                            }).collect();
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
                            let json_rows: Vec<Vec<serde_json::Value>> = rows.iter().map(|row| {
                                row.columns().iter().map(|col| {
                                    let idx = col.ordinal();
                                    if let Ok(v) = row.try_get::<bool, _>(idx) {
                                        return serde_json::Value::Bool(v);
                                    }
                                    if let Ok(v) = row.try_get::<i32, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(v) = row.try_get::<i64, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(v) = row.try_get::<f64, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(v) = row.try_get::<f32, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(v) = row.try_get::<String, _>(idx) {
                                        return serde_json::Value::String(v);
                                    }
                                    serde_json::Value::Null
                                }).collect()
                            }).collect();
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
                            let json_rows: Vec<Vec<serde_json::Value>> = rows.iter().map(|row| {
                                row.columns().iter().map(|col| {
                                    let idx = col.ordinal();
                                    if let Ok(v) = row.try_get::<i64, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(v) = row.try_get::<f64, _>(idx) {
                                        return serde_json::json!(v);
                                    }
                                    if let Ok(v) = row.try_get::<String, _>(idx) {
                                        return serde_json::Value::String(v);
                                    }
                                    serde_json::Value::Null
                                }).collect()
                            }).collect();
                            (columns, json_rows)
                        })
                        .map_err(|e| e.to_string())
                }
                DatabasePool::Clickhouse(c) => {
                    c.query(query).await.map(|r| (r.columns, r.rows))
                }
            };
            let duration_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok((columns, rows)) => {
                    let row_count = rows.len();

                    // Always route to main UI — chat shows status only
                    let _ = app.emit(
                        &format!("ai:action:{}", session_id),
                        serde_json::json!({
                            "action": "ai_execute_sql",
                            "data": {
                                "query": query,
                                "connectionId": pool_id,
                                "database": input["database"].as_str().unwrap_or(""),
                                "rowCount": row_count,
                                "durationMs": duration_ms,
                                "columns": columns,
                            },
                        }),
                    );

                    if row_count == 0 {
                        format!("Query returned 0 rows in {}ms.", duration_ms)
                    } else {
                        format!(
                            "Query returned {} row(s) in {}ms. Columns: {}. Results shown in the SQL results panel.",
                            row_count, duration_ms, columns.join(", ")
                        )
                    }
                }
                Err(e) => format!("Query error: {}", e),
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

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use sqlx::{Column, Row, TypeInfo};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tauri::State;
use tokio::sync::Mutex;
use uuid::Uuid;

// --- Types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlConnectionConfig {
    pub name: String,
    pub driver: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub ssl: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub affected_rows: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub name: String,
    pub table_type: String,
    pub row_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SqlScript {
    pub id: String,
    pub name: String,
    pub connection_id: Option<String>,
    pub database_name: String,
    pub query: String,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SqlSavedConnection {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub host: String,
    pub port: i32,
    pub database_name: String,
    pub username: String,
    pub password: String,
    pub ssl: i32,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

// --- Connection pool enum ---

pub enum DatabasePool {
    Postgres(sqlx::PgPool),
    MySql(sqlx::MySqlPool),
    Sqlite(sqlx::SqlitePool),
}

pub struct SqlConnectionManager {
    pub connections: Mutex<HashMap<String, DatabasePool>>,
}

impl SqlConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
        }
    }
}

// --- Helper to build connection strings ---

fn build_connection_url(config: &SqlConnectionConfig) -> Result<String, String> {
    match config.driver.as_str() {
        "postgresql" => {
            let ssl_mode = if config.ssl { "require" } else { "prefer" };
            Ok(format!(
                "postgres://{}:{}@{}:{}/{}?sslmode={}",
                config.username, config.password, config.host, config.port, config.database, ssl_mode
            ))
        }
        "mysql" => {
            let ssl_mode = if config.ssl { "REQUIRED" } else { "PREFERRED" };
            Ok(format!(
                "mysql://{}:{}@{}:{}/{}?ssl-mode={}",
                config.username, config.password, config.host, config.port, config.database, ssl_mode
            ))
        }
        "sqlite" => Ok(format!("sqlite:{}?mode=rwc", config.database)),
        _ => Err(format!("Unsupported driver: {}", config.driver)),
    }
}

pub async fn create_pool(config: &SqlConnectionConfig) -> Result<DatabasePool, String> {
    let url = build_connection_url(config)?;
    eprintln!("[Clauge SQL] create_pool driver={} host={} port={} db={} ssl={} user={}",
        config.driver, config.host, config.port, config.database, config.ssl, config.username);
    match config.driver.as_str() {
        "postgresql" => {
            use sqlx::postgres::{PgConnectOptions, PgSslMode};
            use std::str::FromStr;
            let base_url = format!(
                "postgres://{}:{}@{}:{}/{}",
                config.username, config.password, config.host, config.port, config.database
            );
            let ssl_mode = if config.ssl { PgSslMode::Require } else { PgSslMode::Prefer };
            eprintln!("[Clauge SQL] PG base_url={} ssl_mode={:?}", base_url.replace(&config.password, "***"), ssl_mode);
            let mut opts = PgConnectOptions::from_str(&base_url)
                .map_err(|e| {
                    eprintln!("[Clauge SQL] PgConnectOptions parse error: {}", e);
                    format!("Invalid PostgreSQL URL: {}", e)
                })?;
            opts = opts.ssl_mode(ssl_mode);
            eprintln!("[Clauge SQL] Connecting to PostgreSQL...");
            let pool = sqlx::PgPool::connect_with(opts)
                .await
                .map_err(|e| {
                    eprintln!("[Clauge SQL] PostgreSQL connection FAILED: {}", e);
                    format!("PostgreSQL connection failed: {}", e)
                })?;
            eprintln!("[Clauge SQL] PostgreSQL connected OK");
            Ok(DatabasePool::Postgres(pool))
        }
        "mysql" => {
            let pool = sqlx::MySqlPool::connect(&url)
                .await
                .map_err(|e| format!("MySQL connection failed: {}", e))?;
            Ok(DatabasePool::MySql(pool))
        }
        "sqlite" => {
            let pool = sqlx::SqlitePool::connect(&url)
                .await
                .map_err(|e| format!("SQLite connection failed: {}", e))?;
            Ok(DatabasePool::Sqlite(pool))
        }
        _ => Err(format!("Unsupported driver: {}", config.driver)),
    }
}

// --- Helper to convert rows to JSON values ---

fn pg_row_to_json(row: &sqlx::postgres::PgRow) -> Vec<serde_json::Value> {
    use sqlx::types::chrono;

    let columns = row.columns();
    columns
        .iter()
        .map(|col| {
            let idx = col.ordinal();
            let type_name = col.type_info().name();

            match type_name {
                "BOOL" => row.try_get::<Option<bool>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::Bool(v))
                    .unwrap_or(serde_json::Value::Null),
                "INT2" | "INT4" | "OID" => row.try_get::<Option<i32>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "INT8" => row.try_get::<Option<i64>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "FLOAT4" => row.try_get::<Option<f32>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "FLOAT8" => row.try_get::<Option<f64>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "NUMERIC" | "DECIMAL" => row.try_get::<Option<rust_decimal::Decimal>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null),
                "UUID" => row.try_get::<Option<uuid::Uuid>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null),
                "TIMESTAMP" => row.try_get::<Option<chrono::NaiveDateTime>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null),
                "TIMESTAMPTZ" => row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v.to_rfc3339()))
                    .unwrap_or(serde_json::Value::Null),
                "DATE" => row.try_get::<Option<chrono::NaiveDate>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null),
                "TIME" => row.try_get::<Option<chrono::NaiveTime>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null),
                "JSON" | "JSONB" => row.try_get::<Option<serde_json::Value>, _>(idx)
                    .ok().flatten()
                    .unwrap_or(serde_json::Value::Null),
                "BYTEA" => row.try_get::<Option<Vec<u8>>, _>(idx)
                    .ok().flatten()
                    .map(|v| {
                        let hex: String = v.iter().map(|b| format!("{:02x}", b)).collect();
                        serde_json::Value::String(format!("\\x{}", hex))
                    })
                    .unwrap_or(serde_json::Value::Null),
                "CHAR" | "VARCHAR" | "TEXT" | "NAME" | "BPCHAR" | "CITEXT" => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                "TIMETZ" => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                "INTERVAL" => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                "INET" | "CIDR" => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                "MACADDR" | "MACADDR8" => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                "MONEY" => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                "BIT" | "VARBIT" => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                "XML" => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                // Geometric types
                "POINT" | "LINE" | "LSEG" | "BOX" | "PATH" | "POLYGON" | "CIRCLE" => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                // Arrays — try as text representation
                t if t.starts_with('_') => row.try_get::<Option<String>, _>(idx)
                    .ok().flatten()
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
                // Fallback: try common types in order
                _ => {
                    // Try String first
                    if let Ok(Some(v)) = row.try_get::<Option<String>, _>(idx) {
                        return serde_json::Value::String(v);
                    }
                    // Try i64
                    if let Ok(Some(v)) = row.try_get::<Option<i64>, _>(idx) {
                        return serde_json::json!(v);
                    }
                    // Try f64
                    if let Ok(Some(v)) = row.try_get::<Option<f64>, _>(idx) {
                        return serde_json::json!(v);
                    }
                    // Try Decimal
                    if let Ok(Some(v)) = row.try_get::<Option<rust_decimal::Decimal>, _>(idx) {
                        return serde_json::Value::String(v.to_string());
                    }
                    // Try bool
                    if let Ok(Some(v)) = row.try_get::<Option<bool>, _>(idx) {
                        return serde_json::Value::Bool(v);
                    }
                    // Try JSON
                    if let Ok(Some(v)) = row.try_get::<Option<serde_json::Value>, _>(idx) {
                        return v;
                    }
                    // Try UUID
                    if let Ok(Some(v)) = row.try_get::<Option<uuid::Uuid>, _>(idx) {
                        return serde_json::Value::String(v.to_string());
                    }
                    // Try timestamp
                    if let Ok(Some(v)) = row.try_get::<Option<chrono::NaiveDateTime>, _>(idx) {
                        return serde_json::Value::String(v.to_string());
                    }
                    if let Ok(Some(v)) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(idx) {
                        return serde_json::Value::String(v.to_rfc3339());
                    }
                    serde_json::Value::Null
                },
            }
        })
        .collect()
}

fn mysql_row_to_json(row: &sqlx::mysql::MySqlRow) -> Vec<serde_json::Value> {
    let columns = row.columns();
    columns
        .iter()
        .map(|col| {
            let type_name = col.type_info().name();
            match type_name {
                "BOOLEAN" | "TINYINT(1)" => row
                    .try_get::<bool, _>(col.ordinal())
                    .map(|v| serde_json::Value::Bool(v))
                    .unwrap_or(serde_json::Value::Null),
                "TINYINT" | "SMALLINT" | "INT" | "MEDIUMINT" => row
                    .try_get::<i32, _>(col.ordinal())
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "BIGINT" => row
                    .try_get::<i64, _>(col.ordinal())
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "FLOAT" | "DOUBLE" | "DECIMAL" => row
                    .try_get::<f64, _>(col.ordinal())
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                _ => row
                    .try_get::<String, _>(col.ordinal())
                    .map(|v| serde_json::Value::String(v))
                    .unwrap_or(serde_json::Value::Null),
            }
        })
        .collect()
}

fn sqlite_row_to_json(row: &sqlx::sqlite::SqliteRow) -> Vec<serde_json::Value> {
    let columns = row.columns();
    columns
        .iter()
        .map(|col| {
            // Try integer first, then float, then string
            if let Ok(v) = row.try_get::<i64, _>(col.ordinal()) {
                return serde_json::json!(v);
            }
            if let Ok(v) = row.try_get::<f64, _>(col.ordinal()) {
                return serde_json::json!(v);
            }
            if let Ok(v) = row.try_get::<String, _>(col.ordinal()) {
                return serde_json::Value::String(v);
            }
            serde_json::Value::Null
        })
        .collect()
}

// --- Tauri commands ---

#[tauri::command]
pub async fn sql_connect(
    manager: State<'_, Arc<SqlConnectionManager>>,
    config: SqlConnectionConfig,
) -> Result<String, String> {
    let pool = create_pool(&config).await?;
    let connection_id = Uuid::new_v4().to_string();
    let mut connections = manager.connections.lock().await;
    connections.insert(connection_id.clone(), pool);
    Ok(connection_id)
}

#[tauri::command]
pub async fn sql_connect_database(
    manager: State<'_, Arc<SqlConnectionManager>>,
    config: SqlConnectionConfig,
    database: String,
    pool_key: Option<String>,
) -> Result<String, String> {
    let mut db_config = config;
    db_config.database = database;
    let pool = create_pool(&db_config).await?;
    let key = pool_key.unwrap_or_else(|| Uuid::new_v4().to_string());
    let mut connections = manager.connections.lock().await;
    // If pool already exists under this key, close the old one first
    if let Some(old) = connections.remove(&key) {
        match old {
            DatabasePool::Postgres(p) => p.close().await,
            DatabasePool::MySql(p) => p.close().await,
            DatabasePool::Sqlite(p) => p.close().await,
        }
    }
    connections.insert(key.clone(), pool);
    Ok(key)
}

#[tauri::command]
pub async fn sql_disconnect(
    manager: State<'_, Arc<SqlConnectionManager>>,
    connection_id: String,
) -> Result<(), String> {
    let mut connections = manager.connections.lock().await;
    if let Some(pool) = connections.remove(&connection_id) {
        match pool {
            DatabasePool::Postgres(p) => p.close().await,
            DatabasePool::MySql(p) => p.close().await,
            DatabasePool::Sqlite(p) => p.close().await,
        }
        Ok(())
    } else {
        Err("Connection not found".to_string())
    }
}

#[tauri::command]
pub async fn sql_test_connection(config: SqlConnectionConfig) -> Result<(), String> {
    let pool = create_pool(&config).await?;
    // Just connecting is enough; close immediately
    match pool {
        DatabasePool::Postgres(p) => p.close().await,
        DatabasePool::MySql(p) => p.close().await,
        DatabasePool::Sqlite(p) => p.close().await,
    }
    Ok(())
}

#[tauri::command]
pub async fn sql_execute_query(
    manager: State<'_, Arc<SqlConnectionManager>>,
    connection_id: String,
    query: String,
) -> Result<SqlQueryResult, String> {
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&connection_id)
        .ok_or_else(|| "Connection not found".to_string())?;

    let start = Instant::now();

    match pool {
        DatabasePool::Postgres(p) => {
            let rows = sqlx::query(&query)
                .fetch_all(p)
                .await
                .map_err(|e| e.to_string())?;
            let duration_ms = start.elapsed().as_millis() as u64;
            let columns: Vec<String> = if rows.is_empty() {
                vec![]
            } else {
                rows[0].columns().iter().map(|c| c.name().to_string()).collect()
            };
            let json_rows: Vec<Vec<serde_json::Value>> =
                rows.iter().map(|r| pg_row_to_json(r)).collect();
            let affected = json_rows.len() as u64;
            Ok(SqlQueryResult {
                columns,
                rows: json_rows,
                affected_rows: affected,
                duration_ms,
            })
        }
        DatabasePool::MySql(p) => {
            let rows = sqlx::query(&query)
                .fetch_all(p)
                .await
                .map_err(|e| e.to_string())?;
            let duration_ms = start.elapsed().as_millis() as u64;
            let columns: Vec<String> = if rows.is_empty() {
                vec![]
            } else {
                rows[0].columns().iter().map(|c| c.name().to_string()).collect()
            };
            let json_rows: Vec<Vec<serde_json::Value>> =
                rows.iter().map(|r| mysql_row_to_json(r)).collect();
            let affected = json_rows.len() as u64;
            Ok(SqlQueryResult {
                columns,
                rows: json_rows,
                affected_rows: affected,
                duration_ms,
            })
        }
        DatabasePool::Sqlite(p) => {
            let rows = sqlx::query(&query)
                .fetch_all(p)
                .await
                .map_err(|e| e.to_string())?;
            let duration_ms = start.elapsed().as_millis() as u64;
            let columns: Vec<String> = if rows.is_empty() {
                vec![]
            } else {
                rows[0].columns().iter().map(|c| c.name().to_string()).collect()
            };
            let json_rows: Vec<Vec<serde_json::Value>> =
                rows.iter().map(|r| sqlite_row_to_json(r)).collect();
            let affected = json_rows.len() as u64;
            Ok(SqlQueryResult {
                columns,
                rows: json_rows,
                affected_rows: affected,
                duration_ms,
            })
        }
    }
}

#[tauri::command]
pub async fn sql_list_databases(
    manager: State<'_, Arc<SqlConnectionManager>>,
    connection_id: String,
) -> Result<Vec<String>, String> {
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&connection_id)
        .ok_or_else(|| "Connection not found".to_string())?;

    match pool {
        DatabasePool::Postgres(p) => {
            let rows = sqlx::query_as::<_, (String,)>(
                "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
            )
            .fetch_all(p)
            .await
            .map_err(|e| e.to_string())?;
            Ok(rows.into_iter().map(|r| r.0).collect())
        }
        DatabasePool::MySql(p) => {
            let rows = sqlx::query_as::<_, (String,)>("SHOW DATABASES")
                .fetch_all(p)
                .await
                .map_err(|e| e.to_string())?;
            Ok(rows.into_iter().map(|r| r.0).collect())
        }
        DatabasePool::Sqlite(_) => {
            // SQLite has only one database per file
            Ok(vec!["main".to_string()])
        }
    }
}

#[tauri::command]
pub async fn sql_create_database(
    manager: State<'_, Arc<SqlConnectionManager>>,
    connection_id: String,
    name: String,
) -> Result<(), String> {
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&connection_id)
        .ok_or_else(|| "Connection not found".to_string())?;

    // Validate name: only alphanumeric, underscores, hyphens allowed
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err("Invalid database name. Use only letters, numbers, underscores, and hyphens.".to_string());
    }

    let stmt = format!("CREATE DATABASE \"{}\"", name);

    match pool {
        DatabasePool::Postgres(p) => {
            sqlx::query(&stmt)
                .execute(p)
                .await
                .map_err(|e| e.to_string())?;
        }
        DatabasePool::MySql(p) => {
            let stmt = format!("CREATE DATABASE `{}`", name);
            sqlx::query(&stmt)
                .execute(p)
                .await
                .map_err(|e| e.to_string())?;
        }
        DatabasePool::Sqlite(_) => {
            return Err("SQLite does not support creating databases".to_string());
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn sql_list_schemas(
    manager: State<'_, Arc<SqlConnectionManager>>,
    connection_id: String,
) -> Result<Vec<String>, String> {
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&connection_id)
        .ok_or_else(|| "Connection not found".to_string())?;

    match pool {
        DatabasePool::Postgres(p) => {
            let rows = sqlx::query_as::<_, (String,)>(
                "SELECT schema_name FROM information_schema.schemata WHERE schema_name NOT IN ('pg_catalog', 'information_schema', 'pg_toast') ORDER BY schema_name",
            )
            .fetch_all(p)
            .await
            .map_err(|e| e.to_string())?;
            Ok(rows.into_iter().map(|r| r.0).collect())
        }
        DatabasePool::MySql(_) => {
            Ok(vec!["default".to_string()])
        }
        DatabasePool::Sqlite(_) => {
            Ok(vec!["main".to_string()])
        }
    }
}

#[tauri::command]
pub async fn sql_list_tables(
    manager: State<'_, Arc<SqlConnectionManager>>,
    connection_id: String,
    database: Option<String>,
    schema: Option<String>,
) -> Result<Vec<TableInfo>, String> {
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&connection_id)
        .ok_or_else(|| "Connection not found".to_string())?;

    match pool {
        DatabasePool::Postgres(p) => {
            let schema_name = schema.unwrap_or_else(|| "public".to_string());
            let rows = sqlx::query_as::<_, (String, String)>(
                "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name",
            )
            .bind(&schema_name)
            .fetch_all(p)
            .await
            .map_err(|e| e.to_string())?;

            Ok(rows
                .into_iter()
                .map(|(name, table_type)| {
                    let tt = if table_type == "BASE TABLE" {
                        "TABLE".to_string()
                    } else {
                        table_type
                    };
                    TableInfo {
                        name,
                        table_type: tt,
                        row_count: None,
                    }
                })
                .collect())
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
            let rows = sqlx::query_as::<_, (String, String)>(&query)
                .fetch_all(p)
                .await
                .map_err(|e| e.to_string())?;

            Ok(rows
                .into_iter()
                .map(|(name, table_type)| {
                    let tt = if table_type == "BASE TABLE" {
                        "TABLE".to_string()
                    } else {
                        table_type
                    };
                    TableInfo {
                        name,
                        table_type: tt,
                        row_count: None,
                    }
                })
                .collect())
        }
        DatabasePool::Sqlite(p) => {
            let rows = sqlx::query_as::<_, (String, String)>(
                "SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .fetch_all(p)
            .await
            .map_err(|e| e.to_string())?;

            Ok(rows
                .into_iter()
                .map(|(name, table_type)| TableInfo {
                    name,
                    table_type: table_type.to_uppercase(),
                    row_count: None,
                })
                .collect())
        }
    }
}

#[tauri::command]
pub async fn sql_describe_table(
    manager: State<'_, Arc<SqlConnectionManager>>,
    connection_id: String,
    table: String,
    schema: Option<String>,
) -> Result<Vec<ColumnInfo>, String> {
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&connection_id)
        .ok_or_else(|| "Connection not found".to_string())?;

    match pool {
        DatabasePool::Postgres(p) => {
            #[derive(sqlx::FromRow)]
            struct PgColumnInfo {
                column_name: String,
                data_type: String,
                is_nullable: String,
                column_default: Option<String>,
                is_pk: Option<bool>,
            }

            let schema_name = schema.unwrap_or_else(|| "public".to_string());
            let rows = sqlx::query_as::<_, PgColumnInfo>(
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
            .bind(&table)
            .bind(&schema_name)
            .fetch_all(p)
            .await
            .map_err(|e| e.to_string())?;

            Ok(rows
                .into_iter()
                .map(|r| ColumnInfo {
                    name: r.column_name,
                    data_type: r.data_type,
                    is_nullable: r.is_nullable == "YES",
                    is_primary_key: r.is_pk.unwrap_or(false),
                    default_value: r.column_default,
                })
                .collect())
        }
        DatabasePool::MySql(p) => {
            #[derive(sqlx::FromRow)]
            struct MysqlColumnInfo {
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

            let rows = sqlx::query_as::<_, MysqlColumnInfo>(&format!(
                "DESCRIBE `{}`",
                table.replace('`', "``")
            ))
            .fetch_all(p)
            .await
            .map_err(|e| e.to_string())?;

            Ok(rows
                .into_iter()
                .map(|r| ColumnInfo {
                    name: r.field,
                    data_type: r.col_type,
                    is_nullable: r.nullable == "YES",
                    is_primary_key: r.key == "PRI",
                    default_value: r.default,
                })
                .collect())
        }
        DatabasePool::Sqlite(p) => {
            #[derive(sqlx::FromRow)]
            struct SqliteColumnInfo {
                name: String,
                #[sqlx(rename = "type")]
                col_type: String,
                notnull: i32,
                dflt_value: Option<String>,
                pk: i32,
            }

            let rows = sqlx::query_as::<_, SqliteColumnInfo>(&format!(
                "PRAGMA table_info(\"{}\")",
                table.replace('"', "\"\"")
            ))
            .fetch_all(p)
            .await
            .map_err(|e| e.to_string())?;

            Ok(rows
                .into_iter()
                .map(|r| ColumnInfo {
                    name: r.name,
                    data_type: r.col_type,
                    is_nullable: r.notnull == 0,
                    is_primary_key: r.pk > 0,
                    default_value: r.dflt_value,
                })
                .collect())
        }
    }
}

// --- Saved connections commands ---

#[tauri::command]
pub async fn sql_save_connection(
    pool: State<'_, SqlitePool>,
    config: SqlConnectionConfig,
) -> Result<SqlSavedConnection, String> {
    let id = Uuid::new_v4().to_string();

    let max_order: (i32,) =
        sqlx::query_as("SELECT COALESCE(MAX(sort_order), -1) FROM sql_connections")
            .fetch_one(pool.inner())
            .await
            .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO sql_connections (id, name, driver, host, port, database_name, username, password, ssl, sort_order)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&config.name)
    .bind(&config.driver)
    .bind(&config.host)
    .bind(config.port as i32)
    .bind(&config.database)
    .bind(&config.username)
    .bind(&config.password)
    .bind(config.ssl as i32)
    .bind(max_order.0 + 1)
    .execute(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, SqlSavedConnection>("SELECT * FROM sql_connections WHERE id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sql_list_saved_connections(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<SqlSavedConnection>, String> {
    sqlx::query_as::<_, SqlSavedConnection>(
        "SELECT * FROM sql_connections ORDER BY sort_order ASC",
    )
    .fetch_all(pool.inner())
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sql_delete_saved_connection(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    sqlx::query("DELETE FROM sql_connections WHERE id = ?")
        .bind(&id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn sql_update_saved_connection(
    pool: State<'_, SqlitePool>,
    id: String,
    config: SqlConnectionConfig,
) -> Result<SqlSavedConnection, String> {
    sqlx::query(
        "UPDATE sql_connections SET name = ?, driver = ?, host = ?, port = ?, database_name = ?, username = ?, password = ?, ssl = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&config.name)
    .bind(&config.driver)
    .bind(&config.host)
    .bind(config.port as i32)
    .bind(&config.database)
    .bind(&config.username)
    .bind(&config.password)
    .bind(config.ssl as i32)
    .bind(&id)
    .execute(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, SqlSavedConnection>("SELECT * FROM sql_connections WHERE id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

// --- SQL Script commands ---

#[tauri::command]
pub async fn sql_save_script(
    pool: State<'_, SqlitePool>,
    name: String,
    connection_id: Option<String>,
    database_name: String,
    query: String,
) -> Result<SqlScript, String> {
    let id = Uuid::new_v4().to_string();

    let max_order: (i32,) =
        sqlx::query_as("SELECT COALESCE(MAX(sort_order), -1) FROM sql_scripts")
            .fetch_one(pool.inner())
            .await
            .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO sql_scripts (id, name, connection_id, database_name, query, sort_order)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&connection_id)
    .bind(&database_name)
    .bind(&query)
    .bind(max_order.0 + 1)
    .execute(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, SqlScript>("SELECT * FROM sql_scripts WHERE id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sql_list_scripts(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<SqlScript>, String> {
    sqlx::query_as::<_, SqlScript>(
        "SELECT * FROM sql_scripts ORDER BY sort_order ASC",
    )
    .fetch_all(pool.inner())
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sql_update_script(
    pool: State<'_, SqlitePool>,
    id: String,
    name: String,
    query: String,
    database_name: Option<String>,
) -> Result<SqlScript, String> {
    sqlx::query(
        "UPDATE sql_scripts SET name = ?, query = ?, database_name = COALESCE(?, database_name), updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&name)
    .bind(&query)
    .bind(&database_name)
    .bind(&id)
    .execute(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, SqlScript>("SELECT * FROM sql_scripts WHERE id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sql_delete_script(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    sqlx::query("DELETE FROM sql_scripts WHERE id = ?")
        .bind(&id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

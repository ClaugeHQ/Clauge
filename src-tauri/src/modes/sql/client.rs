use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use sqlx::{Column, Row, TypeInfo};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tauri::State;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::clickhouse_client::ClickhouseClient;
use super::dialects::{descriptor_for_key, SqlDialect};

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
    /// Optional SSH profile to tunnel through. When `Some`, `host`/`port`
    /// describe the *target DB host as seen from the bastion*; the runtime
    /// rewrites the URL to point at the local end of the tunnel.
    #[serde(default)]
    pub ssh_profile_id: Option<String>,
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
    /// When set, the runtime opens an SSH tunnel through the referenced
    /// `ssh_profiles.id` and connects the DB driver to the local end of the
    /// tunnel instead of `host:port` directly. NULL = no tunnel (legacy).
    pub ssh_profile_id: Option<String>,
}

// --- Connection pool enum ---

pub enum DatabasePool {
    Postgres(sqlx::PgPool),
    MySql(sqlx::MySqlPool),
    Sqlite(sqlx::SqlitePool),
    /// ClickHouse is HTTP-based and not sqlx-compatible; the variant holds
    /// a stateless `ClickhouseClient` that issues `FORMAT JSON` requests
    /// per query. There is no pool to close — `close()` is a no-op.
    Clickhouse(ClickhouseClient),
}

impl Clone for DatabasePool {
    fn clone(&self) -> Self {
        match self {
            Self::Postgres(p) => Self::Postgres(p.clone()),
            Self::MySql(p) => Self::MySql(p.clone()),
            Self::Sqlite(p) => Self::Sqlite(p.clone()),
            Self::Clickhouse(c) => Self::Clickhouse(c.clone()),
        }
    }
}

pub struct SqlConnectionManager {
    pub connections: Mutex<HashMap<String, DatabasePool>>,
    /// Parallel map keyed by the same id as `connections`, holding any
    /// SSH tunnel that backs the connection. Removing a pool also removes
    /// the matching tunnel — its `Drop` closes the listener + SSH session.
    /// Connections without a tunnel never appear here.
    pub tunnels: Mutex<HashMap<String, crate::modes::ssh::tunnel::SshTunnel>>,
}

impl SqlConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
        }
    }
}

// --- Helper to build connection strings ---
//
// The dispatch goes through the dialect registry, but each connection-string
// format stays inline because the syntaxes are inherently per-driver.

fn build_postgres_url(config: &SqlConnectionConfig) -> String {
    let ssl_mode = if config.ssl { "require" } else { "prefer" };
    format!(
        "postgres://{}:{}@{}:{}/{}?sslmode={}",
        config.username, config.password, config.host, config.port, config.database, ssl_mode
    )
}

fn build_mysql_url(config: &SqlConnectionConfig) -> String {
    let ssl_mode = if config.ssl { "REQUIRED" } else { "PREFERRED" };
    format!(
        "mysql://{}:{}@{}:{}/{}?ssl-mode={}",
        config.username, config.password, config.host, config.port, config.database, ssl_mode
    )
}

fn build_sqlite_url(config: &SqlConnectionConfig) -> String {
    format!("sqlite:{}?mode=rwc", config.database)
}

fn build_clickhouse_url(config: &SqlConnectionConfig) -> String {
    // ClickHouse uses an HTTP endpoint, not a libpq-style URL — this is
    // surfaced for diagnostics only; `create_pool` constructs the
    // `ClickhouseClient` directly from the config.
    let scheme = if config.ssl { "https" } else { "http" };
    let host = if config.host.is_empty() { "localhost" } else { config.host.as_str() };
    let port = if config.port == 0 { 8123 } else { config.port };
    format!("{}://{}:{}/?database={}", scheme, host, port, config.database)
}

fn build_connection_url(config: &SqlConnectionConfig) -> Result<String, String> {
    let descriptor = descriptor_for_key(&config.driver)
        .ok_or_else(|| format!("Unsupported driver: {}", config.driver))?;
    Ok(match descriptor.dialect {
        SqlDialect::Postgres => build_postgres_url(config),
        SqlDialect::MySql => build_mysql_url(config),
        SqlDialect::Sqlite => build_sqlite_url(config),
        SqlDialect::Clickhouse => build_clickhouse_url(config),
    })
}

/// Connect a SQL pool, optionally routing through an SSH tunnel.
///
/// When `config.ssh_profile_id` is `Some(_)` and an `app_pool` is provided,
/// we open an SSH tunnel via `modes::ssh::tunnel::open` and rewrite the
/// effective host/port to the local end of that tunnel. The returned
/// `Option<SshTunnel>` MUST be kept alive for the life of the pool; dropping
/// it will close the local listener and the bastion session, and the pool
/// will start failing connection attempts.
///
/// When the saved connection has no tunnel, behaviour is identical to the
/// pre-tunnel `create_pool` path — same URLs, same drivers, same logs.
pub async fn create_pool_with_tunnel(
    config: &SqlConnectionConfig,
    app_pool: Option<&SqlitePool>,
) -> Result<(DatabasePool, Option<crate::modes::ssh::tunnel::SshTunnel>), String> {
    let descriptor = descriptor_for_key(&config.driver)
        .ok_or_else(|| format!("Unsupported driver: {}", config.driver))?;

    // 1. Open the tunnel up-front (when requested). We use the *original*
    // host/port — that's what the bastion needs to connect to. Then we
    // build a tunneled config whose host/port are the local listener,
    // and pass that through the existing URL builder unchanged.
    let (effective_config, tunnel) = match (&config.ssh_profile_id, app_pool) {
        (Some(profile_id), Some(app_pool)) if !profile_id.is_empty() => {
            let t = crate::modes::ssh::tunnel::open(
                app_pool,
                profile_id,
                &config.host,
                config.port,
            )
            .await?;
            let mut tunneled = config.clone();
            tunneled.host = "127.0.0.1".to_string();
            tunneled.port = t.local_port;
            (tunneled, Some(t))
        }
        (Some(profile_id), None) if !profile_id.is_empty() => {
            // We need the app pool to look up the SSH profile — callers
            // must use `create_pool_with_tunnel` with `Some(app_pool)`.
            return Err(format!(
                "ssh_profile_id={} requires the app database pool to be provided",
                profile_id
            ));
        }
        _ => (config.clone(), None),
    };

    let pool = build_pool_inner(&effective_config, descriptor.dialect, app_pool).await?;
    Ok((pool, tunnel))
}

async fn build_pool_inner(
    config: &SqlConnectionConfig,
    dialect: SqlDialect,
    app_pool: Option<&SqlitePool>,
) -> Result<DatabasePool, String> {
    let url = build_connection_url(config)?;
    log::info!("[Clauge SQL] create_pool driver={} host={} port={} db={} ssl={} user={}",
        config.driver, config.host, config.port, config.database, config.ssl, config.username);
    match dialect {
        SqlDialect::Postgres => {
            use sqlx::postgres::{PgConnectOptions, PgSslMode};
            use std::str::FromStr;
            let base_url = format!(
                "postgres://{}:{}@{}:{}/{}",
                config.username, config.password, config.host, config.port, config.database
            );
            let ssl_mode = if config.ssl { PgSslMode::Require } else { PgSslMode::Prefer };
            log::info!("[Clauge SQL] PG base_url={} ssl_mode={:?}", base_url.replace(&config.password, "***"), ssl_mode);
            let mut opts = PgConnectOptions::from_str(&base_url)
                .map_err(|e| {
                    log::error!("[Clauge SQL] PgConnectOptions parse error: {}", e);
                    format!("Invalid PostgreSQL URL: {}", e)
                })?;
            opts = opts.ssl_mode(ssl_mode);
            log::info!("[Clauge SQL] Connecting to PostgreSQL...");
            let pool = sqlx::PgPool::connect_with(opts)
                .await
                .map_err(|e| {
                    log::error!("[Clauge SQL] PostgreSQL connection FAILED: {}", e);
                    format!("PostgreSQL connection failed: {}", e)
                })?;
            log::info!("[Clauge SQL] PostgreSQL connected OK");
            Ok(DatabasePool::Postgres(pool))
        }
        SqlDialect::MySql => {
            let pool = sqlx::MySqlPool::connect(&url)
                .await
                .map_err(|e| format!("MySQL connection failed: {}", e))?;
            Ok(DatabasePool::MySql(pool))
        }
        SqlDialect::Sqlite => {
            let pool = sqlx::SqlitePool::connect(&url)
                .await
                .map_err(|e| format!("SQLite connection failed: {}", e))?;
            Ok(DatabasePool::Sqlite(pool))
        }
        SqlDialect::Clickhouse => {
            // ClickhouseClient::new reads host/port from `config`, so the
            // tunneled config naturally points the HTTP base URL at
            // `127.0.0.1:<local_port>` when an SSH tunnel is in play.
            let client = ClickhouseClient::new(config, app_pool).await?;
            client
                .ping()
                .await
                .map_err(|e| format!("ClickHouse connection failed: {}", e))?;
            log::info!("[Clauge SQL] ClickHouse connected OK");
            Ok(DatabasePool::Clickhouse(client))
        }
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
    app_pool: State<'_, SqlitePool>,
    config: SqlConnectionConfig,
) -> Result<String, String> {
    let (pool, tunnel) = create_pool_with_tunnel(&config, Some(app_pool.inner())).await?;
    let connection_id = Uuid::new_v4().to_string();
    let mut connections = manager.connections.lock().await;
    connections.insert(connection_id.clone(), pool);
    if let Some(t) = tunnel {
        manager.tunnels.lock().await.insert(connection_id.clone(), t);
    }
    Ok(connection_id)
}

#[tauri::command]
pub async fn sql_connect_database(
    manager: State<'_, Arc<SqlConnectionManager>>,
    app_pool: State<'_, SqlitePool>,
    config: SqlConnectionConfig,
    database: String,
    pool_key: Option<String>,
) -> Result<String, String> {
    let mut db_config = config;
    db_config.database = database;
    let (pool, tunnel) = create_pool_with_tunnel(&db_config, Some(app_pool.inner())).await?;
    let key = pool_key.unwrap_or_else(|| Uuid::new_v4().to_string());
    let mut connections = manager.connections.lock().await;
    // If pool already exists under this key, close the old one first
    if let Some(old) = connections.remove(&key) {
        match old {
            DatabasePool::Postgres(p) => p.close().await,
            DatabasePool::MySql(p) => p.close().await,
            DatabasePool::Sqlite(p) => p.close().await,
            // ClickHouse client is stateless HTTP — nothing to close.
            DatabasePool::Clickhouse(_) => {}
        }
    }
    // Drop any prior tunnel under this key so the old SSH session closes
    // before we insert the new one. Using `remove` triggers Drop.
    let _ = manager.tunnels.lock().await.remove(&key);
    connections.insert(key.clone(), pool);
    if let Some(t) = tunnel {
        manager.tunnels.lock().await.insert(key.clone(), t);
    }
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
            DatabasePool::Clickhouse(_) => {}
        }
        // Drop any associated tunnel — its Drop closes the SSH session.
        let _ = manager.tunnels.lock().await.remove(&connection_id);
        Ok(())
    } else {
        Err("Connection not found".to_string())
    }
}

#[tauri::command]
pub async fn sql_test_connection(
    app_pool: State<'_, SqlitePool>,
    config: SqlConnectionConfig,
) -> Result<(), String> {
    let (pool, tunnel) = create_pool_with_tunnel(&config, Some(app_pool.inner())).await?;
    // Just connecting is enough; close immediately. Drop the tunnel after
    // the pool so any in-flight close packets still reach the server.
    match pool {
        DatabasePool::Postgres(p) => p.close().await,
        DatabasePool::MySql(p) => p.close().await,
        DatabasePool::Sqlite(p) => p.close().await,
        DatabasePool::Clickhouse(_) => {}
    }
    drop(tunnel);
    Ok(())
}

#[tauri::command]
pub async fn sql_execute_query(
    manager: State<'_, Arc<SqlConnectionManager>>,
    connection_id: String,
    query: String,
) -> Result<SqlQueryResult, String> {
    // Clone the pool reference and immediately release the mutex so other
    // commands (tab switching, AI panel, sidebar refreshes, concurrent
    // queries from other tabs) are not blocked for the duration of fetch_all.
    // All sqlx pool types and ClickhouseClient are cheaply cloneable (they
    // are internally reference-counted / stateless HTTP clients).
    let pool = {
        let connections = manager.connections.lock().await;
        connections
            .get(&connection_id)
            .ok_or_else(|| "Connection not found".to_string())?
            .clone()
    };

    let start = Instant::now();

    match pool {
        DatabasePool::Postgres(p) => {
            let rows = sqlx::query(&query)
                .fetch_all(&p)
                .await
                .map_err(|e| e.to_string())?;
            let duration_ms = start.elapsed().as_millis() as u64;
            // Column metadata only attaches to returned rows, so for a
            // zero-row SELECT we describe the statement separately to
            // recover the column list. Without this the UI can't tell an
            // empty SELECT apart from a no-data statement (BEGIN/COMMIT/
            // INSERT/etc.) and mislabels the result.
            let columns: Vec<String> = if rows.is_empty() {
                use sqlx::Executor;
                p.describe(&query)
                    .await
                    .map(|d| d.columns.iter().map(|c| c.name().to_string()).collect())
                    .unwrap_or_default()
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
                .fetch_all(&p)
                .await
                .map_err(|e| e.to_string())?;
            let duration_ms = start.elapsed().as_millis() as u64;
            let columns: Vec<String> = if rows.is_empty() {
                use sqlx::Executor;
                p.describe(&query)
                    .await
                    .map(|d| d.columns.iter().map(|c| c.name().to_string()).collect())
                    .unwrap_or_default()
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
                .fetch_all(&p)
                .await
                .map_err(|e| e.to_string())?;
            let duration_ms = start.elapsed().as_millis() as u64;
            let columns: Vec<String> = if rows.is_empty() {
                use sqlx::Executor;
                p.describe(&query)
                    .await
                    .map(|d| d.columns.iter().map(|c| c.name().to_string()).collect())
                    .unwrap_or_default()
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
        DatabasePool::Clickhouse(c) => {
            let result = c.query(&query).await?;
            let duration_ms = start.elapsed().as_millis() as u64;
            Ok(SqlQueryResult {
                columns: result.columns,
                rows: result.rows,
                affected_rows: result.affected,
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
        DatabasePool::Clickhouse(c) => {
            let result = c
                .query("SELECT name FROM system.databases ORDER BY name")
                .await?;
            Ok(result
                .rows
                .into_iter()
                .filter_map(|row| row.into_iter().next())
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect())
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
        DatabasePool::Clickhouse(c) => {
            // ClickHouse uses CREATE DATABASE; identifier quoted with backticks.
            let stmt = format!("CREATE DATABASE `{}`", name.replace('`', "``"));
            c.query(&stmt).await?;
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
        DatabasePool::Clickhouse(c) => {
            // ClickHouse has no separate "schema" concept — surface the
            // active database name as the only schema, mirroring SQLite.
            Ok(vec![c.database.clone()])
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
        DatabasePool::Clickhouse(c) => {
            // Pull both regular tables and views so the UI surfaces views
            // alongside tables — matches the other drivers' behaviour.
            let db_name = database.unwrap_or_else(|| c.database.clone());
            let safe_db = db_name.replace('\'', "''");
            let stmt = format!(
                "SELECT name, engine FROM system.tables WHERE database = '{}' ORDER BY name",
                safe_db
            );
            let result = c.query(&stmt).await?;
            Ok(result
                .rows
                .into_iter()
                .filter_map(|row| {
                    let mut it = row.into_iter();
                    let name = it.next()?.as_str().map(|s| s.to_string())?;
                    let engine = it
                        .next()
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_default();
                    let table_type = if engine.to_lowercase().contains("view") {
                        "VIEW".to_string()
                    } else {
                        "TABLE".to_string()
                    };
                    Some(TableInfo { name, table_type, row_count: None })
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
        DatabasePool::Clickhouse(c) => {
            // ClickHouse exposes column metadata via `system.columns`.
            // Nullability is encoded in the type string (`Nullable(T)`),
            // and primary-key columns are listed in `is_in_primary_key`.
            let db_name = schema.unwrap_or_else(|| c.database.clone());
            let safe_db = db_name.replace('\'', "''");
            let safe_table = table.replace('\'', "''");
            let stmt = format!(
                "SELECT name, type, default_expression, is_in_primary_key \
                 FROM system.columns \
                 WHERE database = '{}' AND table = '{}' \
                 ORDER BY position",
                safe_db, safe_table
            );
            let result = c.query(&stmt).await?;
            Ok(result
                .rows
                .into_iter()
                .filter_map(|row| {
                    let mut it = row.into_iter();
                    let name = it.next()?.as_str().map(|s| s.to_string())?;
                    let data_type = it
                        .next()
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_default();
                    let default_expr = it
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
                    let is_nullable = data_type.starts_with("Nullable(");
                    Some(ColumnInfo {
                        name,
                        data_type,
                        is_nullable,
                        is_primary_key: is_pk,
                        default_value: default_expr,
                    })
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
    use crate::shared::repos::sql_connections as sql_conn_repo;
    let id = Uuid::new_v4().to_string();

    let max_order = sql_conn_repo::max_sort_order(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    sql_conn_repo::insert(
        pool.inner(),
        &id,
        &config.name,
        &config.driver,
        &config.host,
        config.port as i32,
        &config.database,
        &config.username,
        &config.password,
        config.ssl as i32,
        max_order.0 + 1,
        config.ssh_profile_id.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("sql");

    sql_conn_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sql_list_saved_connections(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<SqlSavedConnection>, String> {
    use crate::shared::repos::sql_connections as sql_conn_repo;
    sql_conn_repo::list_all(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sql_delete_saved_connection(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    use crate::shared::repos::sql_connections as sql_conn_repo;
    sql_conn_repo::delete_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("sql");
    Ok(())
}

#[tauri::command]
pub async fn sql_update_saved_connection(
    pool: State<'_, SqlitePool>,
    id: String,
    config: SqlConnectionConfig,
) -> Result<SqlSavedConnection, String> {
    use crate::shared::repos::sql_connections as sql_conn_repo;
    sql_conn_repo::update(
        pool.inner(),
        &id,
        &config.name,
        &config.driver,
        &config.host,
        config.port as i32,
        &config.database,
        &config.username,
        &config.password,
        config.ssl as i32,
        config.ssh_profile_id.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("sql");

    sql_conn_repo::get_by_id(pool.inner(), &id)
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
    use crate::shared::repos::sql_connections as sql_conn_repo;
    let id = Uuid::new_v4().to_string();

    let max_order = sql_conn_repo::max_script_sort_order(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    sql_conn_repo::insert_script(
        pool.inner(),
        &id,
        &name,
        connection_id.as_deref(),
        &database_name,
        &query,
        max_order.0 + 1,
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("sql");

    sql_conn_repo::get_script_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sql_list_scripts(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<SqlScript>, String> {
    use crate::shared::repos::sql_connections as sql_conn_repo;
    sql_conn_repo::list_scripts(pool.inner())
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
    use crate::shared::repos::sql_connections as sql_conn_repo;
    sql_conn_repo::update_script(pool.inner(), &id, &name, &query, database_name.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("sql");

    sql_conn_repo::get_script_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sql_delete_script(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    use crate::shared::repos::sql_connections as sql_conn_repo;
    sql_conn_repo::delete_script(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("sql");
    Ok(())
}

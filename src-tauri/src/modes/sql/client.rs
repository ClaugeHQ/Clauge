use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use sqlx::{Column, Either, Row, TypeInfo};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{oneshot, Mutex, Semaphore};
use uuid::Uuid;

use super::clickhouse_client::ClickhouseClient;
use super::d1_client::D1Client;
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

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    /// Returns rows (SELECT, SHOW, EXPLAIN, WITH, PRAGMA, VALUES, …).
    Select,
    /// Side-effect query (INSERT, UPDATE, DELETE, MERGE, TRUNCATE, …).
    Dml,
    /// Schema / session statement (CREATE, DROP, ALTER, SET, BEGIN, …).
    Ddl,
    /// First verb didn't match anything we know. Treated like SELECT so we
    /// still try to fetch rows; the driver decides what comes back.
    Unknown,
}

/// Classify a query by its leading verb. Strips `--` line comments and
/// `/* … */` block comments before reading the first word. Used to decide
/// whether to call `fetch_all` (rows path) or `execute` (rows_affected path)
/// — sqlx's `fetch_all` reports 0 affected for DML, so DML/DDL must take
/// the execute path or the result-card lies to the user.
pub fn classify_query(q: &str) -> QueryKind {
    let mut s = q.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            let end = rest.find('\n').map(|i| i + 1).unwrap_or(rest.len());
            s = rest[end..].trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("/*") {
            if let Some(close) = rest.find("*/") {
                s = rest[close + 2..].trim_start();
                continue;
            }
            break;
        }
        break;
    }
    let first = s.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    match first.as_str() {
        "SELECT" | "WITH" | "SHOW" | "EXPLAIN" | "DESCRIBE" | "DESC"
        | "VALUES" | "PRAGMA" | "TABLE" => QueryKind::Select,
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "REPLACE" | "CALL"
        | "TRUNCATE" | "COPY" => QueryKind::Dml,
        "CREATE" | "DROP" | "ALTER" | "RENAME" | "GRANT" | "REVOKE"
        | "COMMENT" | "USE" | "SET" | "BEGIN" | "COMMIT" | "ROLLBACK"
        | "SAVEPOINT" | "RELEASE" | "VACUUM" | "ANALYZE" | "CLUSTER"
        | "REINDEX" | "ATTACH" | "DETACH" => QueryKind::Ddl,
        _ => QueryKind::Unknown,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub affected_rows: u64,
    pub duration_ms: u64,
    pub query_kind: QueryKind,
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
    /// Cloudflare D1 — HTTPS-only, SQLite-flavoured. Stateless reqwest
    /// wrapper; `close()` is a no-op.
    D1(D1Client),
}

impl Clone for DatabasePool {
    fn clone(&self) -> Self {
        match self {
            Self::Postgres(p) => Self::Postgres(p.clone()),
            Self::MySql(p) => Self::MySql(p.clone()),
            Self::Sqlite(p) => Self::Sqlite(p.clone()),
            Self::Clickhouse(c) => Self::Clickhouse(c.clone()),
            Self::D1(c) => Self::D1(c.clone()),
        }
    }
}

pub struct SqlConnectionManager {
    /// Pool keyspace: `"{savedConnectionId}:{database}"`. One pool per
    /// `(connectionId, database)` pair. Replaces the previous split
    /// between random-UUID instance pools and per-DB pools.
    pub connections: Mutex<HashMap<String, DatabasePool>>,
    /// Parallel map keyed by the same id as `connections`, holding any
    /// SSH tunnel that backs the connection. Removing a pool also removes
    /// the matching tunnel — its `Drop` closes the listener + SSH session.
    pub tunnels: Mutex<HashMap<String, crate::modes::ssh::tunnel::SshTunnel>>,
    /// Per-pool semaphore. 3 concurrent queries allowed across all tabs
    /// sharing the same `(conn, db)`. 4th `try_acquire` returns immediately
    /// as "Connection busy" rather than queueing.
    pub permits: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// In-flight queries keyed by caller-supplied `query_id`. Drives
    /// cooperative cancel via `oneshot` + server-side kill via `KillHandle`.
    pub in_flight: Mutex<HashMap<String, InFlight>>,
}

pub struct InFlight {
    pub pool_key: String,
    pub cancel: oneshot::Sender<()>,
    pub kill: KillHandle,
}

pub enum KillHandle {
    Postgres { backend_pid: i32 },
    MySql { connection_id: u64 },
    Clickhouse { query_id: String },
    Sqlite,
    D1,
}

impl SqlConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
            permits: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    pub async fn permit_for(&self, pool_key: &str) -> Arc<Semaphore> {
        let mut g = self.permits.lock().await;
        g.entry(pool_key.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(3)))
            .clone()
    }

    pub async fn register_in_flight(&self, query_id: String, info: InFlight) {
        self.in_flight.lock().await.insert(query_id, info);
    }

    pub async fn deregister_in_flight(&self, query_id: &str) -> Option<InFlight> {
        self.in_flight.lock().await.remove(query_id)
    }
}

pub fn pool_key(conn_id: &str, database: &str) -> String {
    format!("{}:{}", conn_id, database)
}

/// Heuristic to recognise dead-socket / closed-pool errors that warrant
/// a one-shot pool rebuild + retry. Matches sqlx + reqwest text. Errs on
/// the side of NOT retrying — false negatives just surface the error to
/// the user; false positives could mask real query errors.
pub fn looks_like_dead_connection(err: &str) -> bool {
    let s = err.to_lowercase();
    s.contains("connection closed")
        || s.contains("connection reset")
        || s.contains("broken pipe")
        || s.contains("server closed")
        || s.contains("tunnel")
        || s.contains("communication link failure")
        || s.contains("poolclosed")
        || s.contains("pool timed out")
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

fn build_d1_url(config: &SqlConnectionConfig) -> String {
    // D1 reuses `host` as account_id and `database` as database_id (see
    // d1_client.rs). The URL is surfaced for diagnostics / logging only;
    // `create_pool` constructs `D1Client` directly from the config.
    format!(
        "https://api.cloudflare.com/client/v4/accounts/{}/d1/database/{}/query",
        config.host, config.database
    )
}

fn build_connection_url(config: &SqlConnectionConfig) -> Result<String, String> {
    let descriptor = descriptor_for_key(&config.driver)
        .ok_or_else(|| format!("Unsupported driver: {}", config.driver))?;
    Ok(match descriptor.dialect {
        SqlDialect::Postgres => build_postgres_url(config),
        SqlDialect::MySql => build_mysql_url(config),
        SqlDialect::Sqlite => build_sqlite_url(config),
        SqlDialect::Clickhouse => build_clickhouse_url(config),
        SqlDialect::D1 => build_d1_url(config),
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

    // User-configurable pool timeouts (Settings → SQL). Fall back to
    // 10s acquire / 30min idle when the app_pool isn't available (e.g.
    // some unit-test paths) or the setting is missing/malformed.
    let (acquire_timeout, idle_timeout) = match app_pool {
        Some(p) => {
            use crate::shared::repos::settings;
            let acquire_ms = settings::get_u64_or(p, "sql_acquire_timeout_ms", 10_000).await;
            let idle_min = settings::get_u64_or(p, "sql_idle_timeout_min", 30).await;
            (Duration::from_millis(acquire_ms), Duration::from_secs(idle_min * 60))
        }
        None => (Duration::from_secs(10), Duration::from_secs(30 * 60)),
    };

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
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(5)
                .min_connections(1)
                .acquire_timeout(acquire_timeout)
                .idle_timeout(Some(idle_timeout))
                .max_lifetime(Some(Duration::from_secs(2 * 60 * 60)))
                // Don't ping before every acquire — adds a roundtrip per
                // query (especially painful over SSH tunnels). We catch
                // dead pools post-hoc via `with_reconnect`'s retry path.
                .test_before_acquire(false)
                .connect_with(opts)
                .await
                .map_err(|e| {
                    log::error!("[Clauge SQL] PostgreSQL connection FAILED: {}", e);
                    format!("PostgreSQL connection failed: {}", e)
                })?;
            log::info!("[Clauge SQL] PostgreSQL connected OK");
            Ok(DatabasePool::Postgres(pool))
        }
        SqlDialect::MySql => {
            let pool = sqlx::mysql::MySqlPoolOptions::new()
                .max_connections(5)
                .min_connections(1)
                .acquire_timeout(acquire_timeout)
                .idle_timeout(Some(idle_timeout))
                .max_lifetime(Some(Duration::from_secs(2 * 60 * 60)))
                // Don't ping before every acquire — adds a roundtrip per
                // query (especially painful over SSH tunnels). We catch
                // dead pools post-hoc via `with_reconnect`'s retry path.
                .test_before_acquire(false)
                .connect(&url)
                .await
                .map_err(|e| format!("MySQL connection failed: {}", e))?;
            Ok(DatabasePool::MySql(pool))
        }
        SqlDialect::Sqlite => {
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(5)
                .min_connections(1)
                .acquire_timeout(acquire_timeout)
                .idle_timeout(Some(idle_timeout))
                .max_lifetime(Some(Duration::from_secs(2 * 60 * 60)))
                // Don't ping before every acquire — adds a roundtrip per
                // query (especially painful over SSH tunnels). We catch
                // dead pools post-hoc via `with_reconnect`'s retry path.
                .test_before_acquire(false)
                .connect(&url)
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
        SqlDialect::D1 => {
            // D1 is always HTTPS to api.cloudflare.com — SSH tunneling
            // would not help and isn't surfaced in the dialog. We ignore
            // any `ssh_profile_id` here.
            let client = D1Client::new(config, app_pool).await?;
            client
                .ping()
                .await
                .map_err(|e| format!("D1 connection failed: {}", e))?;
            log::info!("[Clauge SQL] D1 connected OK");
            Ok(DatabasePool::D1(client))
        }
    }
}

// --- Helper to convert rows to JSON values ---

pub(crate) fn pg_row_to_json(row: &sqlx::postgres::PgRow) -> Vec<serde_json::Value> {
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
                // Arrays. Dispatch by element type (the part after the
                // leading '_' in the Postgres type name) and decode via
                // sqlx's native Vec<T> impl, then serialize as a JSON
                // array. The previous "try as Option<String>" catch-all
                // failed for every array column — sqlx's wire format
                // for arrays isn't TEXT — so cells silently rendered as
                // NULL even when the row had values.
                t if t.starts_with('_') => {
                    // Each arm: try_get as Vec<T>, map elements to JSON,
                    // wrap in Value::Array. Mirrors the scalar branches
                    // above 1:1 for which T to use per element type.
                    macro_rules! arr {
                        ($T:ty, $conv:expr) => {
                            row.try_get::<Option<Vec<$T>>, _>(idx)
                                .ok()
                                .flatten()
                                .map(|v| serde_json::Value::Array(
                                    v.into_iter().map($conv).collect()
                                ))
                                .unwrap_or(serde_json::Value::Null)
                        };
                    }
                    match &t[1..] {
                        "BOOL"      => arr!(bool, serde_json::Value::Bool),
                        "INT2"      => arr!(i16, |n| serde_json::json!(n)),
                        "INT4" | "OID" => arr!(i32, |n| serde_json::json!(n)),
                        "INT8"      => arr!(i64, |n| serde_json::json!(n)),
                        "FLOAT4"    => arr!(f32, |n| serde_json::json!(n)),
                        "FLOAT8"    => arr!(f64, |n| serde_json::json!(n)),
                        "NUMERIC" | "DECIMAL" => arr!(rust_decimal::Decimal,
                            |d: rust_decimal::Decimal| serde_json::Value::String(d.to_string())),
                        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CHAR" | "CITEXT" =>
                            arr!(String, serde_json::Value::String),
                        "UUID"      => arr!(uuid::Uuid,
                            |u: uuid::Uuid| serde_json::Value::String(u.to_string())),
                        "DATE"      => arr!(chrono::NaiveDate,
                            |d: chrono::NaiveDate| serde_json::Value::String(d.to_string())),
                        "TIME"      => arr!(chrono::NaiveTime,
                            |t: chrono::NaiveTime| serde_json::Value::String(t.to_string())),
                        "TIMESTAMP" => arr!(chrono::NaiveDateTime,
                            |t: chrono::NaiveDateTime| serde_json::Value::String(t.to_string())),
                        "TIMESTAMPTZ" => arr!(chrono::DateTime<chrono::Utc>,
                            |t: chrono::DateTime<chrono::Utc>| serde_json::Value::String(t.to_rfc3339())),
                        "JSON" | "JSONB" => arr!(serde_json::Value, |v| v),
                        "BYTEA"     => arr!(Vec<u8>, |b: Vec<u8>| {
                            let hex: String = b.iter().map(|c| format!("{:02x}", c)).collect();
                            serde_json::Value::String(format!("\\x{}", hex))
                        }),
                        // _inet, _cidr, _macaddr, _interval, _money, _bit,
                        // _varbit, _xml, _timetz, geometric arrays, ranges —
                        // sqlx doesn't ship a built-in Vec<T> for these.
                        // Try Vec<String> in case the driver hands us a
                        // usable text rep; otherwise NULL. Better than the
                        // previous catch-all which dropped every array.
                        _ => row.try_get::<Option<Vec<String>>, _>(idx)
                            .ok().flatten()
                            .map(|v| serde_json::Value::Array(
                                v.into_iter().map(serde_json::Value::String).collect()
                            ))
                            .unwrap_or(serde_json::Value::Null),
                    }
                },
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

// MySQL's information_schema and SHOW commands can return name columns as
// either VARCHAR or VARBINARY depending on the server's character set and
// collation settings (e.g. utf8mb4_bin or lower_case_table_names=0). Decode
// as String first; if the column came back binary, fall back to bytes and
// lossy-convert. Never panics, never bubbles a decoding error to the user.
pub fn mysql_decode_string(row: &sqlx::mysql::MySqlRow, idx: usize) -> String {
    if let Ok(s) = row.try_get::<String, _>(idx) {
        return s;
    }
    if let Ok(bytes) = row.try_get::<Vec<u8>, _>(idx) {
        return String::from_utf8_lossy(&bytes).into_owned();
    }
    String::new()
}

pub(crate) fn mysql_row_to_json(row: &sqlx::mysql::MySqlRow) -> Vec<serde_json::Value> {
    use sqlx::types::chrono;
    let columns = row.columns();
    columns
        .iter()
        .map(|col| {
            let idx = col.ordinal();
            let type_name = col.type_info().name();
            match type_name {
                "BOOLEAN" | "TINYINT(1)" => row
                    .try_get::<bool, _>(idx)
                    .map(serde_json::Value::Bool)
                    .unwrap_or(serde_json::Value::Null),
                "TINYINT" | "SMALLINT" | "INT" | "MEDIUMINT" => row
                    .try_get::<i32, _>(idx)
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                // UNSIGNED variants: sqlx-mysql refuses to decode an UNSIGNED
                // column into a signed type, so these used to return NULL.
                // information_schema.tables.TABLE_ROWS is BIGINT UNSIGNED —
                // that bug surfaced as "row counts always NULL" in user-facing
                // diagnostics.
                "TINYINT UNSIGNED" | "SMALLINT UNSIGNED" | "MEDIUMINT UNSIGNED" | "INT UNSIGNED" => row
                    .try_get::<u32, _>(idx)
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "BIGINT" => row
                    .try_get::<i64, _>(idx)
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "BIGINT UNSIGNED" => row
                    .try_get::<u64, _>(idx)
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "FLOAT" => row
                    .try_get::<f32, _>(idx)
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                "DOUBLE" => row
                    .try_get::<f64, _>(idx)
                    .map(|v| serde_json::json!(v))
                    .unwrap_or(serde_json::Value::Null),
                // DECIMAL was f64 → silent precision loss on monetary columns.
                // Use rust_decimal → String, same approach as Postgres.
                "DECIMAL" => row
                    .try_get::<rust_decimal::Decimal, _>(idx)
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null),
                "DATETIME" => row
                    .try_get::<chrono::NaiveDateTime, _>(idx)
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null),
                "TIMESTAMP" => row
                    .try_get::<chrono::DateTime<chrono::Utc>, _>(idx)
                    .map(|v| serde_json::Value::String(v.to_rfc3339()))
                    .or_else(|_| {
                        row.try_get::<chrono::NaiveDateTime, _>(idx)
                            .map(|v| serde_json::Value::String(v.to_string()))
                    })
                    .unwrap_or(serde_json::Value::Null),
                "DATE" => row
                    .try_get::<chrono::NaiveDate, _>(idx)
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null),
                "TIME" => row
                    .try_get::<chrono::NaiveTime, _>(idx)
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null),
                "JSON" => row
                    .try_get::<serde_json::Value, _>(idx)
                    .unwrap_or(serde_json::Value::Null),
                // Binary-storage types. information_schema columns declared as
                // VARCHAR are stored as VARBINARY underneath; sqlx reports the
                // underlying type, so a naive String decode fails and the row
                // appears NULL. Route through mysql_decode_string (handles
                // both String and Vec<u8> paths cleanly).
                "VARBINARY" | "BINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB" => {
                    let s = mysql_decode_string(row, idx);
                    serde_json::Value::String(s)
                }
                // Catch-all for text/enum/set/year/other: same defensive
                // String→Vec<u8> fallback. If the row genuinely has NULL,
                // mysql_decode_string returns "" — distinguish that by
                // checking try_get for null first.
                _ => {
                    if row.try_get::<Option<String>, _>(idx).map(|o| o.is_none()).unwrap_or(false) {
                        return serde_json::Value::Null;
                    }
                    serde_json::Value::String(mysql_decode_string(row, idx))
                }
            }
        })
        .collect()
}

pub(crate) fn sqlite_row_to_json(row: &sqlx::sqlite::SqliteRow) -> Vec<serde_json::Value> {
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

/// Build a `SqlConnectionConfig` from the saved-connection record.
async fn load_saved_config(
    app_pool: &SqlitePool,
    conn_id: &str,
    database_override: &str,
) -> Result<SqlConnectionConfig, String> {
    let saved = crate::shared::repos::sql_connections::get_by_id_optional(app_pool, conn_id)
        .await
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| format!("Saved connection '{}' not found", conn_id))?;

    // For Postgres/MySQL/ClickHouse one connection profile can address many
    // databases on the same server, so the tree node's database name is the
    // authoritative target. For SQLite the saved `database` is the file path
    // (and SQLite only has one logical database per file — "main"); for D1
    // it's the immutable database UUID. Overriding either with the synthetic
    // tree name ("main") points the pool at the wrong file/database, so we
    // keep the saved value for those drivers.
    let driver_lc = saved.driver.to_lowercase();
    let database = match driver_lc.as_str() {
        "sqlite" | "d1" => saved.database_name.clone(),
        _ => database_override.to_string(),
    };

    Ok(SqlConnectionConfig {
        name: saved.name,
        driver: saved.driver,
        host: saved.host,
        port: saved.port as u16,
        database,
        username: saved.username,
        password: saved.password,
        ssl: saved.ssl == 1,
        ssh_profile_id: saved.ssh_profile_id,
    })
}

async fn close_db_pool(pool: DatabasePool) {
    match pool {
        DatabasePool::Postgres(p) => p.close().await,
        DatabasePool::MySql(p) => p.close().await,
        DatabasePool::Sqlite(p) => p.close().await,
        DatabasePool::Clickhouse(_) | DatabasePool::D1(_) => {}
    }
}

/// Idempotent pool opener. Returns immediately if a pool for
/// `(conn_id, database)` already exists; otherwise builds one (including
/// SSH tunnel if the saved record references one) and stores it.
pub async fn ensure_pool_inner(
    manager: &Arc<SqlConnectionManager>,
    app_pool: &SqlitePool,
    conn_id: &str,
    database: &str,
) -> Result<String, String> {
    let key = pool_key(conn_id, database);
    {
        let conns = manager.connections.lock().await;
        if conns.contains_key(&key) {
            return Ok(key);
        }
    }
    let config = load_saved_config(app_pool, conn_id, database).await?;
    let (pool, tunnel) = create_pool_with_tunnel(&config, Some(app_pool)).await?;
    // Race-safe insert — if another caller raced ahead, drop the duplicate.
    let mut conns = manager.connections.lock().await;
    if conns.contains_key(&key) {
        drop(pool);
        drop(tunnel);
        return Ok(key);
    }
    conns.insert(key.clone(), pool);
    if let Some(t) = tunnel {
        manager.tunnels.lock().await.insert(key.clone(), t);
    }
    log::info!("[Clauge SQL] pool opened: {}", key);
    Ok(key)
}

#[tauri::command]
pub async fn sql_ensure_pool(
    manager: State<'_, Arc<SqlConnectionManager>>,
    app_pool: State<'_, SqlitePool>,
    conn_id: String,
    database: String,
) -> Result<String, String> {
    ensure_pool_inner(manager.inner(), app_pool.inner(), &conn_id, &database).await
}

#[tauri::command]
pub async fn sql_disconnect_pool(
    manager: State<'_, Arc<SqlConnectionManager>>,
    conn_id: String,
    database: String,
) -> Result<(), String> {
    let key = pool_key(&conn_id, &database);
    let removed = {
        let mut conns = manager.connections.lock().await;
        conns.remove(&key)
    };
    if let Some(pool) = removed {
        close_db_pool(pool).await;
    }
    let _ = manager.tunnels.lock().await.remove(&key);
    let _ = manager.permits.lock().await.remove(&key);
    Ok(())
}

#[tauri::command]
pub async fn sql_disconnect_connection(
    manager: State<'_, Arc<SqlConnectionManager>>,
    conn_id: String,
) -> Result<(), String> {
    let prefix = format!("{}:", conn_id);
    let drained: Vec<(String, DatabasePool)> = {
        let mut conns = manager.connections.lock().await;
        let keys: Vec<String> = conns.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        keys.into_iter()
            .filter_map(|k| conns.remove(&k).map(|p| (k, p)))
            .collect()
    };
    for (key, pool) in drained {
        close_db_pool(pool).await;
        let _ = manager.tunnels.lock().await.remove(&key);
        let _ = manager.permits.lock().await.remove(&key);
    }
    Ok(())
}

#[tauri::command]
pub async fn sql_cancel_query(
    manager: State<'_, Arc<SqlConnectionManager>>,
    query_id: String,
) -> Result<(), String> {
    let Some(info) = manager.deregister_in_flight(&query_id).await else {
        return Ok(()); // already completed
    };
    // Signal the awaiting select! to unwind first.
    let _ = info.cancel.send(());
    // Best-effort server-side kill on a side acquisition from the same pool.
    let pool_clone = {
        let conns = manager.connections.lock().await;
        conns.get(&info.pool_key).cloned()
    };
    match (info.kill, pool_clone) {
        (KillHandle::Postgres { backend_pid }, Some(DatabasePool::Postgres(p))) => {
            if let Err(e) = sqlx::query("SELECT pg_cancel_backend($1)")
                .bind(backend_pid)
                .execute(&p)
                .await
            {
                log::warn!("[Clauge SQL] pg_cancel_backend failed: {}", e);
            }
        }
        (KillHandle::MySql { connection_id }, Some(DatabasePool::MySql(p))) => {
            if let Err(e) = sqlx::query(&format!("KILL QUERY {}", connection_id))
                .execute(&p)
                .await
            {
                log::warn!("[Clauge SQL] KILL QUERY failed: {}", e);
            }
        }
        (KillHandle::Clickhouse { query_id: ch_qid }, Some(DatabasePool::Clickhouse(c))) => {
            let safe = ch_qid.replace('\'', "''");
            let stmt = format!("KILL QUERY WHERE query_id='{}' SYNC", safe);
            if let Err(e) = c.exec(&stmt).await {
                log::warn!("[Clauge SQL] KILL QUERY WHERE failed: {}", e);
            }
        }
        // SQLite + D1 + missing-pool cases: drop-future is the only mechanism.
        _ => {}
    }
    Ok(())
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
        DatabasePool::D1(_) => {}
    }
    drop(tunnel);
    Ok(())
}

/// Run the actual query against an already-checked-out pool. Pure dispatch
/// by dialect; no in-flight bookkeeping. Postgres + MySQL receive a
/// `PoolConnection` so the caller can attach `pg_cancel_backend(pid)` /
/// `KILL QUERY conn_id` on the same physical connection.
/// Derive a UI hint for which result panel to show, using empirical signals
/// first and the verb classifier only as a last-resort tie-breaker.
///
/// Empirical wins because it catches the cases hardcoded verb lists miss —
/// CTE-led DML (`WITH … INSERT …`), `INSERT … RETURNING *`, stored procs,
/// vendor-specific verbs, etc. The verb fallback only fires for "nothing
/// came back" results where we genuinely can't tell empty-SELECT from DDL.
fn infer_query_kind(query: &str, rows_len: usize, columns_len: usize, affected: u64) -> QueryKind {
    if rows_len > 0 {
        return QueryKind::Select;
    }
    if affected > 0 {
        return QueryKind::Dml;
    }
    if columns_len > 0 {
        // No rows + no affected + columns known → empty SELECT result set.
        return QueryKind::Select;
    }
    // Nothing came back. Fall back to the verb hint to disambiguate
    // empty-SELECT (describe failed) from DDL.
    classify_query(query)
}

async fn run_query_pg(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    query: &str,
) -> Result<SqlQueryResult, String> {
    let start = Instant::now();
    let mut json_rows: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut columns: Vec<String> = Vec::new();
    let mut affected: u64 = 0;
    {
        let mut stream = sqlx::query(query).fetch_many(&mut **conn);
        while let Some(step) = stream.try_next().await.map_err(|e| e.to_string())? {
            match step {
                Either::Left(qr) => affected += qr.rows_affected(),
                Either::Right(row) => {
                    if columns.is_empty() {
                        columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                    }
                    json_rows.push(pg_row_to_json(&row));
                }
            }
        }
    }
    // If nothing came back at all, try describe() to recover column names —
    // happens for `SELECT … WHERE 1=0`-style empty result sets.
    if json_rows.is_empty() && columns.is_empty() {
        use sqlx::Executor;
        if let Ok(d) = (&mut **conn).describe(query).await {
            columns = d.columns.iter().map(|c| c.name().to_string()).collect();
        }
    }
    let kind = infer_query_kind(query, json_rows.len(), columns.len(), affected);
    Ok(SqlQueryResult {
        columns,
        rows: json_rows,
        affected_rows: affected,
        duration_ms: start.elapsed().as_millis() as u64,
        query_kind: kind,
    })
}

async fn run_query_mysql(
    conn: &mut sqlx::pool::PoolConnection<sqlx::MySql>,
    query: &str,
) -> Result<SqlQueryResult, String> {
    let start = Instant::now();
    let mut json_rows: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut columns: Vec<String> = Vec::new();
    let mut affected: u64 = 0;
    {
        let mut stream = sqlx::query(query).fetch_many(&mut **conn);
        while let Some(step) = stream.try_next().await.map_err(|e| e.to_string())? {
            match step {
                Either::Left(qr) => affected += qr.rows_affected(),
                Either::Right(row) => {
                    if columns.is_empty() {
                        columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                    }
                    json_rows.push(mysql_row_to_json(&row));
                }
            }
        }
    }
    if json_rows.is_empty() && columns.is_empty() {
        use sqlx::Executor;
        if let Ok(d) = (&mut **conn).describe(query).await {
            columns = d.columns.iter().map(|c| c.name().to_string()).collect();
        }
    }
    let kind = infer_query_kind(query, json_rows.len(), columns.len(), affected);
    Ok(SqlQueryResult {
        columns,
        rows: json_rows,
        affected_rows: affected,
        duration_ms: start.elapsed().as_millis() as u64,
        query_kind: kind,
    })
}

async fn run_query_sqlite(
    p: &sqlx::SqlitePool,
    query: &str,
) -> Result<SqlQueryResult, String> {
    let start = Instant::now();
    let mut stream = sqlx::query(query).fetch_many(p);
    let mut json_rows: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut columns: Vec<String> = Vec::new();
    let mut affected: u64 = 0;
    while let Some(step) = stream.try_next().await.map_err(|e| e.to_string())? {
        match step {
            Either::Left(qr) => affected += qr.rows_affected(),
            Either::Right(row) => {
                if columns.is_empty() {
                    columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                }
                json_rows.push(sqlite_row_to_json(&row));
            }
        }
    }
    if json_rows.is_empty() && columns.is_empty() {
        use sqlx::Executor;
        if let Ok(d) = p.describe(query).await {
            columns = d.columns.iter().map(|c| c.name().to_string()).collect();
        }
    }
    let kind = infer_query_kind(query, json_rows.len(), columns.len(), affected);
    Ok(SqlQueryResult {
        columns,
        rows: json_rows,
        affected_rows: affected,
        duration_ms: start.elapsed().as_millis() as u64,
        query_kind: kind,
    })
}

#[tauri::command]
pub async fn sql_execute_query(
    app: AppHandle,
    manager: State<'_, Arc<SqlConnectionManager>>,
    app_pool: State<'_, SqlitePool>,
    conn_id: String,
    database: String,
    query: String,
    query_id: String,
) -> Result<SqlQueryResult, String> {
    let key = pool_key(&conn_id, &database);
    ensure_pool_inner(manager.inner(), app_pool.inner(), &conn_id, &database).await?;

    // Per-pool concurrency cap. try_acquire fails immediately if 3 queries
    // are already in flight on the same `(conn, db)` — anti-spam protection.
    let sem = manager.permit_for(&key).await;
    let _permit = sem
        .clone()
        .try_acquire_owned()
        .map_err(|_| "Connection busy — wait for a query to finish or cancel one".to_string())?;

    // First attempt. If it errors with a dead-pool signature, rebuild the
    // pool ONCE and retry — emits `sql:reconnecting` so the UI can flash
    // an amber badge on the affected tab.
    match execute_once(&app, manager.inner(), &key, &query, &query_id).await {
        Ok(r) => Ok(r),
        Err(e) if looks_like_dead_connection(&e) => {
            log::warn!("[Clauge SQL] dead-pool detected ({}), rebuilding {}", e, key);
            // Drop pool + tunnel; ensure_pool_inner will rebuild.
            {
                let mut conns = manager.connections.lock().await;
                if let Some(p) = conns.remove(&key) {
                    close_db_pool(p).await;
                }
            }
            let _ = manager.tunnels.lock().await.remove(&key);
            let _ = app.emit("sql:reconnecting", &key);
            ensure_pool_inner(manager.inner(), app_pool.inner(), &conn_id, &database).await?;
            execute_once(&app, manager.inner(), &key, &query, &query_id).await
        }
        Err(e) => Err(e),
    }
}

/// One attempt at executing a query — captures the dialect-specific kill
/// handle, registers `InFlight`, races the query against a cancel oneshot,
/// and tears down its bookkeeping on every exit path.
async fn execute_once(
    _app: &AppHandle,
    manager: &Arc<SqlConnectionManager>,
    key: &str,
    query: &str,
    query_id: &str,
) -> Result<SqlQueryResult, String> {
    let pool = {
        let conns = manager.connections.lock().await;
        conns
            .get(key)
            .ok_or_else(|| "Connection not found".to_string())?
            .clone()
    };

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

    let result = match pool {
        DatabasePool::Postgres(p) => {
            let mut conn = p.acquire().await.map_err(|e| e.to_string())?;
            // Capture backend pid on the same connection that will run the
            // user query. `pg_cancel_backend(pid)` from a side connection
            // is the standard way to interrupt a long-running statement.
            let pid: (i32,) = sqlx::query_as("SELECT pg_backend_pid()")
                .fetch_one(&mut *conn)
                .await
                .map_err(|e| e.to_string())?;
            let kill = KillHandle::Postgres { backend_pid: pid.0 };
            manager
                .register_in_flight(
                    query_id.to_string(),
                    InFlight { pool_key: key.to_string(), cancel: cancel_tx, kill },
                )
                .await;
            tokio::select! {
                r = run_query_pg(&mut conn, query) => r,
                _ = cancel_rx => Err("Query cancelled".to_string()),
            }
        }
        DatabasePool::MySql(p) => {
            let mut conn = p.acquire().await.map_err(|e| e.to_string())?;
            let cid: (u64,) = sqlx::query_as("SELECT CONNECTION_ID()")
                .fetch_one(&mut *conn)
                .await
                .map_err(|e| e.to_string())?;
            let kill = KillHandle::MySql { connection_id: cid.0 };
            manager
                .register_in_flight(
                    query_id.to_string(),
                    InFlight { pool_key: key.to_string(), cancel: cancel_tx, kill },
                )
                .await;
            tokio::select! {
                r = run_query_mysql(&mut conn, query) => r,
                _ = cancel_rx => Err("Query cancelled".to_string()),
            }
        }
        DatabasePool::Sqlite(p) => {
            manager
                .register_in_flight(
                    query_id.to_string(),
                    InFlight {
                        pool_key: key.to_string(),
                        cancel: cancel_tx,
                        kill: KillHandle::Sqlite,
                    },
                )
                .await;
            tokio::select! {
                r = run_query_sqlite(&p, query) => r,
                _ = cancel_rx => Err("Query cancelled".to_string()),
            }
        }
        DatabasePool::Clickhouse(c) => {
            // The ClickHouse server-side `query_id` matches ours, so cancel
            // dispatches `KILL QUERY WHERE query_id='<uuid>' SYNC`.
            manager
                .register_in_flight(
                    query_id.to_string(),
                    InFlight {
                        pool_key: key.to_string(),
                        cancel: cancel_tx,
                        kill: KillHandle::Clickhouse { query_id: query_id.to_string() },
                    },
                )
                .await;
            let start = Instant::now();
            let fut = c.query_with_id(query, Some(query_id));
            let res = tokio::select! {
                r = fut => r,
                _ = cancel_rx => Err("Query cancelled".to_string()),
            };
            res.map(|result| SqlQueryResult {
                columns: result.columns,
                rows: result.rows,
                affected_rows: result.affected,
                duration_ms: start.elapsed().as_millis() as u64,
                query_kind: classify_query(query),
            })
        }
        DatabasePool::D1(c) => {
            manager
                .register_in_flight(
                    query_id.to_string(),
                    InFlight {
                        pool_key: key.to_string(),
                        cancel: cancel_tx,
                        kill: KillHandle::D1,
                    },
                )
                .await;
            let start = Instant::now();
            let fut = c.query(query);
            let res = tokio::select! {
                r = fut => r,
                _ = cancel_rx => Err("Query cancelled".to_string()),
            };
            res.map(|result| {
                let duration_ms = if result.duration_ms > 0 {
                    result.duration_ms
                } else {
                    start.elapsed().as_millis() as u64
                };
                SqlQueryResult {
                    columns: result.columns,
                    rows: result.rows,
                    affected_rows: result.affected,
                    duration_ms,
                    query_kind: classify_query(query),
                }
            })
        }
    };

    // Always deregister — covers success, error, and cancel paths.
    let _ = manager.deregister_in_flight(query_id).await;
    result
}

#[tauri::command]
pub async fn sql_list_databases(
    manager: State<'_, Arc<SqlConnectionManager>>,
    app_pool: State<'_, SqlitePool>,
    conn_id: String,
    database: String,
) -> Result<Vec<String>, String> {
    ensure_pool_inner(manager.inner(), app_pool.inner(), &conn_id, &database).await?;
    let key = pool_key(&conn_id, &database);
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&key)
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
            let rows = sqlx::query("SHOW DATABASES")
                .fetch_all(p)
                .await
                .map_err(|e| e.to_string())?;
            Ok(rows
                .iter()
                .map(|row| mysql_decode_string(row, 0))
                .filter(|s| !s.is_empty())
                .collect())
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
        DatabasePool::D1(c) => {
            // A D1 connection IS a single database — listing all databases
            // in the Cloudflare account would require a separate /accounts/
            // /d1/database endpoint and an additional permission scope.
            // Surface just the connected database id, matching SQLite.
            Ok(vec![c.database.clone()])
        }
    }
}

#[tauri::command]
pub async fn sql_create_database(
    manager: State<'_, Arc<SqlConnectionManager>>,
    app_pool: State<'_, SqlitePool>,
    conn_id: String,
    database: String,
    name: String,
) -> Result<(), String> {
    ensure_pool_inner(manager.inner(), app_pool.inner(), &conn_id, &database).await?;
    let key = pool_key(&conn_id, &database);
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&key)
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
        DatabasePool::D1(_) => {
            // D1 databases are provisioned via the Cloudflare dashboard or
            // the /accounts/{}/d1/database POST endpoint — not from inside
            // a query. Refuse here rather than fake success.
            return Err("D1 databases must be created from the Cloudflare dashboard or wrangler CLI.".to_string());
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn sql_list_schemas(
    manager: State<'_, Arc<SqlConnectionManager>>,
    app_pool: State<'_, SqlitePool>,
    conn_id: String,
    database: String,
) -> Result<Vec<String>, String> {
    ensure_pool_inner(manager.inner(), app_pool.inner(), &conn_id, &database).await?;
    let key = pool_key(&conn_id, &database);
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&key)
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
        DatabasePool::D1(_) => {
            // D1 is SQLite under the hood; one schema, named "main".
            Ok(vec!["main".to_string()])
        }
    }
}

#[tauri::command]
pub async fn sql_list_tables(
    manager: State<'_, Arc<SqlConnectionManager>>,
    app_pool: State<'_, SqlitePool>,
    conn_id: String,
    database: String,
    schema: Option<String>,
) -> Result<Vec<TableInfo>, String> {
    ensure_pool_inner(manager.inner(), app_pool.inner(), &conn_id, &database).await?;
    let key = pool_key(&conn_id, &database);
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&key)
        .ok_or_else(|| "Connection not found".to_string())?;
    // Optional explicit-database override is only meaningful for MySQL/CH
    // where one pool can introspect other databases on the same server.
    // PG/SQLite/D1 always use the pool's own database.
    let database_opt: Option<String> = Some(database.clone());

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
            let db = database_opt.clone().unwrap_or_default();
            let query = if db.is_empty() {
                "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name".to_string()
            } else {
                format!(
                    "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = '{}' ORDER BY table_name",
                    db.replace('\'', "''")
                )
            };
            let rows = sqlx::query(&query)
                .fetch_all(p)
                .await
                .map_err(|e| e.to_string())?;

            Ok(rows
                .iter()
                .map(|row| {
                    let name = mysql_decode_string(row, 0);
                    let table_type = mysql_decode_string(row, 1);
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
            let db_name = database_opt.clone().unwrap_or_else(|| c.database.clone());
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
        DatabasePool::D1(c) => {
            // D1 is SQLite — use sqlite_master, same query as the local
            // SQLite branch above. Filter out internal `sqlite_*` / `_cf_*`
            // (Cloudflare-internal D1 bookkeeping tables) so they don't
            // clutter the sidebar.
            let result = c
                .query(
                    "SELECT name, type FROM sqlite_master \
                     WHERE type IN ('table', 'view') \
                       AND name NOT LIKE 'sqlite_%' \
                       AND name NOT LIKE '_cf_%' \
                     ORDER BY name",
                )
                .await?;
            Ok(result
                .rows
                .into_iter()
                .filter_map(|row| {
                    let mut it = row.into_iter();
                    let name = it.next()?.as_str().map(|s| s.to_string())?;
                    let table_type = it
                        .next()
                        .and_then(|v| v.as_str().map(|s| s.to_uppercase()))
                        .unwrap_or_else(|| "TABLE".to_string());
                    Some(TableInfo { name, table_type, row_count: None })
                })
                .collect())
        }
    }
}

#[tauri::command]
pub async fn sql_describe_table(
    manager: State<'_, Arc<SqlConnectionManager>>,
    app_pool: State<'_, SqlitePool>,
    conn_id: String,
    database: String,
    table: String,
    schema: Option<String>,
) -> Result<Vec<ColumnInfo>, String> {
    ensure_pool_inner(manager.inner(), app_pool.inner(), &conn_id, &database).await?;
    let key = pool_key(&conn_id, &database);
    let connections = manager.connections.lock().await;
    let pool = connections
        .get(&key)
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
            // DESCRIBE returns Field/Type/Null/Key/Default. Any of these can
            // come back VARBINARY under some collations (e.g. *_bin), so
            // decode each by ordinal with the shared helper.
            let rows = sqlx::query(&format!(
                "DESCRIBE `{}`",
                table.replace('`', "``")
            ))
            .fetch_all(p)
            .await
            .map_err(|e| e.to_string())?;

            Ok(rows
                .iter()
                .map(|row| {
                    let field = mysql_decode_string(row, 0);
                    let col_type = mysql_decode_string(row, 1);
                    let nullable = mysql_decode_string(row, 2);
                    let key = mysql_decode_string(row, 3);
                    // Column 4 ("Default") is nullable — try String, then bytes, then None.
                    let default: Option<String> = row
                        .try_get::<Option<String>, _>(4)
                        .ok()
                        .flatten()
                        .or_else(|| {
                            row.try_get::<Option<Vec<u8>>, _>(4)
                                .ok()
                                .flatten()
                                .map(|b| String::from_utf8_lossy(&b).into_owned())
                        });
                    ColumnInfo {
                        name: field,
                        data_type: col_type,
                        is_nullable: nullable == "YES",
                        is_primary_key: key == "PRI",
                        default_value: default,
                    }
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
        DatabasePool::D1(c) => {
            // D1 is SQLite — use PRAGMA table_info, same shape as the
            // sqlx-sqlite branch above. D1 returns JSON column values as
            // typed primitives, so the int/string discriminators below
            // need to handle both shapes (some clients stringify ints).
            let stmt = format!(
                "PRAGMA table_info(\"{}\")",
                table.replace('"', "\"\"")
            );
            let result = c.query(&stmt).await?;
            Ok(result
                .rows
                .into_iter()
                .filter_map(|row| {
                    // Columns from PRAGMA table_info:
                    //   0:cid 1:name 2:type 3:notnull 4:dflt_value 5:pk
                    let mut it = row.into_iter();
                    let _cid = it.next();
                    let name = it.next()?.as_str().map(|s| s.to_string())?;
                    let data_type = it
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
                    let default_value = it
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
                    Some(ColumnInfo {
                        name,
                        data_type,
                        is_nullable: !notnull,
                        is_primary_key: is_pk,
                        default_value,
                    })
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
    manager: State<'_, Arc<SqlConnectionManager>>,
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

    // Drop any cached pools / tunnels for this connection. The pool was
    // built against the previous host/port/file/credentials, so reusing it
    // after an edit silently points at the old target (most visible with
    // SQLite, where the saved `database` is the actual file path). Closing
    // pools here forces the next request to rebuild against the new config.
    let prefix = format!("{}:", id);
    let drained: Vec<(String, DatabasePool)> = {
        let mut conns = manager.connections.lock().await;
        let keys: Vec<String> = conns
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|k| conns.remove(&k).map(|p| (k, p)))
            .collect()
    };
    for (key, db_pool) in drained {
        close_db_pool(db_pool).await;
        let _ = manager.tunnels.lock().await.remove(&key);
        let _ = manager.permits.lock().await.remove(&key);
        log::info!("[Clauge SQL] pool dropped on config update: {}", key);
    }

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
    connection_id: Option<String>,
) -> Result<SqlScript, String> {
    use crate::shared::repos::sql_connections as sql_conn_repo;
    sql_conn_repo::update_script(
        pool.inner(),
        &id,
        &name,
        &query,
        database_name.as_deref(),
        connection_id.as_deref(),
    )
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

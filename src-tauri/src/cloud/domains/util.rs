// Shared helpers for the domain serializers. Each kind exports a JSON payload
// shaped `{ "version": 1, "tables": { "table_name": [row_obj, ...] } }`,
// gzipped before crossing the wire. We hash the *uncompressed* JSON so the
// hash is stable regardless of compression-level tweaks.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteRow;
use sqlx::{Column, Row, SqlitePool, TypeInfo};
use std::collections::BTreeMap;
use std::io::{Read, Write};

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncPayload {
    pub version: u32,
    pub kind: String,
    /// Table name → array of row objects. BTreeMap keeps stable ordering so
    /// the hash is deterministic across runs.
    pub tables: BTreeMap<String, Vec<Map<String, Value>>>,
}

pub fn empty_payload(kind: &str) -> SyncPayload {
    SyncPayload {
        version: 1,
        kind: kind.to_string(),
        tables: BTreeMap::new(),
    }
}

pub fn payload_is_empty(p: &SyncPayload) -> bool {
    p.tables.values().all(|rows| rows.is_empty())
}

/// Serialize, gzip, base64-encode. Hash is sha256 of the pre-gzip JSON bytes.
pub fn encode(payload: &SyncPayload) -> Result<(String, String), String> {
    let json_bytes = serde_json::to_vec(payload).map_err(|e| format!("serialize: {}", e))?;
    let hash = hex::encode(Sha256::digest(&json_bytes));

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json_bytes).map_err(|e| format!("gzip: {}", e))?;
    let gz = encoder.finish().map_err(|e| format!("gzip finish: {}", e))?;

    Ok((hash, B64.encode(&gz)))
}

/// base64-decode, gunzip, parse JSON.
pub fn decode(payload_b64: &str) -> Result<SyncPayload, String> {
    let gz = B64.decode(payload_b64).map_err(|e| format!("base64: {}", e))?;
    let mut decoder = GzDecoder::new(&gz[..]);
    let mut json = Vec::new();
    decoder.read_to_end(&mut json).map_err(|e| format!("gunzip: {}", e))?;
    serde_json::from_slice(&json).map_err(|e| format!("parse: {}", e))
}

/// Run a SELECT and convert each row to a JSON object keyed by column name.
pub async fn select_rows_as_json(
    pool: &SqlitePool,
    sql: &str,
) -> Result<Vec<Map<String, Value>>, String> {
    let rows = sqlx::query(sql)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("query failed: {}", e))?;
    Ok(rows.iter().map(row_to_json_map).collect())
}

fn row_to_json_map(row: &SqliteRow) -> Map<String, Value> {
    let mut out = Map::new();
    for column in row.columns() {
        let name = column.name();
        let value = column_value(row, column.ordinal());
        out.insert(name.to_string(), value);
    }
    out
}

fn column_value(row: &SqliteRow, idx: usize) -> Value {
    use sqlx::ValueRef;
    // Pull the type tag as an owned String so the immutable borrow on `row`
    // ends before we call try_get below (which also borrows row).
    let type_name = match row.try_get_raw(idx) {
        Ok(r) if r.is_null() => return Value::Null,
        Ok(r) => r.type_info().name().to_string(),
        Err(_) => return Value::Null,
    };
    match type_name.as_str() {
        "TEXT" => row
            .try_get::<String, _>(idx)
            .map(Value::String)
            .unwrap_or(Value::Null),
        "INTEGER" => row
            .try_get::<i64, _>(idx)
            .map(|v| Value::Number(v.into()))
            .unwrap_or(Value::Null),
        "REAL" => row
            .try_get::<f64, _>(idx)
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        "BLOB" => row
            .try_get::<Vec<u8>, _>(idx)
            .map(|b| Value::String(B64.encode(&b)))
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// Build an INSERT statement for the given table+columns and bind values from
/// a row JSON object. Used by importers — caller handles surrounding TX.
pub async fn insert_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    columns: &[&str],
    row: &Map<String, Value>,
) -> Result<(), String> {
    let placeholders: Vec<&str> = columns.iter().map(|_| "?").collect();
    let sql = format!(
        "INSERT OR REPLACE INTO {} ({}) VALUES ({})",
        table,
        columns.join(", "),
        placeholders.join(", "),
    );
    let mut q = sqlx::query(&sql);
    for col in columns {
        let v = row.get(*col).unwrap_or(&Value::Null);
        q = bind_value(q, v);
    }
    q.execute(&mut **tx)
        .await
        .map_err(|e| format!("insert into {}: {}", table, e))?;
    Ok(())
}

/// Public alias used by domain importers that need to drive their own
/// UPSERT statement (e.g. coworkers, which can't use the DELETE+INSERT
/// pattern in `insert_row` because of FK references that would get
/// nulled). The standard `insert_row` helper handles the common case
/// — this is the escape hatch for when the bind logic is needed
/// outside that helper.
pub fn bind_value_to_query<'q>(
    q: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    v: &'q Value,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    bind_value(q, v)
}

fn bind_value<'q>(
    q: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    v: &'q Value,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    match v {
        Value::Null => q.bind::<Option<String>>(None),
        Value::Bool(b) => q.bind(if *b { 1i64 } else { 0i64 }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                q.bind::<Option<String>>(None)
            }
        }
        Value::String(s) => q.bind(s.as_str()),
        Value::Array(_) | Value::Object(_) => q.bind(serde_json::to_string(v).unwrap_or_default()),
    }
}

/// Compute sha256(uncompressed json) for the locally-exported payload, to
/// compare against a remote `content_hash` without round-tripping the bytes.
pub fn hash_of_payload(payload: &SyncPayload) -> String {
    let json = serde_json::to_vec(payload).unwrap_or_default();
    hex::encode(Sha256::digest(&json))
}

/// Declarative description of one syncable table for merge imports.
/// `columns` MUST match the domain's export SELECT list exactly.
#[allow(dead_code)] // wired up by per-domain merge specs (Task 3.2)
pub struct TableSpec {
    pub table: &'static str,
    pub pk: &'static str,
    /// Column used for last-write-wins on same-pk rows. None = local always
    /// wins for existing rows (insert-if-missing only).
    pub updated_at: Option<&'static str>,
    pub columns: &'static [&'static str],
}

/// UPSERT-union merge: inserts rows missing locally; for same-pk rows the
/// newer `updated_at` wins (ties keep local). NEVER deletes local rows.
/// Specs must be ordered FK-parents-first. Single transaction.
#[allow(dead_code)] // wired up by merge_kind dispatch (Task 3.2)
pub async fn merge_import(
    pool: &SqlitePool,
    payload: &SyncPayload,
    specs: &[TableSpec],
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| format!("begin: {}", e))?;
    for spec in specs {
        let rows = payload
            .tables
            .get(spec.table)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let placeholders = spec.columns.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = match spec.updated_at {
            Some(ucol) => {
                let updates = spec
                    .columns
                    .iter()
                    .filter(|c| **c != spec.pk)
                    .map(|c| format!("{} = excluded.{}", c, c))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "INSERT INTO {t} ({cols}) VALUES ({ph}) \
                     ON CONFLICT({pk}) DO UPDATE SET {updates} \
                     WHERE excluded.{u} > {t}.{u}",
                    t = spec.table,
                    cols = spec.columns.join(", "),
                    ph = placeholders,
                    pk = spec.pk,
                    updates = updates,
                    u = ucol,
                )
            }
            None => format!(
                "INSERT OR IGNORE INTO {} ({}) VALUES ({})",
                spec.table,
                spec.columns.join(", "),
                placeholders,
            ),
        };
        for row in rows {
            let mut q = sqlx::query(&sql);
            for col in spec.columns {
                let v = row.get(*col).unwrap_or(&Value::Null);
                q = bind_value(q, v);
            }
            q.execute(&mut *tx)
                .await
                .map_err(|e| format!("merge into {}: {}", spec.table, e))?;
        }
    }
    tx.commit().await.map_err(|e| format!("commit: {}", e))
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    async fn pool_with(table_sql: &str, rows: &[&str]) -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(table_sql).execute(&pool).await.unwrap();
        for r in rows {
            sqlx::query(r).execute(&pool).await.unwrap();
        }
        pool
    }

    fn payload_with(table: &str, rows: Vec<serde_json::Value>) -> SyncPayload {
        let mut p = empty_payload("test");
        p.tables.insert(
            table.into(),
            rows.into_iter()
                .map(|v| v.as_object().unwrap().clone())
                .collect(),
        );
        p
    }

    const T: &str = "CREATE TABLE items (id TEXT PRIMARY KEY, name TEXT, updated_at TEXT)";
    const SPEC: &[TableSpec] = &[TableSpec {
        table: "items",
        pk: "id",
        updated_at: Some("updated_at"),
        columns: &["id", "name", "updated_at"],
    }];

    #[tokio::test]
    async fn merge_inserts_missing_rows() {
        let pool = pool_with(T, &[]).await;
        let p = payload_with("items", vec![serde_json::json!({"id":"a","name":"x","updated_at":"2026-01-02"})]);
        merge_import(&pool, &p, SPEC).await.unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM items").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn merge_newer_incoming_wins_older_loses() {
        let pool = pool_with(T, &["INSERT INTO items VALUES ('a','local','2026-01-05')"]).await;
        let p = payload_with("items", vec![serde_json::json!({"id":"a","name":"remote-old","updated_at":"2026-01-01"})]);
        merge_import(&pool, &p, SPEC).await.unwrap();
        let (name,): (String,) = sqlx::query_as("SELECT name FROM items WHERE id='a'").fetch_one(&pool).await.unwrap();
        assert_eq!(name, "local");
        let p = payload_with("items", vec![serde_json::json!({"id":"a","name":"remote-new","updated_at":"2026-02-01"})]);
        merge_import(&pool, &p, SPEC).await.unwrap();
        let (name,): (String,) = sqlx::query_as("SELECT name FROM items WHERE id='a'").fetch_one(&pool).await.unwrap();
        assert_eq!(name, "remote-new");
    }

    #[tokio::test]
    async fn merge_never_deletes_local_only_rows() {
        let pool = pool_with(T, &["INSERT INTO items VALUES ('local-only','keep','2026-01-01')"]).await;
        let p = payload_with("items", vec![serde_json::json!({"id":"b","name":"new","updated_at":"2026-01-02"})]);
        merge_import(&pool, &p, SPEC).await.unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM items").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 2);
    }

    #[tokio::test]
    async fn merge_without_updated_at_keeps_local() {
        let pool = pool_with(
            "CREATE TABLE plain (id TEXT PRIMARY KEY, name TEXT)",
            &["INSERT INTO plain VALUES ('a','local')"],
        )
        .await;
        let spec = &[TableSpec { table: "plain", pk: "id", updated_at: None, columns: &["id", "name"] }];
        let p = payload_with("plain", vec![
            serde_json::json!({"id":"a","name":"remote"}),
            serde_json::json!({"id":"b","name":"new"}),
        ]);
        merge_import(&pool, &p, spec).await.unwrap();
        let (name,): (String,) = sqlx::query_as("SELECT name FROM plain WHERE id='a'").fetch_one(&pool).await.unwrap();
        assert_eq!(name, "local");
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM plain").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 2);
    }

    #[tokio::test]
    async fn merge_tie_on_updated_at_keeps_local() {
        let pool = pool_with(T, &["INSERT INTO items VALUES ('a','local','2026-01-05')"]).await;
        let p = payload_with("items", vec![serde_json::json!({"id":"a","name":"remote","updated_at":"2026-01-05"})]);
        merge_import(&pool, &p, SPEC).await.unwrap();
        let (name,): (String,) = sqlx::query_as("SELECT name FROM items WHERE id='a'").fetch_one(&pool).await.unwrap();
        assert_eq!(name, "local");
    }

    #[tokio::test]
    async fn merge_is_transactional_per_call() {
        // Second table in the spec doesn't exist -> whole merge must roll back.
        let pool = pool_with(T, &[]).await;
        let specs = &[
            TableSpec { table: "items", pk: "id", updated_at: Some("updated_at"), columns: &["id", "name", "updated_at"] },
            TableSpec { table: "missing_table", pk: "id", updated_at: None, columns: &["id"] },
        ];
        let mut p = payload_with("items", vec![serde_json::json!({"id":"a","name":"x","updated_at":"2026-01-02"})]);
        p.tables.insert("missing_table".into(), vec![serde_json::json!({"id":"z"}).as_object().unwrap().clone()]);
        assert!(merge_import(&pool, &p, specs).await.is_err());
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM items").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0, "failed merge must not leave partial rows");
    }
}

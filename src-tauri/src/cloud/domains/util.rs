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

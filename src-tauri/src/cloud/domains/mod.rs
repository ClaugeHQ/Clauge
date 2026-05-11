pub mod agent;
pub mod explorer;
pub mod nosql;
pub mod rest;
pub mod sql;
pub mod ssh;
pub mod util;

pub const ALL_KINDS: &[&str] = &[
    rest::KIND,
    sql::KIND,
    nosql::KIND,
    agent::KIND,
    ssh::KIND,
    explorer::KIND,
];

/// Build the (hash, base64-gzip-json) tuple for a kind.
pub async fn export_kind(
    pool: &sqlx::SqlitePool,
    kind: &str,
) -> Result<(String, String), String> {
    match kind {
        rest::KIND => rest::export(pool).await,
        sql::KIND => sql::export(pool).await,
        nosql::KIND => nosql::export(pool).await,
        agent::KIND => agent::export(pool).await,
        ssh::KIND => ssh::export(pool).await,
        explorer::KIND => explorer::export(pool).await,
        _ => Err(format!("unknown kind: {}", kind)),
    }
}

pub async fn import_kind(
    pool: &sqlx::SqlitePool,
    kind: &str,
    payload_b64: &str,
) -> Result<(), String> {
    let payload = util::decode(payload_b64)?;
    if payload.kind != kind {
        return Err(format!(
            "payload kind mismatch: header says {}, route says {}",
            payload.kind, kind
        ));
    }
    match kind {
        rest::KIND => rest::import(pool, &payload).await,
        sql::KIND => sql::import(pool, &payload).await,
        nosql::KIND => nosql::import(pool, &payload).await,
        agent::KIND => agent::import(pool, &payload).await,
        ssh::KIND => ssh::import(pool, &payload).await,
        explorer::KIND => explorer::import(pool, &payload).await,
        _ => Err(format!("unknown kind: {}", kind)),
    }
}

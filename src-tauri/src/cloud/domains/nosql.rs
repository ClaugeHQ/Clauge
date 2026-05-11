use sqlx::SqlitePool;

use crate::cloud::domains::util::{empty_payload, encode, insert_row, select_rows_as_json, SyncPayload};

pub const KIND: &str = "nosql";

pub async fn build_payload(pool: &SqlitePool) -> Result<SyncPayload, String> {
    let mut payload = empty_payload(KIND);
    // Excludes `connection_string` AND `password` — both are secrets.
    payload.tables.insert(
        "nosql_connections".into(),
        select_rows_as_json(
            pool,
            "SELECT id, name, driver, host, port, database_name, username, ssl, direct_connection, ssh_profile_id, sort_order, created_at, updated_at FROM nosql_connections ORDER BY sort_order, id",
        ).await?,
    );
    Ok(payload)
}

pub async fn export(pool: &SqlitePool) -> Result<(String, String), String> {
    encode(&build_payload(pool).await?)
}

pub async fn import(pool: &SqlitePool, payload: &SyncPayload) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| format!("begin: {}", e))?;
    if let Some(rows) = payload.tables.get("nosql_connections") {
        for r in rows {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM nosql_connections WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| format!("probe: {}", e))?;
            if exists.is_some() {
                sqlx::query(
                    "UPDATE nosql_connections SET name=?, driver=?, host=?, port=?, database_name=?, username=?, ssl=?, direct_connection=?, ssh_profile_id=?, sort_order=?, created_at=?, updated_at=? WHERE id=?",
                )
                .bind(r.get("name").and_then(|v| v.as_str()))
                .bind(r.get("driver").and_then(|v| v.as_str()))
                .bind(r.get("host").and_then(|v| v.as_str()))
                .bind(r.get("port").and_then(|v| v.as_i64()).unwrap_or(0))
                .bind(r.get("database_name").and_then(|v| v.as_str()))
                .bind(r.get("username").and_then(|v| v.as_str()))
                .bind(r.get("ssl").and_then(|v| v.as_i64()).unwrap_or(0))
                .bind(r.get("direct_connection").and_then(|v| v.as_i64()))
                .bind(r.get("ssh_profile_id").and_then(|v| v.as_str()))
                .bind(r.get("sort_order").and_then(|v| v.as_i64()).unwrap_or(0))
                .bind(r.get("created_at").and_then(|v| v.as_str()))
                .bind(r.get("updated_at").and_then(|v| v.as_str()))
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("update nosql_connections: {}", e))?;
            } else {
                insert_row(&mut tx, "nosql_connections", &[
                    "id","name","driver","host","port","database_name","username","ssl","direct_connection","ssh_profile_id","sort_order","created_at","updated_at",
                ], r).await?;
            }
        }
    }
    tx.commit().await.map_err(|e| format!("commit: {}", e))?;
    Ok(())
}

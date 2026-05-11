use sqlx::SqlitePool;

use crate::cloud::domains::util::{empty_payload, encode, insert_row, select_rows_as_json, SyncPayload};

pub const KIND: &str = "sql";

pub async fn build_payload(pool: &SqlitePool) -> Result<SyncPayload, String> {
    let mut payload = empty_payload(KIND);
    // Strip `password` — kept in the local DB but never crosses the network.
    payload.tables.insert(
        "sql_connections".into(),
        select_rows_as_json(
            pool,
            "SELECT id, name, driver, host, port, database_name, username, ssl, ssh_profile_id, sort_order, created_at, updated_at FROM sql_connections ORDER BY sort_order, id",
        ).await?,
    );
    payload.tables.insert(
        "sql_scripts".into(),
        select_rows_as_json(pool, "SELECT * FROM sql_scripts ORDER BY sort_order, id").await?,
    );
    Ok(payload)
}

pub async fn export(pool: &SqlitePool) -> Result<(String, String), String> {
    encode(&build_payload(pool).await?)
}

pub async fn import(pool: &SqlitePool, payload: &SyncPayload) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| format!("begin: {}", e))?;

    sqlx::query("DELETE FROM sql_scripts").execute(&mut *tx).await.map_err(|e| format!("clear scripts: {}", e))?;

    if let Some(rows) = payload.tables.get("sql_connections") {
        for r in rows {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM sql_connections WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| format!("probe: {}", e))?;
            if exists.is_some() {
                // Targeted UPDATE — `password` column stays untouched.
                sqlx::query(
                    "UPDATE sql_connections SET name=?, driver=?, host=?, port=?, database_name=?, username=?, ssl=?, ssh_profile_id=?, sort_order=?, created_at=?, updated_at=? WHERE id=?",
                )
                .bind(r.get("name").and_then(|v| v.as_str()))
                .bind(r.get("driver").and_then(|v| v.as_str()))
                .bind(r.get("host").and_then(|v| v.as_str()))
                .bind(r.get("port").and_then(|v| v.as_i64()).unwrap_or(0))
                .bind(r.get("database_name").and_then(|v| v.as_str()))
                .bind(r.get("username").and_then(|v| v.as_str()))
                .bind(r.get("ssl").and_then(|v| v.as_i64()).unwrap_or(0))
                .bind(r.get("ssh_profile_id").and_then(|v| v.as_str()))
                .bind(r.get("sort_order").and_then(|v| v.as_i64()).unwrap_or(0))
                .bind(r.get("created_at").and_then(|v| v.as_str()))
                .bind(r.get("updated_at").and_then(|v| v.as_str()))
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("update sql_connections: {}", e))?;
            } else {
                // New conn — password defaults to '' (user re-enters on first use).
                insert_row(&mut tx, "sql_connections", &[
                    "id","name","driver","host","port","database_name","username","ssl","ssh_profile_id","sort_order","created_at","updated_at",
                ], r).await?;
            }
        }
    }

    if let Some(rows) = payload.tables.get("sql_scripts") {
        for r in rows {
            insert_row(&mut tx, "sql_scripts", &[
                "id","name","connection_id","database_name","query","sort_order","created_at","updated_at",
            ], r).await?;
        }
    }

    tx.commit().await.map_err(|e| format!("commit: {}", e))?;
    Ok(())
}

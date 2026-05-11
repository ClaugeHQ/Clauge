use sqlx::SqlitePool;

use crate::cloud::domains::util::{
    empty_payload, encode, select_rows_as_json, SyncPayload,
};

pub const KIND: &str = "rest";

const TABLES: &[&str] = &[
    "collections",
    "requests",
    "request_headers",
    "request_params",
    "environments",
    "env_variables",
];

pub async fn build_payload(pool: &SqlitePool) -> Result<SyncPayload, String> {
    let mut payload = empty_payload(KIND);
    payload.tables.insert(
        "collections".into(),
        select_rows_as_json(pool, "SELECT * FROM collections ORDER BY sort_order, id").await?,
    );
    payload.tables.insert(
        "requests".into(),
        select_rows_as_json(pool, "SELECT * FROM requests ORDER BY sort_order, id").await?,
    );
    payload.tables.insert(
        "request_headers".into(),
        select_rows_as_json(pool, "SELECT * FROM request_headers ORDER BY request_id, sort_order").await?,
    );
    payload.tables.insert(
        "request_params".into(),
        select_rows_as_json(pool, "SELECT * FROM request_params ORDER BY request_id, sort_order").await?,
    );
    payload.tables.insert(
        "environments".into(),
        select_rows_as_json(pool, "SELECT * FROM environments ORDER BY sort_order, id").await?,
    );
    payload.tables.insert(
        "env_variables".into(),
        select_rows_as_json(pool, "SELECT * FROM env_variables ORDER BY environment_id, sort_order").await?,
    );
    Ok(payload)
}

pub async fn export(pool: &SqlitePool) -> Result<(String, String), String> {
    encode(&build_payload(pool).await?)
}

pub async fn import(pool: &SqlitePool, payload: &SyncPayload) -> Result<(), String> {
    use crate::cloud::domains::util::insert_row;

    let mut tx = pool.begin().await.map_err(|e| format!("begin tx: {}", e))?;
    // Clear in FK-safe order (children first).
    for t in &["env_variables", "request_headers", "request_params", "requests", "environments", "collections"] {
        sqlx::query(&format!("DELETE FROM {}", t))
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("clear {}: {}", t, e))?;
    }

    // Insert parents first, then children.
    if let Some(rows) = payload.tables.get("collections") {
        for r in rows {
            insert_row(&mut tx, "collections", &[
                "id", "name", "description", "sort_order", "env_id", "created_at", "updated_at",
            ], r).await?;
        }
    }
    if let Some(rows) = payload.tables.get("environments") {
        for r in rows {
            insert_row(&mut tx, "environments", &[
                "id", "name", "color", "is_default", "sort_order", "created_at", "updated_at",
            ], r).await?;
        }
    }
    if let Some(rows) = payload.tables.get("requests") {
        for r in rows {
            insert_row(&mut tx, "requests", &[
                "id", "collection_id", "name", "description", "method", "url", "body", "body_type",
                "auth_type", "auth_data", "pre_script", "sort_order", "created_at", "updated_at",
            ], r).await?;
        }
    }
    if let Some(rows) = payload.tables.get("request_headers") {
        for r in rows {
            insert_row(&mut tx, "request_headers", &[
                "id", "request_id", "key", "value", "enabled", "sort_order",
            ], r).await?;
        }
    }
    if let Some(rows) = payload.tables.get("request_params") {
        for r in rows {
            insert_row(&mut tx, "request_params", &[
                "id", "request_id", "key", "value", "enabled", "sort_order",
            ], r).await?;
        }
    }
    if let Some(rows) = payload.tables.get("env_variables") {
        for r in rows {
            insert_row(&mut tx, "env_variables", &[
                "id", "environment_id", "key", "value", "is_secret", "sort_order",
            ], r).await?;
        }
    }

    tx.commit().await.map_err(|e| format!("commit: {}", e))?;
    Ok(())
}

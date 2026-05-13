use sqlx::SqlitePool;

use crate::cloud::domains::util::{empty_payload, encode, insert_row, select_rows_as_json, SyncPayload};

pub const KIND: &str = "explorer";

pub async fn build_payload(pool: &SqlitePool) -> Result<SyncPayload, String> {
    let mut payload = empty_payload(KIND);
    // Per migration 8: kind-discriminated columns; secrets are NOT in this
    // table at all — they're in keychain under service "Clauge Explorer".
    // The `last_used_at` is machine-local so we exclude it.
    payload.tables.insert(
        "explorer_connections".into(),
        select_rows_as_json(
            pool,
            "SELECT id, name, kind, accent_color, ssh_profile_id, sftp_working_dir, host, port, username, auth_type, key_path, ftp_passive, ftp_tls, s3_preset, s3_endpoint, s3_region, s3_bucket, s3_path_style, azure_account, azure_container, azure_auth_kind, created_at FROM explorer_connections ORDER BY created_at, id",
        ).await?,
    );
    Ok(payload)
}

pub async fn export(pool: &SqlitePool) -> Result<(String, String), String> {
    encode(&build_payload(pool).await?)
}

pub async fn import(pool: &SqlitePool, payload: &SyncPayload) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| format!("begin: {}", e))?;

    if let Some(rows) = payload.tables.get("explorer_connections") {
        for r in rows {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
            // FK safety net — if the row references an `ssh_profile_id`
            // that doesn't exist locally (orphaned because of a botched
            // restore, manual deletion, or the snapshot pre-dating the
            // profile), drop the reference and keep the row. The FK is
            // `ON DELETE SET NULL` anyway, so NULL is a valid state;
            // schema 8's host/username columns let the row work as
            // direct-host (or stay user-fixable from the UI).
            let raw_profile = r.get("ssh_profile_id").and_then(|v| v.as_str());
            let safe_profile: Option<&str> = match raw_profile {
                Some(pid) if !pid.is_empty() => {
                    let exists = sqlx::query_scalar::<_, i64>(
                        "SELECT 1 FROM ssh_profiles WHERE id = ?",
                    )
                    .bind(pid)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e| format!("probe ssh_profile: {}", e))?;
                    if exists.is_some() { Some(pid) } else { None }
                }
                _ => None,
            };

            let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM explorer_connections WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| format!("probe: {}", e))?;
            if exists.is_some() {
                sqlx::query(
                    "UPDATE explorer_connections SET name=?, kind=?, accent_color=?, ssh_profile_id=?, sftp_working_dir=?, host=?, port=?, username=?, auth_type=?, key_path=?, ftp_passive=?, ftp_tls=?, s3_preset=?, s3_endpoint=?, s3_region=?, s3_bucket=?, s3_path_style=?, azure_account=?, azure_container=?, azure_auth_kind=? WHERE id=?",
                )
                .bind(r.get("name").and_then(|v| v.as_str()))
                .bind(r.get("kind").and_then(|v| v.as_str()))
                .bind(r.get("accent_color").and_then(|v| v.as_str()))
                .bind(safe_profile)
                .bind(r.get("sftp_working_dir").and_then(|v| v.as_str()))
                .bind(r.get("host").and_then(|v| v.as_str()))
                .bind(r.get("port").and_then(|v| v.as_i64()))
                .bind(r.get("username").and_then(|v| v.as_str()))
                .bind(r.get("auth_type").and_then(|v| v.as_str()))
                .bind(r.get("key_path").and_then(|v| v.as_str()))
                .bind(r.get("ftp_passive").and_then(|v| v.as_i64()).unwrap_or(1))
                .bind(r.get("ftp_tls").and_then(|v| v.as_str()).unwrap_or("none"))
                .bind(r.get("s3_preset").and_then(|v| v.as_str()))
                .bind(r.get("s3_endpoint").and_then(|v| v.as_str()))
                .bind(r.get("s3_region").and_then(|v| v.as_str()))
                .bind(r.get("s3_bucket").and_then(|v| v.as_str()))
                .bind(r.get("s3_path_style").and_then(|v| v.as_i64()).unwrap_or(0))
                .bind(r.get("azure_account").and_then(|v| v.as_str()))
                .bind(r.get("azure_container").and_then(|v| v.as_str()))
                .bind(r.get("azure_auth_kind").and_then(|v| v.as_str()))
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("update explorer_connections: {}", e))?;
            } else {
                // Clone the row and replace ssh_profile_id with the
                // FK-checked value so insert_row's generic binder sees
                // NULL when the parent is missing.
                let mut safe_row = r.clone();
                safe_row.insert(
                    "ssh_profile_id".into(),
                    match safe_profile {
                        Some(p) => serde_json::Value::String(p.to_string()),
                        None => serde_json::Value::Null,
                    },
                );
                insert_row(&mut tx, "explorer_connections", &[
                    "id","name","kind","accent_color","ssh_profile_id","sftp_working_dir","host","port","username","auth_type","key_path","ftp_passive","ftp_tls","s3_preset","s3_endpoint","s3_region","s3_bucket","s3_path_style","azure_account","azure_container","azure_auth_kind","created_at",
                ], &safe_row).await?;
            }
        }
    }

    tx.commit().await.map_err(|e| format!("commit: {}", e))?;
    Ok(())
}

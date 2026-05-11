use sqlx::SqlitePool;

use crate::cloud::domains::util::{empty_payload, encode, select_rows_as_json, SyncPayload};

pub const KIND: &str = "ssh";

pub async fn build_payload(pool: &SqlitePool) -> Result<SyncPayload, String> {
    let mut payload = empty_payload(KIND);
    // Metadata only — actual keys/passwords are in keychain on each device.
    // `key_path` is a local filesystem path; we still sync it because users
    // often use the same path layout across machines (~/.ssh/id_ed25519).
    // ssh_known_hosts is NOT synced (TOFU rebuilds naturally).
    payload.tables.insert(
        "ssh_profiles".into(),
        select_rows_as_json(
            pool,
            "SELECT id, name, host, port, username, auth_type, key_path, accent_color, jump_profile_id, proxy_command, created_at FROM ssh_profiles ORDER BY created_at, id",
        ).await?,
    );
    Ok(payload)
}

pub async fn export(pool: &SqlitePool) -> Result<(String, String), String> {
    encode(&build_payload(pool).await?)
}

pub async fn import(pool: &SqlitePool, payload: &SyncPayload) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| format!("begin: {}", e))?;

    if let Some(rows) = payload.tables.get("ssh_profiles") {
        for r in rows {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM ssh_profiles WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| format!("probe: {}", e))?;
            if exists.is_some() {
                sqlx::query(
                    "UPDATE ssh_profiles SET name=?, host=?, port=?, username=?, auth_type=?, key_path=?, accent_color=?, jump_profile_id=?, proxy_command=? WHERE id=?",
                )
                .bind(r.get("name").and_then(|v| v.as_str()))
                .bind(r.get("host").and_then(|v| v.as_str()))
                .bind(r.get("port").and_then(|v| v.as_i64()).unwrap_or(22))
                .bind(r.get("username").and_then(|v| v.as_str()))
                .bind(r.get("auth_type").and_then(|v| v.as_str()))
                .bind(r.get("key_path").and_then(|v| v.as_str()))
                .bind(r.get("accent_color").and_then(|v| v.as_str()))
                .bind(r.get("jump_profile_id").and_then(|v| v.as_str()))
                .bind(r.get("proxy_command").and_then(|v| v.as_str()))
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("update ssh_profiles: {}", e))?;
            } else {
                sqlx::query(
                    "INSERT INTO ssh_profiles (id, name, host, port, username, auth_type, key_path, accent_color, jump_profile_id, proxy_command, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(id)
                .bind(r.get("name").and_then(|v| v.as_str()).unwrap_or(""))
                .bind(r.get("host").and_then(|v| v.as_str()).unwrap_or(""))
                .bind(r.get("port").and_then(|v| v.as_i64()).unwrap_or(22))
                .bind(r.get("username").and_then(|v| v.as_str()).unwrap_or(""))
                .bind(r.get("auth_type").and_then(|v| v.as_str()).unwrap_or("key"))
                .bind(r.get("key_path").and_then(|v| v.as_str()))
                .bind(r.get("accent_color").and_then(|v| v.as_str()))
                .bind(r.get("jump_profile_id").and_then(|v| v.as_str()))
                .bind(r.get("proxy_command").and_then(|v| v.as_str()))
                .bind(r.get("created_at").and_then(|v| v.as_str()).unwrap_or(""))
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("insert ssh_profiles: {}", e))?;
            }
        }
    }

    tx.commit().await.map_err(|e| format!("commit: {}", e))?;
    Ok(())
}

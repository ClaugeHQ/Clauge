// companion_devices store. token_hash deliberately never leaves this
// module's queries — the list shape the frontend sees carries identity
// and liveness fields only.

use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CompanionDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    pub revoked: bool,
}

pub async fn insert(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    platform: &str,
    token_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO companion_devices (id, name, platform, token_hash) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(platform)
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<CompanionDevice>, sqlx::Error> {
    sqlx::query_as::<_, CompanionDevice>(
        "SELECT id, name, platform, created_at, last_seen_at, revoked
         FROM companion_devices ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
}

pub async fn revoke(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE companion_devices SET revoked = 1 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Hard-delete a single device row, dropping its token_hash for good.
pub async fn delete_device(pool: &SqlitePool, device_id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM companion_devices WHERE id = ?")
        .bind(device_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Hard-delete every revoked device row, returning how many were removed.
pub async fn purge_revoked(pool: &SqlitePool) -> Result<u64, String> {
    let res = sqlx::query("DELETE FROM companion_devices WHERE revoked = 1")
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res.rows_affected())
}

/// (id, token_hash) of every device that may still authenticate.
/// Callers compare hashes in constant time — see auth::authorize.
pub async fn active_token_hashes(pool: &SqlitePool) -> Result<Vec<(String, String)>, sqlx::Error> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT id, token_hash FROM companion_devices WHERE revoked = 0",
    )
    .fetch_all(pool)
    .await
}

/// Store/replace the FCM push token reported by the device itself via
/// POST /v1/device/fcm. Push dispatch (D4) reads it back.
pub async fn set_fcm_token(pool: &SqlitePool, id: &str, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE companion_devices SET fcm_token = ? WHERE id = ?")
        .bind(token)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn bump_last_seen(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE companion_devices SET last_seen_at = datetime('now') WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// (device_id, fcm_token) for every non-revoked device that has
/// registered a push token. Push dispatch (D4) targets exactly these.
pub async fn fcm_tokens(pool: &SqlitePool) -> Result<Vec<(String, String)>, sqlx::Error> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT id, fcm_token FROM companion_devices
         WHERE revoked = 0 AND fcm_token IS NOT NULL AND fcm_token <> ''",
    )
    .fetch_all(pool)
    .await
}

/// Drop a device's FCM token after the Worker reports it stale (app
/// uninstalled / registration rotated) so we stop pushing to it.
pub async fn clear_fcm_token(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE companion_devices SET fcm_token = NULL WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Device commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn companion_list_devices(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<Vec<CompanionDevice>, String> {
    list(pool.inner()).await.map_err(|e| e.to_string())
}

/// Soft-revoke: the row stays visible in Settings (greyed out), but
/// auth refuses the token from the next request onward.
#[tauri::command]
pub async fn companion_revoke_device(
    pool: tauri::State<'_, SqlitePool>,
    device_id: String,
) -> Result<(), String> {
    revoke(pool.inner(), &device_id)
        .await
        .map_err(|e| e.to_string())?;
    log::info!("[companion] revoked device {}", device_id);
    Ok(())
}

/// Hard-delete a single (typically already-revoked) device row.
#[tauri::command]
pub async fn companion_delete_device(
    pool: tauri::State<'_, SqlitePool>,
    device_id: String,
) -> Result<(), String> {
    delete_device(pool.inner(), &device_id).await?;
    log::info!("[companion] deleted device {}", device_id);
    Ok(())
}

/// Hard-delete all revoked device rows, returning the count removed.
#[tauri::command]
pub async fn companion_purge_revoked(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<u64, String> {
    let n = purge_revoked(pool.inner()).await?;
    log::info!("[companion] purged {} revoked device(s)", n);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIGRATION_22: &str = include_str!("../../migrations/22_companion_devices.sql");

    async fn test_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::raw_sql(MIGRATION_22)
            .execute(&pool)
            .await
            .expect("apply migration 22");
        pool
    }

    #[tokio::test]
    async fn delete_device_removes_row() {
        let pool = test_pool().await;
        insert(&pool, "dev-1", "Pixel", "android", "hash1")
            .await
            .unwrap();
        assert_eq!(list(&pool).await.unwrap().len(), 1);

        delete_device(&pool, "dev-1").await.unwrap();
        assert!(list(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn purge_revoked_only_deletes_revoked() {
        let pool = test_pool().await;
        insert(&pool, "active", "A", "android", "h-a")
            .await
            .unwrap();
        insert(&pool, "rev-1", "R1", "android", "h-r1")
            .await
            .unwrap();
        insert(&pool, "rev-2", "R2", "ios", "h-r2")
            .await
            .unwrap();
        revoke(&pool, "rev-1").await.unwrap();
        revoke(&pool, "rev-2").await.unwrap();

        let removed = purge_revoked(&pool).await.unwrap();
        assert_eq!(removed, 2);

        let remaining = list(&pool).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "active");
    }
}

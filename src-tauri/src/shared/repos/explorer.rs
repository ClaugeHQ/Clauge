//! Persistence helpers for `explorer_connections`. Mirror the per-aggregate
//! repository style used by other modes (e.g. `repos::ssh_profiles`).

use sqlx::SqlitePool;

use crate::modes::explorer::models::ExplorerConnection;

pub async fn list(pool: &SqlitePool) -> Result<Vec<ExplorerConnection>, sqlx::Error> {
    sqlx::query_as::<_, ExplorerConnection>(
        "SELECT * FROM explorer_connections \
         ORDER BY last_used_at DESC, created_at DESC",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<ExplorerConnection>, sqlx::Error> {
    sqlx::query_as::<_, ExplorerConnection>(
        "SELECT * FROM explorer_connections WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn insert(pool: &SqlitePool, conn: &ExplorerConnection) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO explorer_connections \
         (id, name, kind, accent_color, last_used_at, created_at, \
          ssh_profile_id, sftp_working_dir, \
          host, port, username, auth_type, key_path, \
          ftp_passive, ftp_tls, \
          s3_preset, s3_endpoint, s3_region, s3_bucket, s3_path_style, \
          azure_account, azure_container, azure_auth_kind) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&conn.id)
    .bind(&conn.name)
    .bind(&conn.kind)
    .bind(&conn.accent_color)
    .bind(&conn.last_used_at)
    .bind(&conn.created_at)
    .bind(&conn.ssh_profile_id)
    .bind(&conn.sftp_working_dir)
    .bind(&conn.host)
    .bind(&conn.port)
    .bind(&conn.username)
    .bind(&conn.auth_type)
    .bind(&conn.key_path)
    .bind(conn.ftp_passive)
    .bind(&conn.ftp_tls)
    .bind(&conn.s3_preset)
    .bind(&conn.s3_endpoint)
    .bind(&conn.s3_region)
    .bind(&conn.s3_bucket)
    .bind(conn.s3_path_style)
    .bind(&conn.azure_account)
    .bind(&conn.azure_container)
    .bind(&conn.azure_auth_kind)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update(pool: &SqlitePool, conn: &ExplorerConnection) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE explorer_connections SET \
            name=?, kind=?, accent_color=?, \
            ssh_profile_id=?, sftp_working_dir=?, \
            host=?, port=?, username=?, auth_type=?, key_path=?, \
            ftp_passive=?, ftp_tls=?, \
            s3_preset=?, s3_endpoint=?, s3_region=?, s3_bucket=?, s3_path_style=?, \
            azure_account=?, azure_container=?, azure_auth_kind=? \
         WHERE id=?",
    )
    .bind(&conn.name)
    .bind(&conn.kind)
    .bind(&conn.accent_color)
    .bind(&conn.ssh_profile_id)
    .bind(&conn.sftp_working_dir)
    .bind(&conn.host)
    .bind(&conn.port)
    .bind(&conn.username)
    .bind(&conn.auth_type)
    .bind(&conn.key_path)
    .bind(conn.ftp_passive)
    .bind(&conn.ftp_tls)
    .bind(&conn.s3_preset)
    .bind(&conn.s3_endpoint)
    .bind(&conn.s3_region)
    .bind(&conn.s3_bucket)
    .bind(conn.s3_path_style)
    .bind(&conn.azure_account)
    .bind(&conn.azure_container)
    .bind(&conn.azure_auth_kind)
    .bind(&conn.id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_by_id(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM explorer_connections WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn touch_last_used(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query("UPDATE explorer_connections SET last_used_at = ? WHERE id = ?")
        .bind(ts)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

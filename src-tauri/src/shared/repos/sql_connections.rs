use sqlx::SqlitePool;

use crate::modes::sql::client::{SqlSavedConnection, SqlScript};

// ---------------------------------------------------------------------------
// sql_connections
// ---------------------------------------------------------------------------

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<SqlSavedConnection>, sqlx::Error> {
    sqlx::query_as::<_, SqlSavedConnection>(
        "SELECT * FROM sql_connections ORDER BY sort_order ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<SqlSavedConnection, sqlx::Error> {
    sqlx::query_as::<_, SqlSavedConnection>("SELECT * FROM sql_connections WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Variant of [`get_by_id`] that returns `Ok(None)` when no row matches, instead
/// of `Err(sqlx::Error::RowNotFound)`. Useful for callers that want to fall back
/// to a different lookup path on a missing id.
pub async fn get_by_id_optional(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<SqlSavedConnection>, sqlx::Error> {
    sqlx::query_as::<_, SqlSavedConnection>("SELECT * FROM sql_connections WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn max_sort_order(pool: &SqlitePool) -> Result<(i32,), sqlx::Error> {
    sqlx::query_as("SELECT COALESCE(MAX(sort_order), -1) FROM sql_connections")
        .fetch_one(pool)
        .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    driver: &str,
    host: &str,
    port: i32,
    database: &str,
    username: &str,
    password: &str,
    ssl: i32,
    sort_order: i32,
    ssh_profile_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sql_connections (id, name, driver, host, port, database_name, username, password, ssl, sort_order, ssh_profile_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(driver)
    .bind(host)
    .bind(port)
    .bind(database)
    .bind(username)
    .bind(password)
    .bind(ssl)
    .bind(sort_order)
    .bind(ssh_profile_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_by_id(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sql_connections WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    driver: &str,
    host: &str,
    port: i32,
    database: &str,
    username: &str,
    password: &str,
    ssl: i32,
    ssh_profile_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sql_connections SET name = ?, driver = ?, host = ?, port = ?, database_name = ?, username = ?, password = ?, ssl = ?, ssh_profile_id = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(name)
    .bind(driver)
    .bind(host)
    .bind(port)
    .bind(database)
    .bind(username)
    .bind(password)
    .bind(ssl)
    .bind(ssh_profile_id)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// sql_scripts
// ---------------------------------------------------------------------------

pub async fn list_scripts(pool: &SqlitePool) -> Result<Vec<SqlScript>, sqlx::Error> {
    sqlx::query_as::<_, SqlScript>("SELECT * FROM sql_scripts ORDER BY sort_order ASC")
        .fetch_all(pool)
        .await
}

pub async fn get_script_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<SqlScript, sqlx::Error> {
    sqlx::query_as::<_, SqlScript>("SELECT * FROM sql_scripts WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

pub async fn max_script_sort_order(pool: &SqlitePool) -> Result<(i32,), sqlx::Error> {
    sqlx::query_as("SELECT COALESCE(MAX(sort_order), -1) FROM sql_scripts")
        .fetch_one(pool)
        .await
}

pub async fn insert_script(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    connection_id: Option<&str>,
    database_name: &str,
    query: &str,
    sort_order: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sql_scripts (id, name, connection_id, database_name, query, sort_order)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(connection_id)
    .bind(database_name)
    .bind(query)
    .bind(sort_order)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_script(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    query: &str,
    database_name: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sql_scripts SET name = ?, query = ?, database_name = COALESCE(?, database_name), updated_at = datetime('now') WHERE id = ?",
    )
    .bind(name)
    .bind(query)
    .bind(database_name)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_script(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sql_scripts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

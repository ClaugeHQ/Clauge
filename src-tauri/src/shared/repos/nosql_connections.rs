use sqlx::SqlitePool;

use crate::modes::nosql::client::NoSqlConnection;

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<NoSqlConnection>, sqlx::Error> {
    sqlx::query_as::<_, NoSqlConnection>(
        "SELECT * FROM nosql_connections ORDER BY sort_order ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<NoSqlConnection, sqlx::Error> {
    sqlx::query_as::<_, NoSqlConnection>("SELECT * FROM nosql_connections WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

pub async fn max_sort_order(pool: &SqlitePool) -> Result<(i32,), sqlx::Error> {
    sqlx::query_as("SELECT COALESCE(MAX(sort_order), -1) FROM nosql_connections")
        .fetch_one(pool)
        .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    driver: &str,
    connection_string: &str,
    host: &str,
    port: i32,
    database: &str,
    username: &str,
    password: &str,
    ssl: i32,
    direct_connection: i32,
    sort_order: i32,
    ssh_profile_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO nosql_connections (id, name, driver, connection_string, host, port, database_name, username, password, ssl, direct_connection, sort_order, ssh_profile_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(driver)
    .bind(connection_string)
    .bind(host)
    .bind(port)
    .bind(database)
    .bind(username)
    .bind(password)
    .bind(ssl)
    .bind(direct_connection)
    .bind(sort_order)
    .bind(ssh_profile_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_by_id(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM nosql_connections WHERE id = ?")
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
    connection_string: &str,
    host: &str,
    port: i32,
    database: &str,
    username: &str,
    password: &str,
    ssl: i32,
    direct_connection: i32,
    ssh_profile_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE nosql_connections SET name = ?, driver = ?, connection_string = ?, host = ?, port = ?, database_name = ?, username = ?, password = ?, ssl = ?, direct_connection = ?, ssh_profile_id = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(name)
    .bind(driver)
    .bind(connection_string)
    .bind(host)
    .bind(port)
    .bind(database)
    .bind(username)
    .bind(password)
    .bind(ssl)
    .bind(direct_connection)
    .bind(ssh_profile_id)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

use crate::db::models::Collection;
use crate::shared::repos::collections as collections_repo;

#[tauri::command]
pub async fn list_collections(pool: State<'_, SqlitePool>) -> Result<Vec<Collection>, String> {
    collections_repo::list_all(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_collection(
    pool: State<'_, SqlitePool>,
    name: String,
) -> Result<Collection, String> {
    let id = Uuid::new_v4().to_string();

    let max_order = collections_repo::max_sort_order(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    collections_repo::insert(pool.inner(), &id, &name, max_order.0 + 1)
        .await
        .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("rest");

    collections_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_collection(
    pool: State<'_, SqlitePool>,
    id: String,
    name: String,
    env_id: Option<String>,
) -> Result<Collection, String> {
    collections_repo::update(pool.inner(), &id, &name, env_id.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("rest");

    collections_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_collection(pool: State<'_, SqlitePool>, id: String) -> Result<(), String> {
    collections_repo::delete_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("rest");
    Ok(())
}

#[tauri::command]
pub async fn reorder_collections(
    pool: State<'_, SqlitePool>,
    ids: Vec<String>,
) -> Result<(), String> {
    for (i, id) in ids.iter().enumerate() {
        collections_repo::update_sort_order(pool.inner(), id, i as i32)
            .await
            .map_err(|e| e.to_string())?;
    }
    crate::cloud::scheduler::bump("rest");
    Ok(())
}

use sqlx::SqlitePool;
use std::collections::HashMap;
use tauri::State;
use uuid::Uuid;

use crate::db::models::{EnvVariable, Environment};
use crate::shared::repos::environments as environments_repo;

#[tauri::command]
pub async fn list_environments(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<Environment>, String> {
    environments_repo::list_all(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_environment(
    pool: State<'_, SqlitePool>,
    name: String,
    color: String,
) -> Result<Environment, String> {
    let id = Uuid::new_v4().to_string();

    let max_order = environments_repo::max_sort_order(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    // Auto-set as default if no environments exist yet
    let count = environments_repo::count(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
    let is_default = if count.0 == 0 { 1 } else { 0 };

    environments_repo::insert(
        pool.inner(),
        &id,
        &name,
        &color,
        is_default,
        max_order.0 + 1,
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("rest");

    environments_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_environment(
    pool: State<'_, SqlitePool>,
    id: String,
    name: String,
    color: String,
) -> Result<Environment, String> {
    environments_repo::update(pool.inner(), &id, &name, &color)
        .await
        .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("rest");

    environments_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_environment(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    let env = environments_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;

    // Delete the environment
    environments_repo::delete_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;

    // If it was the default, promote another env to default (if any remain)
    if env.is_default == 1 {
        // No-op if no environments remain
        let _ = environments_repo::promote_first_to_default(pool.inner()).await;
    }

    crate::cloud::scheduler::bump("rest");
    Ok(())
}

#[tauri::command]
pub async fn set_default_environment(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    // Set is_default=0 on all environments
    environments_repo::clear_default_flag(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    // Set is_default=1 on the specified one
    environments_repo::set_default(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn list_env_variables(
    pool: State<'_, SqlitePool>,
    environment_id: String,
) -> Result<Vec<EnvVariable>, String> {
    environments_repo::list_variables(pool.inner(), &environment_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_env_variable(
    pool: State<'_, SqlitePool>,
    environment_id: String,
    key: String,
    value: String,
    is_secret: i32,
) -> Result<EnvVariable, String> {
    let id = Uuid::new_v4().to_string();

    let max_order = environments_repo::max_variable_sort_order(pool.inner(), &environment_id)
        .await
        .map_err(|e| e.to_string())?;

    // Upsert: try to find existing variable with same environment_id + key
    let existing = environments_repo::get_variable_by_env_and_key(
        pool.inner(),
        &environment_id,
        &key,
    )
    .await
    .map_err(|e| e.to_string())?;

    let final_id = if let Some(existing) = existing {
        environments_repo::update_variable_value(pool.inner(), &existing.id, &value, is_secret)
            .await
            .map_err(|e| e.to_string())?;
        existing.id
    } else {
        environments_repo::insert_variable(
            pool.inner(),
            &id,
            &environment_id,
            &key,
            &value,
            is_secret,
            max_order.0 + 1,
        )
        .await
        .map_err(|e| e.to_string())?;
        id
    };

    crate::cloud::scheduler::bump("rest");

    environments_repo::get_variable_by_id(pool.inner(), &final_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_env_variable(
    pool: State<'_, SqlitePool>,
    id: String,
    key: String,
    value: String,
    is_secret: i32,
) -> Result<EnvVariable, String> {
    environments_repo::update_variable(pool.inner(), &id, &key, &value, is_secret)
        .await
        .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("rest");

    environments_repo::get_variable_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_env_variable(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    environments_repo::delete_variable_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("rest");
    Ok(())
}

#[tauri::command]
pub async fn get_env_variables_for_resolution(
    pool: State<'_, SqlitePool>,
    environment_id: String,
) -> Result<HashMap<String, String>, String> {
    let vars = environments_repo::list_variables_unsorted(pool.inner(), &environment_id)
        .await
        .map_err(|e| e.to_string())?;

    let mut map = HashMap::new();
    for var in vars {
        map.insert(var.key, var.value);
    }
    Ok(map)
}

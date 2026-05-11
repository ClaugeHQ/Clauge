use serde::Deserialize;
use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

use crate::db::models::{Request, RequestHeader, RequestParam};
use crate::shared::repos::requests as requests_repo;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestUpdate {
    pub name: Option<String>,
    pub method: Option<String>,
    pub url: Option<String>,
    pub body: Option<String>,
    pub body_type: Option<String>,
    pub auth_type: Option<String>,
    pub auth_data: Option<String>,
    pub pre_script: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct KVInput {
    pub key: String,
    pub value: String,
    pub enabled: i32,
}

#[derive(Debug, serde::Serialize)]
pub struct RequestWithDetails {
    #[serde(flatten)]
    pub request: Request,
    pub headers: Vec<RequestHeader>,
    pub params: Vec<RequestParam>,
}

#[tauri::command]
pub async fn list_requests(
    pool: State<'_, SqlitePool>,
    collection_id: String,
) -> Result<Vec<Request>, String> {
    requests_repo::list_by_collection(pool.inner(), &collection_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_request(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<RequestWithDetails, String> {
    let request = requests_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;

    let headers = requests_repo::list_headers(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;

    let params = requests_repo::list_params(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(RequestWithDetails {
        request,
        headers,
        params,
    })
}

#[tauri::command]
pub async fn create_request(
    pool: State<'_, SqlitePool>,
    collection_id: String,
    name: String,
    method: String,
) -> Result<Request, String> {
    let id = Uuid::new_v4().to_string();

    let max_order = requests_repo::max_sort_order(pool.inner(), &collection_id)
        .await
        .map_err(|e| e.to_string())?;

    requests_repo::insert(
        pool.inner(),
        &id,
        &collection_id,
        &name,
        &method,
        max_order.0 + 1,
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("rest");

    requests_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_request(
    pool: State<'_, SqlitePool>,
    id: String,
    data: RequestUpdate,
) -> Result<Request, String> {
    // Build dynamic update query
    let mut sets: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();

    if let Some(ref name) = data.name {
        sets.push("name = ?".to_string());
        values.push(name.clone());
    }
    if let Some(ref method) = data.method {
        sets.push("method = ?".to_string());
        values.push(method.clone());
    }
    if let Some(ref url) = data.url {
        sets.push("url = ?".to_string());
        values.push(url.clone());
    }
    if let Some(ref body) = data.body {
        sets.push("body = ?".to_string());
        values.push(body.clone());
    }
    if let Some(ref body_type) = data.body_type {
        sets.push("body_type = ?".to_string());
        values.push(body_type.clone());
    }
    if let Some(ref auth_type) = data.auth_type {
        sets.push("auth_type = ?".to_string());
        values.push(auth_type.clone());
    }
    if let Some(ref auth_data) = data.auth_data {
        sets.push("auth_data = ?".to_string());
        values.push(auth_data.clone());
    }
    if let Some(ref pre_script) = data.pre_script {
        sets.push("pre_script = ?".to_string());
        values.push(pre_script.clone());
    }

    requests_repo::update_dynamic(pool.inner(), &sets, &values, &id)
        .await
        .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("rest");

    requests_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_request(pool: State<'_, SqlitePool>, id: String) -> Result<(), String> {
    requests_repo::delete_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::cloud::scheduler::bump("rest");
    Ok(())
}

#[tauri::command]
pub async fn duplicate_request(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<Request, String> {
    let original = requests_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;

    let new_id = Uuid::new_v4().to_string();

    let max_order = requests_repo::max_sort_order(pool.inner(), &original.collection_id)
        .await
        .map_err(|e| e.to_string())?;

    let copy_name = format!("{} (copy)", &original.name);
    requests_repo::insert_full(
        pool.inner(),
        &new_id,
        &original.collection_id,
        &copy_name,
        &original.description,
        &original.method,
        &original.url,
        &original.body,
        &original.body_type,
        &original.auth_type,
        &original.auth_data,
        &original.pre_script,
        max_order.0 + 1,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Duplicate headers
    let headers = requests_repo::list_headers_unsorted(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;

    for h in &headers {
        let hid = Uuid::new_v4().to_string();
        requests_repo::insert_header(
            pool.inner(),
            &hid,
            &new_id,
            &h.key,
            &h.value,
            h.enabled,
            h.sort_order,
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    // Duplicate params
    let params = requests_repo::list_params_unsorted(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?;

    for p in &params {
        let pid = Uuid::new_v4().to_string();
        requests_repo::insert_param(
            pool.inner(),
            &pid,
            &new_id,
            &p.key,
            &p.value,
            p.enabled,
            p.sort_order,
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    crate::cloud::scheduler::bump("rest");

    requests_repo::get_by_id(pool.inner(), &new_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn move_request(
    pool: State<'_, SqlitePool>,
    id: String,
    target_collection_id: String,
) -> Result<Request, String> {
    let max_order = requests_repo::max_sort_order(pool.inner(), &target_collection_id)
        .await
        .map_err(|e| e.to_string())?;

    requests_repo::move_to_collection(pool.inner(), &id, &target_collection_id, max_order.0 + 1)
        .await
        .map_err(|e| e.to_string())?;

    crate::cloud::scheduler::bump("rest");

    requests_repo::get_by_id(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_request_headers(
    pool: State<'_, SqlitePool>,
    request_id: String,
    headers: Vec<KVInput>,
) -> Result<Vec<RequestHeader>, String> {
    // Delete existing headers
    requests_repo::delete_headers_for_request(pool.inner(), &request_id)
        .await
        .map_err(|e| e.to_string())?;

    // Insert new headers
    for (i, h) in headers.iter().enumerate() {
        let id = Uuid::new_v4().to_string();
        requests_repo::insert_header(
            pool.inner(),
            &id,
            &request_id,
            &h.key,
            &h.value,
            h.enabled,
            i as i32,
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    crate::cloud::scheduler::bump("rest");

    requests_repo::list_headers(pool.inner(), &request_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_request_params(
    pool: State<'_, SqlitePool>,
    request_id: String,
    params: Vec<KVInput>,
) -> Result<Vec<RequestParam>, String> {
    // Delete existing params
    requests_repo::delete_params_for_request(pool.inner(), &request_id)
        .await
        .map_err(|e| e.to_string())?;

    // Insert new params
    for (i, p) in params.iter().enumerate() {
        let id = Uuid::new_v4().to_string();
        requests_repo::insert_param(
            pool.inner(),
            &id,
            &request_id,
            &p.key,
            &p.value,
            p.enabled,
            i as i32,
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    crate::cloud::scheduler::bump("rest");

    requests_repo::list_params(pool.inner(), &request_id)
        .await
        .map_err(|e| e.to_string())
}

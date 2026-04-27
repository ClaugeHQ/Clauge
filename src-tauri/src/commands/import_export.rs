use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

use crate::db::models::{Collection, Request, RequestHeader, RequestParam};

// ── Serialization structs ────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub collections_count: usize,
    pub requests_count: usize,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClaugeExport {
    format: String,
    exported_at: String,
    collection: Option<ClaugeCollection>,
    collections: Option<Vec<ClaugeCollection>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ClaugeCollection {
    name: String,
    description: String,
    requests: Vec<ClaugeRequest>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ClaugeRequest {
    name: String,
    method: String,
    url: String,
    body: String,
    body_type: String,
    auth_type: String,
    auth_data: String,
    #[serde(default)]
    pre_script: String,
    headers: Vec<ClaugeKV>,
    params: Vec<ClaugeKV>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ClaugeKV {
    key: String,
    value: String,
    enabled: i32,
}

// ── Postman v2.1 structs ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PostmanCollection {
    info: PostmanInfo,
    item: Vec<PostmanItem>,
    #[serde(default)]
    auth: Option<PostmanAuth>,
}

#[derive(Debug, Deserialize)]
struct PostmanInfo {
    name: String,
    #[serde(default)]
    schema: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PostmanItem {
    name: String,
    #[serde(default)]
    request: Option<PostmanRequest>,
    #[serde(default)]
    item: Option<Vec<PostmanItem>>,
    #[serde(default)]
    auth: Option<PostmanAuth>,
}

#[derive(Debug, Deserialize)]
struct PostmanRequest {
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    header: Option<Vec<PostmanHeader>>,
    #[serde(default)]
    url: Option<PostmanUrl>,
    #[serde(default)]
    body: Option<PostmanBody>,
    #[serde(default)]
    auth: Option<PostmanAuth>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PostmanUrl {
    Raw(String),
    Structured(PostmanUrlStructured),
}

#[derive(Debug, Deserialize)]
struct PostmanUrlStructured {
    #[serde(default)]
    raw: Option<String>,
    #[serde(default)]
    host: Option<Vec<String>>,
    #[serde(default)]
    path: Option<Vec<String>>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    query: Option<Vec<PostmanQuery>>,
}

#[derive(Debug, Deserialize)]
struct PostmanQuery {
    key: String,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    disabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PostmanHeader {
    key: String,
    value: String,
    #[serde(default)]
    disabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PostmanBody {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    raw: Option<String>,
    #[serde(default)]
    urlencoded: Option<Vec<PostmanKV>>,
    #[serde(default)]
    formdata: Option<Vec<PostmanKV>>,
}

#[derive(Debug, Deserialize)]
struct PostmanKV {
    key: String,
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct PostmanAuth {
    #[serde(rename = "type")]
    auth_type: String,
    #[serde(default)]
    bearer: Option<Vec<PostmanAuthKV>>,
    #[serde(default)]
    basic: Option<Vec<PostmanAuthKV>>,
}

#[derive(Debug, Deserialize, Clone)]
struct PostmanAuthKV {
    key: String,
    value: serde_json::Value,
}

// ── Helper: insert a collection into DB ─────────────────────────────

async fn insert_collection(
    pool: &SqlitePool,
    coll: &ClaugeCollection,
) -> Result<(String, usize), String> {
    let coll_id = Uuid::new_v4().to_string();
    let max_order: (i32,) =
        sqlx::query_as("SELECT COALESCE(MAX(sort_order), -1) FROM collections")
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO collections (id, name, description, sort_order) VALUES (?, ?, ?, ?)",
    )
    .bind(&coll_id)
    .bind(&coll.name)
    .bind(&coll.description)
    .bind(max_order.0 + 1)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut req_count = 0usize;
    for (i, req) in coll.requests.iter().enumerate() {
        insert_request(pool, &coll_id, req, i as i32).await?;
        req_count += 1;
    }

    Ok((coll_id, req_count))
}

async fn insert_request(
    pool: &SqlitePool,
    collection_id: &str,
    req: &ClaugeRequest,
    sort_order: i32,
) -> Result<String, String> {
    let req_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO requests (id, collection_id, name, method, url, body, body_type, auth_type, auth_data, pre_script, sort_order) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&req_id)
    .bind(collection_id)
    .bind(&req.name)
    .bind(&req.method)
    .bind(&req.url)
    .bind(&req.body)
    .bind(&req.body_type)
    .bind(&req.auth_type)
    .bind(&req.auth_data)
    .bind(&req.pre_script)
    .bind(sort_order)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Insert headers
    for (j, h) in req.headers.iter().enumerate() {
        let hid = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO request_headers (id, request_id, key, value, enabled, sort_order) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&hid)
        .bind(&req_id)
        .bind(&h.key)
        .bind(&h.value)
        .bind(h.enabled)
        .bind(j as i32)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    // Insert params
    for (j, p) in req.params.iter().enumerate() {
        let pid = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO request_params (id, request_id, key, value, enabled, sort_order) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&pid)
        .bind(&req_id)
        .bind(&p.key)
        .bind(&p.value)
        .bind(p.enabled)
        .bind(j as i32)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(req_id)
}

// ── Helper: load a collection with all requests/headers/params ──────

async fn load_collection_export(
    pool: &SqlitePool,
    coll: &Collection,
) -> Result<ClaugeCollection, String> {
    let requests = sqlx::query_as::<_, Request>(
        "SELECT * FROM requests WHERE collection_id = ? ORDER BY sort_order ASC",
    )
    .bind(&coll.id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut clauge_requests = Vec::new();
    for req in &requests {
        let headers = sqlx::query_as::<_, RequestHeader>(
            "SELECT * FROM request_headers WHERE request_id = ? ORDER BY sort_order ASC",
        )
        .bind(&req.id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

        let params = sqlx::query_as::<_, RequestParam>(
            "SELECT * FROM request_params WHERE request_id = ? ORDER BY sort_order ASC",
        )
        .bind(&req.id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

        clauge_requests.push(ClaugeRequest {
            name: req.name.clone(),
            method: req.method.clone(),
            url: req.url.clone(),
            body: req.body.clone(),
            body_type: req.body_type.clone(),
            auth_type: req.auth_type.clone(),
            auth_data: req.auth_data.clone(),
            pre_script: req.pre_script.clone(),
            headers: headers
                .iter()
                .map(|h| ClaugeKV {
                    key: h.key.clone(),
                    value: h.value.clone(),
                    enabled: h.enabled,
                })
                .collect(),
            params: params
                .iter()
                .map(|p| ClaugeKV {
                    key: p.key.clone(),
                    value: p.value.clone(),
                    enabled: p.enabled,
                })
                .collect(),
        });
    }

    Ok(ClaugeCollection {
        name: coll.name.clone(),
        description: coll.description.clone(),
        requests: clauge_requests,
    })
}

// ── Export Collection ────────────────────────────────────────────────

#[tauri::command]
pub async fn export_collection(
    pool: State<'_, SqlitePool>,
    collection_id: String,
) -> Result<String, String> {
    let coll = sqlx::query_as::<_, Collection>("SELECT * FROM collections WHERE id = ?")
        .bind(&collection_id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| format!("Collection not found: {}", e))?;

    let clauge_coll = load_collection_export(pool.inner(), &coll).await?;

    let export = ClaugeExport {
        format: "clauge/collection/v1".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        collection: Some(clauge_coll),
        collections: None,
    };

    serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
}

// ── Export All Collections ──────────────────────────────────────────

#[tauri::command]
pub async fn export_all_collections(
    pool: State<'_, SqlitePool>,
) -> Result<String, String> {
    let colls = sqlx::query_as::<_, Collection>(
        "SELECT * FROM collections ORDER BY sort_order ASC",
    )
    .fetch_all(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    let mut clauge_colls = Vec::new();
    for coll in &colls {
        clauge_colls.push(load_collection_export(pool.inner(), coll).await?);
    }

    let export = ClaugeExport {
        format: "clauge/collections/v1".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        collection: None,
        collections: Some(clauge_colls),
    };

    serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
}

// ── Import Clauge JSON ──────────────────────────────────────────────

#[tauri::command]
pub async fn import_clauge(
    pool: State<'_, SqlitePool>,
    json: String,
) -> Result<ImportResult, String> {
    let export: ClaugeExport =
        serde_json::from_str(&json).map_err(|e| format!("Invalid Clauge JSON: {}", e))?;

    if !export.format.starts_with("clauge/") {
        return Err("Not a valid Clauge export format".to_string());
    }

    let mut total_collections = 0usize;
    let mut total_requests = 0usize;

    // Single collection
    if let Some(coll) = &export.collection {
        let (_, rc) = insert_collection(pool.inner(), coll).await?;
        total_collections += 1;
        total_requests += rc;
    }

    // Multiple collections
    if let Some(colls) = &export.collections {
        for coll in colls {
            let (_, rc) = insert_collection(pool.inner(), coll).await?;
            total_collections += 1;
            total_requests += rc;
        }
    }

    Ok(ImportResult {
        collections_count: total_collections,
        requests_count: total_requests,
        message: format!(
            "Imported {} collection(s) with {} request(s)",
            total_collections, total_requests
        ),
    })
}

// ── Import Postman v2.1 ─────────────────────────────────────────────

fn extract_postman_url(url: &Option<PostmanUrl>) -> String {
    match url {
        Some(PostmanUrl::Raw(s)) => s.clone(),
        Some(PostmanUrl::Structured(s)) => {
            if let Some(raw) = &s.raw {
                return raw.clone();
            }
            // Reconstruct from host + path
            let protocol = s.protocol.as_deref().unwrap_or("https");
            let host = s
                .host
                .as_ref()
                .map(|h| h.join("."))
                .unwrap_or_default();
            let path = s
                .path
                .as_ref()
                .map(|p| p.join("/"))
                .unwrap_or_default();
            if host.is_empty() {
                return path;
            }
            format!("{}://{}/{}", protocol, host, path)
        }
        None => String::new(),
    }
}

fn extract_postman_params(url: &Option<PostmanUrl>) -> Vec<ClaugeKV> {
    match url {
        Some(PostmanUrl::Structured(s)) => s
            .query
            .as_ref()
            .map(|qs| {
                qs.iter()
                    .map(|q| ClaugeKV {
                        key: q.key.clone(),
                        value: q.value.clone().unwrap_or_default(),
                        enabled: if q.disabled.unwrap_or(false) { 0 } else { 1 },
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn map_postman_body_type(mode: &str) -> &str {
    match mode {
        "raw" => "json",
        "urlencoded" => "form",
        "formdata" => "form",
        _ => "none",
    }
}

fn extract_postman_auth(auth: &Option<PostmanAuth>) -> (String, String) {
    match auth {
        Some(a) => match a.auth_type.as_str() {
            "bearer" => {
                let token = a
                    .bearer
                    .as_ref()
                    .and_then(|b| {
                        b.iter()
                            .find(|kv| kv.key == "token")
                            .map(|kv| match &kv.value {
                                serde_json::Value::String(s) => s.clone(),
                                v => v.to_string(),
                            })
                    })
                    .unwrap_or_default();
                ("bearer".to_string(), token)
            }
            "basic" => {
                let username = a
                    .basic
                    .as_ref()
                    .and_then(|b| {
                        b.iter()
                            .find(|kv| kv.key == "username")
                            .map(|kv| match &kv.value {
                                serde_json::Value::String(s) => s.clone(),
                                v => v.to_string(),
                            })
                    })
                    .unwrap_or_default();
                let password = a
                    .basic
                    .as_ref()
                    .and_then(|b| {
                        b.iter()
                            .find(|kv| kv.key == "password")
                            .map(|kv| match &kv.value {
                                serde_json::Value::String(s) => s.clone(),
                                v => v.to_string(),
                            })
                    })
                    .unwrap_or_default();
                (
                    "basic".to_string(),
                    serde_json::json!({"username": username, "password": password}).to_string(),
                )
            }
            _ => ("none".to_string(), String::new()),
        },
        None => ("none".to_string(), String::new()),
    }
}

fn flatten_postman_items(
    items: &[PostmanItem],
    parent_auth: &Option<PostmanAuth>,
    collections: &mut Vec<(String, Vec<ClaugeRequest>)>,
    collection_name: &str,
) {
    let mut top_requests: Vec<ClaugeRequest> = Vec::new();

    for item in items {
        if let Some(ref request) = item.request {
            // This is a request item
            let method = request
                .method
                .as_deref()
                .unwrap_or("GET")
                .to_uppercase();
            let url = extract_postman_url(&request.url);
            let params = extract_postman_params(&request.url);

            let headers: Vec<ClaugeKV> = request
                .header
                .as_ref()
                .map(|hs| {
                    hs.iter()
                        .map(|h| ClaugeKV {
                            key: h.key.clone(),
                            value: h.value.clone(),
                            enabled: if h.disabled.unwrap_or(false) { 0 } else { 1 },
                        })
                        .collect()
                })
                .unwrap_or_default();

            let (body, body_type) = match &request.body {
                Some(b) => {
                    let bt = b.mode.as_deref().unwrap_or("none");
                    let body_str = match bt {
                        "raw" => b.raw.clone().unwrap_or_default(),
                        "urlencoded" => b
                            .urlencoded
                            .as_ref()
                            .map(|kvs| {
                                kvs.iter()
                                    .map(|kv| {
                                        format!(
                                            "{}={}",
                                            kv.key,
                                            kv.value.as_deref().unwrap_or("")
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join("&")
                            })
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    (body_str, map_postman_body_type(bt).to_string())
                }
                None => (String::new(), "none".to_string()),
            };

            // Auth: request-level > item-level > parent (collection) level
            let effective_auth = request
                .auth
                .as_ref()
                .or(item.auth.as_ref())
                .or(parent_auth.as_ref());
            let (auth_type, auth_data) = extract_postman_auth(&effective_auth.cloned());

            top_requests.push(ClaugeRequest {
                name: item.name.clone(),
                method,
                url,
                body,
                body_type,
                auth_type,
                auth_data,
                pre_script: String::new(),
                headers,
                params,
            });
        } else if let Some(ref sub_items) = item.item {
            // This is a folder — create a sub-collection
            let folder_name = format!("{} / {}", collection_name, item.name);
            flatten_postman_items(
                sub_items,
                &item.auth.as_ref().or(parent_auth.as_ref()).cloned(),
                collections,
                &folder_name,
            );
        }
    }

    if !top_requests.is_empty() {
        // Check if this collection name already exists in the list; merge if so
        if let Some(existing) = collections.iter_mut().find(|(n, _)| n == collection_name) {
            existing.1.extend(top_requests);
        } else {
            collections.push((collection_name.to_string(), top_requests));
        }
    }
}

#[tauri::command]
pub async fn import_postman(
    pool: State<'_, SqlitePool>,
    json: String,
) -> Result<ImportResult, String> {
    let postman: PostmanCollection =
        serde_json::from_str(&json).map_err(|e| format!("Invalid Postman JSON: {}", e))?;

    let mut flat_collections: Vec<(String, Vec<ClaugeRequest>)> = Vec::new();
    flatten_postman_items(
        &postman.item,
        &postman.auth,
        &mut flat_collections,
        &postman.info.name,
    );

    let mut total_collections = 0usize;
    let mut total_requests = 0usize;

    for (name, requests) in &flat_collections {
        let coll = ClaugeCollection {
            name: name.clone(),
            description: String::new(),
            requests: requests.clone(),
        };
        let (_, rc) = insert_collection(pool.inner(), &coll).await?;
        total_collections += 1;
        total_requests += rc;
    }

    Ok(ImportResult {
        collections_count: total_collections,
        requests_count: total_requests,
        message: format!(
            "Imported {} collection(s) with {} request(s) from Postman",
            total_collections, total_requests
        ),
    })
}

// ── Import cURL ─────────────────────────────────────────────────────

fn parse_curl(curl_command: &str) -> Result<ClaugeRequest, String> {
    let input = curl_command.trim();
    if !input.starts_with("curl") && !input.starts_with("curl") {
        return Err("Not a valid cURL command".to_string());
    }

    // Tokenize: handle single/double quotes and backslash-newline continuations
    let normalized = input.replace("\\\n", " ").replace("\\\r\n", " ");
    let tokens = shell_tokenize(&normalized)?;

    let mut method = String::new();
    let mut url = String::new();
    let mut headers: Vec<ClaugeKV> = Vec::new();
    let mut body = String::new();
    let mut auth_type = "none".to_string();
    let mut auth_data = String::new();

    let mut i = 1; // skip "curl"
    while i < tokens.len() {
        let tok = &tokens[i];
        match tok.as_str() {
            "-X" | "--request" => {
                i += 1;
                if i < tokens.len() {
                    method = tokens[i].to_uppercase();
                }
            }
            "-H" | "--header" => {
                i += 1;
                if i < tokens.len() {
                    if let Some((k, v)) = tokens[i].split_once(':') {
                        headers.push(ClaugeKV {
                            key: k.trim().to_string(),
                            value: v.trim().to_string(),
                            enabled: 1,
                        });
                    }
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" => {
                i += 1;
                if i < tokens.len() {
                    body = tokens[i].clone();
                }
            }
            "-u" | "--user" => {
                i += 1;
                if i < tokens.len() {
                    let cred = &tokens[i];
                    if let Some((user, pass)) = cred.split_once(':') {
                        auth_type = "basic".to_string();
                        auth_data = serde_json::json!({
                            "username": user,
                            "password": pass
                        })
                        .to_string();
                    }
                }
            }
            "-b" | "--cookie" => {
                i += 1;
                if i < tokens.len() {
                    headers.push(ClaugeKV {
                        key: "Cookie".to_string(),
                        value: tokens[i].clone(),
                        enabled: 1,
                    });
                }
            }
            _ => {
                // Could be the URL (first non-flag argument)
                if !tok.starts_with('-') && url.is_empty() {
                    url = tok.clone();
                }
            }
        }
        i += 1;
    }

    if method.is_empty() {
        method = if body.is_empty() {
            "GET".to_string()
        } else {
            "POST".to_string()
        };
    }

    let body_type = if body.is_empty() {
        "none".to_string()
    } else {
        // Try to detect JSON
        if body.trim_start().starts_with('{') || body.trim_start().starts_with('[') {
            "json".to_string()
        } else {
            "raw".to_string()
        }
    };

    let name = url
        .split('?')
        .next()
        .and_then(|u| u.split('/').last())
        .unwrap_or("Imported Request")
        .to_string();

    Ok(ClaugeRequest {
        name: if name.is_empty() {
            "Imported Request".to_string()
        } else {
            name
        },
        method,
        url,
        body,
        body_type,
        auth_type,
        auth_data,
        pre_script: String::new(),
        headers,
        params: Vec::new(),
    })
}

fn shell_tokenize(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '\'' => {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    current.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("Unterminated single quote".to_string());
                }
                i += 1; // skip closing quote
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        current.push(chars[i]);
                    } else {
                        current.push(chars[i]);
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("Unterminated double quote".to_string());
                }
                i += 1; // skip closing quote
            }
            ' ' | '\t' => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                i += 1;
            }
            '\\' => {
                i += 1;
                if i < chars.len() {
                    current.push(chars[i]);
                }
                i += 1;
            }
            c => {
                current.push(c);
                i += 1;
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

#[tauri::command]
pub async fn import_curl(
    pool: State<'_, SqlitePool>,
    curl_command: String,
    collection_id: Option<String>,
) -> Result<String, String> {
    let req = parse_curl(&curl_command)?;

    let coll_id = match collection_id {
        Some(id) => {
            // Verify collection exists
            let _coll =
                sqlx::query_as::<_, Collection>("SELECT * FROM collections WHERE id = ?")
                    .bind(&id)
                    .fetch_optional(pool.inner())
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or("Collection not found")?;
            id
        }
        None => {
            // Create or find "Imported" collection
            let existing = sqlx::query_as::<_, Collection>(
                "SELECT * FROM collections WHERE name = 'Imported'",
            )
            .fetch_optional(pool.inner())
            .await
            .map_err(|e| e.to_string())?;

            match existing {
                Some(c) => c.id,
                None => {
                    let id = Uuid::new_v4().to_string();
                    let max_order: (i32,) = sqlx::query_as(
                        "SELECT COALESCE(MAX(sort_order), -1) FROM collections",
                    )
                    .fetch_one(pool.inner())
                    .await
                    .map_err(|e| e.to_string())?;

                    sqlx::query(
                        "INSERT INTO collections (id, name, sort_order) VALUES (?, 'Imported', ?)",
                    )
                    .bind(&id)
                    .bind(max_order.0 + 1)
                    .execute(pool.inner())
                    .await
                    .map_err(|e| e.to_string())?;
                    id
                }
            }
        }
    };

    // Get max sort order in collection
    let max_order: (i32,) = sqlx::query_as(
        "SELECT COALESCE(MAX(sort_order), -1) FROM requests WHERE collection_id = ?",
    )
    .bind(&coll_id)
    .fetch_one(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    let req_id = insert_request(pool.inner(), &coll_id, &req, max_order.0 + 1).await?;

    Ok(req_id)
}

// ── Export as cURL ──────────────────────────────────────────────────

#[tauri::command]
pub async fn export_as_curl(
    pool: State<'_, SqlitePool>,
    request_id: String,
    environment_id: Option<String>,
) -> Result<String, String> {
    let request = sqlx::query_as::<_, Request>("SELECT * FROM requests WHERE id = ?")
        .bind(&request_id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| format!("Request not found: {}", e))?;

    let headers = sqlx::query_as::<_, RequestHeader>(
        "SELECT * FROM request_headers WHERE request_id = ? ORDER BY sort_order ASC",
    )
    .bind(&request_id)
    .fetch_all(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    let params = sqlx::query_as::<_, RequestParam>(
        "SELECT * FROM request_params WHERE request_id = ? ORDER BY sort_order ASC",
    )
    .bind(&request_id)
    .fetch_all(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    // Optionally resolve variables
    let env_vars = if let Some(env_id) = &environment_id {
        crate::db::models::load_env_vars(pool.inner(), env_id).await
    } else {
        std::collections::HashMap::new()
    };

    let resolve = |s: &str| -> String {
        let mut result = s.to_string();
        for (key, value) in &env_vars {
            result = result.replace(&format!("{{{{{}}}}}", key), value);
        }
        result
    };

    // Build URL with query params
    let mut url = resolve(&request.url);
    let enabled_params: Vec<_> = params.iter().filter(|p| p.enabled == 1).collect();
    if !enabled_params.is_empty() {
        let separator = if url.contains('?') { "&" } else { "?" };
        let query: Vec<String> = enabled_params
            .iter()
            .map(|p| format!("{}={}", resolve(&p.key), resolve(&p.value)))
            .collect();
        url = format!("{}{}{}", url, separator, query.join("&"));
    }

    let mut cmd = format!("curl -X {} '{}'", request.method, url);

    for h in headers.iter().filter(|h| h.enabled == 1) {
        cmd += &format!(" \\\n  -H '{}: {}'", resolve(&h.key), resolve(&h.value));
    }

    // Auth
    if request.auth_type == "bearer" && !request.auth_data.is_empty() {
        let token = resolve(&request.auth_data);
        cmd += &format!(" \\\n  -H 'Authorization: Bearer {}'", token);
    } else if request.auth_type == "basic" && !request.auth_data.is_empty() {
        if let Ok(auth) = serde_json::from_str::<serde_json::Value>(&request.auth_data) {
            let user = auth["username"].as_str().unwrap_or("");
            let pass = auth["password"].as_str().unwrap_or("");
            cmd += &format!(" \\\n  -u '{}:{}'", resolve(user), resolve(pass));
        }
    }

    if !request.body.is_empty() && ["POST", "PUT", "PATCH"].contains(&request.method.as_str()) {
        let escaped_body = resolve(&request.body).replace('\'', "'\\''");
        cmd += &format!(" \\\n  -d '{}'", escaped_body);
    }

    Ok(cmd)
}

// REST mode AI tool handlers.
//
// Tools are invoked from the chat surface; each handler returns the
// already-serialised `String` body that goes back to the model as the
// `tool_result`. Behaviour parity note: every match arm here is a
// straight rehoming of the previous `commands::ai::tools::execute_tool`
// REST branches, so outbound model-facing text is byte-identical.
//
// Registration: `register_tools()` is called once from `lib.rs` setup.

use sqlx::SqlitePool;
use tauri::Emitter;

use crate::shared::ai::context::truncate_str;
use crate::shared::ai::dispatch::{register, ToolContext, ToolDescriptor, ToolFuture};

/// Resolve a request identifier — if it looks like a UUID use it directly,
/// otherwise try matching by name (case-insensitive).
async fn resolve_request_id(pool: &SqlitePool, input: &str) -> String {
    if input.len() == 36 && input.contains('-') {
        return input.to_string();
    }
    match sqlx::query_as::<_, (String,)>(
        "SELECT id FROM requests WHERE LOWER(name) = LOWER(?) ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(input)
    .fetch_optional(pool)
    .await
    {
        Ok(Some((id,))) => {
            log::info!("[AI Tool] resolved request name '{}' to id '{}'", input, id);
            id
        }
        _ => input.to_string(),
    }
}

fn execute_current_request<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let app = ctx.app;
        let session_id = ctx.session_id;
        match &ctx.context.current_request {
            Some(req) => {
                let header_pairs: Vec<(String, String)> = req
                    .headers
                    .iter()
                    .filter(|h| h.enabled)
                    .map(|h| (h.key.clone(), h.value.clone()))
                    .collect();

                match crate::modes::rest::http_executor::quick_execute_internal(
                    pool,
                    &req.method,
                    &req.url,
                    &req.body,
                    &header_pairs,
                    "",
                    &req.auth_type,
                    &req.auth_data,
                )
                .await
                {
                    Ok(response) => {
                        let body_preview = if response.body.len() > 2000 {
                            format!("{}... [truncated]", truncate_str(&response.body, 2000))
                        } else {
                            response.body.clone()
                        };
                        let _ = app.emit(
                            &format!("ai:action:{}", session_id),
                            serde_json::json!({
                                "action": "execute_result",
                                "data": {
                                    "status": response.status,
                                    "statusText": response.status_text,
                                    "durationMs": response.duration_ms,
                                    "sizeBytes": response.size_bytes,
                                    "body": body_preview,
                                },
                            }),
                        );
                        const MAX_BODY_BYTES: usize = 4096;
                        let body_sample = crate::shared::ai::sample::format_text_sample(&response.body, MAX_BODY_BYTES);
                        format!(
                            "Status: {} {}\nDuration: {}ms\nSize: {} bytes\nFull body is in the result card — do NOT re-print it. Sample for your reasoning:\n```\n{}\n```",
                            response.status, response.status_text, response.duration_ms, response.size_bytes, body_sample
                        )
                    }
                    Err(e) => format!("Execution failed: {}", e),
                }
            }
            None => "No request is currently open in the active tab.".to_string(),
        }
    })
}

fn list_collections<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let collections = sqlx::query_as::<_, (String, String)>(
            "SELECT id, name FROM collections ORDER BY sort_order ASC",
        )
        .fetch_all(pool)
        .await;

        match collections {
            Ok(colls) => {
                let mut result = Vec::new();
                for (coll_id, coll_name) in &colls {
                    let requests = sqlx::query_as::<_, (String, String, String, String)>(
                        "SELECT id, name, method, url FROM requests WHERE collection_id = ? ORDER BY sort_order ASC",
                    )
                    .bind(coll_id)
                    .fetch_all(pool)
                    .await
                    .unwrap_or_default();

                    let reqs: Vec<serde_json::Value> = requests
                        .iter()
                        .map(|(id, name, method, url)| {
                            serde_json::json!({"id": id, "name": name, "method": method, "url": url})
                        })
                        .collect();

                    result.push(
                        serde_json::json!({"id": coll_id, "name": coll_name, "requests": reqs}),
                    );
                }
                serde_json::to_string_pretty(&result).unwrap_or_else(|_| "[]".to_string())
            }
            Err(e) => format!("Error listing collections: {}", e),
        }
    })
}

fn get_request_details<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let input = ctx.input;
        let request_id_raw = input["request_id"].as_str().unwrap_or("");
        if request_id_raw.is_empty() {
            return "Error: request_id is required".to_string();
        }
        let request_id = resolve_request_id(pool, request_id_raw).await;

        let request = sqlx::query_as::<_, crate::db::models::Request>(
            "SELECT * FROM requests WHERE id = ?",
        )
        .bind(&request_id)
        .fetch_optional(pool)
        .await;

        match request {
            Ok(Some(req)) => {
                let headers = sqlx::query_as::<_, crate::db::models::RequestHeader>(
                    "SELECT * FROM request_headers WHERE request_id = ? ORDER BY sort_order ASC",
                )
                .bind(&request_id)
                .fetch_all(pool)
                .await
                .unwrap_or_default();

                let params = sqlx::query_as::<_, crate::db::models::RequestParam>(
                    "SELECT * FROM request_params WHERE request_id = ? ORDER BY sort_order ASC",
                )
                .bind(&request_id)
                .fetch_all(pool)
                .await
                .unwrap_or_default();

                let hdrs: Vec<serde_json::Value> = headers
                    .iter()
                    .map(|h| {
                        serde_json::json!({"key": h.key, "value": h.value, "enabled": h.enabled == 1})
                    })
                    .collect();
                let prms: Vec<serde_json::Value> = params
                    .iter()
                    .map(|p| {
                        serde_json::json!({"key": p.key, "value": p.value, "enabled": p.enabled == 1})
                    })
                    .collect();

                serde_json::to_string_pretty(&serde_json::json!({
                    "id": req.id, "name": req.name, "method": req.method,
                    "url": req.url, "body": req.body, "bodyType": req.body_type,
                    "authType": req.auth_type, "authData": req.auth_data,
                    "headers": hdrs, "params": prms,
                }))
                .unwrap_or_else(|_| "Error".to_string())
            }
            Ok(None) => "Request not found.".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    })
}

fn list_environments<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let envs = sqlx::query_as::<_, crate::db::models::Environment>(
            "SELECT * FROM environments ORDER BY sort_order ASC",
        )
        .fetch_all(pool)
        .await;

        match envs {
            Ok(environments) => {
                let mut result = Vec::new();
                for env in &environments {
                    let vars = sqlx::query_as::<_, crate::db::models::EnvVariable>(
                        "SELECT * FROM env_variables WHERE environment_id = ? ORDER BY sort_order ASC",
                    )
                    .bind(&env.id)
                    .fetch_all(pool)
                    .await
                    .unwrap_or_default();

                    let var_list: Vec<serde_json::Value> = vars
                        .iter()
                        .map(|v| {
                            serde_json::json!({
                                "key": v.key,
                                "value": if v.is_secret == 1 { "***".to_string() } else { v.value.clone() },
                            })
                        })
                        .collect();

                    result.push(
                        serde_json::json!({"id": env.id, "name": env.name, "variables": var_list}),
                    );
                }
                serde_json::to_string_pretty(&result).unwrap_or_else(|_| "[]".to_string())
            }
            Err(e) => format!("Error: {}", e),
        }
    })
}

fn get_history<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let input = ctx.input;
        let limit = input["limit"].as_i64().unwrap_or(10).min(50);
        let history = sqlx::query_as::<_, (String, String, Option<i32>, Option<i64>, String)>(
            "SELECT method, url, response_status, duration_ms, created_at FROM history ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await;

        match history {
            Ok(entries) => {
                let result: Vec<serde_json::Value> = entries
                    .iter()
                    .map(|(method, url, status, duration, created)| {
                        serde_json::json!({"method": method, "url": url, "status": status, "durationMs": duration, "createdAt": created})
                    })
                    .collect();
                serde_json::to_string_pretty(&result).unwrap_or_else(|_| "[]".to_string())
            }
            Err(e) => format!("Error: {}", e),
        }
    })
}

fn apply_request<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let _ = ctx.app.emit(
            &format!("ai:action:{}", ctx.session_id),
            serde_json::json!({"action": "apply_request", "data": ctx.input}),
        );
        "Request suggestion sent to user. They can click Apply to accept it.".to_string()
    })
}

fn execute_request<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let app = ctx.app;
        let session_id = ctx.session_id;
        let input = ctx.input;
        let request_id_input = input["request_id"].as_str().unwrap_or("");
        let environment_id = input["environment_id"].as_str().unwrap_or("");
        if request_id_input.is_empty() {
            return "Error: request_id is required".to_string();
        }

        let request_id = resolve_request_id(pool, request_id_input).await;

        match crate::modes::rest::http_executor::execute_request_internal(
            pool,
            &request_id,
            environment_id,
        )
        .await
        {
            Ok(response) => {
                let body_preview = if response.body.len() > 2000 {
                    format!("{}... [truncated]", truncate_str(&response.body, 2000))
                } else {
                    response.body.clone()
                };
                let _ = app.emit(
                    &format!("ai:action:{}", session_id),
                    serde_json::json!({
                        "action": "execute_result",
                        "data": {
                            "requestId": request_id,
                            "status": response.status,
                            "statusText": response.status_text,
                            "durationMs": response.duration_ms,
                            "sizeBytes": response.size_bytes,
                            "body": body_preview,
                        },
                    }),
                );
                const MAX_BODY_BYTES: usize = 4096;
                let body_sample = crate::shared::ai::sample::format_text_sample(&response.body, MAX_BODY_BYTES);
                format!(
                    "Status: {} {}\nDuration: {}ms\nSize: {} bytes\nFull body is in the result card — do NOT re-print it. Sample for your reasoning:\n```\n{}\n```",
                    response.status, response.status_text, response.duration_ms, response.size_bytes, body_sample,
                )
            }
            Err(e) => format!("Execution failed: {}", e),
        }
    })
}

fn create_request<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let app = ctx.app;
        let session_id = ctx.session_id;
        let input = ctx.input;
        let collection_id = input["collection_id"].as_str().unwrap_or("");
        let name = input["name"].as_str().unwrap_or("New Request");
        let method = input["method"].as_str().unwrap_or("GET");
        let url = input["url"].as_str().unwrap_or("");
        if collection_id.is_empty() {
            return "Error: collection_id is required".to_string();
        }

        let id = uuid::Uuid::new_v4().to_string();
        let max_order: Result<(i32,), _> = sqlx::query_as(
            "SELECT COALESCE(MAX(sort_order), -1) FROM requests WHERE collection_id = ?",
        )
        .bind(collection_id)
        .fetch_one(pool)
        .await;
        let order = max_order.map(|r| r.0 + 1).unwrap_or(0);

        match sqlx::query(
            "INSERT INTO requests (id, collection_id, name, method, url, sort_order) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(collection_id)
        .bind(name)
        .bind(method)
        .bind(url)
        .bind(order)
        .execute(pool)
        .await
        {
            Ok(_) => {
                let _ = app.emit(
                    &format!("ai:action:{}", session_id),
                    serde_json::json!({
                        "action": "request_created",
                        "data": {
                            "id": id,
                            "collectionId": collection_id,
                            "name": name,
                            "method": method,
                            "url": url,
                        },
                    }),
                );
                format!("Created request '{}' ({} {}) in collection.", name, method, url)
            }
            Err(e) => format!("Failed to create request: {}", e),
        }
    })
}

fn execute_collection<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let app = ctx.app;
        let session_id = ctx.session_id;
        let input = ctx.input;
        let collection_id_input = input["collection_id"].as_str().unwrap_or("");
        let environment_id = input["environment_id"].as_str().unwrap_or("");
        if collection_id_input.is_empty() {
            return "Error: collection_id is required".to_string();
        }

        let mut collection_id = collection_id_input.to_string();
        let mut requests = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT id, name, method, url FROM requests WHERE collection_id = ? ORDER BY sort_order ASC",
        )
        .bind(&collection_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        if requests.is_empty() {
            if let Ok(Some((resolved_id,))) = sqlx::query_as::<_, (String,)>(
                "SELECT id FROM collections WHERE LOWER(name) = LOWER(?)",
            )
            .bind(collection_id_input)
            .fetch_optional(pool)
            .await
            {
                log::info!(
                    "[AI Tool] execute_collection: resolved name '{}' to id '{}'",
                    collection_id_input,
                    resolved_id
                );
                collection_id = resolved_id;
                requests = sqlx::query_as::<_, (String, String, String, String)>(
                    "SELECT id, name, method, url FROM requests WHERE collection_id = ? ORDER BY sort_order ASC",
                )
                .bind(&collection_id)
                .fetch_all(pool)
                .await
                .unwrap_or_default();
            }
        }

        let mut results = Vec::new();
        for (req_id, name, method, url) in &requests {
            let result = match crate::modes::rest::http_executor::execute_request_internal(
                pool,
                req_id,
                environment_id,
            )
            .await
            {
                Ok(res) => serde_json::json!({
                    "name": name, "method": method, "url": url,
                    "status": res.status, "statusText": res.status_text,
                    "durationMs": res.duration_ms,
                }),
                Err(e) => serde_json::json!({
                    "name": name, "method": method, "url": url, "error": e,
                }),
            };
            results.push(result);
        }
        let _ = app.emit(
            &format!("ai:action:{}", session_id),
            serde_json::json!({
                "action": "collection_executed",
                "data": {"results": results},
            }),
        );
        if results.is_empty() {
            format!("No requests found in collection '{}'.", collection_id_input)
        } else {
            let total = results.len();
            let failed = results
                .iter()
                .filter(|r| {
                    r.get("error").is_some()
                        || r.get("status")
                            .and_then(|s| s.as_u64())
                            .map(|s| s >= 400)
                            .unwrap_or(false)
                })
                .count();
            let passed = total - failed;
            const MAX_BYTES: usize = 4096;
            let mut compact = serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string());
            if compact.len() > MAX_BYTES {
                compact.truncate(MAX_BYTES);
                compact.push_str("\n… (summary truncated)");
            }
            format!(
                "Collection executed: {} request(s), {} passed, {} failed. Full report card is in the result panel — do NOT re-print individual results. Per-request summary for your reasoning:\n```json\n{}\n```",
                total, passed, failed, compact
            )
        }
    })
}

fn generate_curl<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let input = ctx.input;
        let request_id_raw = input["request_id"].as_str().unwrap_or("");
        if request_id_raw.is_empty() {
            return "Error: request_id is required".to_string();
        }
        let request_id = resolve_request_id(pool, request_id_raw).await;

        let request = sqlx::query_as::<_, crate::db::models::Request>(
            "SELECT * FROM requests WHERE id = ?",
        )
        .bind(&request_id)
        .fetch_optional(pool)
        .await;

        match request {
            Ok(Some(req)) => {
                let headers = sqlx::query_as::<_, crate::db::models::RequestHeader>(
                    "SELECT * FROM request_headers WHERE request_id = ? AND enabled = 1 ORDER BY sort_order ASC",
                )
                .bind(&request_id)
                .fetch_all(pool)
                .await
                .unwrap_or_default();

                let mut curl = format!("curl -X {} '{}'", req.method, req.url);
                for h in &headers {
                    curl.push_str(&format!(" \\\n  -H '{}: {}'", h.key, h.value));
                }
                if !req.body.is_empty()
                    && matches!(req.method.as_str(), "POST" | "PUT" | "PATCH")
                {
                    curl.push_str(&format!(" \\\n  -d '{}'", req.body.replace('\'', "'\\''")));
                }
                format!("```\n{}\n```", curl)
            }
            Ok(None) => "Request not found.".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    })
}

fn switch_environment<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let app = ctx.app;
        let session_id = ctx.session_id;
        let input = ctx.input;
        let environment_id = input["environment_id"].as_str().unwrap_or("");
        if environment_id.is_empty() {
            return "Error: environment_id is required".to_string();
        }

        let env = sqlx::query_as::<_, (String,)>(
            "SELECT name FROM environments WHERE id = ?",
        )
        .bind(environment_id)
        .fetch_optional(pool)
        .await;

        match env {
            Ok(Some((name,))) => {
                let _ = app.emit(
                    &format!("ai:action:{}", session_id),
                    serde_json::json!({"action": "switch_environment", "data": {"environmentId": environment_id, "name": name}}),
                );
                format!("Switched to '{}' environment.", name)
            }
            Ok(None) => "Environment not found.".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    })
}

fn rename_request<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let app = ctx.app;
        let session_id = ctx.session_id;
        let input = ctx.input;
        let request_id_raw = input["request_id"].as_str().unwrap_or("");
        let new_name = input["new_name"].as_str().unwrap_or("");
        if request_id_raw.is_empty() || new_name.is_empty() {
            return "Error: request_id and new_name are required".to_string();
        }
        let request_id = resolve_request_id(pool, request_id_raw).await;

        match sqlx::query("UPDATE requests SET name = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(new_name)
            .bind(&request_id)
            .execute(pool)
            .await
        {
            Ok(r) => {
                if r.rows_affected() > 0 {
                    let _ = app.emit(
                        &format!("ai:action:{}", session_id),
                        serde_json::json!({"action": "request_modified", "data": {"requestId": request_id}}),
                    );
                    format!("Renamed request to '{}'.", new_name)
                } else {
                    "Request not found.".to_string()
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    })
}

fn delete_request<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let app = ctx.app;
        let session_id = ctx.session_id;
        let input = ctx.input;
        let request_id_raw = input["request_id"].as_str().unwrap_or("");
        if request_id_raw.is_empty() {
            return "Error: request_id is required".to_string();
        }
        let request_id = resolve_request_id(pool, request_id_raw).await;

        match sqlx::query("DELETE FROM requests WHERE id = ?")
            .bind(&request_id)
            .execute(pool)
            .await
        {
            Ok(r) => {
                if r.rows_affected() > 0 {
                    let _ = app.emit(
                        &format!("ai:action:{}", session_id),
                        serde_json::json!({"action": "request_modified", "data": {"requestId": request_id}}),
                    );
                    "Request deleted.".to_string()
                } else {
                    "Request not found.".to_string()
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    })
}

fn duplicate_request<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let app = ctx.app;
        let session_id = ctx.session_id;
        let input = ctx.input;
        let request_id_raw = input["request_id"].as_str().unwrap_or("");
        if request_id_raw.is_empty() {
            return "Error: request_id is required".to_string();
        }
        let request_id = resolve_request_id(pool, request_id_raw).await;

        let request = sqlx::query_as::<_, crate::db::models::Request>(
            "SELECT * FROM requests WHERE id = ?",
        )
        .bind(&request_id)
        .fetch_optional(pool)
        .await;

        match request {
            Ok(Some(req)) => {
                let new_id = uuid::Uuid::new_v4().to_string();
                let new_name = format!("{} (copy)", req.name);

                let max_order: Result<(i32,), _> = sqlx::query_as(
                    "SELECT COALESCE(MAX(sort_order), -1) FROM requests WHERE collection_id = ?",
                )
                .bind(&req.collection_id)
                .fetch_one(pool)
                .await;
                let order = max_order.map(|r| r.0 + 1).unwrap_or(0);

                match sqlx::query(
                    "INSERT INTO requests (id, collection_id, name, method, url, body, body_type, auth_type, auth_data, sort_order) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&new_id)
                .bind(&req.collection_id)
                .bind(&new_name)
                .bind(&req.method)
                .bind(&req.url)
                .bind(&req.body)
                .bind(&req.body_type)
                .bind(&req.auth_type)
                .bind(&req.auth_data)
                .bind(order)
                .execute(pool)
                .await
                {
                    Ok(_) => {
                        let headers = sqlx::query_as::<_, crate::db::models::RequestHeader>(
                            "SELECT * FROM request_headers WHERE request_id = ?",
                        )
                        .bind(&request_id)
                        .fetch_all(pool)
                        .await
                        .unwrap_or_default();

                        for h in &headers {
                            let hid = uuid::Uuid::new_v4().to_string();
                            let _ = sqlx::query(
                                "INSERT INTO request_headers (id, request_id, key, value, enabled, sort_order) VALUES (?, ?, ?, ?, ?, ?)",
                            )
                            .bind(&hid)
                            .bind(&new_id)
                            .bind(&h.key)
                            .bind(&h.value)
                            .bind(h.enabled)
                            .bind(h.sort_order)
                            .execute(pool)
                            .await;
                        }

                        let params = sqlx::query_as::<_, crate::db::models::RequestParam>(
                            "SELECT * FROM request_params WHERE request_id = ?",
                        )
                        .bind(&request_id)
                        .fetch_all(pool)
                        .await
                        .unwrap_or_default();

                        for p in &params {
                            let pid = uuid::Uuid::new_v4().to_string();
                            let _ = sqlx::query(
                                "INSERT INTO request_params (id, request_id, key, value, enabled, sort_order) VALUES (?, ?, ?, ?, ?, ?)",
                            )
                            .bind(&pid)
                            .bind(&new_id)
                            .bind(&p.key)
                            .bind(&p.value)
                            .bind(p.enabled)
                            .bind(p.sort_order)
                            .execute(pool)
                            .await;
                        }

                        let _ = app.emit(
                            &format!("ai:action:{}", session_id),
                            serde_json::json!({"action": "request_created", "data": {"id": new_id, "collectionId": req.collection_id, "name": new_name, "method": req.method, "url": req.url}}),
                        );
                        format!("Duplicated as '{}'.", new_name)
                    }
                    Err(e) => format!("Error: {}", e),
                }
            }
            Ok(None) => "Request not found.".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    })
}

fn create_collection<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let app = ctx.app;
        let session_id = ctx.session_id;
        let input = ctx.input;
        let name = input["name"].as_str().unwrap_or("New Collection");
        let description = input["description"].as_str().unwrap_or("");

        let id = uuid::Uuid::new_v4().to_string();
        let max_order: Result<(i32,), _> = sqlx::query_as(
            "SELECT COALESCE(MAX(sort_order), -1) FROM collections",
        )
        .fetch_one(pool)
        .await;
        let order = max_order.map(|r| r.0 + 1).unwrap_or(0);

        match sqlx::query(
            "INSERT INTO collections (id, name, description, sort_order) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(description)
        .bind(order)
        .execute(pool)
        .await
        {
            Ok(_) => {
                let _ = app.emit(
                    &format!("ai:action:{}", session_id),
                    serde_json::json!({"action": "collection_created", "data": {"id": id, "name": name}}),
                );
                format!("Created collection '{}'.", name)
            }
            Err(e) => format!("Error: {}", e),
        }
    })
}

fn set_env_variable<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let input = ctx.input;
        let env_id = input["environment_id"].as_str().unwrap_or("");
        let key = input["key"].as_str().unwrap_or("");
        let value = input["value"].as_str().unwrap_or("");
        let is_secret = input["is_secret"].as_bool().unwrap_or(true);

        if env_id.is_empty() || key.is_empty() {
            return "Error: environment_id and key are required".to_string();
        }

        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM env_variables WHERE environment_id = ? AND key = ?",
        )
        .bind(env_id)
        .bind(key)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        if let Some((var_id,)) = existing {
            match sqlx::query("UPDATE env_variables SET value = ?, is_secret = ? WHERE id = ?")
                .bind(value)
                .bind(is_secret as i32)
                .bind(&var_id)
                .execute(pool)
                .await
            {
                Ok(_) => format!("Updated variable '{}' in environment.", key),
                Err(e) => format!("Error: {}", e),
            }
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            match sqlx::query(
                "INSERT INTO env_variables (id, environment_id, key, value, is_secret, sort_order) VALUES (?, ?, ?, ?, ?, 0)",
            )
            .bind(&id)
            .bind(env_id)
            .bind(key)
            .bind(value)
            .bind(is_secret as i32)
            .execute(pool)
            .await
            {
                Ok(_) => format!("Created variable '{}' in environment.", key),
                Err(e) => format!("Error: {}", e),
            }
        }
    })
}

fn search_history<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let pool = ctx.pool;
        let input = ctx.input;
        let query = input["query"].as_str().unwrap_or("");
        let status_filter = input["status"].as_str().unwrap_or("");
        let limit = input["limit"].as_u64().unwrap_or(20).min(50) as i64;

        let mut sql = String::from(
            "SELECT method, url, resolved_url, response_status, duration_ms, created_at FROM history WHERE 1=1",
        );
        let mut binds: Vec<String> = Vec::new();

        if !query.is_empty() {
            sql.push_str(" AND (url LIKE ? OR resolved_url LIKE ?)");
            binds.push(format!("%{}%", query));
            binds.push(format!("%{}%", query));
        }
        if !status_filter.is_empty() {
            if let Ok(code) = status_filter.parse::<i32>() {
                sql.push_str(" AND response_status = ?");
                binds.push(code.to_string());
            } else if status_filter.ends_with("xx") {
                if let Ok(base) = status_filter[..1].parse::<i32>() {
                    sql.push_str(" AND response_status >= ? AND response_status < ?");
                    binds.push((base * 100).to_string());
                    binds.push(((base + 1) * 100).to_string());
                }
            }
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        binds.push(limit.to_string());

        let mut q = sqlx::query(&sql);
        for b in &binds {
            q = q.bind(b);
        }

        match q.fetch_all(pool).await {
            Ok(rows) => {
                let results: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|r| {
                        use sqlx::Row;
                        serde_json::json!({
                            "method": r.get::<String, _>("method"),
                            "url": r.get::<String, _>("url"),
                            "resolvedUrl": r.get::<Option<String>, _>("resolved_url"),
                            "status": r.get::<Option<i32>, _>("response_status"),
                            "durationMs": r.get::<Option<i64>, _>("duration_ms"),
                            "createdAt": r.get::<String, _>("created_at"),
                        })
                    })
                    .collect();
                serde_json::to_string(&results).unwrap_or("[]".to_string())
            }
            Err(e) => format!("Error: {}", e),
        }
    })
}

/// Register every REST-mode AI tool with the shared dispatch registry.
/// Called once from `lib.rs` setup.
pub fn register_tools() {
    use crate::shared::ai::dispatch::ToolExecutor;
    let entries: &[(&'static str, &'static str, ToolExecutor)] = &[
        (
            "execute_current_request",
            "Execute the request currently open in the active tab",
            execute_current_request,
        ),
        (
            "list_collections",
            "List all REST collections with their requests",
            list_collections,
        ),
        (
            "get_request_details",
            "Fetch the full details of a saved REST request",
            get_request_details,
        ),
        (
            "list_environments",
            "List REST environments and their variables (secrets masked)",
            list_environments,
        ),
        (
            "get_history",
            "List recent REST request history entries",
            get_history,
        ),
        (
            "apply_request",
            "Send a request suggestion to the user for approval",
            apply_request,
        ),
        (
            "execute_request",
            "Execute a saved REST request by id (or name) optionally with an environment",
            execute_request,
        ),
        (
            "create_request",
            "Create a new REST request inside a collection",
            create_request,
        ),
        (
            "execute_collection",
            "Execute every request in a collection sequentially",
            execute_collection,
        ),
        (
            "generate_curl",
            "Generate a cURL command string for a saved request",
            generate_curl,
        ),
        (
            "switch_environment",
            "Switch the active REST environment",
            switch_environment,
        ),
        (
            "rename_request",
            "Rename a saved REST request",
            rename_request,
        ),
        (
            "delete_request",
            "Delete a saved REST request",
            delete_request,
        ),
        (
            "duplicate_request",
            "Duplicate a saved REST request including headers and params",
            duplicate_request,
        ),
        (
            "create_collection",
            "Create a new REST collection",
            create_collection,
        ),
        (
            "set_env_variable",
            "Create or update an environment variable",
            set_env_variable,
        ),
        (
            "search_history",
            "Search REST request history with optional URL substring and status filters",
            search_history,
        ),
    ];

    for (name, description, executor) in entries {
        register(ToolDescriptor {
            name,
            mode: "rest",
            description,
            schema: serde_json::json!({}),
            executor: *executor,
        });
    }
}

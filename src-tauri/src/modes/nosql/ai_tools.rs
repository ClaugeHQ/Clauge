use mongodb::bson::{self, Document};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio_stream::StreamExt;

use crate::shared::ai::types::ChatContext;
use super::client::NoSqlConnections;

/// Commands rejected by `redis_execute` because they wipe data, block the
/// server, or change global config. The user can still run these from the
/// raw Redis tab — the AI tool is the only path gated here.
const REDIS_BLOCKED: &[&str] = &[
    "FLUSHDB", "FLUSHALL", "SHUTDOWN", "DEBUG", "MIGRATE",
    "REPLICAOF", "SLAVEOF", "CLUSTER", "FAILOVER", "RESET",
    "SAVE", "BGSAVE", "BGREWRITEAOF", "SWAPDB", "MOVE",
    "SCRIPT", "EVAL", "EVALSHA", "FUNCTION",
    "RENAME", "RENAMENX",
    "ACL",
];

fn is_blocked_redis_command(cmd: &str) -> bool {
    let upper = cmd.to_ascii_uppercase();
    if REDIS_BLOCKED.iter().any(|c| *c == upper) {
        return true;
    }
    // CONFIG SET / CONFIG RESETSTAT are mutations; CONFIG GET is read-only.
    // Whole CONFIG family is blocked here unless explicitly read-only.
    if upper == "CONFIG" { return true; }
    if upper == "KEYS" { return true; } // server-blocking, force SCAN path
    false
}

/// Truncate a JSON value to a max serialized byte size. Returns the original
/// value if already small, or a stub `{ "_truncated": true, "_bytes": N, "_preview": "..." }`
/// for documents that would blow the model context.
fn cap_doc_size(val: serde_json::Value, max_bytes: usize) -> serde_json::Value {
    let s = serde_json::to_string(&val).unwrap_or_default();
    if s.len() <= max_bytes { return val; }
    let preview: String = s.chars().take(400).collect();
    serde_json::json!({
        "_truncated": true,
        "_bytes": s.len(),
        "_preview": preview,
    })
}

/// Parse a Redis command string respecting quoted arguments.
/// e.g. `SET key "hello world"` → ["SET", "key", "hello world"]
fn parse_redis_args(input: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some(&(i, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            chars.next(); // skip opening quote
            let start = i + quote.len_utf8();
            let mut end = start;
            while let Some(&(j, ch)) = chars.peek() {
                if ch == quote {
                    end = j;
                    chars.next(); // skip closing quote
                    break;
                }
                end = j + ch.len_utf8();
                chars.next();
            }
            args.push(&input[start..end]);
        } else {
            let start = i;
            let mut end = start;
            while let Some(&(j, ch)) = chars.peek() {
                if ch.is_whitespace() { break; }
                end = j + ch.len_utf8();
                chars.next();
            }
            args.push(&input[start..end]);
        }
    }
    args
}

/// Validate that a collection exists; return error message with suggestions if not
async fn validate_collection(client: &mongodb::Client, database: &str, collection: &str) -> Option<String> {
    let existing = match client.database(database)
        .list_collection_names()
        .await {
        Ok(names) => names,
        Err(e) => return Some(format!("Failed to list collections in \"{}\": {}", database, e)),
    };

    if existing.iter().any(|c| c == collection) {
        return None; // Collection exists
    }

    let coll_lower = collection.to_lowercase();
    let suggestions: Vec<&String> = existing.iter()
        .filter(|c| c.to_lowercase().contains(&coll_lower) || coll_lower.contains(&c.to_lowercase()))
        .take(5)
        .collect();

    if suggestions.is_empty() {
        let all_names = existing.iter().take(20).map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
        Some(format!(
            "Collection \"{}\" does not exist in database \"{}\". Available collections: {}",
            collection, database, all_names
        ))
    } else {
        let names = suggestions.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
        Some(format!(
            "Collection \"{}\" does not exist in database \"{}\". Did you mean: {}?",
            collection, database, names
        ))
    }
}

pub async fn execute_nosql_tool(
    tool_name: &str,
    input: &serde_json::Value,
    _context: &ChatContext,
    pool: &SqlitePool,
    app: &AppHandle,
    session_id: &str,
    nosql_conns: &NoSqlConnections,
) -> String {
    match tool_name {
        "list_nosql_connections" => {
            let conns = crate::shared::repos::nosql_connections::list_all(pool).await;

            match conns {
                Ok(rows) => {
                    let result: Vec<serde_json::Value> = rows.iter().map(|c| {
                        serde_json::json!({
                            "id": c.id,
                            "name": c.name,
                            "driver": c.driver,
                            "host": c.host,
                            "port": c.port,
                            "database": c.database_name,
                            "username": c.username,
                            "uses_ssh_tunnel": c.ssh_profile_id.is_some(),
                        })
                    }).collect();
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| "[]".to_string())
                }
                Err(e) => format!("Error: {}", e),
            }
        }
        "list_nosql_databases" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            if connection_id.is_empty() {
                return "Error: connection_id is required".to_string();
            }
            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Mongo(client)) => {
                    let client = client.clone();
                    drop(guard);
                    match client.list_database_names().await {
                        Ok(dbs) => {
                            let filtered: Vec<&str> = dbs.iter()
                                .filter(|d| *d != "admin" && *d != "config" && *d != "local")
                                .map(|d| d.as_str())
                                .collect();
                            serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "[]".to_string())
                        }
                        Err(e) => format!("Error listing databases: {}", e),
                    }
                }
                Some(super::client::NoSqlPool::Redis(_)) => {
                    "This is a Redis connection. Redis does not have named databases — use redis_list_keys or redis_execute instead.".to_string()
                }
                None => "No active connection with that ID. Use list_nosql_connections to see saved connections, then ensure the connection is active.".to_string(),
            }
        }
        "list_nosql_collections" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let database = input["database"].as_str().unwrap_or("");
            if connection_id.is_empty() || database.is_empty() {
                return "Error: connection_id and database are required".to_string();
            }
            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Mongo(client)) => {
                    let client = client.clone();
                    drop(guard);
                    match client.database(database).list_collection_names().await {
                        Ok(colls) => {
                            serde_json::to_string_pretty(&colls).unwrap_or_else(|_| "[]".to_string())
                        }
                        Err(e) => format!("Error listing collections: {}", e),
                    }
                }
                Some(super::client::NoSqlPool::Redis(_)) => {
                    "This is a Redis connection, not MongoDB.".to_string()
                }
                None => "No active connection with that ID.".to_string(),
            }
        }
        "find_documents" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let database = input["database"].as_str().unwrap_or("");
            let collection = input["collection"].as_str().unwrap_or("");
            let filter_str = input["filter"].as_str().unwrap_or("{}");
            let default_limit = crate::shared::repos::settings::get_i64_or(pool, "nosql_default_find_limit", 50).await;
            let max_limit = crate::shared::repos::settings::get_i64_or(pool, "nosql_max_find_limit", 100).await;
            let limit = input["limit"].as_i64().unwrap_or(default_limit).min(max_limit) as i64;

            if connection_id.is_empty() || database.is_empty() || collection.is_empty() {
                return "Error: connection_id, database, and collection are required".to_string();
            }

            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Mongo(client)) => {
                    let client = client.clone();
                    drop(guard);

                    // Validate collection exists before querying
                    if let Some(err) = validate_collection(&client, database, collection).await {
                        return err;
                    }

                    let filter_doc = match parse_filter(filter_str) {
                        Ok(d) => d,
                        Err(e) => return e,
                    };

                    let coll = client.database(database).collection::<Document>(collection);
                    let options = mongodb::options::FindOptions::builder().limit(Some(limit)).build();
                    match coll.find(filter_doc).with_options(options).await {
                        Ok(mut cursor) => {
                            let mut docs: Vec<serde_json::Value> = Vec::new();
                            while let Some(result) = cursor.next().await {
                                match result {
                                    Ok(doc) => {
                                        if let Ok(val) = bson_doc_to_json(&doc) {
                                            docs.push(cap_doc_size(val, 4096));
                                        }
                                        if docs.len() >= limit as usize {
                                            break;
                                        }
                                    }
                                    Err(e) => return format!("Error reading cursor: {}", e),
                                }
                            }
                            let count = docs.len();

                            // Always route to main UI — chat shows status only
                            let _ = app.emit(
                                &format!("ai:action:{}", session_id),
                                serde_json::json!({
                                    "action": "ai_execute_nosql",
                                    "data": {
                                        "filter": filter_str,
                                        "connectionId": connection_id,
                                        "database": database,
                                        "collection": collection,
                                        "count": count,
                                    },
                                }),
                            );

                            if count == 0 {
                                "No documents matched the filter.".to_string()
                            } else {
                                const MAX_DOCS: usize = 10;
                                const MAX_BYTES: usize = 4096;
                                let sample = crate::shared::ai::sample::format_doc_sample(&docs, MAX_DOCS, MAX_BYTES);
                                let shown = count.min(MAX_DOCS);
                                let header = if count > shown {
                                    format!(
                                        "Found {} document(s). Showing first {} for your reasoning — full result is in the document viewer, do NOT re-print it to the user.",
                                        count, shown
                                    )
                                } else {
                                    format!(
                                        "Found {} document(s). Full result is in the document viewer, do NOT re-print it to the user — use the sample below only for your reasoning.",
                                        count
                                    )
                                };
                                format!("{}\n```json\n{}\n```", header, sample)
                            }
                        }
                        Err(e) => format!("Error executing find: {}", e),
                    }
                }
                Some(super::client::NoSqlPool::Redis(_)) => {
                    "This is a Redis connection, not MongoDB.".to_string()
                }
                None => "No active connection with that ID.".to_string(),
            }
        }
        "count_documents" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let database = input["database"].as_str().unwrap_or("");
            let collection = input["collection"].as_str().unwrap_or("");
            let filter_str = input["filter"].as_str().unwrap_or("{}");

            if connection_id.is_empty() || database.is_empty() || collection.is_empty() {
                return "Error: connection_id, database, and collection are required".to_string();
            }

            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Mongo(client)) => {
                    let client = client.clone();
                    drop(guard);

                    if let Some(err) = validate_collection(&client, database, collection).await {
                        return err;
                    }

                    let filter_doc = match parse_filter(filter_str) {
                        Ok(d) => d,
                        Err(e) => return e,
                    };

                    let coll = client.database(database).collection::<Document>(collection);
                    match coll.count_documents(filter_doc).await {
                        Ok(count) => format!("{}", count),
                        Err(e) => format!("Error counting documents: {}", e),
                    }
                }
                Some(super::client::NoSqlPool::Redis(_)) => {
                    "This is a Redis connection, not MongoDB.".to_string()
                }
                None => "No active connection with that ID.".to_string(),
            }
        }
        "aggregate" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let database = input["database"].as_str().unwrap_or("");
            let collection = input["collection"].as_str().unwrap_or("");
            let pipeline_str = input["pipeline"].as_str().unwrap_or("[]");
            let default_limit = crate::shared::repos::settings::get_i64_or(pool, "nosql_default_find_limit", 50).await;
            let max_limit = crate::shared::repos::settings::get_i64_or(pool, "nosql_max_find_limit", 100).await;
            let limit = input["limit"].as_i64().unwrap_or(default_limit).min(max_limit) as usize;

            if connection_id.is_empty() || database.is_empty() || collection.is_empty() {
                return "Error: connection_id, database, and collection are required".to_string();
            }

            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Mongo(client)) => {
                    let client = client.clone();
                    drop(guard);

                    if let Some(err) = validate_collection(&client, database, collection).await {
                        return err;
                    }

                    let pipeline_val: Vec<serde_json::Value> = match serde_json::from_str(pipeline_str) {
                        Ok(v) => v,
                        Err(e) => return format!("Error parsing pipeline JSON: {}", e),
                    };

                    let pipeline: Vec<Document> = pipeline_val.iter().filter_map(|v| {
                        bson::to_bson(v).ok().and_then(|b| match b {
                            bson::Bson::Document(d) => Some(d),
                            _ => None,
                        })
                    }).collect();

                    let coll = client.database(database).collection::<Document>(collection);
                    match coll.aggregate(pipeline).await {
                        Ok(mut cursor) => {
                            let mut docs: Vec<serde_json::Value> = Vec::new();
                            while let Some(result) = cursor.next().await {
                                match result {
                                    Ok(doc) => {
                                        if let Ok(val) = bson_doc_to_json(&doc) {
                                            docs.push(cap_doc_size(val, 4096));
                                        }
                                        if docs.len() >= limit {
                                            break;
                                        }
                                    }
                                    Err(e) => return format!("Error reading aggregation cursor: {}", e),
                                }
                            }
                            let count = docs.len();
                            let _ = app.emit(
                                &format!("ai:action:{}", session_id),
                                serde_json::json!({
                                    "action": "documents_result",
                                    "data": { "documents": docs, "count": count, "collection": collection, "label": "Aggregation Results" },
                                }),
                            );
                            if count == 0 {
                                "Aggregation returned 0 results.".to_string()
                            } else {
                                const MAX_DOCS: usize = 10;
                                const MAX_BYTES: usize = 4096;
                                let sample = crate::shared::ai::sample::format_doc_sample(&docs, MAX_DOCS, MAX_BYTES);
                                let shown = count.min(MAX_DOCS);
                                let header = if count > shown {
                                    format!(
                                        "Aggregation returned {} result(s). Showing first {} for your reasoning — full result is in the document viewer, do NOT re-print it to the user.",
                                        count, shown
                                    )
                                } else {
                                    format!(
                                        "Aggregation returned {} result(s). Full result is in the document viewer, do NOT re-print it to the user — use the sample below only for your reasoning.",
                                        count
                                    )
                                };
                                format!("{}\n```json\n{}\n```", header, sample)
                            }
                        }
                        Err(e) => format!("Error executing aggregation: {}", e),
                    }
                }
                Some(super::client::NoSqlPool::Redis(_)) => {
                    "This is a Redis connection, not MongoDB.".to_string()
                }
                None => "No active connection with that ID.".to_string(),
            }
        }
        "redis_list_keys" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let pattern = input["pattern"].as_str().unwrap_or("*");
            let limit = input["limit"].as_u64().unwrap_or(100).min(1000) as usize;

            if connection_id.is_empty() {
                return "Error: connection_id is required".to_string();
            }

            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Redis(cm)) => {
                    let mut conn = cm.clone();
                    drop(guard);

                    let mut keys: Vec<String> = Vec::new();
                    let mut cursor: u64 = 0;
                    let mut iterations: u32 = 0;
                    const MAX_ITERATIONS: u32 = 200;

                    loop {
                        let res: Result<(u64, Vec<String>), _> = redis::cmd("SCAN")
                            .arg(cursor)
                            .arg("MATCH")
                            .arg(pattern)
                            .arg("COUNT")
                            .arg(500)
                            .query_async(&mut conn)
                            .await;

                        match res {
                            Ok((next_cursor, batch)) => {
                                keys.extend(batch);
                                cursor = next_cursor;
                                iterations += 1;
                                if cursor == 0 { break; }
                                if keys.len() >= limit { break; }
                                if iterations >= MAX_ITERATIONS { break; }
                            }
                            Err(e) => return format!("Error scanning keys: {}", e),
                        }
                    }

                    let total_seen = keys.len();
                    let capped: Vec<&str> = keys.iter().take(limit).map(|s| s.as_str()).collect();
                    let result = serde_json::json!({
                        "count": total_seen,
                        "keys": capped,
                        "truncated": total_seen > limit || cursor != 0,
                        "scan_complete": cursor == 0,
                    });
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| "[]".to_string())
                }
                Some(super::client::NoSqlPool::Mongo(_)) => {
                    "This is a MongoDB connection, not Redis.".to_string()
                }
                None => "No active connection with that ID.".to_string(),
            }
        }
        "redis_execute" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let command_str = input["command"].as_str().unwrap_or("");

            if connection_id.is_empty() || command_str.is_empty() {
                return "Error: connection_id and command are required".to_string();
            }

            let parts: Vec<&str> = parse_redis_args(command_str);
            if parts.is_empty() {
                return "Error: empty command".to_string();
            }

            if is_blocked_redis_command(parts[0]) {
                return format!(
                    "Error: '{}' is blocked from AI invocation because it can wipe data, change global config, or block the Redis server. If the user explicitly wants to run it, they can use the Redis console directly.",
                    parts[0].to_ascii_uppercase()
                );
            }

            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Redis(cm)) => {
                    let mut conn = cm.clone();
                    drop(guard);

                    let mut cmd = redis::cmd(parts[0]);
                    for arg in &parts[1..] {
                        cmd.arg(*arg);
                    }

                    match cmd.query_async::<redis::Value>(&mut conn).await {
                        Ok(value) => format_redis_value(&value),
                        Err(e) => format!("Error: {}", e),
                    }
                }
                Some(super::client::NoSqlPool::Mongo(_)) => {
                    "This is a MongoDB connection, not Redis.".to_string()
                }
                None => "No active connection with that ID.".to_string(),
            }
        }
        "apply_nosql_query" => {
            let query = input["query"].as_str().unwrap_or("");
            if query.is_empty() {
                return "Error: query is required".to_string();
            }
            let _ = app.emit(
                &format!("ai:action:{}", session_id),
                serde_json::json!({"action": "apply_nosql_query", "data": {"query": query}}),
            );
            "Query written to the user's editor.".to_string()
        }
        "sample_documents" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let database = input["database"].as_str().unwrap_or("");
            let collection = input["collection"].as_str().unwrap_or("");

            if connection_id.is_empty() || database.is_empty() || collection.is_empty() {
                return "Error: connection_id, database, and collection are required".to_string();
            }

            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Mongo(client)) => {
                    let client = client.clone();
                    drop(guard);
                    let coll = client.database(database).collection::<Document>(collection);
                    let opts = mongodb::options::FindOptions::builder().limit(5).build();
                    match coll.find(Document::new()).with_options(opts).await {
                        Ok(mut cursor) => {
                            let mut docs: Vec<serde_json::Value> = Vec::new();
                            while let Some(Ok(doc)) = cursor.next().await {
                                if docs.len() >= 5 { break; }
                                if let Ok(val) = bson_doc_to_json(&doc) {
                                    docs.push(cap_doc_size(val, 4096));
                                }
                            }
                            if docs.is_empty() {
                                "Collection is empty.".to_string()
                            } else {
                                let count = docs.len();
                                let _ = app.emit(
                                    &format!("ai:action:{}", session_id),
                                    serde_json::json!({
                                        "action": "documents_result",
                                        "data": { "documents": docs, "count": count, "collection": collection, "label": "Sample Documents" },
                                    }),
                                );
                                let sample = crate::shared::ai::sample::format_doc_sample(&docs, 5, 4096);
                                format!(
                                    "{} sample document(s) from '{}'. Also shown in the document viewer — use these for your reasoning, do NOT re-print to the user:\n```json\n{}\n```",
                                    count, collection, sample
                                )
                            }
                        }
                        Err(e) => format!("Error: {}", e),
                    }
                }
                _ => "Error: not a MongoDB connection or not active.".to_string(),
            }
        }
        "insert_documents" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let database = input["database"].as_str().unwrap_or("");
            let collection = input["collection"].as_str().unwrap_or("");
            let documents = &input["documents"];

            if connection_id.is_empty() || database.is_empty() || collection.is_empty() {
                return "Error: connection_id, database, collection, and documents are required".to_string();
            }

            let docs_array = if let Some(arr) = documents.as_array() {
                arr.clone()
            } else if documents.is_object() {
                vec![documents.clone()]
            } else {
                return "Error: documents must be a JSON array or object".to_string();
            };

            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Mongo(client)) => {
                    let client = client.clone();
                    drop(guard);
                    let coll = client.database(database).collection::<Document>(collection);
                    let mut inserted = 0;
                    let mut errors = Vec::new();
                    for doc_val in &docs_array {
                        match bson::to_bson(doc_val) {
                            Ok(bson::Bson::Document(doc)) => {
                                match coll.insert_one(doc).await {
                                    Ok(_) => inserted += 1,
                                    Err(e) => errors.push(format!("{}", e)),
                                }
                            }
                            _ => errors.push("Invalid document format".to_string()),
                        }
                    }
                    if errors.is_empty() {
                        format!("Inserted {} document(s).", inserted)
                    } else {
                        format!("Inserted {}. Errors: {}", inserted, errors.join("; "))
                    }
                }
                _ => "Error: not a MongoDB connection or not active.".to_string(),
            }
        }
        "get_collection_stats" => {
            let connection_id = input["connection_id"].as_str().unwrap_or("");
            let database = input["database"].as_str().unwrap_or("");
            let collection = input["collection"].as_str().unwrap_or("");

            if connection_id.is_empty() || database.is_empty() || collection.is_empty() {
                return "Error: connection_id, database, and collection are required".to_string();
            }

            let guard = nosql_conns.pools.lock().await;
            match guard.get(connection_id) {
                Some(super::client::NoSqlPool::Mongo(client)) => {
                    let client = client.clone();
                    drop(guard);
                    let db = client.database(database);
                    let cmd = bson::doc! { "collStats": collection };
                    match db.run_command(cmd).await {
                        Ok(result) => {
                            // Extract useful fields
                            let count = result.get_i64("count").unwrap_or(0);
                            let size = result.get_i64("size").unwrap_or(0);
                            let avg_obj_size = result.get_i64("avgObjSize").unwrap_or(0);
                            let storage_size = result.get_i64("storageSize").unwrap_or(0);
                            let indexes = result.get_i32("nindexes").unwrap_or(0);
                            format!(
                                "Collection: {}\nDocuments: {}\nSize: {} bytes\nAvg document size: {} bytes\nStorage: {} bytes\nIndexes: {}",
                                collection, count, size, avg_obj_size, storage_size, indexes
                            )
                        }
                        Err(e) => format!("Error: {}", e),
                    }
                }
                _ => "Error: not a MongoDB connection or not active.".to_string(),
            }
        }
        _ => format!("Unknown NoSQL tool: {}", tool_name),
    }
}

/// Parse a JSON filter string into a BSON Document.
fn parse_filter(filter_str: &str) -> Result<Document, String> {
    let filter_val: serde_json::Value = serde_json::from_str(filter_str)
        .map_err(|e| format!("Error parsing filter JSON: {}", e))?;
    let bson_val = bson::to_bson(&filter_val)
        .map_err(|e| format!("Error converting to BSON: {}", e))?;
    match bson_val {
        bson::Bson::Document(d) => Ok(d),
        _ => Ok(Document::new()),
    }
}

/// Convert a BSON Document to a serde_json::Value.
fn bson_doc_to_json(doc: &Document) -> Result<serde_json::Value, String> {
    let bson_val = bson::Bson::Document(doc.clone());
    let json_val = bson_val.clone().into_relaxed_extjson();
    Ok(json_val)
}

/// Format a Redis Value into a human-readable string.
fn format_redis_value(value: &redis::Value) -> String {
    match value {
        redis::Value::Nil => "(nil)".to_string(),
        redis::Value::Int(i) => format!("(integer) {}", i),
        redis::Value::BulkString(data) => {
            match String::from_utf8(data.clone()) {
                Ok(s) => format!("\"{}\"", s),
                Err(_) => format!("(binary data, {} bytes)", data.len()),
            }
        }
        redis::Value::Array(arr) => {
            if arr.is_empty() {
                return "(empty array)".to_string();
            }
            let items: Vec<String> = arr.iter().enumerate().map(|(i, v)| {
                format!("{}) {}", i + 1, format_redis_value(v))
            }).collect();
            items.join("\n")
        }
        redis::Value::SimpleString(s) => s.clone(),
        redis::Value::Okay => "OK".to_string(),
        redis::Value::Boolean(b) => format!("(boolean) {}", b),
        redis::Value::Double(d) => format!("(double) {}", d),
        redis::Value::Map(pairs) => {
            let items: Vec<String> = pairs.iter().enumerate().map(|(i, (k, v))| {
                format!("{}) {} -> {}", i + 1, format_redis_value(k), format_redis_value(v))
            }).collect();
            items.join("\n")
        }
        _ => format!("{:?}", value),
    }
}

// --- Dispatch registry integration -----------------------------------------
//
// Every NoSQL tool flows through `execute_nosql_tool` and its `match`. The
// registry executor is a thin adapter that re-dispatches by tool name.

use crate::shared::ai::dispatch::{register, ToolContext, ToolDescriptor, ToolFuture};

macro_rules! nosql_tool_executor {
    ($name:literal) => {{
        fn exec<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
            Box::pin(async move {
                execute_nosql_tool(
                    $name,
                    ctx.input,
                    ctx.context,
                    ctx.pool,
                    ctx.app,
                    ctx.session_id,
                    ctx.nosql_conns,
                )
                .await
            })
        }
        exec as crate::shared::ai::dispatch::ToolExecutor
    }};
}

/// Register every NoSQL-mode AI tool with the shared dispatch registry.
pub fn register_tools() {
    let entries: &[(&'static str, &'static str, crate::shared::ai::dispatch::ToolExecutor)] = &[
        ("list_nosql_connections", "List saved NoSQL connections", nosql_tool_executor!("list_nosql_connections")),
        ("list_nosql_databases", "List databases on a NoSQL connection", nosql_tool_executor!("list_nosql_databases")),
        ("list_nosql_collections", "List MongoDB collections in a database", nosql_tool_executor!("list_nosql_collections")),
        ("find_documents", "Find MongoDB documents by filter", nosql_tool_executor!("find_documents")),
        ("count_documents", "Count MongoDB documents matching a filter", nosql_tool_executor!("count_documents")),
        ("aggregate", "Run a MongoDB aggregation pipeline", nosql_tool_executor!("aggregate")),
        ("apply_nosql_query", "Send a NoSQL query suggestion to the user for approval", nosql_tool_executor!("apply_nosql_query")),
        ("redis_list_keys", "List Redis keys matching a pattern", nosql_tool_executor!("redis_list_keys")),
        ("redis_execute", "Execute an arbitrary Redis command", nosql_tool_executor!("redis_execute")),
        ("sample_documents", "Sample documents from a MongoDB collection", nosql_tool_executor!("sample_documents")),
        ("insert_documents", "Insert documents into a MongoDB collection", nosql_tool_executor!("insert_documents")),
        ("get_collection_stats", "Get stats for a MongoDB collection", nosql_tool_executor!("get_collection_stats")),
    ];

    for (name, description, executor) in entries {
        register(ToolDescriptor {
            name,
            mode: "nosql",
            description,
            schema: serde_json::json!({}),
            executor: *executor,
        });
    }
}

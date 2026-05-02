//! Explorer mode AI tool registration.
//!
//! Read-only tools (`fs_list`, `fs_stat`, `fs_read`, `fs_search`,
//! `fs_get_url`) execute directly against the active session. Write-side
//! tools (`fs_write`, `fs_delete`, `fs_mkdir`, `fs_rename`,
//! `fs_upload_local`, `fs_download`) follow the SSH `execute_shell`
//! frontend-confirmation pattern — Rust emits `ai:tool_pending:<session>`
//! and waits for the frontend to run the operation and resolve via
//! `ai_resolve_pending_tool`. The frontend wiring for the write side
//! lands in a follow-up build; for now those tools return a clear "not
//! yet available" message to the model.

use std::time::Duration;

use tauri::{Emitter, Manager};

use crate::modes::explorer::session::ExplorerSessions;
use crate::shared::ai::dispatch::{register, ToolContext, ToolDescriptor, ToolFuture};
use crate::shared::ai::types::PendingFrontendTools;

fn input_str<'a>(ctx: &'a ToolContext<'a>, key: &str) -> Option<String> {
    ctx.input
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

async fn require_session(
    ctx: &ToolContext<'_>,
) -> Result<(std::sync::Arc<dyn crate::modes::explorer::fs::RemoteFs>, String), String> {
    let tab_key = input_str(ctx, "tabKey").ok_or_else(|| {
        "Tool error: this tool requires a `tabKey` argument identifying the active Explorer tab"
            .to_string()
    })?;
    let sessions = ctx.app.state::<ExplorerSessions>();
    let fs = sessions.get(&tab_key).await.ok_or_else(|| {
        format!(
            "No active Explorer session for tab `{}` — open a connection first",
            tab_key
        )
    })?;
    Ok((fs, tab_key))
}

// ─── Read-only tools ──────────────────────────────────────────────────

fn fs_list<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let path = input_str(ctx, "path").unwrap_or_else(|| "/".to_string());
        let (fs, _tab) = match require_session(ctx).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match fs.list(&path).await {
            Ok(entries) => serde_json::to_string(&entries).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Tool error: {}", e),
        }
    })
}

fn fs_stat<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let path = match input_str(ctx, "path") {
            Some(p) => p,
            None => return "Tool error: `path` is required".to_string(),
        };
        let (fs, _tab) = match require_session(ctx).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match fs.stat(&path).await {
            Ok(stat) => serde_json::to_string(&stat).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Tool error: {}", e),
        }
    })
}

fn fs_read<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        use base64::Engine;
        use futures::StreamExt;
        let path = match input_str(ctx, "path") {
            Some(p) => p,
            None => return "Tool error: `path` is required".to_string(),
        };
        let max_bytes = ctx
            .input
            .get("maxBytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(64 * 1024); // 64 KB default for AI reads — keep token cost sane
        let (fs, _tab) = match require_session(ctx).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        let mut s = match fs.read(&path, None).await {
            Ok(s) => s,
            Err(e) => return format!("Tool error: {}", e),
        };
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = s.next().await {
            match chunk {
                Ok(c) => {
                    buf.extend_from_slice(&c);
                    if buf.len() as u64 > max_bytes {
                        return format!(
                            "Tool error: file exceeds {} byte cap for AI preview (request a smaller maxBytes or use the UI to download)",
                            max_bytes
                        );
                    }
                }
                Err(e) => return format!("Tool error: {}", e),
            }
        }
        // Best effort: try UTF-8 first; fall back to base64 for binary.
        match String::from_utf8(buf.clone()) {
            Ok(text) => text,
            Err(_) => format!(
                "<binary {} bytes; base64-encoded>\n{}",
                buf.len(),
                base64::engine::general_purpose::STANDARD.encode(&buf)
            ),
        }
    })
}

fn fs_search<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let prefix = input_str(ctx, "prefix").unwrap_or_else(|| "/".to_string());
        let glob = input_str(ctx, "glob").unwrap_or_else(|| "*".to_string());
        let (fs, _tab) = match require_session(ctx).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match fs.search(&prefix, &glob).await {
            Ok(entries) => serde_json::to_string(&entries).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Tool error: {}", e),
        }
    })
}

fn fs_get_url<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    Box::pin(async move {
        let path = match input_str(ctx, "path") {
            Some(p) => p,
            None => return "Tool error: `path` is required".to_string(),
        };
        let ttl = ctx
            .input
            .get("ttlSecs")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);
        let (fs, _tab) = match require_session(ctx).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match fs.presigned_url(&path, ttl).await {
            Ok(Some(url)) => url,
            Ok(None) => "(presigned URLs are only available for S3 / Azure Blob backends)".to_string(),
            Err(e) => format!("Tool error: {}", e),
        }
    })
}

// ─── Write-side tools — frontend-confirmed via PendingFrontendTools ───

fn pending_write_tool<'a>(ctx: &'a ToolContext<'a>, tool_name: &'static str) -> ToolFuture<'a> {
    Box::pin(async move {
        let app = ctx.app;
        let tool_use_id = ctx.tool_use_id;
        let session_id = ctx.session_id;
        let pending = app.state::<PendingFrontendTools>();
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        pending.map.lock().insert(tool_use_id.to_string(), tx);
        let _ = app.emit(
            &format!("ai:tool_pending:{}", session_id),
            serde_json::json!({
                "toolUseId": tool_use_id,
                "tool": tool_name,
                // The full input payload is forwarded so the frontend
                // confirmation modal can show the user exactly what was
                // requested and run the matching `explorer_fs_*` command.
                "input": ctx.input,
            }),
        );
        match tokio::time::timeout(Duration::from_secs(180), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => "Tool result channel closed unexpectedly.".to_string(),
            Err(_) => {
                app.state::<PendingFrontendTools>()
                    .map
                    .lock()
                    .remove(tool_use_id);
                "User did not respond to the approval prompt within 3 minutes.".to_string()
            }
        }
    })
}

fn fs_write<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    pending_write_tool(ctx, "fs_write")
}
fn fs_delete<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    pending_write_tool(ctx, "fs_delete")
}
fn fs_mkdir<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    pending_write_tool(ctx, "fs_mkdir")
}
fn fs_rename<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    pending_write_tool(ctx, "fs_rename")
}
fn fs_upload_local<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    pending_write_tool(ctx, "fs_upload_local")
}
fn fs_download<'a>(ctx: &'a ToolContext<'a>) -> ToolFuture<'a> {
    pending_write_tool(ctx, "fs_download")
}

pub fn register_tools() {
    let mode = "explorer";
    register(ToolDescriptor {
        name: "fs_list",
        mode,
        description:
            "List entries in a remote directory. Returns files and directories with name, size, modified time, and permissions when available.",
        schema: serde_json::json!({}),
        executor: fs_list,
    });
    register(ToolDescriptor {
        name: "fs_stat",
        mode,
        description: "Get metadata for a single remote path.",
        schema: serde_json::json!({}),
        executor: fs_stat,
    });
    register(ToolDescriptor {
        name: "fs_read",
        mode,
        description:
            "Read the contents of a remote file. Capped at 64 KB by default; set `maxBytes` to override.",
        schema: serde_json::json!({}),
        executor: fs_read,
    });
    register(ToolDescriptor {
        name: "fs_search",
        mode,
        description: "Recursively search under a prefix for entries whose name matches a POSIX-style glob.",
        schema: serde_json::json!({}),
        executor: fs_search,
    });
    register(ToolDescriptor {
        name: "fs_get_url",
        mode,
        description: "Generate a temporary signed URL for an object (S3 / Azure Blob only).",
        schema: serde_json::json!({}),
        executor: fs_get_url,
    });
    register(ToolDescriptor {
        name: "fs_write",
        mode,
        description: "Write content to a remote file (requires user confirmation).",
        schema: serde_json::json!({}),
        executor: fs_write,
    });
    register(ToolDescriptor {
        name: "fs_delete",
        mode,
        description: "Delete one or more remote paths (requires user confirmation).",
        schema: serde_json::json!({}),
        executor: fs_delete,
    });
    register(ToolDescriptor {
        name: "fs_mkdir",
        mode,
        description: "Create a remote directory (requires user confirmation).",
        schema: serde_json::json!({}),
        executor: fs_mkdir,
    });
    register(ToolDescriptor {
        name: "fs_rename",
        mode,
        description: "Rename / move a remote path (requires user confirmation).",
        schema: serde_json::json!({}),
        executor: fs_rename,
    });
    register(ToolDescriptor {
        name: "fs_upload_local",
        mode,
        description: "Upload a local file to a remote path (requires user confirmation).",
        schema: serde_json::json!({}),
        executor: fs_upload_local,
    });
    register(ToolDescriptor {
        name: "fs_download",
        mode,
        description: "Download a remote file to a local path (requires user confirmation).",
        schema: serde_json::json!({}),
        executor: fs_download,
    });
}

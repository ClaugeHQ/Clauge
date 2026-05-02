//! Upload / download progress + cancellation.
//!
//! Each in-flight transfer is identified by a frontend-generated UUID and
//! tracked in `Transfers`. The Tauri commands stream bytes between local
//! disk and the active `RemoteFs` session, emitting progress events on
//! the `EXPLORER_TRANSFER_EVENT` channel after each chunk (and at start /
//! end). A cancel flag — flipped by `explorer_cancel_transfer` — is read
//! between chunks; the streaming body short-circuits to `FsError::Cancelled`
//! when set, which propagates out of the backend write/read.

use bytes::Bytes;
use futures::stream::{self, BoxStream};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;

use crate::modes::explorer::models::FsError;
use crate::modes::explorer::session::ExplorerSessions;

/// Single Tauri event channel — frontend filters by transfer id in payload.
/// Avoids creating one listener per transfer.
const EVENT: &str = "explorer:transfer";

/// 256 KiB chunks — small enough to feel responsive, large enough that we
/// don't drown the SFTP packet pipeline in tiny PDUs.
const CHUNK_SIZE: usize = 256 * 1024;

#[derive(Default)]
pub struct Transfers {
    /// Map: transfer_id → cancel flag. Removed on completion / cancel.
    flags: RwLock<HashMap<String, Arc<AtomicBool>>>,
}

impl Transfers {
    async fn register(&self, id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.flags.write().await.insert(id.to_string(), flag.clone());
        flag
    }

    async fn unregister(&self, id: &str) {
        self.flags.write().await.remove(id);
    }

    async fn cancel(&self, id: &str) -> bool {
        if let Some(flag) = self.flags.read().await.get(id) {
            flag.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TransferEvent<'a> {
    id: &'a str,
    direction: &'a str, // "upload" | "download"
    state: &'a str,     // "running" | "completed" | "failed" | "cancelled"
    bytes_done: u64,
    bytes_total: Option<u64>,
    error: Option<String>,
    /// Display labels — frontend uses these in the progress panel without
    /// having to look them up.
    name: &'a str,
    remote_path: &'a str,
    local_path: &'a str,
}

fn emit(app: &AppHandle, e: TransferEvent<'_>) {
    let _ = app.emit(EVENT, e);
}

#[tauri::command]
pub async fn explorer_upload_file(
    app: AppHandle,
    sessions: State<'_, ExplorerSessions>,
    transfers: State<'_, Transfers>,
    transfer_id: String,
    tab_key: String,
    local_path: String,
    remote_path: String,
) -> Result<(), String> {
    let fs = sessions
        .get(&tab_key)
        .await
        .ok_or("no active session for this tab")?;

    let local = PathBuf::from(&local_path);
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let total = tokio::fs::metadata(&local)
        .await
        .ok()
        .map(|m| m.len());

    let cancel = transfers.register(&transfer_id).await;

    emit(
        &app,
        TransferEvent {
            id: &transfer_id,
            direction: "upload",
            state: "running",
            bytes_done: 0,
            bytes_total: total,
            error: None,
            name: &name,
            remote_path: &remote_path,
            local_path: &local_path,
        },
    );

    // Build the streaming body. Each yielded chunk also fires a progress
    // event so the UI can update without polling. Cancel flag is checked
    // between chunks — a flipped flag yields one final Err and the SFTP /
    // S3 / Azure / FTP write returns FsError::Cancelled to us.
    let body = build_upload_stream(
        local.clone(),
        cancel.clone(),
        app.clone(),
        transfer_id.clone(),
        name.clone(),
        remote_path.clone(),
        local_path.clone(),
        total,
    );

    let result = fs.write(&remote_path, body, total).await;

    transfers.unregister(&transfer_id).await;

    match result {
        Ok(()) => {
            emit(
                &app,
                TransferEvent {
                    id: &transfer_id,
                    direction: "upload",
                    state: "completed",
                    bytes_done: total.unwrap_or(0),
                    bytes_total: total,
                    error: None,
                    name: &name,
                    remote_path: &remote_path,
                    local_path: &local_path,
                },
            );
            Ok(())
        }
        Err(FsError::Cancelled) => {
            emit(
                &app,
                TransferEvent {
                    id: &transfer_id,
                    direction: "upload",
                    state: "cancelled",
                    bytes_done: 0,
                    bytes_total: total,
                    error: None,
                    name: &name,
                    remote_path: &remote_path,
                    local_path: &local_path,
                },
            );
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            emit(
                &app,
                TransferEvent {
                    id: &transfer_id,
                    direction: "upload",
                    state: "failed",
                    bytes_done: 0,
                    bytes_total: total,
                    error: Some(msg.clone()),
                    name: &name,
                    remote_path: &remote_path,
                    local_path: &local_path,
                },
            );
            Err(msg)
        }
    }
}

fn build_upload_stream(
    local: PathBuf,
    cancel: Arc<AtomicBool>,
    app: AppHandle,
    transfer_id: String,
    name: String,
    remote_path: String,
    local_path: String,
    total: Option<u64>,
) -> BoxStream<'static, Result<Bytes, FsError>> {
    Box::pin(async_stream::try_stream! {
        let mut file = File::open(&local)
            .await
            .map_err(|e| FsError::Other { detail: format!("open local: {}", e) })?;
        let mut buf = vec![0u8; CHUNK_SIZE];
        let mut done: u64 = 0;
        loop {
            if cancel.load(Ordering::SeqCst) {
                Err(FsError::Cancelled)?;
            }
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| FsError::Other { detail: format!("read local: {}", e) })?;
            if n == 0 {
                break;
            }
            done += n as u64;
            yield Bytes::copy_from_slice(&buf[..n]);
            // Emit after the yield so backpressure-driven pacing is reflected.
            emit(
                &app,
                TransferEvent {
                    id: &transfer_id,
                    direction: "upload",
                    state: "running",
                    bytes_done: done,
                    bytes_total: total,
                    error: None,
                    name: &name,
                    remote_path: &remote_path,
                    local_path: &local_path,
                },
            );
        }
    })
}

#[tauri::command]
pub async fn explorer_download_file(
    app: AppHandle,
    sessions: State<'_, ExplorerSessions>,
    transfers: State<'_, Transfers>,
    transfer_id: String,
    tab_key: String,
    remote_path: String,
    local_path: String,
) -> Result<(), String> {
    use futures::StreamExt;
    let fs = sessions
        .get(&tab_key)
        .await
        .ok_or("no active session for this tab")?;

    // Best-effort total: stat the remote so the progress bar has a denominator.
    let total = fs.stat(&remote_path).await.ok().and_then(|s| s.size);

    let name = remote_path
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();

    let cancel = transfers.register(&transfer_id).await;

    emit(
        &app,
        TransferEvent {
            id: &transfer_id,
            direction: "download",
            state: "running",
            bytes_done: 0,
            bytes_total: total,
            error: None,
            name: &name,
            remote_path: &remote_path,
            local_path: &local_path,
        },
    );

    let result: Result<(), FsError> = async {
        let mut stream = fs.read(&remote_path, None).await?;
        let mut out = File::create(&local_path)
            .await
            .map_err(|e| FsError::Other { detail: format!("create local: {}", e) })?;
        let mut done: u64 = 0;
        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                // Best-effort cleanup of the partial download.
                drop(out);
                let _ = tokio::fs::remove_file(&local_path).await;
                return Err(FsError::Cancelled);
            }
            let chunk = chunk?;
            out.write_all(&chunk)
                .await
                .map_err(|e| FsError::Other { detail: format!("write local: {}", e) })?;
            done += chunk.len() as u64;
            emit(
                &app,
                TransferEvent {
                    id: &transfer_id,
                    direction: "download",
                    state: "running",
                    bytes_done: done,
                    bytes_total: total,
                    error: None,
                    name: &name,
                    remote_path: &remote_path,
                    local_path: &local_path,
                },
            );
        }
        out.shutdown()
            .await
            .map_err(|e| FsError::Other { detail: format!("close local: {}", e) })?;
        Ok(())
    }
    .await;

    transfers.unregister(&transfer_id).await;

    match result {
        Ok(()) => {
            emit(
                &app,
                TransferEvent {
                    id: &transfer_id,
                    direction: "download",
                    state: "completed",
                    bytes_done: total.unwrap_or(0),
                    bytes_total: total,
                    error: None,
                    name: &name,
                    remote_path: &remote_path,
                    local_path: &local_path,
                },
            );
            Ok(())
        }
        Err(FsError::Cancelled) => {
            emit(
                &app,
                TransferEvent {
                    id: &transfer_id,
                    direction: "download",
                    state: "cancelled",
                    bytes_done: 0,
                    bytes_total: total,
                    error: None,
                    name: &name,
                    remote_path: &remote_path,
                    local_path: &local_path,
                },
            );
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            emit(
                &app,
                TransferEvent {
                    id: &transfer_id,
                    direction: "download",
                    state: "failed",
                    bytes_done: 0,
                    bytes_total: total,
                    error: Some(msg.clone()),
                    name: &name,
                    remote_path: &remote_path,
                    local_path: &local_path,
                },
            );
            Err(msg)
        }
    }
}

// Stub stream import — pull in `_` to keep linker happy if no upload yet.
fn _stream_anchor() {
    let _: BoxStream<'static, Result<Bytes, FsError>> = Box::pin(stream::empty());
}

#[tauri::command]
pub async fn explorer_cancel_transfer(
    transfers: State<'_, Transfers>,
    transfer_id: String,
) -> Result<bool, String> {
    Ok(transfers.cancel(&transfer_id).await)
}

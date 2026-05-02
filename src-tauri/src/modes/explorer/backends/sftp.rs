//! SFTP backend.
//!
//! Layers an SFTP subsystem on top of an authenticated russh session
//! (the helper from `modes::ssh::ssh_session`). Reuses the existing
//! SSH profile model verbatim — an `explorer_connections` row of kind
//! `'sftp'` with `ssh_profile_id` set is the canonical wiring.

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, BoxStream};
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{OpenFlags, StatusCode};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::modes::explorer::backends::{glob_pattern, mime_for, posix_join};
use crate::modes::explorer::fs::RemoteFs;
use crate::modes::explorer::models::{DirEntry, FsError, Stat};
use crate::modes::ssh::ssh_session::ClientHandler;

/// SFTP backend. Holds the SftpSession plus a strong ref to the underlying
/// russh `Handle` so the SSH session isn't dropped under us while open.
pub struct SftpBackend {
    session: Arc<Mutex<SftpSession>>,
    /// Held for lifetime extension only — never used directly.
    _ssh_handle: Arc<russh::client::Handle<ClientHandler>>,
}

impl SftpBackend {
    pub async fn open(
        ssh_handle: russh::client::Handle<ClientHandler>,
    ) -> Result<Self, FsError> {
        let channel = ssh_handle
            .channel_open_session()
            .await
            .map_err(|e| FsError::NetworkError {
                detail: format!("open channel: {}", e),
            })?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| FsError::NetworkError {
                detail: format!("request sftp subsystem: {}", e),
            })?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(map_sftp_err)?;
        Ok(Self {
            session: Arc::new(Mutex::new(sftp)),
            _ssh_handle: Arc::new(ssh_handle),
        })
    }
}

fn map_sftp_err(e: SftpError) -> FsError {
    match e {
        SftpError::Status(s) => match s.status_code {
            StatusCode::NoSuchFile => FsError::NotFound { path: s.error_message },
            StatusCode::PermissionDenied => FsError::PermissionDenied {
                path: s.error_message.clone(),
                detail: s.error_message,
            },
            _ => FsError::Other { detail: s.error_message },
        },
        other => FsError::Other { detail: other.to_string() },
    }
}

fn unix_to_rfc3339(secs: u32) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// Standard chmod-style "drwxr-xr-x" formatting from a Unix permissions u32.
fn format_mode(mode: u32) -> String {
    let file_type = if mode & 0o040000 != 0 {
        'd'
    } else if mode & 0o120000 != 0 {
        'l'
    } else {
        '-'
    };
    let pc = |bit: u32, ch: char| if mode & bit != 0 { ch } else { '-' };
    format!(
        "{}{}{}{}{}{}{}{}{}{}",
        file_type,
        pc(0o400, 'r'), pc(0o200, 'w'), pc(0o100, 'x'),
        pc(0o040, 'r'), pc(0o020, 'w'), pc(0o010, 'x'),
        pc(0o004, 'r'), pc(0o002, 'w'), pc(0o001, 'x'),
    )
}

#[async_trait]
impl RemoteFs for SftpBackend {
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let session = self.session.lock().await;
        let read_dir = session.read_dir(path).await.map_err(map_sftp_err)?;
        let mut out = Vec::new();
        for entry in read_dir {
            let name = entry.file_name();
            let ft = entry.file_type();
            let kind = if ft.is_dir() {
                "dir"
            } else if ft.is_symlink() {
                "symlink"
            } else if ft.is_file() {
                "file"
            } else {
                "other"
            };
            let meta = entry.metadata();
            out.push(DirEntry {
                path: posix_join(path, &name),
                name,
                kind: kind.to_string(),
                size: meta.size,
                modified: meta.mtime.map(unix_to_rfc3339),
                permissions: meta.permissions.map(format_mode),
                symlink_target: None,
            });
        }
        Ok(out)
    }

    async fn stat(&self, path: &str) -> Result<Stat, FsError> {
        let session = self.session.lock().await;
        let meta = session.metadata(path).await.map_err(map_sftp_err)?;
        // FileAttributes exposes `is_regular` (POSIX terminology) for what
        // most users call "a file". `file_type()` is the cleaner accessor
        // and gives us the dir / symlink / file / other discriminator.
        let ft = meta.file_type();
        let kind = if ft.is_dir() {
            "dir"
        } else if ft.is_symlink() {
            "symlink"
        } else if ft.is_file() {
            "file"
        } else {
            "other"
        };
        Ok(Stat {
            kind: kind.to_string(),
            size: meta.size,
            modified: meta.mtime.map(unix_to_rfc3339),
            permissions: meta.permissions.map(format_mode),
            mime: mime_for(path),
            is_binary: None,
        })
    }

    async fn read(
        &self,
        path: &str,
        range: Option<(u64, u64)>,
    ) -> Result<BoxStream<'static, Result<Bytes, FsError>>, FsError> {
        let session = self.session.lock().await;
        let mut file = session
            .open_with_flags(path.to_string(), OpenFlags::READ)
            .await
            .map_err(map_sftp_err)?;
        let mut buf = Vec::new();
        if let Some((start, end)) = range {
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| FsError::Other { detail: format!("seek: {}", e) })?;
            let len = end.saturating_sub(start) as usize;
            buf.resize(len, 0);
            file.read_exact(&mut buf)
                .await
                .map_err(|e| FsError::Other { detail: format!("read: {}", e) })?;
        } else {
            file.read_to_end(&mut buf)
                .await
                .map_err(|e| FsError::Other { detail: format!("read: {}", e) })?;
        }
        Ok(Box::pin(stream::iter(vec![Ok(Bytes::from(buf))])))
    }

    async fn write(
        &self,
        path: &str,
        body: BoxStream<'static, Result<Bytes, FsError>>,
        _size_hint: Option<u64>,
    ) -> Result<(), FsError> {
        use futures::StreamExt;
        let session = self.session.lock().await;
        let mut file = session.create(path.to_string()).await.map_err(map_sftp_err)?;
        let mut body = body;
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            file.write_all(&chunk)
                .await
                .map_err(|e| FsError::Other { detail: format!("write: {}", e) })?;
        }
        file.shutdown()
            .await
            .map_err(|e| FsError::Other { detail: format!("close: {}", e) })?;
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<(), FsError> {
        let session = self.session.lock().await;
        // Try as file first; fall back to directory.
        match session.remove_file(path.to_string()).await {
            Ok(_) => Ok(()),
            Err(_) => session
                .remove_dir(path.to_string())
                .await
                .map_err(map_sftp_err),
        }
    }

    async fn mkdir(&self, path: &str) -> Result<(), FsError> {
        let session = self.session.lock().await;
        session
            .create_dir(path.to_string())
            .await
            .map_err(map_sftp_err)
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        let session = self.session.lock().await;
        session
            .rename(from.to_string(), to.to_string())
            .await
            .map_err(map_sftp_err)
    }

    async fn search(&self, prefix: &str, glob: &str) -> Result<Vec<DirEntry>, FsError> {
        let pattern = glob_pattern(glob).map_err(|e| FsError::Other { detail: e })?;
        let mut out: Vec<DirEntry> = Vec::new();
        let mut stack: Vec<(String, usize)> = vec![(prefix.to_string(), 0)];
        while let Some((path, depth)) = stack.pop() {
            if depth > 8 || out.len() >= 5000 {
                continue;
            }
            // Releasing the session lock between iterations keeps other ops
            // (e.g. cancellation) responsive.
            let entries = match self.list(&path).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            for e in entries {
                if pattern.is_match(&e.name) {
                    out.push(e.clone());
                    if out.len() >= 5000 {
                        return Ok(out);
                    }
                }
                if e.kind == "dir" {
                    stack.push((e.path.clone(), depth + 1));
                }
            }
        }
        Ok(out)
    }

    async fn presigned_url(
        &self,
        _path: &str,
        _ttl_secs: u64,
    ) -> Result<Option<String>, FsError> {
        Ok(None)
    }

    async fn home_dir(&self) -> Result<Option<String>, FsError> {
        // SFTP `realpath(".")` (exposed by russh-sftp as `canonicalize`).
        // Servers chroot most users into `/home/<user>` (or similar) and
        // refuse to list `/`, so naively starting at `/` produces "access
        // denied" on first list. FileZilla / WinSCP both call this on
        // connect — we mirror that. Returning Err here would block the
        // session, so we degrade to None on failure (caller falls back
        // to `/`).
        let session = self.session.lock().await;
        match session.canonicalize(".").await {
            Ok(p) if !p.is_empty() => Ok(Some(p)),
            _ => Ok(None),
        }
    }
}

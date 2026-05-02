//! FTP / FTPS backend (suppaftp tokio runtime, rustls TLS).
//!
//! Single control connection — operations serialised behind a Mutex.
//! Default mode is passive; suppaftp uses passive by default. Implicit
//! FTPS is gated behind a deprecated feature in suppaftp and is omitted
//! here — users who need it should use explicit mode (`into_secure()`).

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use suppaftp::tokio::AsyncFtpStream;
use suppaftp::list::File as FtpFile;
use suppaftp::types::FileType as FtpFileType;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

use crate::modes::explorer::backends::{glob_pattern, mime_for};
use crate::modes::explorer::fs::RemoteFs;
use crate::modes::explorer::models::{DirEntry, FsError, Stat};

/// Plain FTP only for v1. FTPS-explicit needs suppaftp's `into_secure`
/// flow, which is awkward to type in tokio runtime mode (the upgraded
/// stream type differs from the initial one and the public surface
/// doesn't expose a clean polymorphic path). Tracking as v2 polish.
pub struct FtpBackend {
    stream: Arc<Mutex<AsyncFtpStream>>,
}

impl FtpBackend {
    pub async fn connect(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
        tls: &str, // "none" — "explicit"/"implicit" rejected for now
    ) -> Result<Self, FsError> {
        if tls == "explicit" || tls == "implicit" {
            return Err(FsError::Other {
                detail: format!(
                    "FTPS ({}) is not yet supported by Explorer. Use plain FTP or SFTP.",
                    tls
                ),
            });
        }
        let mut s = AsyncFtpStream::connect(format!("{}:{}", host, port))
            .await
            .map_err(|e| FsError::NetworkError {
                detail: format!("ftp connect: {}", e),
            })?;
        s.login(username, password).await.map_err(|e| FsError::AuthError {
            detail: format!("ftp login: {}", e),
        })?;
        let _ = s.transfer_type(FtpFileType::Binary).await;
        Ok(Self {
            stream: Arc::new(Mutex::new(s)),
        })
    }
}

fn map_err(e: suppaftp::FtpError) -> FsError {
    let s = e.to_string();
    if s.contains("550") || s.to_lowercase().contains("not found") {
        FsError::NotFound { path: String::new() }
    } else if s.contains("530") {
        FsError::AuthError { detail: s }
    } else {
        FsError::Other { detail: s }
    }
}

fn systime_to_rfc3339(t: SystemTime) -> Option<String> {
    let dur = t.duration_since(UNIX_EPOCH).ok()?;
    chrono::DateTime::from_timestamp(dur.as_secs() as i64, dur.subsec_nanos())
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn parse_mlsd_lines(lines: &[String]) -> Vec<FtpFile> {
    lines
        .iter()
        .filter_map(|l| FtpFile::from_str(l).ok())
        .collect()
}

#[async_trait]
impl RemoteFs for FtpBackend {
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let mut s = self.stream.lock().await;
        let lines = s.mlsd(Some(path)).await.map_err(map_err)?;
        let files = parse_mlsd_lines(&lines);
        let mut out = Vec::with_capacity(files.len());
        for f in files {
            if f.name() == "." || f.name() == ".." {
                continue;
            }
            let kind = if f.is_directory() {
                "dir"
            } else if f.is_symlink() {
                "symlink"
            } else if f.is_file() {
                "file"
            } else {
                "other"
            };
            let full_path = if path.ends_with('/') {
                format!("{}{}", path, f.name())
            } else {
                format!("{}/{}", path.trim_end_matches('/'), f.name())
            };
            out.push(DirEntry {
                name: f.name().to_string(),
                path: full_path,
                kind: kind.to_string(),
                size: Some(f.size() as u64),
                modified: systime_to_rfc3339(f.modified()),
                permissions: None,
                symlink_target: f.symlink().map(|p| p.to_string_lossy().into_owned()),
            });
        }
        Ok(out)
    }

    async fn stat(&self, path: &str) -> Result<Stat, FsError> {
        let mut s = self.stream.lock().await;
        let line = s.mlst(Some(path)).await.map_err(map_err)?;
        let f = FtpFile::from_str(&line).map_err(|e| FsError::Other {
            detail: format!("parse mlst: {}", e),
        })?;
        let kind = if f.is_directory() {
            "dir"
        } else if f.is_symlink() {
            "symlink"
        } else if f.is_file() {
            "file"
        } else {
            "other"
        };
        Ok(Stat {
            kind: kind.to_string(),
            size: Some(f.size() as u64),
            modified: systime_to_rfc3339(f.modified()),
            permissions: None,
            mime: mime_for(path),
            is_binary: None,
        })
    }

    async fn read(
        &self,
        path: &str,
        _range: Option<(u64, u64)>,
    ) -> Result<BoxStream<'static, Result<Bytes, FsError>>, FsError> {
        // Use the closure form `retr`. suppaftp expects a boxed pinned
        // future returning `(reader, value)`.
        let mut s = self.stream.lock().await;
        let buf: Vec<u8> = s
            .retr(path, |reader| {
                Box::pin(async move {
                    let mut buf = Vec::new();
                    let mut r = reader;
                    r.read_to_end(&mut buf)
                        .await
                        .map_err(suppaftp::FtpError::ConnectionError)?;
                    Ok((buf, r))
                })
            })
            .await
            .map_err(map_err)?;
        Ok(Box::pin(stream::iter(vec![Ok(Bytes::from(buf))])))
    }

    async fn write(
        &self,
        path: &str,
        body: BoxStream<'static, Result<Bytes, FsError>>,
        _size_hint: Option<u64>,
    ) -> Result<(), FsError> {
        let mut buf = Vec::new();
        let mut body = body;
        while let Some(chunk) = body.next().await {
            buf.extend_from_slice(&chunk?);
        }
        let mut cursor = std::io::Cursor::new(buf);
        let mut s = self.stream.lock().await;
        s.put_file(path, &mut cursor)
            .await
            .map(|_| ())
            .map_err(map_err)
    }

    async fn delete(&self, path: &str) -> Result<(), FsError> {
        let mut s = self.stream.lock().await;
        if s.rm(path).await.is_ok() {
            return Ok(());
        }
        s.rmdir(path).await.map_err(map_err)
    }

    async fn mkdir(&self, path: &str) -> Result<(), FsError> {
        let mut s = self.stream.lock().await;
        s.mkdir(path).await.map_err(map_err)
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        let mut s = self.stream.lock().await;
        s.rename(from, to).await.map_err(map_err)
    }

    async fn search(&self, prefix: &str, glob: &str) -> Result<Vec<DirEntry>, FsError> {
        let pat = glob_pattern(glob).map_err(|e| FsError::Other { detail: e })?;
        let mut out: Vec<DirEntry> = Vec::new();
        let mut stack: Vec<(String, usize)> = vec![(prefix.to_string(), 0)];
        while let Some((path, depth)) = stack.pop() {
            if depth > 8 || out.len() >= 5000 {
                continue;
            }
            let entries = match self.list(&path).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            for e in entries {
                if pat.is_match(&e.name) {
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
        // FTP `PWD` after auth — same role as SFTP realpath(".").
        let mut s = self.stream.lock().await;
        match s.pwd().await {
            Ok(p) if !p.is_empty() => Ok(Some(p)),
            _ => Ok(None),
        }
    }
}

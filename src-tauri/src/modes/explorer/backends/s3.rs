//! S3-family backend (AWS, R2, MinIO, Wasabi, Backblaze B2, GCS-S3compat,
//! Custom). One Rust impl, presets carried in `s3_presets.rs`.
//!
//! All HTTP goes through the proxy-aware `build_app_http_client` so corp
//! users behind a mandatory proxy can reach the public providers.
//!
//! Path semantics:
//!   - explorer paths look like `/<bucket>/<key>` with the bucket as the
//!     first segment. Within this backend we strip that prefix and operate
//!     on the bucket-relative key.

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use rusty_s3::actions::{
    DeleteObject, GetObject, HeadObject, ListObjectsV2, PutObject, S3Action,
};
use rusty_s3::{Bucket, Credentials, UrlStyle};
use std::time::Duration;
use url::Url;

use crate::modes::explorer::backends::{glob_pattern, mime_for};
use crate::modes::explorer::fs::RemoteFs;
use crate::modes::explorer::models::{DirEntry, FsError, Stat};

const SIGNED_URL_TTL: Duration = Duration::from_secs(60);

pub struct S3Backend {
    bucket: Bucket,
    creds: Credentials,
    http: reqwest::Client,
    bucket_name: String,
}

impl S3Backend {
    pub fn new(
        endpoint: &str,
        region: &str,
        bucket: &str,
        access_key: &str,
        secret_key: &str,
        path_style: bool,
        http: reqwest::Client,
    ) -> Result<Self, FsError> {
        if bucket.is_empty() {
            return Err(FsError::Other {
                detail: "S3 bucket name is required".to_string(),
            });
        }
        let endpoint_url = Url::parse(endpoint).map_err(|e| FsError::Other {
            detail: format!("invalid S3 endpoint URL '{}': {}", endpoint, e),
        })?;
        let url_style = if path_style {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };
        let bucket_obj = Bucket::new(
            endpoint_url,
            url_style,
            bucket.to_string(),
            region.to_string(),
        )
        .map_err(|e| FsError::Other {
            detail: format!("invalid S3 bucket spec: {:?}", e),
        })?;
        let creds = Credentials::new(access_key.to_string(), secret_key.to_string());
        Ok(Self {
            bucket: bucket_obj,
            creds,
            http,
            bucket_name: bucket.to_string(),
        })
    }

    /// Strip the leading `/<bucket>/` from a full explorer path → object key.
    fn key_of(&self, path: &str) -> Result<String, FsError> {
        let trimmed = path.trim_start_matches('/');
        let prefix = format!("{}/", self.bucket_name);
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            Ok(rest.to_string())
        } else if trimmed == self.bucket_name || trimmed.is_empty() {
            Ok(String::new())
        } else {
            Err(FsError::Other {
                detail: format!(
                    "path '{}' is not in bucket '{}'",
                    path, self.bucket_name
                ),
            })
        }
    }

    fn map_status(status: u16, body: &str, path: &str) -> FsError {
        match status {
            404 => FsError::NotFound {
                path: path.to_string(),
            },
            403 => FsError::PermissionDenied {
                path: path.to_string(),
                detail: body.to_string(),
            },
            _ => FsError::Other {
                detail: format!("S3 {} on {}: {}", status, path, body),
            },
        }
    }
}

#[async_trait]
impl RemoteFs for S3Backend {
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let key_prefix_raw = self.key_of(path)?;
        let key_prefix = if key_prefix_raw.is_empty() {
            String::new()
        } else if key_prefix_raw.ends_with('/') {
            key_prefix_raw
        } else {
            format!("{}/", key_prefix_raw)
        };

        let mut action = ListObjectsV2::new(&self.bucket, Some(&self.creds));
        action.with_delimiter("/");
        if !key_prefix.is_empty() {
            action.with_prefix(key_prefix.clone());
        }
        let signed = action.sign(SIGNED_URL_TTL);
        let resp = self
            .http
            .get(signed)
            .send()
            .await
            .map_err(|e| FsError::NetworkError {
                detail: e.to_string(),
            })?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Self::map_status(status.as_u16(), &body, path));
        }
        let parsed = ListObjectsV2::parse_response(body.as_bytes()).map_err(|e| {
            FsError::Other {
                detail: format!("parse list: {}", e),
            }
        })?;

        let mut entries = Vec::new();
        for cp in &parsed.common_prefixes {
            let cleaned = cp.prefix.trim_end_matches('/');
            let name = cleaned.rsplit('/').next().unwrap_or(cleaned).to_string();
            entries.push(DirEntry {
                name: name.clone(),
                path: format!("/{}/{}", self.bucket_name, cleaned),
                kind: "dir".to_string(),
                size: None,
                modified: None,
                permissions: None,
                symlink_target: None,
            });
        }
        for obj in &parsed.contents {
            // Skip the placeholder prefix object we'd create via mkdir.
            if obj.key == key_prefix {
                continue;
            }
            let name = obj.key.rsplit('/').next().unwrap_or(&obj.key).to_string();
            entries.push(DirEntry {
                name,
                path: format!("/{}/{}", self.bucket_name, obj.key),
                kind: "file".to_string(),
                size: Some(obj.size),
                modified: Some(obj.last_modified.clone()),
                permissions: None,
                symlink_target: None,
            });
        }
        Ok(entries)
    }

    async fn stat(&self, path: &str) -> Result<Stat, FsError> {
        let key = self.key_of(path)?;
        let action = HeadObject::new(&self.bucket, Some(&self.creds), &key);
        let signed = action.sign(SIGNED_URL_TTL);
        let resp = self
            .http
            .head(signed)
            .send()
            .await
            .map_err(|e| FsError::NetworkError {
                detail: e.to_string(),
            })?;
        if !resp.status().is_success() {
            return Err(Self::map_status(resp.status().as_u16(), "", path));
        }
        let size = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok().and_then(|s| s.parse::<u64>().ok()));
        let modified = resp
            .headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok().map(|s| s.to_string()));
        Ok(Stat {
            kind: "file".to_string(),
            size,
            modified,
            permissions: None,
            mime: mime_for(path),
            is_binary: None,
        })
    }

    async fn read(
        &self,
        path: &str,
        range: Option<(u64, u64)>,
    ) -> Result<BoxStream<'static, Result<Bytes, FsError>>, FsError> {
        let key = self.key_of(path)?;
        let action = GetObject::new(&self.bucket, Some(&self.creds), &key);
        let signed = action.sign(SIGNED_URL_TTL);
        let mut req = self.http.get(signed);
        if let Some((s, e)) = range {
            req = req.header("Range", format!("bytes={}-{}", s, e));
        }
        let resp = req.send().await.map_err(|e| FsError::NetworkError {
            detail: e.to_string(),
        })?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(Self::map_status(status, &body, path));
        }
        let stream = resp
            .bytes_stream()
            .map(|r| {
                r.map(Bytes::from).map_err(|e| FsError::NetworkError {
                    detail: e.to_string(),
                })
            })
            .boxed();
        Ok(stream)
    }

    async fn write(
        &self,
        path: &str,
        body: BoxStream<'static, Result<Bytes, FsError>>,
        size_hint: Option<u64>,
    ) -> Result<(), FsError> {
        // Buffer the body (single PUT). Multipart upload is a v2 feature for
        // very large files; cap at 5 GB which is also S3's single-PUT limit.
        let mut buf = Vec::with_capacity(size_hint.unwrap_or(0) as usize);
        let mut body = body;
        while let Some(chunk) = body.next().await {
            buf.extend_from_slice(&chunk?);
            if buf.len() as u64 > 5 * 1024 * 1024 * 1024 {
                return Err(FsError::Other {
                    detail: "single-PUT upload exceeds 5 GB; multipart not yet implemented"
                        .to_string(),
                });
            }
        }
        let key = self.key_of(path)?;
        let action = PutObject::new(&self.bucket, Some(&self.creds), &key);
        let signed = action.sign(SIGNED_URL_TTL);
        let resp =
            self.http
                .put(signed)
                .body(buf)
                .send()
                .await
                .map_err(|e| FsError::NetworkError {
                    detail: e.to_string(),
                })?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(Self::map_status(status, &body, path));
        }
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<(), FsError> {
        let key = self.key_of(path)?;
        let action = DeleteObject::new(&self.bucket, Some(&self.creds), &key);
        let signed = action.sign(SIGNED_URL_TTL);
        let resp = self
            .http
            .delete(signed)
            .send()
            .await
            .map_err(|e| FsError::NetworkError {
                detail: e.to_string(),
            })?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(Self::map_status(status, &body, path));
        }
        Ok(())
    }

    async fn mkdir(&self, path: &str) -> Result<(), FsError> {
        // S3 has no real directories. Convention: 0-byte object with a
        // trailing slash. Listings interpret these as folders.
        let mut key = self.key_of(path)?;
        if !key.ends_with('/') {
            key.push('/');
        }
        let action = PutObject::new(&self.bucket, Some(&self.creds), &key);
        let signed = action.sign(SIGNED_URL_TTL);
        let resp = self
            .http
            .put(signed)
            .body(Vec::<u8>::new())
            .send()
            .await
            .map_err(|e| FsError::NetworkError {
                detail: e.to_string(),
            })?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(Self::map_status(status, &body, path));
        }
        Ok(())
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        // S3 has no rename — it's a server-side copy + delete.
        let src_key = self.key_of(from)?;
        let dst_key = self.key_of(to)?;
        let copy_source = format!("/{}/{}", self.bucket_name, src_key);
        let mut action = PutObject::new(&self.bucket, Some(&self.creds), &dst_key);
        action.headers_mut().insert("x-amz-copy-source", copy_source);
        let signed = action.sign(SIGNED_URL_TTL);
        let resp = self
            .http
            .put(signed)
            .body(Vec::<u8>::new())
            .send()
            .await
            .map_err(|e| FsError::NetworkError {
                detail: e.to_string(),
            })?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(Self::map_status(status, &body, from));
        }
        self.delete(from).await
    }

    async fn search(&self, prefix: &str, glob: &str) -> Result<Vec<DirEntry>, FsError> {
        let pat = glob_pattern(glob).map_err(|e| FsError::Other { detail: e })?;
        let key_prefix = self.key_of(prefix)?;
        let mut continuation_token: Option<String> = None;
        let mut out: Vec<DirEntry> = Vec::new();

        loop {
            let mut action = ListObjectsV2::new(&self.bucket, Some(&self.creds));
            if !key_prefix.is_empty() {
                action.with_prefix(key_prefix.clone());
            }
            if let Some(c) = &continuation_token {
                action.with_continuation_token(c.clone());
            }
            let signed = action.sign(SIGNED_URL_TTL);
            let resp =
                self.http
                    .get(signed)
                    .send()
                    .await
                    .map_err(|e| FsError::NetworkError {
                        detail: e.to_string(),
                    })?;
            let body = resp.text().await.unwrap_or_default();
            let parsed = ListObjectsV2::parse_response(body.as_bytes()).map_err(|e| {
                FsError::Other {
                    detail: format!("parse list: {}", e),
                }
            })?;
            for obj in &parsed.contents {
                let name = obj.key.rsplit('/').next().unwrap_or(&obj.key);
                if pat.is_match(name) {
                    out.push(DirEntry {
                        name: name.to_string(),
                        path: format!("/{}/{}", self.bucket_name, obj.key),
                        kind: "file".to_string(),
                        size: Some(obj.size),
                        modified: Some(obj.last_modified.clone()),
                        permissions: None,
                        symlink_target: None,
                    });
                    if out.len() >= 5000 {
                        return Ok(out);
                    }
                }
            }
            match parsed.next_continuation_token {
                Some(tok) => continuation_token = Some(tok),
                None => break,
            }
        }
        Ok(out)
    }

    async fn presigned_url(
        &self,
        path: &str,
        ttl_secs: u64,
    ) -> Result<Option<String>, FsError> {
        let key = self.key_of(path)?;
        let action = GetObject::new(&self.bucket, Some(&self.creds), &key);
        let signed = action.sign(Duration::from_secs(ttl_secs));
        Ok(Some(signed.to_string()))
    }
}

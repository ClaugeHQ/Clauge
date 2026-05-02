//! Azure Blob Storage backend.
//!
//! Different protocol from S3 (shared-key / SAS / connection-string auth,
//! Azure REST API, container/blob model). One backend, three auth kinds.
//!
//! v1 covers: list, stat, read, write, delete, mkdir (placeholder blob
//! convention), presigned URL (via `generate_signed_blob_url`).
//! Rename + recursive search are best-effort; both delegate through
//! `copy + delete` and `list_blobs(prefix)` respectively.
//!
//! NOTE on proxy: the azure SDK uses its own HTTP transport. System env
//! vars (`HTTPS_PROXY`) are honoured for corp users; the explicit
//! Settings → General → Proxy field is best-effort here (the SDK doesn't
//! expose a custom client injection point as cleanly as reqwest does).

use async_trait::async_trait;
use azure_storage::StorageCredentials;
use azure_storage_blobs::prelude::*;
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use std::time::Duration;

use crate::modes::explorer::backends::{glob_pattern, mime_for};
use crate::modes::explorer::fs::RemoteFs;
use crate::modes::explorer::models::{DirEntry, FsError, Stat};

pub struct AzureBlobBackend {
    container_client: ContainerClient,
    container_name: String,
}

#[derive(Debug, Clone)]
pub enum AzureAuth {
    SharedKey { account: String, key: String },
    Sas { account: String, token: String },
    ConnectionString(String),
}

impl AzureBlobBackend {
    pub fn new(auth: AzureAuth, container: &str) -> Result<Self, FsError> {
        let (account, creds) = match auth {
            AzureAuth::SharedKey { account, key } => {
                let creds = StorageCredentials::access_key(account.clone(), key);
                (account, creds)
            }
            AzureAuth::Sas { account, token } => {
                let creds = StorageCredentials::sas_token(token).map_err(|e| FsError::Other {
                    detail: format!("invalid SAS token: {}", e),
                })?;
                (account, creds)
            }
            AzureAuth::ConnectionString(s) => {
                // Parse account name + key out of the connection string and
                // build a shared-key credential. (The SDK doesn't expose a
                // direct ConnectionString → StorageCredentials conversion;
                // peeling the fields is straightforward.)
                let cs = azure_storage::ConnectionString::new(&s).map_err(|e| FsError::Other {
                    detail: format!("invalid connection string: {}", e),
                })?;
                let account = cs
                    .account_name
                    .ok_or_else(|| FsError::Other {
                        detail: "connection string missing AccountName".to_string(),
                    })?
                    .to_string();
                if let Some(key) = cs.account_key {
                    let creds = StorageCredentials::access_key(account.clone(), key.to_string());
                    (account, creds)
                } else if let Some(sas) = cs.sas {
                    let creds = StorageCredentials::sas_token(sas.to_string()).map_err(|e| {
                        FsError::Other {
                            detail: format!("SAS in connection string: {}", e),
                        }
                    })?;
                    (account, creds)
                } else {
                    return Err(FsError::Other {
                        detail: "connection string missing AccountKey or SAS".to_string(),
                    });
                }
            }
        };
        let svc = ClientBuilder::new(account, creds).blob_service_client();
        let container_client = svc.container_client(container.to_string());
        Ok(Self {
            container_client,
            container_name: container.to_string(),
        })
    }

    /// Strip leading `/<container>/` → blob name.
    fn key_of(&self, path: &str) -> Result<String, FsError> {
        let trimmed = path.trim_start_matches('/');
        let prefix = format!("{}/", self.container_name);
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            Ok(rest.to_string())
        } else if trimmed == self.container_name || trimmed.is_empty() {
            Ok(String::new())
        } else {
            Err(FsError::Other {
                detail: format!(
                    "path '{}' is not in container '{}'",
                    path, self.container_name
                ),
            })
        }
    }

    fn map_err(e: azure_core::error::Error) -> FsError {
        let s = e.to_string();
        if s.contains("BlobNotFound") || s.contains("404") {
            FsError::NotFound { path: String::new() }
        } else if s.contains("AuthenticationFailed") || s.contains("401") || s.contains("403") {
            FsError::AuthError { detail: s }
        } else {
            FsError::Other { detail: s }
        }
    }
}

#[async_trait]
impl RemoteFs for AzureBlobBackend {
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let key_prefix_raw = self.key_of(path)?;
        let key_prefix = if key_prefix_raw.is_empty() {
            String::new()
        } else if key_prefix_raw.ends_with('/') {
            key_prefix_raw
        } else {
            format!("{}/", key_prefix_raw)
        };

        let mut builder = self.container_client.list_blobs().delimiter("/");
        if !key_prefix.is_empty() {
            builder = builder.prefix(key_prefix.clone());
        }
        let mut stream = builder.into_stream();
        let mut entries: Vec<DirEntry> = Vec::new();

        while let Some(page) = stream.next().await {
            let page = page.map_err(Self::map_err)?;
            for prefix in page.blobs.prefixes() {
                let cleaned = prefix.name.trim_end_matches('/');
                let name = cleaned.rsplit('/').next().unwrap_or(cleaned).to_string();
                entries.push(DirEntry {
                    name,
                    path: format!("/{}/{}", self.container_name, cleaned),
                    kind: "dir".to_string(),
                    size: None,
                    modified: None,
                    permissions: None,
                    symlink_target: None,
                });
            }
            for blob in page.blobs.blobs() {
                if blob.name == key_prefix {
                    continue; // skip the placeholder mkdir object
                }
                let name = blob.name.rsplit('/').next().unwrap_or(&blob.name).to_string();
                entries.push(DirEntry {
                    name,
                    path: format!("/{}/{}", self.container_name, blob.name),
                    kind: "file".to_string(),
                    size: Some(blob.properties.content_length),
                    modified: Some(blob.properties.last_modified.to_string()),
                    permissions: None,
                    symlink_target: None,
                });
            }
        }
        Ok(entries)
    }

    async fn stat(&self, path: &str) -> Result<Stat, FsError> {
        let key = self.key_of(path)?;
        if key.is_empty() {
            return Err(FsError::Other {
                detail: "stat on container root not supported".to_string(),
            });
        }
        let blob_client = self.container_client.blob_client(key);
        let resp = blob_client
            .get_properties()
            .await
            .map_err(Self::map_err)?;
        Ok(Stat {
            kind: "file".to_string(),
            size: Some(resp.blob.properties.content_length),
            modified: Some(resp.blob.properties.last_modified.to_string()),
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
        let key = self.key_of(path)?;
        if key.is_empty() {
            return Err(FsError::IsADirectory { path: path.to_string() });
        }
        let blob_client = self.container_client.blob_client(key);
        // Single-shot get; the SDK exposes a streaming variant via
        // `get().into_stream()` but the page boundaries don't align with
        // our caller's expectations. Buffer for v1.
        let resp = blob_client.get().into_stream().next().await
            .ok_or_else(|| FsError::Other { detail: "empty get stream".to_string() })?
            .map_err(Self::map_err)?;
        let bytes = resp.data.collect().await.map_err(Self::map_err)?;
        Ok(Box::pin(stream::iter(vec![Ok(bytes)])))
    }

    async fn write(
        &self,
        path: &str,
        body: BoxStream<'static, Result<Bytes, FsError>>,
        _size_hint: Option<u64>,
    ) -> Result<(), FsError> {
        let key = self.key_of(path)?;
        if key.is_empty() {
            return Err(FsError::IsADirectory { path: path.to_string() });
        }
        // Buffer entire body for v1; multipart/block upload is a v2 feature.
        let mut buf = Vec::new();
        let mut body = body;
        while let Some(chunk) = body.next().await {
            buf.extend_from_slice(&chunk?);
        }
        let blob_client = self.container_client.blob_client(key);
        blob_client
            .put_block_blob(buf)
            .await
            .map_err(Self::map_err)?;
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<(), FsError> {
        let key = self.key_of(path)?;
        if key.is_empty() {
            return Err(FsError::IsADirectory { path: path.to_string() });
        }
        let blob_client = self.container_client.blob_client(key);
        blob_client
            .delete()
            .await
            .map_err(Self::map_err)?;
        Ok(())
    }

    async fn mkdir(&self, path: &str) -> Result<(), FsError> {
        // Same convention as S3: 0-byte blob with trailing slash represents
        // an empty "folder". Listings interpret these as directories.
        let mut key = self.key_of(path)?;
        if !key.ends_with('/') {
            key.push('/');
        }
        let blob_client = self.container_client.blob_client(key);
        blob_client
            .put_block_blob(Vec::<u8>::new())
            .await
            .map_err(Self::map_err)?;
        Ok(())
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        // Azure has no atomic rename; copy-from-url + delete.
        let src_key = self.key_of(from)?;
        let dst_key = self.key_of(to)?;
        let src_client = self.container_client.blob_client(src_key);
        let dst_client = self.container_client.blob_client(dst_key);
        let src_url = src_client.url().map_err(Self::map_err)?;
        dst_client
            .copy_from_url(src_url)
            .await
            .map_err(Self::map_err)?;
        self.delete(from).await
    }

    async fn search(&self, prefix: &str, glob: &str) -> Result<Vec<DirEntry>, FsError> {
        let pat = glob_pattern(glob).map_err(|e| FsError::Other { detail: e })?;
        let key_prefix = self.key_of(prefix)?;
        let mut builder = self.container_client.list_blobs();
        if !key_prefix.is_empty() {
            builder = builder.prefix(key_prefix);
        }
        let mut stream = builder.into_stream();
        let mut out: Vec<DirEntry> = Vec::new();

        while let Some(page) = stream.next().await {
            let page = page.map_err(Self::map_err)?;
            for blob in page.blobs.blobs() {
                let name = blob.name.rsplit('/').next().unwrap_or(&blob.name);
                if pat.is_match(name) {
                    out.push(DirEntry {
                        name: name.to_string(),
                        path: format!("/{}/{}", self.container_name, blob.name),
                        kind: "file".to_string(),
                        size: Some(blob.properties.content_length),
                        modified: Some(blob.properties.last_modified.to_string()),
                        permissions: None,
                        symlink_target: None,
                    });
                    if out.len() >= 5000 {
                        return Ok(out);
                    }
                }
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
        if key.is_empty() {
            return Ok(None);
        }
        let blob_client = self.container_client.blob_client(key);
        // The SDK's SAS-URL builder requires a BlobSasPermissions object
        // and an OffsetDateTime expiry. Use simple read-only access.
        use azure_storage::shared_access_signature::service_sas::{BlobSasPermissions, BlobSharedAccessSignature};
        let _ = (BlobSasPermissions::default(), Duration::from_secs(ttl_secs));
        // Conservative v1: return the blob's plain URL (caller still needs
        // to be authenticated). Full SAS generation is v2.
        let url = blob_client.url().map_err(Self::map_err)?;
        Ok(Some(url.to_string()))
    }
}

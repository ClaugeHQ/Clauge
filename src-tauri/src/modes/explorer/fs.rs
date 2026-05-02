//! `RemoteFs` — uniform remote-filesystem trait. Each backend implements
//! it; the session layer + AI tools target the trait directly.
//!
//! Path conventions (enforced by every backend):
//!   - leading slash always present (`/var/log/syslog`, `/my-bucket/key`)
//!   - forward slashes only; backslashes never appear
//!   - bucket / container is the FIRST path component for S3 / Azure

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;

use crate::modes::explorer::models::{DirEntry, FsError, Stat};

#[async_trait]
pub trait RemoteFs: Send + Sync {
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError>;
    async fn stat(&self, path: &str) -> Result<Stat, FsError>;

    /// Optional byte range; `None` means "the whole file".
    async fn read(
        &self,
        path: &str,
        range: Option<(u64, u64)>,
    ) -> Result<BoxStream<'static, Result<Bytes, FsError>>, FsError>;

    /// Write streaming body. Caller signals end-of-stream by exhausting it.
    /// `size_hint` (when known) lets the backend pre-allocate or pick a
    /// multipart upload strategy.
    async fn write(
        &self,
        path: &str,
        body: BoxStream<'static, Result<Bytes, FsError>>,
        size_hint: Option<u64>,
    ) -> Result<(), FsError>;

    async fn delete(&self, path: &str) -> Result<(), FsError>;
    async fn mkdir(&self, path: &str) -> Result<(), FsError>;
    async fn rename(&self, from: &str, to: &str) -> Result<(), FsError>;

    /// Recursive search under `prefix` matching the (POSIX-style) `glob`.
    /// Backends cap depth and result count to avoid runaway operations.
    async fn search(&self, prefix: &str, glob: &str) -> Result<Vec<DirEntry>, FsError>;

    /// Returns a presigned URL for backends that support it (S3, Azure
    /// Blob). `Ok(None)` for backends that don't (SFTP, FTP).
    async fn presigned_url(
        &self,
        path: &str,
        ttl_secs: u64,
    ) -> Result<Option<String>, FsError>;

    /// Server-side default starting directory for this session — what
    /// FileZilla calls "remote home". For SFTP this is `realpath(".")`;
    /// for FTP it's `PWD` after auth. Returning `None` means the caller
    /// should fall back to its own default (usually `/` or the bucket).
    /// Object-storage backends (S3, Azure) keep this as `None` since they
    /// have no per-user home concept.
    async fn home_dir(&self) -> Result<Option<String>, FsError> {
        Ok(None)
    }
}

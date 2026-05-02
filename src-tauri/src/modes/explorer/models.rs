//! Serde structs shared between Rust and TypeScript via Tauri commands.

use serde::{Deserialize, Serialize};

/// One row in `explorer_connections`. Kind-discriminated columns are all
/// `Option`; only the ones relevant to `kind` are populated.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerConnection {
    pub id: String,
    pub name: String,
    /// `'sftp' | 'ftp' | 's3' | 'azure_blob'`
    pub kind: String,
    pub accent_color: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,

    // SFTP — preferred path is to reuse an existing ssh_profiles row.
    pub ssh_profile_id: Option<String>,
    pub sftp_working_dir: Option<String>,

    // SFTP-direct + FTP shared.
    pub host: Option<String>,
    pub port: Option<i64>,
    pub username: Option<String>,
    pub auth_type: Option<String>,
    pub key_path: Option<String>,

    // FTP-specific.
    pub ftp_passive: i64,
    pub ftp_tls: Option<String>,

    // S3-specific.
    pub s3_preset: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_region: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_path_style: i64,

    // Azure Blob-specific.
    pub azure_account: Option<String>,
    pub azure_container: Option<String>,
    pub azure_auth_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    /// Full POSIX path with leading slash.
    pub path: String,
    /// `'file' | 'dir' | 'symlink' | 'other'`
    pub kind: String,
    pub size: Option<u64>,
    /// RFC3339 with millisecond precision (WKWebView-safe).
    pub modified: Option<String>,
    /// chmod-style "drwxr-xr-x" for SFTP/FTP; None for S3/Azure.
    pub permissions: Option<String>,
    pub symlink_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stat {
    pub kind: String,
    pub size: Option<u64>,
    pub modified: Option<String>,
    pub permissions: Option<String>,
    pub mime: Option<String>,
    pub is_binary: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub direction: String, // 'upload' | 'download'
    pub connection_id: String,
    pub local_path: Option<String>,
    pub remote_path: String,
    pub bytes_total: Option<u64>,
    pub bytes_done: u64,
    pub state: String, // 'pending' | 'running' | 'completed' | 'failed' | 'cancelled'
    pub error: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

/// Typed errors mapped from each backend. Frontend matches on `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "data")]
pub enum FsError {
    NotFound { path: String },
    PermissionDenied { path: String, detail: String },
    AlreadyExists { path: String },
    IsADirectory { path: String },
    NotADirectory { path: String },
    NetworkError { detail: String },
    AuthError { detail: String },
    Cancelled,
    Other { detail: String },
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::NotFound { path } => write!(f, "Not found: {}", path),
            FsError::PermissionDenied { path, detail } => {
                write!(f, "Permission denied on {}: {}", path, detail)
            }
            FsError::AlreadyExists { path } => write!(f, "Already exists: {}", path),
            FsError::IsADirectory { path } => write!(f, "Is a directory: {}", path),
            FsError::NotADirectory { path } => write!(f, "Not a directory: {}", path),
            FsError::NetworkError { detail } => write!(f, "Network error: {}", detail),
            FsError::AuthError { detail } => write!(f, "Authentication error: {}", detail),
            FsError::Cancelled => write!(f, "Cancelled"),
            FsError::Other { detail } => write!(f, "{}", detail),
        }
    }
}

impl std::error::Error for FsError {}

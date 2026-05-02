//! Explorer mode — remote file system browser.
//!
//! Vertical slice covering SFTP, FTP, S3-compatible (AWS / R2 / MinIO /
//! Wasabi / B2 / GCS), and Azure Blob storage behind a single shared
//! `RemoteFs` trait + uniform UI + uniform AI tool surface.
//!
//! Layout:
//!   - `models`      — shared serde structs (ExplorerConnection, DirEntry,
//!                     Stat, Transfer, FsError)
//!   - `fs`          — `RemoteFs` trait
//!   - `connections` — Tauri commands for connection CRUD
//!   - `session`     — active-session map (tabKey → Box<dyn RemoteFs>) +
//!                     fs operation Tauri commands
//!   - `transfers`   — long-running upload/download progress events
//!   - `ai_tools`    — 11 AI tools, registered via shared dispatch
//!   - `backends`    — per-protocol RemoteFs impls

pub mod ai_tools;
pub mod backends;
pub mod connections;
pub mod fs;
pub mod models;
pub mod session;
pub mod transfers;

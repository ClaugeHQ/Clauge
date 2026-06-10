// Local snapshots — recovery copies written BEFORE any destructive import
// (pull, conflict resolution, merge, snapshot-restore itself). File format
// is exactly the wire payload (gzip JSON), so restore = import_kind.
// Invariant: callers must treat a snapshot failure as ABORT.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use parking_lot::Mutex;
use serde::Serialize;
use sqlx::SqlitePool;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::cloud::domains::{export_kind, import_kind, ALL_KINDS};

pub const MAX_SNAPSHOTS: usize = 30;

static SNAPSHOT_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn snapshot_dir_cell() -> &'static Mutex<Option<PathBuf>> {
    SNAPSHOT_DIR.get_or_init(|| Mutex::new(None))
}

pub fn init(app_data_dir: &std::path::Path) {
    *snapshot_dir_cell().lock() = Some(app_data_dir.join("sync-snapshots"));
}

#[cfg(test)]
fn set_dir_for_test(dir: PathBuf) {
    *snapshot_dir_cell().lock() = Some(dir);
}

fn dir() -> Result<PathBuf, String> {
    snapshot_dir_cell()
        .lock()
        .clone()
        .ok_or_else(|| "snapshot dir not initialised".to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub file_name: String,
    pub kind: String,
    pub reason: String,
    pub created_at: String,
    pub size_bytes: u64,
}

/// Export `kind` and write it to disk. Filename: `<utc-ts>__<kind>__<reason>.json.gz`.
pub async fn snapshot_kind(pool: &SqlitePool, kind: &str, reason: &str) -> Result<PathBuf, String> {
    let (_hash, payload_b64) = export_kind(pool, kind).await?;
    let gz = B64
        .decode(&payload_b64)
        .map_err(|e| format!("snapshot decode: {}", e))?;
    let d = dir()?;
    fs::create_dir_all(&d).map_err(|e| format!("snapshot dir: {}", e))?;
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let path = d.join(format!("{}__{}__{}.json.gz", ts, kind, reason));
    fs::write(&path, &gz).map_err(|e| format!("snapshot write: {}", e))?;
    prune(&d);
    Ok(path)
}

/// Keep only the newest MAX_SNAPSHOTS files (filenames sort chronologically).
fn prune(d: &PathBuf) {
    let Ok(entries) = fs::read_dir(d) else { return };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json.gz"))
        .collect();
    names.sort();
    while names.len() > MAX_SNAPSHOTS {
        let victim = names.remove(0);
        let _ = fs::remove_file(d.join(victim));
    }
}

#[allow(dead_code)]
pub fn list_snapshots() -> Result<Vec<SnapshotInfo>, String> {
    let d = dir()?;
    let Ok(entries) = fs::read_dir(&d) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for e in entries.filter_map(|e| e.ok()) {
        let name = match e.file_name().into_string() {
            Ok(n) if n.ends_with(".json.gz") => n,
            _ => continue,
        };
        let stem = name.trim_end_matches(".json.gz");
        let parts: Vec<&str> = stem.splitn(3, "__").collect();
        if parts.len() != 3 {
            continue;
        }
        out.push(SnapshotInfo {
            file_name: name.clone(),
            kind: parts[1].to_string(),
            reason: parts[2].to_string(),
            created_at: parts[0].to_string(),
            size_bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
        });
    }
    out.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    Ok(out)
}

/// Restore a snapshot file. Snapshots the CURRENT state of that kind first.
/// `file_name` must be a bare name — path components rejected.
pub async fn restore_snapshot(pool: &SqlitePool, file_name: &str) -> Result<(), String> {
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return Err("invalid snapshot name".to_string());
    }
    let stem = file_name.trim_end_matches(".json.gz");
    let parts: Vec<&str> = stem.splitn(3, "__").collect();
    if parts.len() != 3 {
        return Err("invalid snapshot name".to_string());
    }
    let kind = parts[1];
    if !ALL_KINDS.contains(&kind) {
        return Err(format!("unknown kind in snapshot: {}", kind));
    }
    let path = dir()?.join(file_name);
    let gz = fs::read(&path).map_err(|e| format!("snapshot read: {}", e))?;

    snapshot_kind(pool, kind, "pre-snapshot-restore").await?;
    import_kind(pool, kind, &B64.encode(&gz)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE ssh_profiles (id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL, \
             port INTEGER NOT NULL, username TEXT NOT NULL, auth_type TEXT NOT NULL, key_path TEXT, \
             accent_color TEXT, jump_profile_id TEXT, proxy_command TEXT, created_at TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ssh_profiles (id, name, host, port, username, auth_type, created_at) \
             VALUES ('p1', 'box', 'h', 22, 'u', 'key', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn snapshot_writes_file_and_prunes() {
        let dir = std::env::temp_dir().join(format!("clauge-snap-test-{}", uuid::Uuid::new_v4()));
        set_dir_for_test(dir.clone());
        let pool = test_pool().await;

        let p = snapshot_kind(&pool, "ssh", "pre-pull").await.unwrap();
        assert!(p.exists());
        assert!(p.file_name().unwrap().to_string_lossy().contains("__ssh__pre-pull"));

        for _ in 0..35 {
            snapshot_kind(&pool, "ssh", "pre-pull").await.unwrap();
        }
        let count = std::fs::read_dir(&dir).unwrap().count();
        assert!(count <= MAX_SNAPSHOTS);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn restore_roundtrip() {
        let dir = std::env::temp_dir().join(format!("clauge-snap-test-{}", uuid::Uuid::new_v4()));
        set_dir_for_test(dir.clone());
        let pool = test_pool().await;

        let path = snapshot_kind(&pool, "ssh", "manual").await.unwrap();
        sqlx::query("DELETE FROM ssh_profiles").execute(&pool).await.unwrap();

        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        restore_snapshot(&pool, &file_name).await.unwrap();

        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ssh_profiles")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn restore_rejects_traversal_names() {
        let dir = std::env::temp_dir().join(format!("clauge-snap-test-{}", uuid::Uuid::new_v4()));
        set_dir_for_test(dir.clone());
        let pool = test_pool().await;
        assert!(restore_snapshot(&pool, "../evil.json.gz").await.is_err());
        assert!(restore_snapshot(&pool, "nope__badkind__x.json.gz").await.is_err());
    }
}

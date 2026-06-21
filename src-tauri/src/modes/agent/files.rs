// Agent mode — local file explorer backend.
//
// A small `std::fs` surface plus a `notify` watcher that powers the
// in-tab file explorer (browse the active session's working tree, view /
// edit / save files, basic file ops, and drag-to-context). This is a
// LOCAL filesystem subsystem and intentionally separate from Explorer
// mode's remote `RemoteFs` trait.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::shared::cli::registry::runner_for;

/// Directory/file NAMES hidden from the tree and dropped from watcher
/// events — heavy / noisy dirs the agent churns. Matched by path-segment
/// name, never by substring, so a session rooted *inside* one of these
/// (e.g. a worktree under `.clauge-worktrees/`) still lists its contents.
const IGNORED_NAMES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    ".svelte-kit",
    ".clauge-worktrees",
];

/// Files larger than this open read-only (no edit/save).
const MAX_EDITABLE_BYTES: u64 = 2 * 1024 * 1024; // 2 MB

fn is_ignored_name(name: &str) -> bool {
    IGNORED_NAMES.contains(&name)
}

/// True if `path` lies inside an ignored dir *relative to `root`*. The root
/// prefix is stripped first so an ignored segment that's part of the root
/// itself (a worktree path) doesn't wrongly hide everything below it.
fn rel_is_ignored(root: &str, path: &str) -> bool {
    let root_n = root.replace('\\', "/");
    let path_n = path.replace('\\', "/");
    let rel = path_n.strip_prefix(&root_n).unwrap_or(&path_n);
    rel.split('/').any(|seg| IGNORED_NAMES.contains(&seg))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Last-modified time in epoch milliseconds (0 if unavailable).
    pub modified: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    /// UTF-8 text, or `None` when binary / too large.
    pub content: Option<String>,
    pub is_binary: bool,
    pub size: u64,
    pub too_large: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FsChange {
    pub paths: Vec<String>,
}

/// Holds the single active filesystem watcher. Dropping the watcher
/// (replacing it with `None`) stops it. One watcher at a time — the
/// explorer follows the active session's root.
pub struct FsWatchState(pub Arc<Mutex<Option<notify::RecommendedWatcher>>>);

impl Default for FsWatchState {
    fn default() -> Self {
        FsWatchState(Arc::new(Mutex::new(None)))
    }
}

fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// List the immediate children of `path`. Directories first, then files,
/// each group sorted case-insensitively by name. Ignored dirs are hidden.
#[tauri::command]
pub fn agent_fs_list_dir(path: String) -> Result<Vec<FsEntry>, String> {
    let mut entries: Vec<FsEntry> = Vec::new();
    let read = std::fs::read_dir(&path).map_err(|e| e.to_string())?;
    for item in read {
        let item = match item {
            Ok(i) => i,
            Err(_) => continue,
        };
        let name = item.file_name().to_string_lossy().to_string();
        if is_ignored_name(&name) {
            continue;
        }
        let p = item.path();
        let p_str = p.to_string_lossy().to_string();
        let meta = match item.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        entries.push(FsEntry {
            name,
            path: p_str,
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified: modified_ms(&meta),
        });
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

/// Read a file as UTF-8 text. Binary files and files over
/// [`MAX_EDITABLE_BYTES`] return no content (the UI shows a read-only
/// notice instead of garbage).
#[tauri::command]
pub fn agent_fs_read_file(path: String) -> Result<FileContent, String> {
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    let size = meta.len();
    if size > MAX_EDITABLE_BYTES {
        return Ok(FileContent { content: None, is_binary: false, size, too_large: true });
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    // Binary heuristic: a NUL byte in the first 8 KB, or invalid UTF-8.
    let probe = &bytes[..bytes.len().min(8192)];
    if probe.contains(&0) {
        return Ok(FileContent { content: None, is_binary: true, size, too_large: false });
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(FileContent { content: Some(text), is_binary: false, size, too_large: false }),
        Err(_) => Ok(FileContent { content: None, is_binary: true, size, too_large: false }),
    }
}

#[tauri::command]
pub fn agent_fs_write_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn agent_fs_rename(from: String, to: String) -> Result<(), String> {
    std::fs::rename(&from, &to).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn agent_fs_delete(path: String) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        std::fs::remove_dir_all(&path).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(&path).map_err(|e| e.to_string())
    }
}

/// Create a new file (empty) or directory. Parent directories are created
/// as needed.
#[tauri::command]
pub fn agent_fs_create(path: String, is_dir: bool) -> Result<(), String> {
    if is_dir {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())
    } else {
        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, "").map_err(|e| e.to_string())
    }
}

/// Reveal a path in the OS file manager (selecting it where supported).
#[tauri::command]
pub fn agent_fs_reveal(path: String) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let result = Command::new("open").args(["-R", &path]).spawn();
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer").arg(format!("/select,{path}")).spawn();
    #[cfg(target_os = "linux")]
    let result = {
        // xdg-open can't select; open the containing directory instead.
        let dir = Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        Command::new("xdg-open").arg(dir).spawn()
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}

/// Start (or replace) the filesystem watcher on `path`. Change events —
/// minus ignored dirs — are forwarded over `on_event`. The frontend
/// debounces. Replacing the watcher drops the previous one.
#[tauri::command]
pub fn agent_fs_watch_start(
    state: State<'_, FsWatchState>,
    path: String,
    on_event: Channel<FsChange>,
) -> Result<(), String> {
    use notify::{Event, RecursiveMode, Watcher};

    let root = path.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(ev) = res {
            let paths: Vec<String> = ev
                .paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .filter(|p| !rel_is_ignored(&root, p))
                .collect();
            if !paths.is_empty() {
                let _ = on_event.send(FsChange { paths });
            }
        }
    })
    .map_err(|e| e.to_string())?;

    watcher
        .watch(Path::new(&path), RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;

    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    *guard = Some(watcher); // dropping the previous watcher (if any) stops it
    Ok(())
}

#[tauri::command]
pub fn agent_fs_watch_stop(state: State<'_, FsWatchState>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    *guard = None;
    Ok(())
}

/// Provider-native in-prompt file reference for drag-to-context. The
/// per-CLI syntax lives behind `CliRunner::file_reference`; this command
/// is the only call site, so no provider strings leak into the frontend.
#[tauri::command]
pub fn agent_file_reference(provider: String, rel_path: String) -> String {
    runner_for(&provider).file_reference(&rel_path)
}

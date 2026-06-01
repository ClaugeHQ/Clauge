// Canvas writes intentionally do NOT call `crate::cloud::scheduler::bump(...)`.
// Canvas state (tile positions, viewport) is per-device by v1 design — see
// `Ctx/Clauge/superpowers/specs/2026-06-01-canvas-mode-design.md` §1
// (out-of-scope: Cloud sync of Canvas layouts).

use sqlx::sqlite::SqlitePool;
use tauri::State;

use super::db;
use super::models::{CanvasTile, CanvasViewport, TabRef, TileGeometryUpdate};

/// Reconcile canvas_tiles against the frontend-provided open-tab list.
#[tauri::command]
pub async fn canvas_resolve_tiles(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    open_tab_refs: Vec<TabRef>,
) -> Result<Vec<CanvasTile>, String> {
    db::resolve_tiles(pool.inner(), &workspace_id, &open_tab_refs)
        .await
        .map_err(|e| e.to_string())
}

/// Read all tiles for a workspace, ordered by z.
#[tauri::command]
pub async fn canvas_list_tiles(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
) -> Result<Vec<CanvasTile>, String> {
    db::list_tiles(pool.inner(), &workspace_id)
        .await
        .map_err(|e| e.to_string())
}

/// Batched geometry write from the frontend's drag/resize debouncer.
#[tauri::command]
pub async fn canvas_upsert_tiles_batch(
    pool: State<'_, SqlitePool>,
    updates: Vec<TileGeometryUpdate>,
) -> Result<(), String> {
    db::upsert_tiles_batch(pool.inner(), &updates)
        .await
        .map_err(|e| e.to_string())
}

/// Toggle a tile's minimized flag.
#[tauri::command]
pub async fn canvas_minimize_tile(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    tab_id: String,
    minimized: bool,
) -> Result<(), String> {
    db::set_minimized(pool.inner(), &workspace_id, &tab_id, minimized)
        .await
        .map_err(|e| e.to_string())
}

/// Hard-delete a single tile (source-mode tab closed).
#[tauri::command]
pub async fn canvas_delete_tile(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    tab_id: String,
) -> Result<(), String> {
    db::delete_tile(pool.inner(), &workspace_id, &tab_id)
        .await
        .map_err(|e| e.to_string())
}

/// Read the workspace's viewport (defaults if unset; not persisted).
#[tauri::command]
pub async fn canvas_get_viewport(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
) -> Result<CanvasViewport, String> {
    db::get_viewport(pool.inner(), &workspace_id)
        .await
        .map_err(|e| e.to_string())
}

/// Upsert the workspace's viewport.
#[tauri::command]
pub async fn canvas_set_viewport(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    offset_x: f64,
    offset_y: f64,
    zoom: f64,
    last_focused_tab: Option<String>,
) -> Result<(), String> {
    db::set_viewport(
        pool.inner(),
        &workspace_id,
        offset_x,
        offset_y,
        zoom,
        last_focused_tab.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())
}

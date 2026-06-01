import { invoke } from '@tauri-apps/api/core';

/** Must match the tab_kind CHECK constraint in src-tauri/migrations/19_canvas_tables.sql and 20_canvas_shell_terminal.sql. */
export type TabKind =
  | 'agent_terminal'
  | 'ssh_terminal'
  | 'sql_editor'
  | 'rest_request'
  | 'mongo_query'
  | 'redis_query'
  | 'workspace_note'
  | 'workspace_board'
  | 'shell_terminal';

export interface CanvasTile {
  workspaceId: string;
  tabId: string;
  tabKind: TabKind;
  x: number;
  y: number;
  width: number;
  height: number;
  zOrder: number;
  /** SQLite INTEGER on wire; 0 = visible, 1 = minimized. */
  minimized: number;
  createdAt: string;
  updatedAt: string;
}

export interface CanvasViewport {
  workspaceId: string;
  offsetX: number;
  offsetY: number;
  zoom: number;
  lastFocusedTab: string | null;
  updatedAt: string;
}

export interface TabRef {
  tabKind: TabKind;
  tabId: string;
}

export interface TileGeometryUpdate {
  workspaceId: string;
  tabId: string;
  x: number;
  y: number;
  width: number;
  height: number;
  zOrder: number;
}

export const canvasResolveTiles = (workspaceId: string, openTabRefs: TabRef[]) =>
  invoke<CanvasTile[]>('canvas_resolve_tiles', { workspaceId, openTabRefs });

export const canvasListTiles = (workspaceId: string) =>
  invoke<CanvasTile[]>('canvas_list_tiles', { workspaceId });

export const canvasUpsertTilesBatch = (updates: TileGeometryUpdate[]) =>
  invoke<void>('canvas_upsert_tiles_batch', { updates });

export const canvasMinimizeTile = (workspaceId: string, tabId: string, minimized: boolean) =>
  invoke<void>('canvas_minimize_tile', { workspaceId, tabId, minimized });

export const canvasDeleteTile = (workspaceId: string, tabId: string) =>
  invoke<void>('canvas_delete_tile', { workspaceId, tabId });

export const canvasGetViewport = (workspaceId: string) =>
  invoke<CanvasViewport>('canvas_get_viewport', { workspaceId });

export const canvasSetViewport = (
  workspaceId: string,
  offsetX: number,
  offsetY: number,
  zoom: number,
  lastFocusedTab: string | null,
) =>
  invoke<void>('canvas_set_viewport', { workspaceId, offsetX, offsetY, zoom, lastFocusedTab });

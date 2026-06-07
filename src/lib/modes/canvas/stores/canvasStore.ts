import { writable, derived, get } from 'svelte/store';
import type { CanvasRegion, CanvasTile, TabKind, TabRef } from '$lib/modes/canvas/commands';
import type { AppMode } from '$lib/stores/app';
import {
  canvasListRegions,
  canvasResolveTiles,
  canvasSetViewport,
  canvasUpsertRegion,
  canvasUpsertTilesBatch,
} from '$lib/modes/canvas/commands';

export interface ViewportState {
  offsetX: number;
  offsetY: number;
  zoom: number;
}

export type InteractionState = 'idle' | 'panning' | 'zooming' | 'dragging' | 'resizing';

export const ZOOM_MIN = 0.25;
export const ZOOM_MAX = 3.0;
export const ZOOM_DEFAULT = 1.0;
export const VIEWPORT_FLUSH_DEBOUNCE_MS = 1000;
export const TILE_FLUSH_DEBOUNCE_MS = 500;

// Tile map keyed by tabId. Callers MUST treat the Map as immutable:
// `tilesByTab.update(m => { const next = new Map(m); next.set(id, tile); return next; })`.
// Mutating in place skips subscriber notifications. Phase 3 will introduce
// typed setter helpers; until then, follow the copy-on-mutate pattern.
export const tilesByTab = writable<Map<string, CanvasTile>>(new Map());
export const regionsById = writable<Map<string, CanvasRegion>>(new Map());
export const viewport = writable<ViewportState>({ offsetX: 0, offsetY: 0, zoom: ZOOM_DEFAULT });
export const interactionState = writable<InteractionState>('idle');
export const focusedTabId = writable<string | null>(null);

export interface RegionDraftPreview {
  x: number;
  y: number;
  width: number;
  height: number;
  color: string;
}
/** Live rectangle shown while the user is Shift+dragging to create a region. */
export const regionPreview = writable<RegionDraftPreview | null>(null);

/**
 * Signal to the CanvasRegion for `regionId` to enter rename mode as
 * soon as it mounts. Cleared by the region once consumed. Set by the
 * region creation paths (toolbar button, Shift+drag) so the user can
 * type the project name immediately without hunting for the rename
 * affordance.
 */
export const pendingRenameRegionId = writable<string | null>(null);

// Debounced viewport flush — single timer.
let viewportFlushTimer: ReturnType<typeof setTimeout> | null = null;
// Singleton: only one canvas workspace is active at a time (Clauge is single-window).
let currentWorkspaceId: string | null = null;

export function setActiveWorkspace(workspaceId: string | null): void {
  currentWorkspaceId = workspaceId;
}

export function flushViewportSoon(): void {
  if (!currentWorkspaceId) return;
  if (viewportFlushTimer) clearTimeout(viewportFlushTimer);
  viewportFlushTimer = setTimeout(() => {
    viewportFlushTimer = null;
    void flushViewportNow();
  }, VIEWPORT_FLUSH_DEBOUNCE_MS);
}

export async function flushViewportNow(): Promise<void> {
  if (!currentWorkspaceId) return;
  if (viewportFlushTimer) {
    clearTimeout(viewportFlushTimer);
    viewportFlushTimer = null;
  }
  const v = get(viewport);
  const focused = get(focusedTabId);
  await canvasSetViewport(currentWorkspaceId, v.offsetX, v.offsetY, v.zoom, focused);
}

// Replace store contents from backend. Phase 2 passes an empty open-tab list
// so backend creates no tiles. Phase 3 wires in real adapters.
export async function loadCanvas(workspaceId: string, openTabRefs: TabRef[]): Promise<void> {
  setActiveWorkspace(workspaceId);
  const [tiles, regions] = await Promise.all([
    canvasResolveTiles(workspaceId, openTabRefs),
    canvasListRegions(workspaceId),
  ]);
  const tileMap = new Map<string, CanvasTile>();
  for (const t of tiles) tileMap.set(t.tabId, t);
  tilesByTab.set(tileMap);
  const regionMap = new Map<string, CanvasRegion>();
  for (const r of regions) regionMap.set(r.regionId, r);
  regionsById.set(regionMap);
}

// Dirty-tile tracking for Phase 3's geometry flush.
const dirtyTiles = new Set<string>();
let tileFlushTimer: ReturnType<typeof setTimeout> | null = null;

export function markTileDirty(tabId: string): void {
  dirtyTiles.add(tabId);
  if (tileFlushTimer) clearTimeout(tileFlushTimer);
  tileFlushTimer = setTimeout(() => {
    tileFlushTimer = null;
    void flushDirtyTilesNow();
  }, TILE_FLUSH_DEBOUNCE_MS);
}

export async function flushDirtyTilesNow(): Promise<void> {
  if (!currentWorkspaceId || dirtyTiles.size === 0) return;
  if (tileFlushTimer) {
    clearTimeout(tileFlushTimer);
    tileFlushTimer = null;
  }
  const map = get(tilesByTab);
  const snapshot = [...dirtyTiles];
  const updates = snapshot
    .map((id) => map.get(id))
    .filter((t): t is CanvasTile => !!t)
    .map((t) => ({
      workspaceId: t.workspaceId,
      tabId: t.tabId,
      x: t.x,
      y: t.y,
      width: t.width,
      height: t.height,
      zOrder: t.zOrder,
      regionId: t.regionId,
    }));
  dirtyTiles.clear();
  if (updates.length === 0) return;
  try {
    await canvasUpsertTilesBatch(updates);
  } catch (err) {
    // Re-queue snapshot ids so the next flush retries them.
    for (const id of snapshot) dirtyTiles.add(id);
    throw err;
  }
}

// Z-ordered list for rendering — Phase 3 consumes this.
export const tilesSortedByZ = derived(tilesByTab, ($map) =>
  [...$map.values()].sort((a, b) => a.zOrder - b.zOrder),
);

// Regions render behind tiles; sort by zOrder so explicit ordering still works
// if we add it later. Today every region has zOrder 0 so the sort is stable.
export const regionsSortedByZ = derived(regionsById, ($map) =>
  [...$map.values()].sort((a, b) => a.zOrder - b.zOrder),
);

// region_id -> [tile, ...]. Used by region drag to translate children
// in lockstep without a per-frame scan over all tiles.
export const tilesByRegion = derived(tilesByTab, ($map) => {
  const out = new Map<string, CanvasTile[]>();
  for (const t of $map.values()) {
    if (!t.regionId) continue;
    const arr = out.get(t.regionId);
    if (arr) arr.push(t);
    else out.set(t.regionId, [t]);
  }
  return out;
});

const dirtyRegions = new Set<string>();
let regionFlushTimer: ReturnType<typeof setTimeout> | null = null;

export function markRegionDirty(regionId: string): void {
  dirtyRegions.add(regionId);
  if (regionFlushTimer) clearTimeout(regionFlushTimer);
  regionFlushTimer = setTimeout(() => {
    regionFlushTimer = null;
    void flushDirtyRegionsNow();
  }, TILE_FLUSH_DEBOUNCE_MS);
}

export async function flushDirtyRegionsNow(): Promise<void> {
  if (!currentWorkspaceId || dirtyRegions.size === 0) return;
  if (regionFlushTimer) {
    clearTimeout(regionFlushTimer);
    regionFlushTimer = null;
  }
  const map = get(regionsById);
  const snapshot = [...dirtyRegions];
  dirtyRegions.clear();
  const targets = snapshot
    .map((id) => map.get(id))
    .filter((r): r is CanvasRegion => !!r);
  if (targets.length === 0) return;
  try {
    await Promise.all(targets.map((r) => canvasUpsertRegion(r)));
  } catch (err) {
    for (const id of snapshot) dirtyRegions.add(id);
    throw err;
  }
}

/**
 * Maps a focused tile's TabKind back to its source mode (the value of $mode
 * when the user navigates to that tab's home). Used by future AI panel /
 * hotkey routing on canvas; safe to consume now even though plumbing is
 * still hidden behind v2 polish work.
 */
const TAB_KIND_TO_MODE: Partial<Record<TabKind, AppMode>> = {
  agent_terminal: 'agent',
  ssh_terminal: 'ssh',
  shell_terminal: 'canvas',
  sql_editor: 'sql',
  rest_request: 'rest',
  mongo_query: 'nosql',
  redis_query: 'nosql',
  explorer_file_browser: 'explorer',
  workspace_note: 'workspace',
  workspace_board: 'workspace',
};

export const focusedTileMode = derived(
  [focusedTabId, tilesByTab],
  ([$id, $map]): AppMode | null => {
    if (!$id) return null;
    const tile = $map.get($id);
    if (!tile) return null;
    return TAB_KIND_TO_MODE[tile.tabKind as TabKind] ?? null;
  },
);

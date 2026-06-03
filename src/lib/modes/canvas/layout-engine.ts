import type { CanvasTile } from '$lib/modes/canvas/commands';
import { ZOOM_MIN, ZOOM_MAX } from '$lib/modes/canvas/stores/canvasStore';

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Compute a fit-all viewport that frames every tile with a small margin.
 * Returns null if there are no tiles.
 */
export function fitAllViewport(
  tiles: CanvasTile[],
  containerSize: { width: number; height: number },
  marginPx = 80,
): { offsetX: number; offsetY: number; zoom: number } | null {
  if (tiles.length === 0) return null;

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const t of tiles) {
    if (t.x < minX) minX = t.x;
    if (t.y < minY) minY = t.y;
    if (t.x + t.width > maxX) maxX = t.x + t.width;
    if (t.y + t.height > maxY) maxY = t.y + t.height;
  }

  const tilesWidth = maxX - minX;
  const tilesHeight = maxY - minY;
  const availW = Math.max(1, containerSize.width - 2 * marginPx);
  const availH = Math.max(1, containerSize.height - 2 * marginPx);
  const zoomX = availW / Math.max(1, tilesWidth);
  const zoomY = availH / Math.max(1, tilesHeight);
  // Clamp to the same bounds the zoomable action enforces, never exceed 1.5x.
  const zoom = Math.max(ZOOM_MIN, Math.min(zoomX, zoomY, 1.5, ZOOM_MAX));

  // Centre the bounding box in the container.
  const offsetX = (containerSize.width - tilesWidth * zoom) / 2 - minX * zoom;
  const offsetY = (containerSize.height - tilesHeight * zoom) / 2 - minY * zoom;

  return { offsetX, offsetY, zoom };
}

/**
 * Compute a cascade-positioned rect for a newly-spawned tile based on how many
 * tiles already exist. Wraps every 5 tiles.
 */
export function nextCascadeRect(
  existing: CanvasTile[],
  defaultSize = { width: 720, height: 480 },
): Rect {
  const wrap = 5;
  const step = 40;
  const base = 80;
  const i = existing.length;
  return {
    x: base + (i % wrap) * step,
    y: base + (i % wrap) * step,
    width: defaultSize.width,
    height: defaultSize.height,
  };
}

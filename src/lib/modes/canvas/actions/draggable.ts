import { get } from 'svelte/store';
import {
  tilesByTab,
  interactionState,
  markTileDirty,
  viewport,
} from '$lib/modes/canvas/stores/canvasStore';
import { snapGuides, type GuideLine } from '$lib/modes/canvas/stores/snapGuidesStore';

interface DraggableParams {
  tabId: string;
}

const SNAP_THRESHOLD_PX = 8;

export function draggable(node: HTMLElement, params: DraggableParams) {
  let tabId = params.tabId;
  let dragging = false;
  let pointerId: number | null = null;
  let startCanvasX = 0;
  let startCanvasY = 0;
  let startTileX = 0;
  let startTileY = 0;

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return; // only left-button drags tiles
    // Don't start a drag if the user clicked a button (e.g., the tile's
    // close X). Buttons inside the title bar own their own click events.
    const target = e.target as HTMLElement | null;
    if (target?.closest('button')) return;
    // Yield to svelte-dnd-action draggable items (board cards, etc.) so
    // card drags inside a tile body don't fight the tile drag.
    if (target?.closest('[draggable="true"]')) return;
    const tile = get(tilesByTab).get(tabId);
    if (!tile) return;
    e.stopPropagation();
    e.preventDefault();
    dragging = true;
    pointerId = e.pointerId;
    node.setPointerCapture(e.pointerId);
    const v = get(viewport);
    startCanvasX = (e.clientX - v.offsetX) / v.zoom;
    startCanvasY = (e.clientY - v.offsetY) / v.zoom;
    startTileX = tile.x;
    startTileY = tile.y;
    interactionState.set('dragging');
    document.body.classList.add('cv-interacting');
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging || e.pointerId !== pointerId) return;
    const v = get(viewport);
    const tiles = get(tilesByTab);
    const tile = tiles.get(tabId);
    const curCanvasX = (e.clientX - v.offsetX) / v.zoom;
    const curCanvasY = (e.clientY - v.offsetY) / v.zoom;
    let nextX = startTileX + (curCanvasX - startCanvasX);
    let nextY = startTileY + (curCanvasY - startCanvasY);

    if (tile) {
      const w = tile.width;
      const h = tile.height;

      // Collect every candidate from every other tile, using the unshifted
      // nextX/nextY so multi-neighbor scenarios don't double-snap.
      let bestX: { delta: number; target: number } | null = null;
      let bestY: { delta: number; target: number } | null = null;

      for (const other of tiles.values()) {
        if (other.tabId === tabId) continue;
        const xs = [
          { tile: nextX, target: other.x },
          { tile: nextX + w, target: other.x + other.width },
          { tile: nextX + w / 2, target: other.x + other.width / 2 },
        ];
        const ys = [
          { tile: nextY, target: other.y },
          { tile: nextY + h, target: other.y + other.height },
          { tile: nextY + h / 2, target: other.y + other.height / 2 },
        ];
        for (const c of xs) {
          const d = c.target - c.tile;
          if (Math.abs(d) <= SNAP_THRESHOLD_PX && (bestX === null || Math.abs(d) < Math.abs(bestX.delta))) {
            bestX = { delta: d, target: c.target };
          }
        }
        for (const c of ys) {
          const d = c.target - c.tile;
          if (Math.abs(d) <= SNAP_THRESHOLD_PX && (bestY === null || Math.abs(d) < Math.abs(bestY.delta))) {
            bestY = { delta: d, target: c.target };
          }
        }
      }

      const guides: GuideLine[] = [];
      if (bestX) {
        nextX += bestX.delta;
        guides.push({ axis: 'x', position: bestX.target });
      }
      if (bestY) {
        nextY += bestY.delta;
        guides.push({ axis: 'y', position: bestY.target });
      }
      snapGuides.set(guides);
    }

    tilesByTab.update((m) => {
      const next = new Map(m);
      const cur = next.get(tabId);
      if (cur) next.set(tabId, { ...cur, x: nextX, y: nextY });
      return next;
    });
  }

  function endDrag(e: PointerEvent) {
    if (!dragging || e.pointerId !== pointerId) return;
    dragging = false;
    pointerId = null;
    try {
      node.releasePointerCapture(e.pointerId);
    } catch {
      // Capture may already be released.
    }
    interactionState.set('idle');
    document.body.classList.remove('cv-interacting');
    snapGuides.set([]);
    markTileDirty(tabId);
  }

  node.addEventListener('pointerdown', onPointerDown);
  node.addEventListener('pointermove', onPointerMove);
  node.addEventListener('pointerup', endDrag);
  node.addEventListener('pointercancel', endDrag);

  return {
    update(p: DraggableParams) {
      tabId = p.tabId;
    },
    destroy() {
      node.removeEventListener('pointerdown', onPointerDown);
      node.removeEventListener('pointermove', onPointerMove);
      node.removeEventListener('pointerup', endDrag);
      node.removeEventListener('pointercancel', endDrag);
      if (dragging) {
        snapGuides.set([]);
        document.body.classList.remove('cv-interacting');
      }
    },
  };
}

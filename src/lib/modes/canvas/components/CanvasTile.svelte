<script lang="ts">
  import type { CanvasTile as Tile } from '$lib/modes/canvas/commands';
  import { tilesByTab, focusedTabId, markTileDirty } from '$lib/modes/canvas/stores/canvasStore';
  import CanvasTileTitleBar from './CanvasTileTitleBar.svelte';
  import CanvasTileBody from './CanvasTileBody.svelte';
  import CanvasResizeHandles from './CanvasResizeHandles.svelte';

  let { tile }: { tile: Tile } = $props();

  const isFocused = $derived($focusedTabId === tile.tabId);

  function bringToFront() {
    let maxZ = tile.zOrder;
    for (const t of $tilesByTab.values()) {
      if (t.zOrder > maxZ) maxZ = t.zOrder;
    }
    if (tile.zOrder < maxZ) {
      tilesByTab.update((m) => {
        const next = new Map(m);
        const cur = next.get(tile.tabId);
        if (cur) next.set(tile.tabId, { ...cur, zOrder: maxZ + 1 });
        return next;
      });
      markTileDirty(tile.tabId);
    }
    focusedTabId.set(tile.tabId);
  }
</script>

<div
  class="cv-tile"
  class:focused={isFocused}
  style="left: {tile.x}px; top: {tile.y}px; width: {tile.width}px; height: {tile.height}px; z-index: {tile.zOrder};"
  onpointerdown={bringToFront}
>
  <CanvasTileTitleBar {tile} />
  <CanvasTileBody {tile} />
  <CanvasResizeHandles {tile} />
</div>

<style>
  .cv-tile {
    position: absolute;
    background: var(--cv-tile-bg, var(--c));
    border: 1px solid var(--cv-tile-border, var(--b1));
    border-radius: 8px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    pointer-events: auto;
    contain: strict;
  }
  .cv-tile.focused {
    border-color: var(--cv-tile-focus, var(--acc));
    box-shadow: 0 4px 16px color-mix(in srgb, var(--acc) 25%, transparent), 0 4px 12px rgba(0, 0, 0, 0.5);
  }
</style>

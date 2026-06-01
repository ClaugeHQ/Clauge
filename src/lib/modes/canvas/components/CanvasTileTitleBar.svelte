<script lang="ts">
  import type { CanvasTile as Tile } from '$lib/modes/canvas/commands';
  import { canvasAdapterRegistry } from '$lib/modes/canvas/adapter-registry';
  import { draggable } from '$lib/modes/canvas/actions/draggable';

  let { tile }: { tile: Tile } = $props();

  const adapter = $derived(canvasAdapterRegistry.get(tile.tabKind));
  const meta = $derived(adapter?.getMeta(tile.tabId) ?? { title: tile.tabId });

  function onDoubleClick() {
    adapter?.openInHomeMode(tile.tabId);
  }
</script>

<div
  class="cv-tile-titlebar"
  use:draggable={{ tabId: tile.tabId }}
  ondblclick={onDoubleClick}
>
  {#if meta.statusDot}
    <span class="cv-tile-dot" style="background: {meta.statusDot};"></span>
  {/if}
  <span class="cv-tile-title">{meta.title}{meta.dirty ? ' •' : ''}</span>
</div>

<style>
  .cv-tile-titlebar {
    height: 26px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    background: var(--cv-tile-titlebar-bg, var(--surface-card));
    border-bottom: 1px solid var(--cv-tile-border, var(--b1));
    cursor: grab;
    user-select: none;
    font-size: 12px;
    color: var(--cv-tile-title-fg, var(--t1));
  }
  .cv-tile-titlebar:active {
    cursor: grabbing;
  }
  .cv-tile-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .cv-tile-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>

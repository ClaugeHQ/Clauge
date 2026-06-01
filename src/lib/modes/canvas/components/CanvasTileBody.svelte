<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { CanvasTile as Tile } from '$lib/modes/canvas/commands';
  import { canvasAdapterRegistry } from '$lib/modes/canvas/adapter-registry';

  let { tile }: { tile: Tile } = $props();

  let slotEl: HTMLDivElement | undefined = $state();
  let destroyFn: (() => void) | null = null;

  // Hardcoded Phase 2 workspace stub — Phase 5 wires real workspace.
  const WORKSPACE_ID = '__phase2_stub__';

  onMount(() => {
    const adapter = canvasAdapterRegistry.get(tile.tabKind);
    if (!adapter || !slotEl) return;

    if (adapter.mountStrategy === 'reparent') {
      adapter.attach?.(tile.tabId, slotEl);
    } else if (adapter.mountStrategy === 'remount') {
      const r = adapter.render?.(tile.tabId, slotEl, { workspaceId: WORKSPACE_ID });
      destroyFn = r?.destroy ?? null;
    }
  });

  onDestroy(() => {
    const adapter = canvasAdapterRegistry.get(tile.tabKind);
    if (adapter?.mountStrategy === 'reparent' && slotEl) {
      adapter.detach?.(tile.tabId, slotEl);
    }
    destroyFn?.();
  });
</script>

<div bind:this={slotEl} class="cv-tile-body"></div>

<style>
  .cv-tile-body {
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
    background: var(--cv-tile-body-bg, transparent);
    color: var(--cv-tile-body-fg, var(--t1));
  }
</style>

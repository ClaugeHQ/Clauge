<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import type { CanvasTile as Tile } from '$lib/modes/canvas/commands';
  import { canvasAdapterRegistry } from '$lib/modes/canvas/adapter-registry';
  import { agentTerminalMap } from '$lib/modes/agent/stores';
  import { sshTerminalMap } from '$lib/modes/ssh/stores';
  import { shellTerminals } from '$lib/modes/canvas/stores/shellTerminalsStore';
  import { mode } from '$lib/stores/app';

  let { tile }: { tile: Tile } = $props();

  let slotEl: HTMLDivElement | undefined = $state();
  let destroyFn: (() => void) | null = null;

  // Hardcoded Phase 2 workspace stub — Phase 5 wires real workspace.
  const WORKSPACE_ID = '__phase2_stub__';

  let resizeObserver: ResizeObserver | null = null;
  let fitTimer: ReturnType<typeof setTimeout> | null = null;

  onMount(() => {
    const adapter = canvasAdapterRegistry.get(tile.tabKind);
    if (!adapter || !slotEl) return;

    if (adapter.mountStrategy === 'reparent') {
      adapter.attach?.(tile.tabId, slotEl);
    } else if (adapter.mountStrategy === 'remount') {
      const r = adapter.render?.(tile.tabId, slotEl, { workspaceId: WORKSPACE_ID });
      destroyFn = r?.destroy ?? null;
    }

    if (slotEl) {
      resizeObserver = new ResizeObserver(() => {
        if (fitTimer !== null) clearTimeout(fitTimer);
        fitTimer = setTimeout(() => {
          fitTimer = null;
          const kind = tile.tabKind;
          let fitAddon: { fit: () => void } | undefined;
          if (kind === 'agent_terminal') {
            fitAddon = get(agentTerminalMap).get(tile.tabId)?.fitAddon;
          } else if (kind === 'ssh_terminal') {
            fitAddon = get(sshTerminalMap).get(tile.tabId)?.fitAddon;
          } else if (kind === 'shell_terminal') {
            fitAddon = get(shellTerminals).get(tile.tabId)?.internal?.fitAddon;
          }
          try {
            fitAddon?.fit();
          } catch {
            // Slot may not have measurable dimensions yet; next observer tick will fit.
          }
        }, 60);
      });
      resizeObserver.observe(slotEl);
    }
  });

  // Reparent kinds: re-claim the live DOM each time canvas becomes the
  // active mode. Home-mode panels (SqlPanel, NoteView, etc.) stay
  // mounted across mode switches and may have stolen the container back
  // while canvas was hidden.
  $effect(() => {
    if ($mode !== 'canvas') return;
    if (!slotEl) return;
    const adapter = canvasAdapterRegistry.get(tile.tabKind);
    if (adapter?.mountStrategy === 'reparent') {
      adapter.attach?.(tile.tabId, slotEl);
    }
  });

  onDestroy(() => {
    if (resizeObserver) {
      resizeObserver.disconnect();
      resizeObserver = null;
    }
    if (fitTimer !== null) {
      clearTimeout(fitTimer);
      fitTimer = null;
    }
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

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    loadCanvas,
    flushViewportNow,
    flushDirtyTilesNow,
    setActiveWorkspace,
    viewport,
  } from '$lib/modes/canvas/stores/canvasStore';
  import { canvasGetViewport } from '$lib/modes/canvas/commands';
  import { canvasAdapterRegistry } from '$lib/modes/canvas/adapter-registry';
  import { agentTerminalAdapter } from '$lib/modes/agent/canvas-adapter';
  import { sshTerminalAdapter } from '$lib/modes/ssh/canvas-adapter';
  import { shellTerminalAdapter } from '$lib/modes/canvas/adapters/shellTerminalAdapter';
  import { loadCanvasSettings } from '$lib/modes/canvas/stores/canvasSettingsStore';
  import CanvasViewport from './CanvasViewport.svelte';

  // Phase 2 stub: hardcoded workspace id so the surface mounts. Phase 4
  // wires this to the real active-workspace store.
  const ACTIVE_WORKSPACE_ID = '__phase2_stub__';

  // Clear stale registrations (e.g. HMR) before registering real adapters.
  canvasAdapterRegistry.clear();
  canvasAdapterRegistry.register(agentTerminalAdapter);
  canvasAdapterRegistry.register(sshTerminalAdapter);
  canvasAdapterRegistry.register(shellTerminalAdapter);

  onMount(async () => {
    await loadCanvasSettings();
    setActiveWorkspace(ACTIVE_WORKSPACE_ID);
    const v = await canvasGetViewport(ACTIVE_WORKSPACE_ID);
    viewport.set({ offsetX: v.offsetX, offsetY: v.offsetY, zoom: v.zoom });

    // Union open tabs from all registered adapters.
    const agentTabs = agentTerminalAdapter
      .listOpenTabs(ACTIVE_WORKSPACE_ID)
      .map((t) => ({ tabKind: 'agent_terminal' as const, tabId: t.id }));
    const sshTabs = sshTerminalAdapter
      .listOpenTabs(ACTIVE_WORKSPACE_ID)
      .map((t) => ({ tabKind: 'ssh_terminal' as const, tabId: t.id }));
    const shellTabs = shellTerminalAdapter
      .listOpenTabs(ACTIVE_WORKSPACE_ID)
      .map((t) => ({ tabKind: 'shell_terminal' as const, tabId: t.id }));
    await loadCanvas(ACTIVE_WORKSPACE_ID, [...agentTabs, ...sshTabs, ...shellTabs]);
  });

  onDestroy(() => {
    // Svelte does not await async onDestroy callbacks; fire-and-forget.
    void flushViewportNow();
    void flushDirtyTilesNow();
  });
</script>

<div class="cv-panel">
  <CanvasViewport />
</div>

<style>
  .cv-panel {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    position: relative;
    overflow: hidden;
  }
</style>

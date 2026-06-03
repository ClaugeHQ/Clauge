<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
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
  import {
    shellTerminalAdapter,
    performShellClose,
  } from '$lib/modes/canvas/adapters/shellTerminalAdapter';
  import { canvasEnabled } from '$lib/modes/canvas/stores/canvasEnabled';
  import CanvasViewport from './CanvasViewport.svelte';
  import CanvasIntro from './CanvasIntro.svelte';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';

  // Phase 2 stub: hardcoded workspace id so the surface mounts. Phase 4
  // wires this to the real active-workspace store.
  const ACTIVE_WORKSPACE_ID = '__phase2_stub__';

  let resolveTimer: ReturnType<typeof setTimeout> | null = null;
  let unsubscribes: Array<() => void> = [];
  let adapterUnsubscribes: Array<() => void> = [];
  let initialized = false;

  let showShellCloseConfirm = $state(false);
  let shellCloseTargetId = $state<string | null>(null);

  async function confirmShellClose() {
    if (shellCloseTargetId !== null) {
      await performShellClose(shellCloseTargetId);
    }
    shellCloseTargetId = null;
    showShellCloseConfirm = false;
  }

  function cancelShellClose() {
    shellCloseTargetId = null;
    showShellCloseConfirm = false;
  }

  async function resolveTilesNow() {
    if (resolveTimer) {
      clearTimeout(resolveTimer);
      resolveTimer = null;
    }
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
  }

  function scheduleResolve() {
    if (resolveTimer) clearTimeout(resolveTimer);
    resolveTimer = setTimeout(() => {
      resolveTimer = null;
      void resolveTilesNow();
    }, 150);
  }

  async function initCanvas() {
    if (initialized) return;
    initialized = true;

    // Clear stale registrations (e.g. HMR) before registering real adapters.
    canvasAdapterRegistry.clear();
    canvasAdapterRegistry.register(agentTerminalAdapter);
    canvasAdapterRegistry.register(sshTerminalAdapter);
    canvasAdapterRegistry.register(shellTerminalAdapter);

    setActiveWorkspace(ACTIVE_WORKSPACE_ID);
    const v = await canvasGetViewport(ACTIVE_WORKSPACE_ID);
    viewport.set({ offsetX: v.offsetX, offsetY: v.offsetY, zoom: v.zoom });

    // Initial resolve.
    await resolveTilesNow();

    // Subscribe each adapter so newly-opened or newly-closed tabs trigger
    // a debounced re-resolve.
    adapterUnsubscribes.push(
      agentTerminalAdapter.subscribe(ACTIVE_WORKSPACE_ID, scheduleResolve),
      sshTerminalAdapter.subscribe(ACTIVE_WORKSPACE_ID, scheduleResolve),
      shellTerminalAdapter.subscribe(ACTIVE_WORKSPACE_ID, scheduleResolve),
    );
  }

  function teardownCanvas() {
    if (resolveTimer) {
      clearTimeout(resolveTimer);
      resolveTimer = null;
    }
    for (const u of adapterUnsubscribes) u();
    adapterUnsubscribes = [];
    canvasAdapterRegistry.clear();
    initialized = false;
    void flushViewportNow();
    void flushDirtyTilesNow();
  }

  onMount(() => {
    // Subscribe to canvasEnabled so we react to settings changes while
    // this panel is mounted (e.g., user enables Canvas from Settings while
    // already on the Canvas sidebar tab).
    const unsub = canvasEnabled.subscribe((enabled) => {
      if (enabled) {
        void initCanvas();
      } else {
        teardownCanvas();
      }
    });
    unsubscribes.push(unsub);

    const onShellCloseRequest = (e: Event) => {
      const tabId = (e as CustomEvent<{ tabId: string }>).detail?.tabId;
      if (typeof tabId !== 'string') return;
      shellCloseTargetId = tabId;
      showShellCloseConfirm = true;
    };
    window.addEventListener('canvas:request-shell-close', onShellCloseRequest);

    return () => {
      window.removeEventListener('canvas:request-shell-close', onShellCloseRequest);
    };
  });

  onDestroy(() => {
    for (const u of unsubscribes) u();
    unsubscribes = [];
    teardownCanvas();
  });
</script>

<div class="cv-panel">
  {#if $canvasEnabled}
    <CanvasViewport />
  {:else}
    <CanvasIntro />
  {/if}
</div>

<ConfirmDialog
  bind:show={showShellCloseConfirm}
  title="Close terminal?"
  message="The session will be terminated."
  confirmText="Close"
  onconfirm={confirmShellClose}
  oncancel={cancelShellClose}
/>

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

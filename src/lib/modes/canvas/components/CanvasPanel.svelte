<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import {
    loadCanvas,
    flushViewportNow,
    flushDirtyTilesNow,
    flushDirtyRegionsNow,
    setActiveWorkspace,
    viewport,
  } from '$lib/modes/canvas/stores/canvasStore';
  import { canvasGetViewport, type TabRef } from '$lib/modes/canvas/commands';
  import { canvasAdapterRegistry } from '$lib/modes/canvas/adapter-registry';
  import { agentTerminalAdapter } from '$lib/modes/agent/canvas-adapter';
  import { sshTerminalAdapter } from '$lib/modes/ssh/canvas-adapter';
  import {
    shellTerminalAdapter,
    performShellClose,
  } from '$lib/modes/canvas/adapters/shellTerminalAdapter';
  import { restRequestAdapter } from '$lib/modes/rest/canvas-adapter';
  import { explorerFileBrowserAdapter } from '$lib/modes/explorer/canvas-adapter';
  import {
    mongoQueryAdapter,
    redisQueryAdapter,
  } from '$lib/modes/nosql/canvas-adapter';
  import { sqlEditorAdapter } from '$lib/modes/sql/canvas-adapter';
  import { canvasEnabled } from '$lib/modes/canvas/stores/canvasEnabled';
  import { startRenderScaleSync } from '$lib/modes/canvas/services/xtermRenderScale';
  import { mode } from '$lib/stores/app';
  import CanvasViewport from './CanvasViewport.svelte';
  import CanvasIntro from './CanvasIntro.svelte';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';

  // Phase 2 stub: hardcoded workspace id so the surface mounts. Phase 4
  // wires this to the real active-workspace store.
  const ACTIVE_WORKSPACE_ID = '__phase2_stub__';

  let resolveTimer: ReturnType<typeof setTimeout> | null = null;
  let unsubscribes: Array<() => void> = [];
  let adapterUnsubscribes: Array<() => void> = [];
  let stopRenderScaleSync: (() => void) | null = null;
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
    const refs: TabRef[] = [];
    for (const adapter of canvasAdapterRegistry.list()) {
      const size = adapter.defaultSpawnSize;
      for (const t of adapter.listOpenTabs(ACTIVE_WORKSPACE_ID)) {
        refs.push({
          tabKind: adapter.tabKind,
          tabId: t.id,
          defaultWidth: size?.width,
          defaultHeight: size?.height,
        });
      }
    }
    await loadCanvas(ACTIVE_WORKSPACE_ID, refs);
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
    canvasAdapterRegistry.register(explorerFileBrowserAdapter);
    canvasAdapterRegistry.register(restRequestAdapter);
    canvasAdapterRegistry.register(mongoQueryAdapter);
    canvasAdapterRegistry.register(redisQueryAdapter);
    canvasAdapterRegistry.register(sqlEditorAdapter);

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
      explorerFileBrowserAdapter.subscribe(ACTIVE_WORKSPACE_ID, scheduleResolve),
      restRequestAdapter.subscribe(ACTIVE_WORKSPACE_ID, scheduleResolve),
      mongoQueryAdapter.subscribe(ACTIVE_WORKSPACE_ID, scheduleResolve),
      redisQueryAdapter.subscribe(ACTIVE_WORKSPACE_ID, scheduleResolve),
      sqlEditorAdapter.subscribe(ACTIVE_WORKSPACE_ID, scheduleResolve),
    );
  }

  function teardownCanvas() {
    if (resolveTimer) {
      clearTimeout(resolveTimer);
      resolveTimer = null;
    }
    for (const u of adapterUnsubscribes) u();
    adapterUnsubscribes = [];
    stopRenderScaleSyncIfActive();
    canvasAdapterRegistry.clear();
    initialized = false;
    void flushViewportNow();
    void flushDirtyTilesNow();
    void flushDirtyRegionsNow();
  }

  function startRenderScaleSyncIfNeeded() {
    if (stopRenderScaleSync) return;
    stopRenderScaleSync = startRenderScaleSync();
  }

  function stopRenderScaleSyncIfActive() {
    if (!stopRenderScaleSync) return;
    stopRenderScaleSync();
    stopRenderScaleSync = null;
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

    const unsubMode = mode.subscribe((m) => {
      if (m === 'canvas' && get(canvasEnabled)) {
        startRenderScaleSyncIfNeeded();
      } else {
        stopRenderScaleSyncIfActive();
      }
    });
    unsubscribes.push(unsubMode);

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

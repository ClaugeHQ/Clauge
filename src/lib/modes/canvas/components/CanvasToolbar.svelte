<script lang="ts">
  import { homeDir } from '@tauri-apps/api/path';
  import {
    viewport,
    ZOOM_MIN,
    ZOOM_MAX,
    ZOOM_DEFAULT,
    flushViewportSoon,
  } from '$lib/modes/canvas/stores/canvasStore';
  import { spawnShellTerminal } from '$lib/modes/canvas/services/shellTerminalLifecycle';
  import { createRegion } from '$lib/modes/canvas/services/regionLifecycle';

  const ACTIVE_WORKSPACE_ID = '__phase2_stub__';
  const REGION_DEFAULT_W = 700;
  const REGION_DEFAULT_H = 460;

  async function openTerminal() {
    let cwd: string;
    try {
      cwd = await homeDir();
    } catch {
      cwd = '/';
    }
    try {
      await spawnShellTerminal(ACTIVE_WORKSPACE_ID, cwd);
    } catch (err) {
      console.error('[canvas] failed to spawn shell terminal:', err);
    }
  }

  async function newRegion() {
    // Place at the centre of the visible viewport so the user sees the
    // new region immediately without having to pan to find it.
    const v = $viewport;
    const cx = (window.innerWidth / 2 - v.offsetX) / v.zoom - REGION_DEFAULT_W / 2;
    const cy = (window.innerHeight / 2 - v.offsetY) / v.zoom - REGION_DEFAULT_H / 2;
    await createRegion({
      workspaceId: ACTIVE_WORKSPACE_ID,
      x: cx,
      y: cy,
      width: REGION_DEFAULT_W,
      height: REGION_DEFAULT_H,
    });
  }

  function setZoom(z: number) {
    viewport.update((v) => ({
      ...v,
      zoom: Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, z)),
    }));
    flushViewportSoon();
  }

  function reset() {
    viewport.set({ offsetX: 0, offsetY: 0, zoom: ZOOM_DEFAULT });
    flushViewportSoon();
  }

  const zoomPct = $derived(Math.round($viewport.zoom * 100));
</script>

<div class="cv-toolbar">
  <button
    class="cv-tb-btn cv-tb-term"
    onclick={openTerminal}
    title="Open terminal in home directory"
    aria-label="Open terminal"
  >
    <!-- Inline SVG: terminal prompt icon -->
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <polyline points="4 17 10 11 4 5"></polyline>
      <line x1="12" y1="19" x2="20" y2="19"></line>
    </svg>
  </button>
  <button
    class="cv-tb-btn cv-tb-region"
    onclick={newRegion}
    title="New region — group tiles by project (or Shift+drag on canvas)"
    aria-label="New region"
  >
    <!-- Inline SVG: dashed rect with corner marks -->
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <rect x="4" y="4" width="16" height="16" rx="2" stroke-dasharray="3 3"/>
      <line x1="9" y1="12" x2="15" y2="12"/>
      <line x1="12" y1="9" x2="12" y2="15"/>
    </svg>
  </button>
  <button class="cv-tb-btn" onclick={() => setZoom($viewport.zoom / 1.2)} aria-label="Zoom out">−</button>
  <button class="cv-tb-btn cv-tb-pct" onclick={reset} title="Reset view" aria-label="Reset view">{zoomPct}%</button>
  <button class="cv-tb-btn" onclick={() => setZoom($viewport.zoom * 1.2)} aria-label="Zoom in">+</button>
</div>

<style>
  .cv-toolbar {
    position: absolute;
    bottom: 12px;
    right: 12px;
    display: flex;
    gap: 4px;
    padding: 4px;
    background: var(--c);
    border-radius: 8px;
    backdrop-filter: blur(8px);
  }
  .cv-tb-btn {
    height: 28px;
    min-width: 28px;
    padding: 0 8px;
    background: transparent;
    color: var(--t1);
    border: 0;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .cv-tb-btn:hover {
    background: var(--surface-hover);
  }
  .cv-tb-pct {
    min-width: 56px;
    font-variant-numeric: tabular-nums;
  }
  .cv-tb-term {
    color: var(--t1);
  }
</style>

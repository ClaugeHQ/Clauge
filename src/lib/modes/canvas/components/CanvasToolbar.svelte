<script lang="ts">
  import { onMount } from 'svelte';
  import { homeDir } from '@tauri-apps/api/path';
  import {
    viewport,
    ZOOM_MIN,
    ZOOM_MAX,
    ZOOM_DEFAULT,
    flushViewportSoon,
  } from '$lib/modes/canvas/stores/canvasStore';
  import {
    shellDefaultCwd,
    setShellDefaultCwd,
    type ShellCwdChoice,
  } from '$lib/modes/canvas/stores/canvasSettingsStore';
  import { spawnShellTerminal } from '$lib/modes/canvas/services/shellTerminalLifecycle';

  const ACTIVE_WORKSPACE_ID = '__phase2_stub__';
  // Phase 5 will replace this hardcoded workspace project root.
  const WORKSPACE_ROOT_STUB = '/Users/macbook/Personal';

  let menuOpen = $state(false);
  let menuX = $state(0);
  let menuY = $state(0);
  let longPressTimer: ReturnType<typeof setTimeout> | null = null;

  async function resolveCwd(choice: ShellCwdChoice): Promise<string> {
    if (choice === 'home') {
      try {
        return await homeDir();
      } catch {
        return '/';
      }
    }
    if (choice === 'workspace') {
      return WORKSPACE_ROOT_STUB;
    }
    return choice; // absolute path
  }

  async function openTerminal() {
    const cwd = await resolveCwd($shellDefaultCwd);
    try {
      await spawnShellTerminal(ACTIVE_WORKSPACE_ID, cwd);
    } catch (err) {
      console.error('[canvas] failed to spawn shell terminal:', err);
    }
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

  function showMenuAt(clientX: number, clientY: number) {
    menuX = clientX;
    menuY = clientY;
    menuOpen = true;
  }

  function onTerminalPointerDown(e: PointerEvent) {
    longPressTimer = setTimeout(() => {
      showMenuAt(e.clientX, e.clientY);
      longPressTimer = null;
    }, 500);
  }

  function onTerminalPointerUp() {
    if (longPressTimer !== null) {
      clearTimeout(longPressTimer);
      longPressTimer = null;
    }
  }

  function onTerminalContextMenu(e: MouseEvent) {
    e.preventDefault();
    showMenuAt(e.clientX, e.clientY);
  }

  async function pickHome() {
    await setShellDefaultCwd('home');
    menuOpen = false;
  }

  async function pickWorkspace() {
    await setShellDefaultCwd('workspace');
    menuOpen = false;
  }

  async function pickBrowse() {
    menuOpen = false;
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true, multiple: false, title: 'Choose terminal working directory' });
      if (typeof selected === 'string' && selected.length > 0) {
        await setShellDefaultCwd(selected);
      }
    } catch {
      // plugin-dialog unavailable in this build — no-op.
    }
  }

  const zoomPct = $derived(Math.round($viewport.zoom * 100));

  function closeMenuOnOutside(e: MouseEvent) {
    if (!menuOpen) return;
    // Bail if the click is on the terminal button itself or inside the cwd menu —
    // their own onclick handlers manage menuOpen state.
    const target = e.target as HTMLElement | null;
    if (target?.closest('.cv-tb-term') || target?.closest('.cv-cwd-menu')) return;
    menuOpen = false;
  }

  onMount(() => {
    window.addEventListener('pointerdown', closeMenuOnOutside);
    return () => window.removeEventListener('pointerdown', closeMenuOnOutside);
  });
</script>

<div class="cv-toolbar">
  <button
    class="cv-tb-btn cv-tb-term"
    onclick={openTerminal}
    onpointerdown={onTerminalPointerDown}
    onpointerup={onTerminalPointerUp}
    oncontextmenu={onTerminalContextMenu}
    title="Open terminal (right-click for options)"
    aria-label="Open terminal"
  >
    <!-- Inline SVG: terminal prompt icon -->
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <polyline points="4 17 10 11 4 5"></polyline>
      <line x1="12" y1="19" x2="20" y2="19"></line>
    </svg>
  </button>
  <button class="cv-tb-btn" onclick={() => setZoom($viewport.zoom / 1.2)} aria-label="Zoom out">−</button>
  <button class="cv-tb-btn cv-tb-pct" onclick={reset} title="Reset view" aria-label="Reset view">{zoomPct}%</button>
  <button class="cv-tb-btn" onclick={() => setZoom($viewport.zoom * 1.2)} aria-label="Zoom in">+</button>
</div>

{#if menuOpen}
  <div
    class="cv-cwd-menu"
    style="left: {menuX}px; top: {menuY}px;"
    onclick={(e) => e.stopPropagation()}
  >
    <button class="cv-menu-item" class:active={$shellDefaultCwd === 'home'} onclick={pickHome}>
      Home
    </button>
    <button class="cv-menu-item" class:active={$shellDefaultCwd === 'workspace'} onclick={pickWorkspace}>
      Workspace
    </button>
    <button class="cv-menu-item" onclick={pickBrowse}>
      Browse…
    </button>
  </div>
{/if}

<style>
  .cv-toolbar {
    position: absolute;
    bottom: 12px;
    right: 12px;
    display: flex;
    gap: 4px;
    padding: 4px;
    background: rgba(0, 0, 0, 0.5);
    border-radius: 8px;
    backdrop-filter: blur(8px);
  }
  .cv-tb-btn {
    height: 28px;
    min-width: 28px;
    padding: 0 8px;
    background: transparent;
    color: rgba(255, 255, 255, 0.85);
    border: 0;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .cv-tb-btn:hover {
    background: rgba(255, 255, 255, 0.08);
  }
  .cv-tb-pct {
    min-width: 56px;
    font-variant-numeric: tabular-nums;
  }
  .cv-tb-term {
    color: rgba(255, 255, 255, 0.85);
  }
  .cv-cwd-menu {
    position: fixed;
    background: rgba(0, 0, 0, 0.85);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 6px;
    padding: 4px;
    backdrop-filter: blur(8px);
    z-index: 1000;
    min-width: 140px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .cv-menu-item {
    background: transparent;
    color: rgba(255, 255, 255, 0.85);
    border: 0;
    text-align: left;
    padding: 6px 12px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
  }
  .cv-menu-item:hover {
    background: rgba(255, 255, 255, 0.08);
  }
  .cv-menu-item.active {
    background: rgba(74, 158, 255, 0.2);
    color: rgba(255, 255, 255, 1);
  }
</style>

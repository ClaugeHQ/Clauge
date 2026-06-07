<script lang="ts">
  import { tick } from 'svelte';
  import type { CanvasRegion as Region } from '$lib/modes/canvas/commands';
  import {
    regionsById,
    markRegionDirty,
    tilesByRegion,
    tilesByTab,
    markTileDirty,
    pendingRenameRegionId,
  } from '$lib/modes/canvas/stores/canvasStore';
  import { canvasDeleteRegion } from '$lib/modes/canvas/commands';
  import { regionDraggable } from '$lib/modes/canvas/actions/regionDraggable';
  import { regionResizable } from '$lib/modes/canvas/actions/regionResizable';

  let { region }: { region: Region } = $props();

  let editingName = $state(false);
  let nameInput = $state<HTMLInputElement | undefined>();
  let menuOpen = $state(false);
  let menuX = $state(0);
  let menuY = $state(0);

  async function startRename() {
    editingName = true;
    await tick();
    nameInput?.focus();
    nameInput?.select();
  }

  // Newly-created regions ask to be renamed via the pendingRenameRegionId
  // store so the user can type the project name immediately.
  $effect(() => {
    if ($pendingRenameRegionId === region.regionId) {
      pendingRenameRegionId.set(null);
      void startRename();
    }
  });

  function commitName(e: Event) {
    const value = (e.target as HTMLInputElement).value.trim() || 'Region';
    if (value !== region.name) {
      regionsById.update((m) => {
        const next = new Map(m);
        const cur = next.get(region.regionId);
        if (cur) next.set(region.regionId, { ...cur, name: value });
        return next;
      });
      markRegionDirty(region.regionId);
    }
    editingName = false;
  }

  function onNameKey(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      (e.target as HTMLInputElement).blur();
    } else if (e.key === 'Escape') {
      editingName = false;
    }
  }

  function openContextMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    menuX = e.clientX;
    menuY = e.clientY;
    menuOpen = true;
  }

  function closeMenu() {
    menuOpen = false;
  }

  async function deleteRegion() {
    menuOpen = false;
    const children = $tilesByRegion.get(region.regionId) ?? [];
    // Optimistic: clear the region locally first so the UI reacts
    // immediately; the backend call detaches any child tiles
    // (delete_children = false) on the SQL side.
    regionsById.update((m) => {
      const next = new Map(m);
      next.delete(region.regionId);
      return next;
    });
    tilesByTab.update((m) => {
      const next = new Map(m);
      for (const t of children) {
        const cur = next.get(t.tabId);
        if (cur) next.set(t.tabId, { ...cur, regionId: null });
      }
      return next;
    });
    for (const t of children) markTileDirty(t.tabId);
    try {
      await canvasDeleteRegion(region.workspaceId, region.regionId, false);
    } catch {
      // Best-effort; if the delete fails the next reload picks up the truth.
    }
  }

  const fillStyle = $derived(`background: color-mix(in srgb, ${region.color} 12%, transparent);`);
  const borderStyle = $derived(`border: 1px solid ${region.color};`);

  const handles: { dir: 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw'; cursor: string }[] = [
    { dir: 'ne', cursor: 'nesw-resize' },
    { dir: 'nw', cursor: 'nwse-resize' },
    { dir: 'se', cursor: 'nwse-resize' },
    { dir: 'sw', cursor: 'nesw-resize' },
  ];
</script>

<div
  class="cv-region"
  style="left: {region.x}px; top: {region.y}px; width: {region.width}px; height: {region.height}px; z-index: {region.zOrder}; {fillStyle} {borderStyle}"
  oncontextmenu={openContextMenu}
  use:regionDraggable={{ regionId: region.regionId }}
>
  <div class="cv-region-label" style="color: {region.color};">
    {#if editingName}
      <input
        bind:this={nameInput}
        class="cv-region-name-input"
        value={region.name}
        onblur={commitName}
        onkeydown={onNameKey}
        style="color: {region.color};"
      />
    {:else}
      <button
        type="button"
        class="cv-region-name"
        title="Double-click to rename"
        ondblclick={startRename}
        onpointerdown={(e) => e.stopPropagation()}
      >{region.name}</button>
    {/if}
    <button
      type="button"
      class="cv-region-close"
      title="Delete region (tiles stay)"
      aria-label="Delete region"
      onpointerdown={(e) => e.stopPropagation()}
      onclick={deleteRegion}
    >
      <svg width="11" height="11" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
        <line x1="2" y1="2" x2="12" y2="12"/>
        <line x1="12" y1="2" x2="2" y2="12"/>
      </svg>
    </button>
  </div>

  {#each handles as h (h.dir)}
    <div
      class="cv-region-handle cv-rh-{h.dir}"
      style="cursor: {h.cursor};"
      use:regionResizable={{ regionId: region.regionId, dir: h.dir }}
    ></div>
  {/each}
</div>

{#if menuOpen}
  <div
    class="cv-region-menu"
    style="left: {menuX}px; top: {menuY}px;"
    onpointerdown={(e) => e.stopPropagation()}
  >
    <button type="button" onclick={() => { closeMenu(); void startRename(); }}>Rename</button>
    <button type="button" onclick={() => { menuOpen = false; void deleteRegion(); }}>Delete region</button>
  </div>
  <div
    class="cv-region-menu-scrim"
    onpointerdown={closeMenu}
    onclick={closeMenu}
    role="presentation"
  ></div>
{/if}

<style>
  .cv-region {
    position: absolute;
    border-radius: 10px;
    pointer-events: auto;
    cursor: grab;
    /* No `contain: paint` — would clip the outside-top-left label. */
    contain: layout;
  }
  .cv-region:active {
    cursor: grabbing;
  }
  .cv-region-label {
    position: absolute;
    top: -22px;
    left: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.02em;
    user-select: none;
  }
  .cv-region-name {
    background: transparent;
    border: none;
    padding: 1px 4px;
    color: inherit;
    font: inherit;
    cursor: text;
    border-radius: 3px;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cv-region-name:hover {
    background: color-mix(in srgb, currentColor 10%, transparent);
  }
  .cv-region-name-input {
    background: var(--c);
    border: 1px solid currentColor;
    border-radius: 3px;
    padding: 1px 6px;
    font: inherit;
    outline: none;
    min-width: 100px;
  }
  .cv-region-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    background: transparent;
    border: none;
    border-radius: 3px;
    color: var(--t3);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity 80ms linear;
  }
  .cv-region:hover .cv-region-close,
  .cv-region-close:focus-visible {
    opacity: 1;
  }
  .cv-region-close:hover {
    color: #e74c3c;
    background: color-mix(in srgb, #e74c3c 15%, transparent);
  }
  .cv-region-handle {
    position: absolute;
    width: 14px;
    height: 14px;
    z-index: 1;
  }
  .cv-rh-ne { top: -7px; right: -7px; }
  .cv-rh-nw { top: -7px; left: -7px; }
  .cv-rh-se { bottom: -7px; right: -7px; }
  .cv-rh-sw { bottom: -7px; left: -7px; }

  .cv-region-menu {
    position: fixed;
    z-index: 1000;
    background: var(--c);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 3px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 140px;
    box-shadow: 0 8px 24px rgba(0,0,0,0.4);
  }
  .cv-region-menu button {
    background: transparent;
    border: none;
    text-align: left;
    padding: 6px 10px;
    color: var(--t1);
    font: 12px var(--ui);
    border-radius: 3px;
    cursor: pointer;
  }
  .cv-region-menu button:hover {
    background: var(--surface-hover);
  }
  .cv-region-menu-scrim {
    position: fixed;
    inset: 0;
    z-index: 999;
  }
</style>

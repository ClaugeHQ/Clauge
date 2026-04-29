<script lang="ts">
  import type { Collection, Request } from '$lib/types';
  import { activeCollectionId, deleteCollection, updateCollection, createRequest, loadCollections, loadRequest, collectionsRefreshTrigger } from '$lib/modes/rest/stores';
  import { tabs, addTab, activateTab } from '$lib/shared/stores/tabs';
  import { METHOD_COLORS, METHOD_COLORS_LIGHT } from '$lib/utils/theme';
  import { appearance } from '$lib/stores/settings';
  import { get } from 'svelte/store';
  import { showContextMenu } from '$lib/components/shared/contextmenu';
  import { showToast } from '$lib/components/shared/toast';
  import { friendlyError } from '$lib/utils/errors';
  import * as cmd from '$lib/commands';
  import RequestItem from './RequestItem.svelte';
  import InlineInput from './InlineInput.svelte';
  import ConfirmDialog from '$lib/components/shared/ConfirmDialog.svelte';

  interface Props {
    collection: Collection;
    searchQuery?: string;
    ondeleted?: () => void;
  }

  let { collection, searchQuery = '', ondeleted }: Props = $props();

  let expanded = $state(false);
  let requests = $state<Request[]>([]);
  let loaded = $state(false);
  let addingRequest = $state(false);
  let renaming = $state(false);
  let showDeleteConfirm = $state(false);
  let dragOverReqIndex = $state<number | null>(null);
  let isDragging = $state(false);

  const isActive = $derived($activeCollectionId === collection.id);

  const filteredRequests = $derived(
    searchQuery
      ? requests.filter(r =>
          r.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
          r.url.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : requests
  );

  // Auto-expand when search matches requests
  $effect(() => {
    if (searchQuery && filteredRequests.length > 0 && !expanded) {
      expanded = true;
      if (!loaded) loadRequests();
    }
  });

  // Re-fetch requests when collections are refreshed (e.g. after saving a new request)
  $effect(() => {
    const _ = $collectionsRefreshTrigger;
    if (loaded) {
      cmd.listRequests(collection.id).then(r => { requests = r; }).catch(() => {});
    }
  });

  async function loadRequests() {
    try {
      requests = await cmd.listRequests(collection.id);
      loaded = true;
    } catch (err) {
      // Failed to load requests — silently ignore
    }
  }

  async function toggle() {
    expanded = !expanded;
    if (expanded && !loaded) {
      await loadRequests();
    }
  }

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Rename',
        action: () => { renaming = true; },
      },
      {
        label: 'Add Request',
        action: () => {
          expanded = true;
          if (!loaded) loadRequests();
          addingRequest = true;
        },
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Delete',
        danger: true,
        action: () => { showDeleteConfirm = true; },
      },
    ]);
  }

  async function handleRename(newName: string) {
    renaming = false;
    if (newName === collection.name) return;
    try {
      await updateCollection(collection.id, newName, collection.envId);
      collection.name = newName;
      showToast('Collection renamed', 'success');
    } catch (err) {
      showToast('Failed to rename collection', 'error');
    }
  }

  function cancelRename() {
    renaming = false;
  }

  function handleAddClick(e: MouseEvent) {
    e.stopPropagation();
    if (!expanded) {
      expanded = true;
      if (!loaded) loadRequests();
    }
    addingRequest = true;
  }

  async function handleAddRequest(name: string) {
    addingRequest = false;
    try {
      const req = await createRequest(collection.id, name, 'GET');
      requests = [...requests, req];

      // Auto-select the new request and open a tab
      activeCollectionId.set(collection.id);
      await loadRequest(req.id);
      const label = `GET ${name}`;
      const methodColors = get(appearance)?.theme === 'light' ? METHOD_COLORS_LIGHT : METHOD_COLORS;
      const colors2 = methodColors['GET'];
      addTab(label, 'rest', req.id, colors2.color);

      showToast('Request created', 'success');
    } catch (err: any) {
      showToast(friendlyError(err), 'error');
      console.error('Create request error:', err);
    }
  }

  function cancelAddRequest() {
    addingRequest = false;
  }

  async function handleDeleteCollection() {
    try {
      await deleteCollection(collection.id);
      showToast('Collection deleted', 'success');
      ondeleted?.();
    } catch (err) {
      showToast('Failed to delete collection', 'error');
    }
  }

  async function handleRequestChanged() {
    await loadRequests();
  }

  // Drag & drop for the collection header
  function handleDragStart(e: DragEvent) {
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/collection-id', collection.id);
    isDragging = true;
  }

  function handleDragEnd() {
    isDragging = false;
  }

  // Drag & drop for requests within this collection
  function handleReqDragOver(e: DragEvent, index: number) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverReqIndex = index;
  }

  function handleReqDragLeave() {
    dragOverReqIndex = null;
  }

  async function handleReqDrop(e: DragEvent, targetIndex: number) {
    e.preventDefault();
    e.stopPropagation();
    dragOverReqIndex = null;

    const sourceReqId = e.dataTransfer?.getData('text/request-id');
    const sourceCollId = e.dataTransfer?.getData('text/request-collection-id');

    if (!sourceReqId) return;

    if (sourceCollId && sourceCollId !== collection.id) {
      // Moving request from another collection
      try {
        await cmd.moveRequest(sourceReqId, collection.id);
        showToast('Request moved', 'success');
        await loadRequests();
        // Trigger reload in source collection too
        await loadCollections();
      } catch (err) {
        showToast('Failed to move request', 'error');
      }
      return;
    }

    // Reorder within same collection
    const currentList = [...requests];
    const sourceIndex = currentList.findIndex(r => r.id === sourceReqId);
    if (sourceIndex === -1 || sourceIndex === targetIndex) return;

    const [moved] = currentList.splice(sourceIndex, 1);
    currentList.splice(targetIndex, 0, moved);
    requests = currentList;
    // Note: backend reorder for requests not implemented yet, just update UI
  }

  // Allow dropping requests into empty collection body area
  function handleBodyDragOver(e: DragEvent) {
    if (e.dataTransfer?.types.includes('text/request-id')) {
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    }
  }

  async function handleBodyDrop(e: DragEvent) {
    e.preventDefault();
    const sourceReqId = e.dataTransfer?.getData('text/request-id');
    const sourceCollId = e.dataTransfer?.getData('text/request-collection-id');
    if (!sourceReqId || sourceCollId === collection.id) return;

    try {
      await cmd.moveRequest(sourceReqId, collection.id);
      showToast('Request moved', 'success');
      if (!expanded) { expanded = true; }
      await loadRequests();
      await loadCollections();
    } catch (err) {
      showToast('Failed to move request', 'error');
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="ncoll" class:dragging={isDragging}>
  <div
    class="ncoll-hdr"
    class:active={isActive}
    onclick={toggle}
    oncontextmenu={handleContextMenu}
    draggable="true"
    ondragstart={handleDragStart}
    ondragend={handleDragEnd}
  >
    <div class="drag-handle" title="Drag to reorder">
      <svg viewBox="0 0 24 24" width="10" height="10"><circle cx="8" cy="6" r="1.5" fill="currentColor"/><circle cx="16" cy="6" r="1.5" fill="currentColor"/><circle cx="8" cy="12" r="1.5" fill="currentColor"/><circle cx="16" cy="12" r="1.5" fill="currentColor"/><circle cx="8" cy="18" r="1.5" fill="currentColor"/><circle cx="16" cy="18" r="1.5" fill="currentColor"/></svg>
    </div>
    <div class="coll-icon" style="background:rgba(79,148,212,0.15);color:#4f94d4">
      <svg viewBox="0 0 24 24"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>
    </div>
    {#if renaming}
      <InlineInput
        value={collection.name}
        placeholder="Collection name..."
        onsubmit={handleRename}
        oncancel={cancelRename}
      />
    {:else}
      <span class="ncoll-name">{collection.name}</span>
    {/if}
    <button class="coll-add" title="Add request" onclick={handleAddClick}>
      <svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>
    </button>
    <svg class="ncoll-arr" class:open={expanded} viewBox="0 0 24 24">
      <path d="M9 18l6-6-6-6" stroke="currentColor" fill="none" stroke-width="1.8" stroke-linecap="round"/>
    </svg>
  </div>
  <div
    class="ncoll-body"
    style="max-height:{expanded ? (filteredRequests.length + (addingRequest ? 1 : 0)) * 40 + 200 + 'px' : '0'}"
    ondragover={handleBodyDragOver}
    ondrop={handleBodyDrop}
  >
    {#each filteredRequests as req, i (req.id)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="req-drop-zone"
        class:drop-above={dragOverReqIndex === i}
        ondragover={(e) => handleReqDragOver(e, i)}
        ondragleave={handleReqDragLeave}
        ondrop={(e) => handleReqDrop(e, i)}
      >
        <RequestItem request={req} ondeleted={handleRequestChanged} />
      </div>
    {/each}
    {#if addingRequest}
      <div class="inline-add-req">
        <InlineInput
          placeholder="Request name..."
          onsubmit={handleAddRequest}
          oncancel={cancelAddRequest}
        />
      </div>
    {/if}
  </div>
</div>

<ConfirmDialog
  bind:show={showDeleteConfirm}
  title="Delete Collection"
  message="Are you sure you want to delete '{collection.name}'? All requests in this collection will be deleted. This cannot be undone."
  confirmText="Delete"
  onconfirm={handleDeleteCollection}
/>

<style>
  .ncoll {
    border-bottom: 1px solid var(--b1);
    transition: opacity 0.15s;
  }
  .ncoll.dragging {
    opacity: 0.4;
  }
  .ncoll-hdr {
    padding: 7px 8px;
    display: flex;
    align-items: center;
    gap: 7px;
    cursor: pointer;
    transition: background 0.1s;
    user-select: none;
    position: relative;
  }
  .ncoll-hdr:hover {
    background: var(--n2);
  }
  .ncoll-hdr.active {
    background: var(--n2);
  }
  .drag-handle {
    width: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t4);
    flex-shrink: 0;
    cursor: grab;
    opacity: 0.3;
    transition: opacity 0.15s, color 0.15s;
  }
  .ncoll-hdr:hover .drag-handle {
    opacity: 0.7;
  }
  .drag-handle:hover {
    color: var(--t2);
  }
  .drag-handle:active {
    cursor: grabbing;
  }
  .coll-icon {
    width: 16px;
    height: 16px;
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .coll-icon svg {
    width: 10px;
    height: 10px;
    stroke: currentColor;
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
  }
  .ncoll-name {
    font-size: 12px;
    color: var(--t2);
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ncoll-arr {
    width: 12px;
    height: 12px;
    stroke: var(--t3);
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
    flex-shrink: 0;
    transition: transform 0.18s;
  }
  .ncoll-arr.open {
    transform: rotate(90deg);
  }
  .ncoll-body {
    overflow: hidden;
    background: var(--e);
    transition: max-height 0.2s ease;
  }
  .coll-add {
    width: 18px;
    height: 18px;
    border-radius: 4px;
    border: none;
    background: transparent;
    display: none;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    flex-shrink: 0;
    color: var(--t3);
    transition: background 0.1s, color 0.1s;
  }
  .ncoll-hdr:hover .coll-add {
    display: flex;
  }
  .coll-add:hover {
    background: var(--b1);
    color: var(--t1);
  }
  .coll-add svg {
    width: 11px;
    height: 11px;
    stroke: currentColor;
    fill: none;
    stroke-width: 2.2;
    stroke-linecap: round;
  }
  .inline-add-req {
    display: flex;
    gap: 4px;
    padding: 6px 10px;
    align-items: center;
    background: var(--e);
    border-bottom: 1px solid var(--b1);
  }
  .req-drop-zone {
    position: relative;
  }
  .req-drop-zone.drop-above::before {
    content: '';
    position: absolute;
    top: 0;
    left: 24px;
    right: 8px;
    height: 2px;
    background: var(--acc);
    border-radius: 1px;
    z-index: 10;
  }
</style>

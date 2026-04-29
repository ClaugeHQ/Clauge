<script lang="ts">
  import type { Request, Collection } from '$lib/types';
  import { METHOD_COLORS, METHOD_COLORS_LIGHT } from '$lib/utils/theme';
  import { appearance } from '$lib/stores/settings';
  import { activeRequestId, activeCollectionId, loadRequest, deleteRequest, collections, loadCollections } from '$lib/modes/rest/stores';
  import { tabs, activeTabId, addTab, activateTab, updateTab } from '$lib/shared/stores/tabs';
  import { showContextMenu } from '$lib/components/shared/contextmenu';
  import { showToast } from '$lib/components/shared/toast';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { get } from 'svelte/store';
  import * as cmd from '$lib/commands';
  import InlineInput from './InlineInput.svelte';
  import ConfirmDialog from '$lib/components/shared/ConfirmDialog.svelte';

  interface Props {
    request: Request;
    ondeleted?: () => void;
  }

  let { request, ondeleted }: Props = $props();

  let renaming = $state(false);
  let showDeleteConfirm = $state(false);
  let isDragging = $state(false);

  const methodColors = $derived($appearance?.theme === 'light' ? METHOD_COLORS_LIGHT : METHOD_COLORS);
  const colors = $derived(methodColors[request.method] ?? { color: '#888', bg: '#1a1a1a' });
  const isActive = $derived($activeRequestId === request.id);
  const methodLabel = $derived(request.method === 'DELETE' ? 'DEL' : request.method);

  async function handleClick() {
    activeCollectionId.set(request.collectionId);
    await loadRequest(request.id);

    // Open or activate a tab for this request
    const allTabs = get(tabs);
    const existing = allTabs.find(t => t.key === request.id);
    const tabColors = (get(appearance)?.theme === 'light' ? METHOD_COLORS_LIGHT : METHOD_COLORS)[request.method] ?? { color: '#888', bg: '#1a1a1a' };
    if (existing) {
      updateTab(existing.id, { dot: tabColors.color });
      activateTab(existing.id);
    } else {
      const label = `${request.method} ${request.name || request.url || '/new'}`;
      addTab(label, 'rest', request.id, tabColors.color);
    }
  }

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();

    const collList: Collection[] = [];
    collections.subscribe(c => { collList.push(...c); })();

    // Build "Move to..." items for other collections
    const moveItems = collList
      .filter(c => c.id !== request.collectionId)
      .map(c => ({
        label: c.name,
        action: async () => {
          try {
            await cmd.moveRequest(request.id, c.id);
            ondeleted?.(); // triggers reload in source collection
            await loadCollections(); // refresh entire sidebar
            showToast(`Moved to ${c.name}`, 'success');
          } catch (err) {
            showToast('Failed to move request', 'error');
          }
        },
      }));

    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Rename',
        action: () => { renaming = true; },
      },
      {
        label: 'Duplicate',
        action: async () => {
          try {
            await cmd.duplicateRequest(request.id);
            showToast('Request duplicated', 'success');
            ondeleted?.(); // triggers reload
          } catch (err) {
            showToast('Failed to duplicate request', 'error');
          }
        },
      },
      {
        label: 'Copy as cURL',
        action: async () => {
          try {
            const curl = await cmd.exportAsCurl(request.id);
            await writeText(curl);
            showToast('Copied cURL to clipboard', 'success');
          } catch (err) {
            showToast('Failed to copy cURL', 'error');
          }
        },
      },
      ...(moveItems.length > 0 ? [
        { label: '', action: () => {}, separator: true },
        ...moveItems.map(item => ({
          label: `Move to ${item.label}`,
          action: item.action,
        })),
      ] : []),
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
    if (newName === request.name) return;
    try {
      await cmd.updateRequest(request.id, { name: newName });
      request.name = newName;
      showToast('Request renamed', 'success');
    } catch (err) {
      showToast('Failed to rename request', 'error');
    }
  }

  function cancelRename() {
    renaming = false;
  }

  async function handleDelete() {
    try {
      await deleteRequest(request.id);
      showToast('Request deleted', 'success');
      ondeleted?.();
    } catch (err) {
      showToast('Failed to delete request', 'error');
    }
  }

  function handleDragStart(e: DragEvent) {
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/request-id', request.id);
    e.dataTransfer.setData('text/request-collection-id', request.collectionId);
    isDragging = true;
  }

  function handleDragEnd() {
    isDragging = false;
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="nreq"
  class:on={isActive}
  class:dragging={isDragging}
  onclick={handleClick}
  oncontextmenu={handleContextMenu}
  draggable="true"
  ondragstart={handleDragStart}
  ondragend={handleDragEnd}
>
  <div class="drag-handle" title="Drag to reorder">
    <svg viewBox="0 0 24 24" width="10" height="10"><circle cx="8" cy="6" r="1.5" fill="currentColor"/><circle cx="16" cy="6" r="1.5" fill="currentColor"/><circle cx="8" cy="12" r="1.5" fill="currentColor"/><circle cx="16" cy="12" r="1.5" fill="currentColor"/><circle cx="8" cy="18" r="1.5" fill="currentColor"/><circle cx="16" cy="18" r="1.5" fill="currentColor"/></svg>
  </div>
  <span class="nreq-method" style="background:{colors.bg};color:{colors.color}">{methodLabel}</span>
  {#if renaming}
    <InlineInput
      value={request.name}
      placeholder="Request name..."
      onsubmit={handleRename}
      oncancel={cancelRename}
    />
  {:else}
    <span class="nreq-name">{request.name || request.url || 'Untitled'}</span>
  {/if}
</div>

<ConfirmDialog
  bind:show={showDeleteConfirm}
  title="Delete Request"
  message="Are you sure you want to delete '{request.name || 'Untitled'}'? This cannot be undone."
  confirmText="Delete"
  onconfirm={handleDelete}
/>

<style>
  .nreq {
    padding: 5px 8px 5px 24px;
    display: flex;
    align-items: center;
    gap: 7px;
    cursor: pointer;
    transition: background 0.1s, opacity 0.15s;
    border-left: 2px solid transparent;
  }
  .nreq.dragging {
    opacity: 0.4;
  }
  .nreq:hover {
    background: rgba(255,255,255,0.04);
    color: var(--t1);
  }
  .drag-handle {
    width: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t4);
    flex-shrink: 0;
    cursor: grab;
    opacity: 0.3;
    transition: opacity 0.15s, color 0.15s;
  }
  .nreq:hover .drag-handle {
    opacity: 0.7;
  }
  .drag-handle:hover {
    color: var(--t2);
  }
  .drag-handle:active {
    cursor: grabbing;
  }
  .nreq.on {
    background: var(--n2);
    border-left-color: var(--acc);
  }
  .nreq-method {
    font-size: 9px;
    font-weight: 700;
    font-family: var(--mono);
    padding: 1px 5px;
    border-radius: 3px;
    flex-shrink: 0;
    letter-spacing: 0.04em;
  }
  .nreq-name {
    font-size: 11.5px;
    color: var(--t2);
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--mono);
  }
  .nreq.on .nreq-name {
    color: var(--t1);
  }
</style>

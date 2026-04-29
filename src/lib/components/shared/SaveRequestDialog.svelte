<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { collections, createRequest, loadRequest } from '$lib/modes/rest/stores';
  import { getDraft, updateTab, markClean, tabs } from '$lib/shared/stores/tabs';
  import { METHOD_COLORS, METHOD_COLORS_LIGHT } from '$lib/utils/theme';
  import { appearance } from '$lib/stores/settings';
  import { loadCollections } from '$lib/modes/rest/stores';

  interface Props {
    show: boolean;
    tabId: number;
    onclose?: () => void;
  }

  let { show = $bindable(), tabId, onclose }: Props = $props();

  let name = $state('');
  let selectedCollectionId = $state('');
  let saving = $state(false);

  const draft = $derived(getDraft(tabId));
  const method = $derived(draft?.method ?? 'GET');
  const activeMethodColors = $derived($appearance?.theme === 'light' ? METHOD_COLORS_LIGHT : METHOD_COLORS);
  const methodColor = $derived(activeMethodColors[method] ?? activeMethodColors.GET);

  // Pre-fill name from URL path when dialog opens
  $effect(() => {
    if (show && draft?.url) {
      try {
        const urlStr = draft.url.startsWith('http') ? draft.url : 'https://' + draft.url;
        const parsed = new URL(urlStr);
        const pathParts = parsed.pathname.split('/').filter(Boolean);
        name = pathParts.length > 0 ? pathParts[pathParts.length - 1] : draft.url;
      } catch {
        name = draft.url || 'New Request';
      }
    } else if (show) {
      name = 'New Request';
    }
  });

  // Set default collection when dialog opens
  $effect(() => {
    if (show && $collections.length > 0 && !selectedCollectionId) {
      selectedCollectionId = $collections[0].id;
    }
  });

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && show) {
      e.preventDefault();
      close();
    }
    if (e.key === 'Enter' && show && !saving) {
      e.preventDefault();
      handleSave();
    }
  }

  function close() {
    show = false;
    name = '';
    selectedCollectionId = '';
    saving = false;
    onclose?.();
  }

  function handleOverlayClick(e: MouseEvent) {
    if (e.target === e.currentTarget) {
      close();
    }
  }

  async function handleSave() {
    if (!name.trim() || !selectedCollectionId || saving) return;
    saving = true;
    try {
      const req = await createRequest(selectedCollectionId, name.trim(), method);
      // Now update the tab to point to the new request
      updateTab(tabId, { key: req.id, unsaved: false, label: method + ' ' + name.trim() });
      markClean(tabId);
      // Reload collections to reflect the new request in the nav
      await loadCollections();
      // Load the request so activeRequest is set
      await loadRequest(req.id);
      // If draft has more data, commit it
      if (draft) {
        const { commitRequest } = await import('$lib/modes/rest/stores');
        await commitRequest(req.id, draft);
      }
      close();
    } catch (err) {
      console.error('Failed to save request:', err);
      saving = false;
    }
  }

  onMount(() => {
    window.addEventListener('keydown', handleKeydown);
  });

  onDestroy(() => {
    window.removeEventListener('keydown', handleKeydown);
  });
</script>

{#if show}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="q-modal-overlay" onclick={handleOverlayClick}>
    <div class="q-save-dialog">
      <div class="q-modal-hdr">
        <span class="q-modal-title">Save Request</span>
        <button class="q-modal-close" onclick={close}>&times;</button>
      </div>
      <div class="q-save-body">
        <div class="q-save-method">
          <span class="method-badge" style="color: {methodColor.color}; background: {methodColor.bg};">{method}</span>
        </div>

        <label class="q-save-label">
          <span class="q-save-label-text">Name</span>
          <input
            class="q-save-input"
            type="text"
            bind:value={name}
            placeholder="Request name"
            autofocus
          />
        </label>

        <label class="q-save-label">
          <span class="q-save-label-text">Collection</span>
          <select class="q-save-select" bind:value={selectedCollectionId}>
            {#each $collections as coll (coll.id)}
              <option value={coll.id}>{coll.name}</option>
            {/each}
          </select>
        </label>
      </div>
      <div class="q-save-actions">
        <button class="q-save-cancel" onclick={close}>Cancel</button>
        <button
          class="q-save-ok"
          onclick={handleSave}
          disabled={!name.trim() || !selectedCollectionId || saving}
        >{saving ? 'Saving...' : 'Save'}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .q-modal-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(4px);
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    animation: fadeIn 0.15s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .q-save-dialog {
    width: 400px;
    background: var(--modal-bg, #101016);
    border: 1px solid var(--b1);
    border-radius: 10px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    animation: modalUp 0.18s ease;
    overflow: hidden;
  }

  @keyframes modalUp {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to { opacity: 1; transform: none; }
  }

  .q-modal-hdr {
    display: flex;
    align-items: center;
    padding: 14px 18px;
    border-bottom: 1px solid var(--b1);
    background: var(--e);
  }

  .q-modal-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }

  .q-modal-close {
    margin-left: auto;
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: default;
    transition: background 0.12s;
    color: var(--t3);
    font-size: 16px;
  }

  .q-modal-close:hover {
    background: var(--c);
    color: var(--t1);
  }

  .q-save-body {
    padding: 18px 22px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .q-save-method {
    display: flex;
    align-items: center;
  }

  .method-badge {
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    padding: 4px 10px;
    border-radius: 5px;
  }

  .q-save-label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .q-save-label-text {
    font-size: 11.5px;
    color: var(--t2);
    font-family: var(--ui);
    font-weight: 500;
  }

  .q-save-input,
  .q-save-select {
    height: 34px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 0 12px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--t1);
    outline: none;
    transition: border-color 0.15s;
  }

  .q-save-input:focus,
  .q-save-select:focus {
    border-color: var(--acc);
  }

  .q-save-select {
    cursor: pointer;
  }

  .q-save-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 22px;
    border-top: 1px solid var(--b1);
  }

  .q-save-cancel {
    height: 30px;
    padding: 0 16px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: pointer;
    transition: border-color 0.1s, color 0.1s;
  }

  .q-save-cancel:hover {
    border-color: var(--b2);
    color: var(--t1);
  }

  .q-save-ok {
    height: 30px;
    padding: 0 16px;
    border-radius: 5px;
    border: none;
    background: var(--acc);
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    font-family: var(--ui);
    cursor: pointer;
    transition: opacity 0.12s;
  }

  .q-save-ok:hover {
    opacity: 0.85;
  }

  .q-save-ok:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>

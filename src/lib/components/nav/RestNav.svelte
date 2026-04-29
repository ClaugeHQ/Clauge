<script lang="ts">
  import { collections, loadCollections, createCollection } from '$lib/modes/rest/stores';
  import { showToast } from '$lib/components/shared/toast';
  import CollectionItem from './CollectionItem.svelte';
  import InlineInput from './InlineInput.svelte';
  import * as cmd from '$lib/commands';

  interface Props {
    searchQuery?: string;
  }

  let { searchQuery = '' }: Props = $props();

  let addingCollection = $state(false);
  let dragOverIndex = $state<number | null>(null);

  const filteredCollections = $derived(
    searchQuery
      ? $collections.filter(c =>
          c.name.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : $collections
  );

  export function showAddCollection() {
    addingCollection = true;
  }

  async function handleAddCollection(name: string) {
    addingCollection = false;
    try {
      await createCollection(name);
      showToast('Collection created', 'success');
    } catch (err) {
      showToast('Failed to create collection', 'error');
    }
  }

  function cancelAddCollection() {
    addingCollection = false;
  }

  async function handleCollectionDeleted() {
    await loadCollections();
  }

  function handleDragOver(e: DragEvent, index: number) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverIndex = index;
  }

  function handleDragLeave() {
    dragOverIndex = null;
  }

  async function handleDrop(e: DragEvent, targetIndex: number) {
    e.preventDefault();
    dragOverIndex = null;

    const sourceId = e.dataTransfer?.getData('text/collection-id');
    if (!sourceId) return;

    const currentList = [...$collections];
    const sourceIndex = currentList.findIndex(c => c.id === sourceId);
    if (sourceIndex === -1 || sourceIndex === targetIndex) return;

    // Reorder locally
    const [moved] = currentList.splice(sourceIndex, 1);
    currentList.splice(targetIndex, 0, moved);

    // Update store immediately for snappy UI
    collections.set(currentList);

    // Persist to backend
    try {
      await cmd.reorderCollections(currentList.map(c => c.id));
    } catch (err) {
      showToast('Failed to reorder', 'error');
      await loadCollections(); // rollback
    }
  }
</script>

<div class="rest-nav">
  {#if filteredCollections.length === 0 && !addingCollection}
    <div class="nav-empty">
      {#if searchQuery}
        <span>No results for "{searchQuery}"</span>
      {:else}
        <span>No collections yet</span>
        <button class="nav-empty-btn" onclick={() => addingCollection = true}>
          + New Collection
        </button>
      {/if}
    </div>
  {:else}
    {#each filteredCollections as coll, i (coll.id)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="coll-drop-zone"
        class:drop-above={dragOverIndex === i}
        ondragover={(e) => handleDragOver(e, i)}
        ondragleave={handleDragLeave}
        ondrop={(e) => handleDrop(e, i)}
      >
        <CollectionItem
          collection={coll}
          {searchQuery}
          ondeleted={handleCollectionDeleted}
        />
      </div>
    {/each}
  {/if}
  {#if addingCollection}
    <div class="inline-add-coll">
      <InlineInput
        placeholder="Collection name..."
        onsubmit={handleAddCollection}
        oncancel={cancelAddCollection}
      />
    </div>
  {/if}
</div>

<style>
  .rest-nav {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .nav-empty {
    padding: 24px 12px;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  .nav-empty-btn {
    padding: 5px 12px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11px;
    font-family: var(--mono);
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .nav-empty-btn:hover {
    background: var(--c);
    border-color: var(--b2);
    color: var(--t1);
  }
  .inline-add-coll {
    padding: 8px 10px;
    border-bottom: 1px solid var(--b1);
  }
  .coll-drop-zone {
    position: relative;
  }
  .coll-drop-zone.drop-above::before {
    content: '';
    position: absolute;
    top: 0;
    left: 8px;
    right: 8px;
    height: 2px;
    background: var(--acc);
    border-radius: 1px;
    z-index: 10;
  }
</style>

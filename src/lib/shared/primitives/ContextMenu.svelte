<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { contextMenu, closeContextMenu } from './contextmenu';
  import { CLICK_OUTSIDE_GUARD_MS } from '$lib/shared/constants/timings';

  let menuEl: HTMLDivElement | undefined = $state();

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && $contextMenu.show) {
      e.preventDefault();
      closeContextMenu();
    }
  }

  function handleClickOutside(e: MouseEvent) {
    if (menuEl && !menuEl.contains(e.target as Node)) {
      closeContextMenu();
    }
  }

  function adjustPosition(el: HTMLDivElement) {
    const rect = el.getBoundingClientRect();
    if (rect.right > window.innerWidth) {
      el.style.left = `${window.innerWidth - rect.width - 8}px`;
    }
    if (rect.bottom > window.innerHeight) {
      el.style.top = `${window.innerHeight - rect.height - 8}px`;
    }
  }

  $effect(() => {
    if ($contextMenu.show && menuEl) {
      adjustPosition(menuEl);
    }
  });

  onMount(() => {
    window.addEventListener('keydown', handleKeydown);
    setTimeout(() => {
      window.addEventListener('click', handleClickOutside);
    }, CLICK_OUTSIDE_GUARD_MS);
  });

  onDestroy(() => {
    window.removeEventListener('keydown', handleKeydown);
    window.removeEventListener('click', handleClickOutside);
  });
</script>

{#if $contextMenu.show}
  <div
    class="ctx-menu"
    bind:this={menuEl}
    style="left: {$contextMenu.x}px; top: {$contextMenu.y}px"
  >
    <div class="ctx-items" class:ctx-scrollable={$contextMenu.scrollable}>
      {#each $contextMenu.items as item}
        {#if item.separator}
          <div class="ctx-sep"></div>
        {:else}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="ctx-item"
            class:danger={item.danger}
            onclick={() => { closeContextMenu(); item.action(); }}
          >
            {#if item.icon}
              <span class="ctx-icon">{@html item.icon}</span>
            {/if}
            {#if item.sub}
              <span class="ctx-item-body">
                <span class="ctx-item-label">{item.label}</span>
                <span class="ctx-item-sub">{item.sub}</span>
              </span>
            {:else}
              {item.label}
            {/if}
          </div>
        {/if}
      {/each}
    </div>
    {#if $contextMenu.stickyFooter}
      {@const footer = $contextMenu.stickyFooter}
      <div class="ctx-sep"></div>
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="ctx-item" onclick={() => { closeContextMenu(); footer.action(); }}>
        {#if footer.icon}
          <span class="ctx-icon">{@html footer.icon}</span>
        {/if}
        {footer.label}
      </div>
    {/if}
  </div>
{/if}

<style>
  .ctx-menu {
    position: fixed;
    background: var(--n);
    backdrop-filter: blur(24px);
    -webkit-backdrop-filter: blur(24px);
    border: 1px solid var(--b1);
    border-radius: var(--radius-lg);
    box-shadow: 0 8px 32px rgba(0,0,0,0.5);
    z-index: 2000;
    min-width: 180px;
    padding: 4px 0;
    animation: fadeIn 0.1s ease;
    display: flex;
    flex-direction: column;
  }
  .ctx-items {
    display: contents;
  }
  .ctx-scrollable {
    display: block;
    max-height: 320px;
    overflow-y: auto;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .ctx-item {
    padding: 7px 14px;
    font-size: 12.5px;
    color: var(--t2);
    font-family: var(--ui);
    cursor: default;
    transition: background 0.08s;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .ctx-item:hover {
    background: var(--c);
    color: var(--t1);
  }

  .ctx-item.danger {
    color: var(--err);
  }

  .ctx-item.danger:hover {
    background: rgba(240,68,68,0.1);
  }

  .ctx-sep {
    height: 1px;
    background: var(--b1);
    margin: 4px 0;
  }

  .ctx-icon {
    font-size: 14px;
    width: 16px;
    text-align: center;
    flex-shrink: 0;
  }

  .ctx-item-body {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .ctx-item-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ctx-item-sub {
    font-size: 10.5px;
    color: var(--t4);
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>

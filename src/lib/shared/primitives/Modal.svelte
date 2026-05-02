<script lang="ts">
  import { onMount, onDestroy } from 'svelte';

  interface Props {
    show: boolean;
    title: string;
    width?: string;
    onclose?: () => void;
    children?: import('svelte').Snippet;
  }

  let { show = $bindable(), title, width = '480px', onclose, children }: Props = $props();

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && show) {
      e.preventDefault();
      close();
    }
  }

  function close() {
    show = false;
    onclose?.();
  }

  function handleOverlayClick(_e: MouseEvent) {
    // Intentionally no-op — modal closes only via X button or Escape key
  }

  // Teleport the modal subtree to <body> so it always renders relative to
  // the viewport. Without this, an ancestor with a non-none `transform`
  // (e.g. NavPanel.overlay's slide-in animation) creates a containing block
  // and clips the modal's `position: fixed` overlay to the nav-panel area.
  // This was hitting every modal opened from inside the floating nav.
  function teleportToBody(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentElement === document.body) node.remove();
      },
    };
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
  <div class="q-modal-overlay" use:teleportToBody onclick={handleOverlayClick}>
    <div class="q-modal" style="width: {width}">
      <div class="q-modal-hdr">
        <span class="q-modal-title">{title}</span>
        <button class="q-modal-close" onclick={close}>&times;</button>
      </div>
      <div class="q-modal-body">
        {#if children}
          {@render children()}
        {/if}
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
    background: rgba(0,0,0,0.4);
    backdrop-filter: blur(8px);
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

  .q-modal {
    background: var(--modal-bg, #101016);
    border: 1px solid var(--b1);
    border-radius: var(--radius-xl);
    box-shadow: 0 24px 48px rgba(0,0,0,0.5);
    display: flex;
    flex-direction: column;
    animation: modalUp 0.18s ease;
    max-height: 85vh;
    max-width: 90vw;
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
    flex-shrink: 0;
  }

  .q-modal-title {
    font-size: 16px;
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

  .q-modal-body {
    padding: 20px 24px;
    overflow-y: auto;
  }
</style>

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';

  interface Props {
    show: boolean;
    title: string;
    message: string;
    confirmText?: string;
    confirmColor?: string;
    /** Optional third button (e.g. "Don't Save"). When set, the dialog
     *  shows three buttons: Cancel · {discardText} · {confirmText}. Used
     *  by REST/SQL "save before close" prompts so they don't have to roll
     *  their own dialog markup. */
    discardText?: string;
    onconfirm?: () => void;
    oncancel?: () => void;
    ondiscard?: () => void;
  }

  let {
    show = $bindable(),
    title,
    message,
    confirmText = 'Delete',
    confirmColor = 'var(--err)',
    discardText,
    onconfirm,
    oncancel,
    ondiscard,
  }: Props = $props();

  function discard() {
    show = false;
    ondiscard?.();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && show) {
      e.preventDefault();
      cancel();
    }
  }

  function cancel() {
    show = false;
    oncancel?.();
  }

  function confirm() {
    show = false;
    onconfirm?.();
  }

  function handleOverlayClick(e: MouseEvent) {
    if (e.target === e.currentTarget) {
      cancel();
    }
  }

  onMount(() => {
    window.addEventListener('keydown', handleKeydown);
  });

  onDestroy(() => {
    window.removeEventListener('keydown', handleKeydown);
  });

  /** Lift the dialog out of any ancestor stacking context (e.g. NavPanel
   *  whose overlay-mode animation creates a containing block via
   *  transform). Without this the modal can render clipped or behind the
   *  panel that triggered it — same fix used by `Modal.svelte`. */
  function teleportToBody(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentElement === document.body) node.remove();
      },
    };
  }
</script>

{#if show}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="q-modal-overlay" use:teleportToBody onclick={handleOverlayClick}>
    <div class="q-confirm">
      <div class="q-modal-hdr">
        <span class="q-modal-title">{title}</span>
      </div>
      <div class="q-confirm-body">{message}</div>
      <div class="q-confirm-actions">
        <button class="q-confirm-cancel" onclick={cancel}>Cancel</button>
        {#if discardText}
          <button class="q-confirm-cancel" onclick={discard}>{discardText}</button>
        {/if}
        <button
          class="q-confirm-ok"
          style="background: {confirmColor}"
          onclick={confirm}
        >{confirmText}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .q-modal-overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: rgba(0, 0, 0, 0.6);
    z-index: 9999;
    display: flex;
    align-items: center;
    justify-content: center;
    animation: fadeIn 0.15s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .q-confirm {
    width: 360px;
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
    font-size: 16px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }

  .q-confirm-body {
    padding: 18px 22px;
    font-size: 13px;
    color: var(--t2);
    line-height: 1.5;
  }

  .q-confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 22px;
    border-top: 1px solid var(--b1);
  }

  .q-confirm-cancel {
    height: 30px;
    padding: 0 16px;
    border-radius: 8px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: pointer;
    transition: border-color 0.1s, color 0.1s;
  }

  .q-confirm-cancel:hover {
    border-color: var(--b2);
    color: var(--t1);
  }

  .q-confirm-ok {
    height: 30px;
    padding: 0 16px;
    border-radius: 8px;
    border: none;
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    font-family: var(--ui);
    cursor: pointer;
    transition: opacity 0.12s;
  }

  .q-confirm-ok:hover {
    opacity: 0.85;
  }
</style>

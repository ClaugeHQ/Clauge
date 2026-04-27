<script lang="ts">
  import { toasts } from './toast';
</script>

{#if $toasts.length > 0}
  <div class="toast-container">
    {#each $toasts as toast (toast.id)}
      <div class="clauge-toast {toast.type}">
        <span class="toast-icon">
          {#if toast.type === 'success'}
            &#10003;
          {:else if toast.type === 'error'}
            &#10005;
          {:else}
            &#8505;
          {/if}
        </span>
        <span class="toast-msg">{toast.message}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-container {
    position: fixed;
    bottom: 40px;
    right: 20px;
    z-index: 1200;
    display: flex;
    flex-direction: column-reverse;
    gap: 8px;
    pointer-events: none;
  }

  .clauge-toast {
    padding: 8px 16px;
    border-radius: var(--radius-lg);
    font-family: var(--mono);
    font-size: 12px;
    color: #fff;
    display: flex;
    align-items: center;
    gap: 8px;
    animation: slideIn 0.2s ease;
    pointer-events: auto;
    white-space: nowrap;
    background: var(--n);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border: 1px solid var(--b1);
  }

  @keyframes slideIn {
    from {
      opacity: 0;
      transform: translateX(20px);
    }
    to {
      opacity: 1;
      transform: translateX(0);
    }
  }

  .clauge-toast.success {
    background: rgba(29,200,128,0.9);
    color: #061a10;
    border-color: rgba(29,200,128,0.3);
  }

  .clauge-toast.error {
    background: rgba(240,68,68,0.9);
    color: #fff;
    border-color: rgba(240,68,68,0.3);
  }

  .clauge-toast.info {
    background: rgba(79,148,212,0.9);
    color: #fff;
    border-color: rgba(79,148,212,0.3);
  }

  .toast-icon {
    font-size: 14px;
    line-height: 1;
    flex-shrink: 0;
  }

  .toast-msg {
    line-height: 1.3;
  }
</style>

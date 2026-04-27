<script lang="ts">
  import { onMount } from 'svelte';

  interface Props {
    show: boolean;
    command: string;
    reason: string;
    target: string;       // "user@host" for display
    onApprove: () => void;
    onCancel: () => void;
  }

  let { show, command, reason, target, onApprove, onCancel }: Props = $props();

  let cancelBtn: HTMLButtonElement | undefined = $state();

  // Default focus on Cancel — Enter cancels, deliberate click required for Approve.
  $effect(() => {
    if (show) {
      requestAnimationFrame(() => cancelBtn?.focus());
    }
  });

  function handleKeydown(e: KeyboardEvent) {
    if (!show) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      onCancel();
    }
  }

  onMount(() => {
    window.addEventListener('keydown', handleKeydown);
    return () => window.removeEventListener('keydown', handleKeydown);
  });
</script>

{#if show}
  <div class="ssh-modal-backdrop" role="presentation" onclick={onCancel}>
    <div class="ssh-modal" role="dialog" aria-modal="true" aria-labelledby="ssh-confirm-title" onclick={(e) => e.stopPropagation()}>
      <header class="ssh-modal-header">
        <span class="ssh-modal-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="4 17 10 11 4 5" />
            <line x1="12" y1="19" x2="20" y2="19" />
          </svg>
        </span>
        <h3 id="ssh-confirm-title">Run command on <span class="ssh-modal-target">{target}</span>?</h3>
      </header>

      <div class="ssh-modal-body">
        <pre class="ssh-modal-cmd"><code>{command}</code></pre>
        {#if reason}
          <div class="ssh-modal-reason">
            <span class="ssh-modal-reason-label">Why:</span>
            <span class="ssh-modal-reason-text">{reason}</span>
          </div>
        {/if}
        <p class="ssh-modal-note">
          Output will stream into the terminal and be sent back to the AI for follow-up.
          Secrets matching common patterns are redacted before reaching the AI.
        </p>
      </div>

      <footer class="ssh-modal-footer">
        <button type="button" class="ssh-modal-btn ssh-modal-cancel" bind:this={cancelBtn} onclick={onCancel}>Cancel</button>
        <button type="button" class="ssh-modal-btn ssh-modal-approve" onclick={onApprove}>Approve & run</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .ssh-modal-backdrop {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(4px); -webkit-backdrop-filter: blur(4px);
    display: flex; align-items: center; justify-content: center;
    z-index: 9999;
    animation: ssh-fade 0.12s ease;
  }
  @keyframes ssh-fade {
    from { opacity: 0; }
    to { opacity: 1; }
  }
  .ssh-modal {
    width: 100%;
    max-width: 540px;
    background: var(--n);
    border: 1px solid var(--b1);
    border-radius: var(--radius-lg, 10px);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.45);
    color: var(--t1);
    font-family: var(--ui);
    overflow: hidden;
  }
  .ssh-modal-header {
    display: flex; align-items: center; gap: 10px;
    padding: 14px 18px 8px;
  }
  .ssh-modal-icon { color: var(--ssh, #10b981); display: flex; }
  .ssh-modal-header h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--t1);
  }
  .ssh-modal-target {
    color: var(--ssh, #10b981);
    font-family: var(--mono);
    font-weight: 600;
  }
  .ssh-modal-body {
    padding: 8px 18px 14px;
  }
  .ssh-modal-cmd {
    margin: 0 0 12px;
    padding: 12px 14px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--b1);
    border-left: 3px solid var(--ssh, #10b981);
    border-radius: 6px;
    font-family: var(--mono);
    font-size: 12.5px;
    color: var(--t1);
    overflow-x: auto;
    white-space: pre;
  }
  .ssh-modal-cmd code { background: transparent; padding: 0; font-size: inherit; }
  .ssh-modal-reason {
    display: flex; gap: 8px; align-items: baseline;
    margin-bottom: 10px;
    font-size: 12.5px;
  }
  .ssh-modal-reason-label {
    color: var(--t3);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-size: 10px;
    padding-top: 2px;
  }
  .ssh-modal-reason-text { color: var(--t2); line-height: 1.5; }
  .ssh-modal-note {
    margin: 8px 0 0;
    color: var(--t4);
    font-size: 11.5px;
    line-height: 1.5;
  }
  .ssh-modal-footer {
    display: flex; justify-content: flex-end; gap: 8px;
    padding: 12px 18px 14px;
    border-top: 1px solid var(--b1);
    background: var(--n2);
  }
  .ssh-modal-btn {
    padding: 7px 14px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .ssh-modal-btn:hover { background: rgba(255,255,255,0.04); border-color: var(--b2); color: var(--t1); }
  .ssh-modal-btn:focus-visible {
    outline: 2px solid var(--acc);
    outline-offset: 2px;
  }
  .ssh-modal-cancel:focus-visible {
    outline-color: var(--t3);
  }
  .ssh-modal-approve {
    background: var(--ssh, #10b981);
    border-color: var(--ssh, #10b981);
    color: #042817;
    font-weight: 600;
  }
  .ssh-modal-approve:hover {
    background: color-mix(in srgb, var(--ssh, #10b981) 90%, white);
    color: #042817;
  }
</style>

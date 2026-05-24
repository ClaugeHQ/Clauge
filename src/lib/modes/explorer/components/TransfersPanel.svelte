<script lang="ts">
  /**
   * Bottom-anchored progress panel for active uploads / downloads.
   *
   * Driven by `explorerTransfers` — that store is fed by the global
   * `explorer:transfer` listener registered in `setupTransferListener`,
   * so this component is purely a renderer. Auto-hides when the list
   * is empty.
   */
  import { explorerTransfers } from '$lib/modes/explorer/stores';
  import { cancelTransfer } from '$lib/modes/explorer/commands';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';

  /** Pretty byte-size — matches FilesBrowser.formatSize but inlined here
   *  to keep this component self-contained. */
  function fmtBytes(b: number | null): string {
    if (b === null || b === undefined) return '—';
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
    if (b < 1024 * 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MB`;
    return `${(b / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  function pct(done: number, total: number | null): number {
    if (!total || total <= 0) return 0;
    return Math.min(100, Math.round((done / total) * 100));
  }

  async function handleCancel(id: string) {
    try {
      await cancelTransfer(id);
    } catch (e: any) {
      errorToast('Cancel failed', e);
    }
  }

  let collapsed = $state(false);
  // svelte-ignore non_reactive_update
  let runningCount = $derived($explorerTransfers.filter((t) => t.state === 'running').length);
</script>

{#if $explorerTransfers.length > 0}
  <div class="tp" class:collapsed>
    <button class="tp-header" onclick={() => (collapsed = !collapsed)} title={collapsed ? 'Expand' : 'Collapse'}>
      <span class="tp-title">
        {runningCount > 0 ? `Transferring (${runningCount})` : 'Transfers'}
      </span>
      <svg class="tp-caret" class:rot={collapsed} viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="6 9 12 15 18 9"/>
      </svg>
    </button>

    {#if !collapsed}
      <div class="tp-list">
        {#each $explorerTransfers as t (t.id)}
          <div class="tp-row" class:err={t.state === 'failed'} class:done={t.state === 'completed'}>
            <span class="tp-dir" title={t.direction}>
              {#if t.direction === 'upload'}
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M12 19V5"/><path d="M5 12l7-7 7 7"/>
                </svg>
              {:else}
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M12 5v14"/><path d="M5 12l7 7 7-7"/>
                </svg>
              {/if}
            </span>

            <div class="tp-body">
              <div class="tp-line">
                <span class="tp-name" title={t.direction === 'upload' ? t.localPath : t.remotePath}>{t.name}</span>
                <span class="tp-pct">
                  {#if t.state === 'running'}
                    {pct(t.bytesDone, t.bytesTotal)}%
                  {:else if t.state === 'completed'}
                    Done
                  {:else if t.state === 'cancelled'}
                    Cancelled
                  {:else if t.state === 'failed'}
                    Failed
                  {/if}
                </span>
              </div>
              <div class="tp-bar-track">
                <div
                  class="tp-bar-fill"
                  class:indet={t.state === 'running' && !t.bytesTotal}
                  style:width="{t.state === 'completed' ? 100 : pct(t.bytesDone, t.bytesTotal)}%"
                ></div>
              </div>
              <div class="tp-meta">
                {fmtBytes(t.bytesDone)}{#if t.bytesTotal} / {fmtBytes(t.bytesTotal)}{/if}
                {#if t.error}<span class="tp-err">— {t.error}</span>{/if}
              </div>
            </div>

            {#if t.state === 'running'}
              <button class="tp-cancel" onclick={() => handleCancel(t.id)} title="Cancel">
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                  <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .tp {
    position: absolute;
    right: 16px;
    bottom: 16px;
    width: 360px;
    max-height: 50%;
    background: var(--n);
    border: 1px solid var(--b1);
    border-radius: 8px;
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    z-index: 10;
    animation: tp-slide-in 0.2s ease;
  }
  @keyframes tp-slide-in {
    from { transform: translateY(8px); opacity: 0; }
    to   { transform: translateY(0);   opacity: 1; }
  }
  .tp-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 8px 12px;
    background: var(--n2);
    border: none;
    border-bottom: 1px solid var(--b1);
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12px;
    cursor: default;
  }
  .tp.collapsed .tp-header { border-bottom: none; }
  .tp-header:hover { color: var(--t1); }
  .tp-title { font-weight: 500; }
  .tp-caret { transition: transform 0.15s; }
  .tp-caret.rot { transform: rotate(-90deg); }
  .tp-list {
    overflow-y: auto;
    padding: 6px 0;
  }
  .tp-list::-webkit-scrollbar { width: 4px; }
  .tp-list::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }
  .tp-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--b-subtle);
  }
  .tp-row:last-child { border-bottom: none; }
  .tp-dir {
    color: var(--acc);
    flex-shrink: 0;
    display: inline-flex;
  }
  .tp-row.err .tp-dir { color: var(--err); }
  .tp-row.done .tp-dir { color: var(--ok, #1dc880); }
  .tp-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .tp-line { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .tp-name {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--t1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .tp-pct {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--t3);
    flex-shrink: 0;
  }
  .tp-row.err .tp-pct { color: var(--err); }
  .tp-row.done .tp-pct { color: var(--ok, #1dc880); }
  .tp-bar-track {
    height: 4px;
    background: var(--b-subtle);
    border-radius: 2px;
    overflow: hidden;
  }
  .tp-bar-fill {
    height: 100%;
    background: var(--acc);
    border-radius: 2px;
    transition: width 0.15s linear;
  }
  .tp-row.err .tp-bar-fill { background: var(--err); }
  .tp-row.done .tp-bar-fill { background: var(--ok, #1dc880); }
  /* Indeterminate sweep when total size is unknown (S3 stat sometimes
     omits size for streaming objects). */
  .tp-bar-fill.indet {
    width: 30% !important;
    animation: tp-indet 1.2s linear infinite;
  }
  @keyframes tp-indet {
    0%   { transform: translateX(-100%); }
    100% { transform: translateX(400%); }
  }
  .tp-meta {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t4);
  }
  .tp-err { color: var(--err); }
  .tp-cancel {
    width: 24px;
    height: 24px;
    border: 1px solid var(--b1);
    border-radius: 4px;
    background: transparent;
    color: var(--t3);
    cursor: default;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    transition: color 0.1s, border-color 0.1s, background 0.1s;
  }
  .tp-cancel:hover {
    color: var(--err);
    border-color: var(--err);
    background: rgba(240, 68, 68, 0.08);
  }
</style>

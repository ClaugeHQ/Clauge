<script lang="ts">
  // Conflict resolver — per-kind Merge / Keep mine / Use cloud rows, plus
  // all-kinds footer actions (Merge all / Keep my changes / Use other
  // device's) for users who don't want to decide item by item.
  //
  // Behaviour:
  //   - Reads $cloudConflicts (already kept in sync with Rust events).
  //   - On open, doesn't try to compute fancy per-kind summary stats —
  //     friendly copy + clear primary action wins over technical detail
  //     for the kinds of users hitting this case.
  //   - Per-kind resolution goes through cloudResolveKind; all-kinds
  //     resolution loops the conflicted kinds either merging,
  //     force-pushing, or pulling+importing them.
  //   - Any path that imports remote data (useRemote / merge) reloads
  //     the domain stores afterwards so the UI reflects the new local
  //     state immediately.
  //   - Mid-resolution races: if `cloud:conflicts-changed` fires while
  //     the modal is open, the body re-renders with the new list.
  import { cloudConflicts } from '$lib/stores/cloud';
  import { cloudResolveKeepLocal, cloudResolveUseRemote, cloudResolveKind, cloudGetConflicts } from '$lib/commands/cloud';
  import { reloadSyncedStores } from '$lib/commands/syncReload';
  import { showToast } from '$lib/shared/primitives/toast';
  import { kindLabel } from '$lib/shared/utils/kind-label';

  /** Teleport the modal subtree to <body>. The Settings pane is mounted
   *  inside .app-workspace which may sit beneath a transformed ancestor
   *  (or, more pragmatically, a flex container with overflow:hidden) —
   *  either of which clips a position:fixed overlay to the ancestor
   *  instead of the viewport. Re-parenting to body sidesteps the whole
   *  containing-block question. Same pattern Modal.svelte uses. */
  function teleportToBody(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentElement === document.body) node.remove();
      },
    };
  }

  interface Props {
    show: boolean;
  }

  let { show = $bindable() }: Props = $props();

  let busy = $state<'keep' | 'use' | 'merge' | null>(null);
  let kindBusy = $state<string | null>(null);

  /** Refresh the store after a resolve. The Rust resolve commands clear
   *  per-kind conflict flags but don't emit `cloud:conflicts-changed`
   *  themselves (that event fires only from the scheduler loop, which
   *  isn't on this path). Pulling the fresh list directly keeps the
   *  avatar dot + Settings "Action Required" + this modal's body all
   *  in sync without waiting for the next auto-push tick. */
  async function refreshConflictsStore() {
    try {
      const fresh = await cloudGetConflicts();
      cloudConflicts.set(fresh);
    } catch (e) {
      console.warn('[Cloud] refresh conflicts:', e);
    }
  }

  async function resolveOne(kind: string, strategy: 'merge' | 'keepLocal' | 'useRemote') {
    if (busy || kindBusy) return;
    kindBusy = kind;
    try {
      await cloudResolveKind(kind, strategy);
      await refreshConflictsStore();
      if (strategy !== 'keepLocal') await reloadSyncedStores();
      showToast(`${kindLabel(kind)} resolved`, 'success');
      if ($cloudConflicts.length === 0) show = false;
    } catch (e: any) {
      showToast(`Couldn’t resolve ${kindLabel(kind)}: ${e?.message ?? e}`, 'error');
    } finally {
      kindBusy = null;
    }
  }

  async function mergeAll() {
    if (busy || kindBusy) return;
    busy = 'merge';
    try {
      for (const k of [...$cloudConflicts]) await cloudResolveKind(k, 'merge');
      await refreshConflictsStore();
      await reloadSyncedStores();
      showToast('Merged both devices’ changes', 'success');
      if ($cloudConflicts.length === 0) show = false;
    } catch (e: any) {
      showToast(`Couldn’t merge: ${e?.message ?? e}`, 'error');
    } finally {
      busy = null;
    }
  }

  async function keepLocal() {
    if (busy || kindBusy) return;
    busy = 'keep';
    try {
      await cloudResolveKeepLocal();
      await refreshConflictsStore();
      showToast('Kept this device’s version', 'success');
      show = false;
    } catch (e: any) {
      showToast(`Couldn’t resolve: ${e?.message ?? e}`, 'error');
    } finally {
      busy = null;
    }
  }

  async function useRemote() {
    if (busy || kindBusy) return;
    busy = 'use';
    try {
      await cloudResolveUseRemote();
      await refreshConflictsStore();
      await reloadSyncedStores();
      showToast('Used the other device’s version', 'success');
      show = false;
    } catch (e: any) {
      showToast(`Couldn’t resolve: ${e?.message ?? e}`, 'error');
    } finally {
      busy = null;
    }
  }

  function close() {
    if (busy || kindBusy) return;
    show = false;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && show) {
      e.preventDefault();
      close();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if show}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="cr-overlay" use:teleportToBody onclick={close}>
    <div class="cr-modal modal-card" onclick={(e: MouseEvent) => e.stopPropagation()} role="dialog" aria-modal="true">
      <header class="cr-hdr">
        <span class="cr-title">Some changes need your attention</span>
        <button class="cr-close" onclick={close} aria-label="Close" disabled={!!busy}>&times;</button>
      </header>

      <div class="cr-body">
        <p class="cr-lead">
          Your other device made changes while you were editing on this one.
          Resolve each item below, or use a footer button to apply one
          choice to everything that diverged.
        </p>

        {#if $cloudConflicts.length === 0}
          <div class="cr-affected">
            <span class="cr-affected-list">Nothing to resolve.</span>
          </div>
        {:else}
          <div class="cr-rows">
            {#each $cloudConflicts as kind (kind)}
              <div class="cr-row">
                <span class="cr-row-label">{kindLabel(kind)}</span>
                <div class="cr-row-actions">
                  <button
                    class="cr-row-btn cr-row-btn-primary"
                    onclick={() => resolveOne(kind, 'merge')}
                    disabled={!!busy || !!kindBusy}
                  >
                    {kindBusy === kind ? 'Resolving…' : 'Merge'}
                  </button>
                  <button
                    class="cr-row-btn"
                    onclick={() => resolveOne(kind, 'keepLocal')}
                    disabled={!!busy || !!kindBusy}
                  >
                    Keep mine
                  </button>
                  <button
                    class="cr-row-btn"
                    onclick={() => resolveOne(kind, 'useRemote')}
                    disabled={!!busy || !!kindBusy}
                  >
                    Use cloud
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}

        <p class="cr-warn">
          A snapshot of this device's data is saved before any change —
          Settings → Account → Local snapshots.
        </p>
      </div>

      <footer class="cr-foot">
        <button class="cr-btn cr-btn-secondary" onclick={close} disabled={!!busy || !!kindBusy}>
          Close
        </button>
        <button
          class="cr-btn cr-btn-secondary"
          onclick={useRemote}
          disabled={!!busy || !!kindBusy || $cloudConflicts.length === 0}
        >
          {busy === 'use' ? 'Applying…' : 'Use other device’s'}
        </button>
        <button
          class="cr-btn cr-btn-secondary"
          onclick={keepLocal}
          disabled={!!busy || !!kindBusy || $cloudConflicts.length === 0}
        >
          {busy === 'keep' ? 'Saving…' : 'Keep my changes'}
        </button>
        <button
          class="cr-btn cr-btn-primary"
          onclick={mergeAll}
          disabled={!!busy || !!kindBusy || $cloudConflicts.length === 0}
        >
          {busy === 'merge' ? 'Merging…' : 'Merge all'}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .cr-overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim-strong);
    z-index: var(--z-modal);
    display: flex;
    align-items: center;
    justify-content: center;
    animation: cr-fade 0.15s ease;
  }
  @keyframes cr-fade {
    from { opacity: 0; }
    to   { opacity: 1; }
  }
  .cr-modal {
    width: min(520px, 92vw);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: cr-rise 0.18s ease;
  }
  @keyframes cr-rise {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to   { opacity: 1; transform: none; }
  }
  .cr-hdr {
    display: flex;
    align-items: center;
    padding: 14px 18px;
    border-bottom: 1px solid var(--b1);
    background: var(--e);
  }
  .cr-title {
    font-size: 14.5px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }
  .cr-close {
    margin-left: auto;
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t3);
    font-size: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: default;
    transition: background 0.12s, color 0.12s;
  }
  .cr-close:hover { background: var(--c); color: var(--t1); }
  .cr-close:disabled { opacity: 0.4; }

  .cr-body {
    padding: 18px 22px;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 13px;
    line-height: 1.55;
  }
  .cr-lead { margin: 0 0 14px; }
  .cr-affected {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 10px 12px;
    margin-bottom: 12px;
    border: 1px solid var(--b1);
    border-radius: 8px;
    background: var(--surface-hover);
    font-size: 12.5px;
  }
  .cr-affected-label {
    color: var(--t3);
    font-weight: 600;
  }
  .cr-affected-list { color: var(--t1); }

  .cr-rows {
    display: flex;
    flex-direction: column;
    margin-bottom: 12px;
    border: 1px solid var(--b1);
    border-radius: 8px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .cr-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
  }
  .cr-row + .cr-row {
    border-top: 1px solid var(--b1);
  }
  .cr-row-label {
    color: var(--t1);
    font-size: 12.5px;
    font-weight: 500;
    flex: 1;
    min-width: 0;
  }
  .cr-row-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
  .cr-row-btn {
    height: 24px;
    padding: 0 10px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 11.5px;
    font-weight: 500;
    cursor: default;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .cr-row-btn:disabled { opacity: 0.5; }
  .cr-row-btn:hover:not(:disabled) {
    background: var(--surface-hover);
    border-color: var(--b2);
  }
  .cr-row-btn-primary {
    background: var(--acc);
    border-color: var(--acc);
    color: #fff;
    font-weight: 600;
  }
  .cr-row-btn-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .cr-warn {
    margin: 0;
    padding: 10px 12px;
    border-radius: 8px;
    border: 1px solid color-mix(in srgb, var(--warn, #f5a623) 30%, var(--b1));
    background: color-mix(in srgb, var(--warn, #f5a623) 8%, transparent);
    color: var(--t2);
    font-size: 12px;
  }

  .cr-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px 16px;
    border-top: 1px solid var(--b1);
  }
  .cr-btn {
    height: 32px;
    padding: 0 14px;
    border-radius: 7px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 12.5px;
    font-weight: 500;
    cursor: default;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .cr-btn:disabled { opacity: 0.5; }
  .cr-btn-secondary:hover:not(:disabled) {
    background: var(--surface-hover);
    border-color: var(--b2);
  }
  .cr-btn-primary {
    background: var(--acc);
    border-color: var(--acc);
    color: #fff;
    font-weight: 600;
  }
  .cr-btn-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
</style>

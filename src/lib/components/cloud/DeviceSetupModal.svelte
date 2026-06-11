<script lang="ts">
  // Set up this device — shown when BOTH this device and the cloud account
  // contain data and the device has never synced. The user picks how to
  // combine them; a snapshot is taken Rust-side before anything destructive.
  //
  //   merge → cloudMergeAll      (keeps both, newer edits win)
  //   cloud → cloudSyncRestore   (cloud copy replaces this device)
  //   keep  → cloudForcePushAll  (this device overwrites the cloud)
  //
  // All three mark has-synced server-side; markSynced() updates the
  // in-memory store. Closing without choosing is allowed — hasSyncedOnce
  // stays false so the modal reappears on the next boot.
  import { showDeviceSetup, cloudUser, markSynced } from '$lib/stores/cloud';
  import { cloudMergeAll, cloudForcePushAll, cloudSyncRestore } from '$lib/commands/cloud';
  import { reloadSyncedStores } from '$lib/commands/syncReload';
  import { announceRestoreCompletion } from '$lib/stores/missingCredentials';
  import { showToast } from '$lib/shared/primitives/toast';

  /** Teleport the modal subtree to <body>. Same pattern as
   *  ConflictResolverModal / Modal.svelte — sidesteps clipping by
   *  transformed or overflow:hidden ancestors. */
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

  let busy = $state<'merge' | 'keep' | 'cloud' | null>(null);
  let selected = $state<'merge' | 'cloud' | 'keep' | null>('merge');

  const busyLabels = { merge: 'Merging…', cloud: 'Restoring…', keep: 'Pushing…' } as const;

  async function run(choice: 'merge' | 'keep' | 'cloud') {
    if (busy) return;
    busy = choice;
    try {
      if (choice === 'merge') await cloudMergeAll();
      else if (choice === 'keep') await cloudForcePushAll();
      else await cloudSyncRestore();
      markSynced();
      await reloadSyncedStores();
      // 'merge' and 'cloud' both import remote rows whose secrets were
      // stripped server-side — surface the same credentials notice the
      // Sidebar restore flow shows. 'keep' only pushes, nothing imported.
      if (choice !== 'keep') await announceRestoreCompletion();
      showToast('Device set up', 'success');
      showDeviceSetup.set(false);
    } catch (e: any) {
      showToast(`Setup failed: ${e?.message ?? e}`, 'error');
    } finally {
      busy = null;
    }
  }

  function close() {
    if (busy) return;
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
  <div class="ds-overlay" use:teleportToBody onclick={close}>
    <div class="ds-modal modal-card" onclick={(e: MouseEvent) => e.stopPropagation()} role="dialog" aria-modal="true">
      <header class="ds-hdr">
        <div class="ds-hdr-text">
          <span class="ds-title">Set up this device</span>
          {#if $cloudUser}
            <span class="ds-account">
              Signed in as {$cloudUser.email || $cloudUser.displayName || $cloudUser.slug}
            </span>
          {/if}
        </div>
        <button class="ds-close" onclick={close} aria-label="Close" disabled={!!busy}>&times;</button>
      </header>

      <div class="ds-body">
        <p class="ds-lead">
          Both this device and your cloud account contain data. Choose how
          to combine them — a snapshot of this device is saved before any
          change.
        </p>

        <div class="ds-options">
          <button
            class="ds-option"
            class:selected={selected === 'merge'}
            aria-pressed={selected === 'merge'}
            onclick={() => (selected = 'merge')}
            disabled={!!busy}
          >
            <span class="ds-option-head">
              <span class="ds-radio" aria-hidden="true"></span>
              <span class="ds-option-title">Merge (recommended)</span>
            </span>
            <span class="ds-option-caption">Keeps everything from both devices. Newer edits win.</span>
          </button>
          <button
            class="ds-option"
            class:selected={selected === 'cloud'}
            aria-pressed={selected === 'cloud'}
            onclick={() => (selected = 'cloud')}
            disabled={!!busy}
          >
            <span class="ds-option-head">
              <span class="ds-radio" aria-hidden="true"></span>
              <span class="ds-option-title">Use cloud copy</span>
            </span>
            <span class="ds-option-caption">Replace this device's data with the cloud copy.</span>
          </button>
          <button
            class="ds-option"
            class:selected={selected === 'keep'}
            aria-pressed={selected === 'keep'}
            onclick={() => (selected = 'keep')}
            disabled={!!busy}
          >
            <span class="ds-option-head">
              <span class="ds-radio" aria-hidden="true"></span>
              <span class="ds-option-title">Keep this device's data</span>
            </span>
            <span class="ds-option-caption">Overwrite the cloud with this device's data.</span>
          </button>
        </div>
      </div>

      <footer class="ds-foot">
        <button class="ds-later" onclick={close} disabled={!!busy}>
          Decide later
        </button>
        <button
          class="ds-continue"
          onclick={() => selected && run(selected)}
          disabled={!selected || !!busy}
        >
          {busy ? busyLabels[busy] : 'Continue'}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .ds-overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim-strong);
    z-index: var(--z-modal);
    display: flex;
    align-items: center;
    justify-content: center;
    animation: ds-fade 0.15s ease;
  }
  @keyframes ds-fade {
    from { opacity: 0; }
    to   { opacity: 1; }
  }
  .ds-modal {
    width: min(520px, 92vw);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: ds-rise 0.18s ease;
  }
  @keyframes ds-rise {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to   { opacity: 1; transform: none; }
  }
  .ds-hdr {
    display: flex;
    align-items: center;
    padding: 14px 18px;
    border-bottom: 1px solid var(--b1);
    background: var(--e);
  }
  .ds-hdr-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ds-title {
    font-size: 14.5px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }
  .ds-account {
    font-size: 11.5px;
    color: var(--t3);
    font-family: var(--ui);
  }
  .ds-close {
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
  .ds-close:hover { background: var(--c); color: var(--t1); }
  .ds-close:disabled { opacity: 0.4; }

  .ds-body {
    padding: 18px 22px;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 13px;
    line-height: 1.55;
  }
  .ds-lead { margin: 0 0 14px; }

  .ds-options {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .ds-option {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 3px;
    text-align: left;
    padding: 10px 14px;
    border-radius: 8px;
    border: 1px solid var(--b1);
    background: transparent;
    font-family: var(--ui);
    cursor: default;
    transition: background 0.12s, border-color 0.12s;
  }
  .ds-option:disabled { opacity: 0.5; }
  .ds-option:hover:not(:disabled) {
    background: var(--surface-hover);
    border-color: var(--b2);
  }
  .ds-option.selected {
    border-color: var(--acc);
    background: color-mix(in srgb, var(--acc) 10%, transparent);
  }
  .ds-option.selected:hover:not(:disabled) {
    border-color: var(--acc);
    background: color-mix(in srgb, var(--acc) 14%, transparent);
  }
  .ds-option-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ds-radio {
    flex: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1.5px solid var(--b2);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: border-color 0.12s;
  }
  .ds-radio::after {
    content: '';
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--acc);
    transform: scale(0);
    transition: transform 0.12s;
  }
  .ds-option.selected .ds-radio { border-color: var(--acc); }
  .ds-option.selected .ds-radio::after { transform: scale(1); }
  .ds-option-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--t1);
  }
  .ds-option-caption {
    font-size: 12px;
    color: var(--t3);
    padding-left: 22px;
  }

  .ds-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 18px 16px;
    border-top: 1px solid var(--b1);
  }
  .ds-later {
    height: 28px;
    padding: 0 10px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 12px;
    cursor: default;
    transition: color 0.12s, background 0.12s;
  }
  .ds-later:hover:not(:disabled) { color: var(--t1); background: var(--surface-hover); }
  .ds-later:disabled { opacity: 0.5; }
  .ds-continue {
    height: 28px;
    padding: 0 14px;
    border-radius: 6px;
    border: 1px solid var(--acc);
    background: var(--acc);
    color: #fff;
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 600;
    cursor: default;
    transition: filter 0.12s, opacity 0.12s;
  }
  .ds-continue:hover:not(:disabled) { filter: brightness(1.08); }
  .ds-continue:disabled { opacity: 0.5; }
</style>

<script lang="ts">
  import { onMount } from 'svelte';
  import { tabs, activeTabId } from '$lib/shared/stores/tabs';
  import {
    explorerConnections,
    activeExplorerConnection,
    explorerConnStates,
    loadExplorerConnections,
    setupTransferListener,
  } from '$lib/modes/explorer/stores';
  import { connectionIdFromTabKey } from '$lib/modes/explorer/tabkey';
  import FilesBrowser from './FilesBrowser.svelte';
  import TransfersPanel from './TransfersPanel.svelte';

  // The set of explorer tabs currently open. Each tab has an associated
  // session held in Rust state (keyed by tabKey).
  const explorerTabs = $derived($tabs.filter((t) => t.mode === 'explorer'));
  const activeTab = $derived(
    explorerTabs.find((t) => t.id === $activeTabId) ?? null,
  );
  const activeTabKey = $derived(activeTab?.key ?? null);

  // Whenever the active tab changes, sync activeExplorerConnection so other
  // components (nav, AI panel, etc.) see the same connection.
  $effect(() => {
    if (!activeTabKey) {
      activeExplorerConnection.set(null);
      return;
    }
    const connId = connectionIdFromTabKey(activeTabKey);
    const conn = $explorerConnections.find((c) => c.id === connId) ?? null;
    activeExplorerConnection.set(conn);
  });

  onMount(() => {
    loadExplorerConnections();
    // Idempotent — multiple mounts won't double-register.
    setupTransferListener();
  });
</script>

<div class="explorer-panel">
  {#if activeTab && activeTabKey}
    {#key activeTabKey}
      <FilesBrowser tabKey={activeTabKey} connectionId={connectionIdFromTabKey(activeTabKey)} />
    {/key}
  {:else}
    <div class="empty">
      <div class="empty-icon">
        <svg viewBox="0 0 24 24" width="40" height="40"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" stroke="var(--t4)" fill="none" stroke-width="1.5" stroke-linejoin="round"/></svg>
      </div>
      <p class="empty-text">Pick or add a connection to start browsing</p>
      <p class="empty-hint">SFTP &middot; FTP &middot; S3-compatible &middot; Azure Blob</p>
    </div>
  {/if}

  <!-- Floating progress panel — visible whenever any transfer is active
       across any explorer tab. Auto-hides when empty. -->
  <TransfersPanel />
</div>

<style>
  .explorer-panel {
    flex: 1;
    display: flex;
    overflow: hidden;
    min-height: 0;
    position: relative; /* anchor for the floating TransfersPanel */
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--t3);
  }
  .empty-icon { opacity: 0.5; margin-bottom: 4px; }
  .empty-text { font-size: 13px; font-family: var(--ui); color: var(--t2); margin: 0; }
  .empty-hint { font-size: 11px; font-family: var(--mono); color: var(--t3); margin: 0; }
</style>

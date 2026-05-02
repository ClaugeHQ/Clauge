<script lang="ts">
  import { mode } from '$lib/stores/app';
  import { activeHistoryEntry } from '$lib/modes/rest/stores';
  import AgentPanel from '$lib/modes/agent/components/AgentPanel.svelte';
  import RestPanel from '$lib/modes/rest/components/RestPanel.svelte';
  import SqlPanel from '$lib/modes/sql/components/SqlPanel.svelte';
  import NoSqlPanel from '$lib/modes/nosql/components/NoSqlPanel.svelte';
  import SshPanel from '$lib/modes/ssh/components/SshPanel.svelte';
  import ExplorerPanel from '$lib/modes/explorer/components/ExplorerPanel.svelte';
  import HistoryViewer from '$lib/modes/rest/components/HistoryViewer.svelte';
</script>

<div class="workspace">
  {#if $mode === 'agent'}
    <AgentPanel />
  {:else if $mode === 'history'}
    {#if $activeHistoryEntry}
      <HistoryViewer />
    {:else}
      <div class="history-empty">
        <svg viewBox="0 0 24 24" width="36" height="36"><circle cx="12" cy="12" r="10" stroke="var(--t4)" fill="none" stroke-width="1.2"/><path d="M12 6v6l4 2" stroke="var(--t4)" fill="none" stroke-width="1.2" stroke-linecap="round"/></svg>
        <span>Select an entry from history to view details</span>
      </div>
    {/if}
  {:else if $mode === 'rest'}
    <RestPanel />
  {:else if $mode === 'sql'}
    <SqlPanel />
  {:else if $mode === 'nosql'}
    <NoSqlPanel />
  {:else if $mode === 'ssh'}
    <SshPanel />
  {:else if $mode === 'explorer'}
    <ExplorerPanel />
  {/if}
</div>

<style>
  .workspace {
    flex: 1;
    display: flex;
    min-height: 0;
    overflow: hidden;
  }
  .history-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--t3);
    font-size: 13px;
    font-family: var(--ui);
  }
</style>

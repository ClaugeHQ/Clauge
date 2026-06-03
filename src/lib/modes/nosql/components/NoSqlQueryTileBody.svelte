<script lang="ts">
  import {
    nosqlTabState,
    nosqlConnections,
    connectedNoSqlIds,
    nosqlLiveConnectionIds,
  } from '../stores';
  import DocumentViewer from './DocumentViewer.svelte';
  import RedisViewer from './RedisViewer.svelte';

  let { tabId }: { tabId: number } = $props();

  const tabData = $derived($nosqlTabState.get(tabId));
  const conn = $derived(
    tabData ? $nosqlConnections.find((c) => c.id === tabData.connectionId) ?? null : null,
  );
  const isConnected = $derived(conn ? $connectedNoSqlIds.has(conn.id) : false);
  const liveId = $derived(conn ? $nosqlLiveConnectionIds[conn.id] ?? null : null);
  const isRedis = $derived(conn?.driver === 'redis');
  const database = $derived(tabData?.database ?? '');
  const collection = $derived(tabData?.collection ?? '');
  const filter = $derived(tabData?.filterQuery ?? '{}');
</script>

<div class="cv-nosql-tile">
  {#if !conn}
    <div class="cv-nosql-empty">No connection bound to this tab</div>
  {:else if !isConnected || !liveId}
    <div class="cv-nosql-empty">
      "{conn.name}" is not connected<br />
      <span class="cv-nosql-hint">Open this tab in NoSQL mode to connect</span>
    </div>
  {:else if isRedis}
    <RedisViewer connectionId={liveId} />
  {:else if database && collection}
    <DocumentViewer
      connectionId={liveId}
      {database}
      {collection}
      initialFilter={filter}
    />
  {:else}
    <div class="cv-nosql-empty">
      Select a database and collection<br />
      <span class="cv-nosql-hint">Open this tab in NoSQL mode to pick one</span>
    </div>
  {/if}
</div>

<style>
  .cv-nosql-tile {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 0;
    min-width: 0;
  }
  .cv-nosql-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-size: 12px;
    color: var(--t3);
    font-family: var(--mono);
    text-align: center;
    line-height: 1.6;
  }
  .cv-nosql-hint {
    font-size: 11px;
    color: var(--t4);
  }
</style>

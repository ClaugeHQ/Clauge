<script lang="ts">
  import {
    nosqlTabState,
    nosqlConnections,
    connectedNoSqlIds,
    nosqlLiveConnectionIds,
    setNoSqlTabData,
    activeNoSqlConnectionId,
  } from '../stores';
  import { nosqlListDatabases, nosqlListCollections } from '../commands';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';
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
  const tileDriver = $derived<'mongodb' | 'redis'>(conn?.driver === 'redis' ? 'redis' : 'mongodb');
  const database = $derived(tabData?.database ?? '');
  const collection = $derived(tabData?.collection ?? '');
  const filter = $derived(tabData?.filterQuery ?? '{}');

  let dbCache = $state<Map<string, string[]>>(new Map());
  let collCache = $state<Map<string, string[]>>(new Map());
  let loadingDbs = $state(false);
  let loadingColls = $state(false);

  const driverConns = $derived(
    $nosqlConnections.filter((c) => c.driver === tileDriver),
  );

  interface ConnGroup {
    connId: string;
    connName: string;
    databases: string[];
  }

  const connGroups = $derived.by<ConnGroup[]>(() => {
    if (tileDriver !== 'mongodb') return [];
    const groups: ConnGroup[] = [];
    for (const c of $nosqlConnections) {
      if (c.driver !== 'mongodb' || !$connectedNoSqlIds.has(c.id)) continue;
      groups.push({ connId: c.id, connName: c.name, databases: dbCache.get(c.id) ?? [] });
    }
    return groups;
  });

  $effect(() => {
    if (tileDriver !== 'mongodb') return;
    for (const c of $nosqlConnections) {
      if (c.driver !== 'mongodb' || !$connectedNoSqlIds.has(c.id)) continue;
      const live = $nosqlLiveConnectionIds[c.id];
      if (live && !dbCache.has(c.id)) {
        loadDatabases(c.id, live);
      }
    }
  });

  let prevCollKey = '';
  $effect(() => {
    if (tileDriver !== 'mongodb') return;
    const connId = tabData?.connectionId ?? '';
    const live = connId ? $nosqlLiveConnectionIds[connId] ?? null : null;
    const key = `${connId}:${database}`;
    if (live && database && key !== prevCollKey) {
      prevCollKey = key;
      if (!collCache.has(key)) {
        queueMicrotask(() => loadCollections(connId, database));
      }
    }
  });

  const collections = $derived(
    collCache.get(`${tabData?.connectionId ?? ''}:${database}`) ?? [],
  );

  async function loadDatabases(connId: string, live: string) {
    loadingDbs = true;
    try {
      const dbs = await nosqlListDatabases(live);
      dbCache = new Map([...dbCache, [connId, dbs]]);
    } catch (e) {
      showToast(friendlyError(e), 'error');
    } finally {
      loadingDbs = false;
    }
  }

  async function loadCollections(connId: string, db: string) {
    const live = $nosqlLiveConnectionIds[connId];
    if (!live) return;
    const key = `${connId}:${db}`;
    loadingColls = true;
    try {
      const colls = await nosqlListCollections(live, db);
      collCache = new Map([...collCache, [key, colls]]);
      if (colls.length > 0 && !collection) {
        setNoSqlTabData(tabId, { collection: colls[0] });
      }
    } catch (e) {
      showToast(friendlyError(e), 'error');
    } finally {
      loadingColls = false;
    }
  }

  let connDropdownOpen = $state(false);
  let dbDropdownOpen = $state(false);

  function toggleConnDropdown(e: MouseEvent) {
    e.stopPropagation();
    connDropdownOpen = !connDropdownOpen;
    dbDropdownOpen = false;
  }

  function toggleDbDropdown(e: MouseEvent) {
    e.stopPropagation();
    dbDropdownOpen = !dbDropdownOpen;
    connDropdownOpen = false;
  }

  function handleWindowClick() {
    connDropdownOpen = false;
    dbDropdownOpen = false;
  }

  function pickConnection(connId: string) {
    if (connId === (tabData?.connectionId ?? '')) {
      connDropdownOpen = false;
      return;
    }
    activeNoSqlConnectionId.set(connId);
    setNoSqlTabData(tabId, {
      connectionId: connId,
      database: '',
      collection: '',
      filterQuery: '{}',
    });
    connDropdownOpen = false;
  }

  function pickDatabase(connId: string, db: string) {
    activeNoSqlConnectionId.set(connId);
    setNoSqlTabData(tabId, {
      connectionId: connId,
      database: db,
      collection: '',
      filterQuery: '{}',
    });
    dbDropdownOpen = false;
  }

  function handleCollChange(e: Event) {
    const val = (e.target as HTMLSelectElement).value;
    setNoSqlTabData(tabId, { collection: val, filterQuery: '{}' });
  }
</script>

<svelte:window onclick={handleWindowClick} />

<div class="cv-nosql-tile">
  <div class="cv-nosql-bar">
    <div class="cv-pill-wrap">
      <button class="cv-pill cv-pill-conn" onclick={toggleConnDropdown}>
        <span class="cv-pill-driver">{tileDriver === 'redis' ? 'REDIS' : 'MONGO'}</span>
        <svg viewBox="0 0 24 24" width="12" height="12">
          {#if tileDriver === 'redis'}
            <path d="M3 6h18M3 12h18M3 18h18" />
          {:else}
            <ellipse cx="12" cy="5" rx="9" ry="3" />
            <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
            <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
          {/if}
        </svg>
        <span class="cv-pill-name">{conn?.name ?? 'Select connection'}</span>
        {#if conn && !isConnected}
          <span class="cv-pill-state">offline</span>
        {/if}
        <svg class="cv-pill-chev" viewBox="0 0 24 24" width="10" height="10"><path d="M6 9l6 6 6-6" /></svg>
      </button>
      {#if connDropdownOpen}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="cv-dd" onclick={(e) => e.stopPropagation()}>
          {#if driverConns.length === 0}
            <div class="cv-dd-empty">No {tileDriver === 'redis' ? 'Redis' : 'MongoDB'} connections</div>
          {:else}
            {#each driverConns as c}
              <button
                class="cv-dd-item"
                class:active={c.id === (tabData?.connectionId ?? '')}
                onclick={() => pickConnection(c.id)}
              >
                <span
                  class="cv-dd-dot"
                  class:on={$connectedNoSqlIds.has(c.id)}
                ></span>
                <span class="cv-dd-name">{c.name}</span>
              </button>
            {/each}
          {/if}
        </div>
      {/if}
    </div>

    {#if !isRedis && conn}
      <span class="cv-bar-sep">/</span>
      <div class="cv-pill-wrap">
        <button
          class="cv-pill cv-pill-db"
          onclick={toggleDbDropdown}
          disabled={!isConnected}
        >
          <svg viewBox="0 0 24 24" width="12" height="12">
            <ellipse cx="12" cy="5" rx="9" ry="3" />
            <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
            <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
          </svg>
          <span class="cv-pill-name">{database || (loadingDbs ? 'Loading...' : 'Select database')}</span>
          <svg class="cv-pill-chev" viewBox="0 0 24 24" width="10" height="10"><path d="M6 9l6 6 6-6" /></svg>
        </button>
        {#if dbDropdownOpen}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="cv-dd" onclick={(e) => e.stopPropagation()}>
            {#if connGroups.length === 0}
              <div class="cv-dd-empty">No databases available</div>
            {:else}
              {#each connGroups as group, gi}
                {#if gi > 0}<div class="cv-dd-sep"></div>{/if}
                <div class="cv-dd-group">{group.connName}</div>
                {#if group.databases.length === 0}
                  <div class="cv-dd-empty-sub">Loading databases...</div>
                {:else}
                  {#each group.databases as db}
                    <button
                      class="cv-dd-item cv-dd-item-sub"
                      class:active={(tabData?.connectionId ?? '') === group.connId && database === db}
                      onclick={() => pickDatabase(group.connId, db)}
                    >
                      <span class="cv-dd-name">{db}</span>
                    </button>
                  {/each}
                {/if}
              {/each}
            {/if}
          </div>
        {/if}
      </div>
      <span class="cv-bar-sep">/</span>
      <select
        class="cv-coll-sel"
        value={collection}
        onchange={handleCollChange}
        disabled={loadingColls || !database}
      >
        {#if collections.length === 0}
          <option value="">{loadingColls ? 'Loading...' : 'No collections'}</option>
        {/if}
        {#each collections as coll}
          <option value={coll}>{coll}</option>
        {/each}
      </select>
    {/if}
  </div>

  <div class="cv-nosql-body">
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
        <span class="cv-nosql-hint">Use the pickers above</span>
      </div>
    {/if}
  </div>
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
  .cv-nosql-body {
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

  .cv-nosql-bar {
    height: 34px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    padding: 0 10px;
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
  }
  .cv-bar-sep {
    color: var(--t4);
    font-size: 13px;
    font-family: var(--mono);
    flex-shrink: 0;
  }
  .cv-pill-wrap {
    position: relative;
  }
  .cv-pill {
    height: 24px;
    padding: 0 8px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: var(--surface-hover);
    color: var(--t1);
    font-size: 12px;
    font-family: var(--mono);
    cursor: default;
    display: flex;
    align-items: center;
    gap: 6px;
    transition: border-color 0.15s;
    max-width: 280px;
  }
  .cv-pill:hover:not(:disabled) {
    border-color: var(--b2);
  }
  .cv-pill:disabled {
    opacity: 0.55;
  }
  .cv-pill svg:not(.cv-pill-chev) {
    stroke: var(--acc);
    fill: none;
    stroke-width: 1.5;
    stroke-linecap: round;
    flex-shrink: 0;
  }
  .cv-pill-driver {
    font-size: 9px;
    color: var(--t2);
    font-family: var(--ui);
    padding: 2px 5px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--acc) 18%, transparent);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .cv-pill-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 160px;
  }
  .cv-pill-state {
    font-size: 9px;
    color: var(--t3);
    font-family: var(--ui);
    padding: 2px 5px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--err) 14%, transparent);
    letter-spacing: 0.04em;
  }
  .cv-pill-chev {
    stroke: var(--t3);
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex-shrink: 0;
  }

  .cv-dd {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    min-width: 220px;
    max-height: 320px;
    overflow-y: auto;
    background: var(--modal-bg, var(--n));
    border: 1px solid var(--b2);
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.55);
    z-index: 500;
    padding: 4px 0;
  }
  .cv-dd::-webkit-scrollbar { width: 4px; }
  .cv-dd::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }
  .cv-dd-empty,
  .cv-dd-empty-sub {
    padding: 8px 12px;
    font-size: 11px;
    color: var(--t4);
    font-family: var(--mono);
  }
  .cv-dd-empty-sub { padding-left: 28px; }
  .cv-dd-group {
    padding: 6px 12px 4px;
    font-size: 10px;
    font-weight: 500;
    color: var(--t3);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .cv-dd-sep {
    height: 1px;
    background: var(--b1);
    margin: 4px 0;
  }
  .cv-dd-item {
    width: 100%;
    padding: 6px 12px;
    font-size: 12px;
    color: var(--t2);
    font-family: var(--mono);
    border: none;
    background: transparent;
    cursor: default;
    text-align: left;
    display: flex;
    align-items: center;
    gap: 8px;
    transition: background 0.08s;
  }
  .cv-dd-item-sub { padding-left: 28px; }
  .cv-dd-item:hover {
    background: color-mix(in srgb, var(--acc) 8%, transparent);
    color: var(--t1);
  }
  .cv-dd-item.active {
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    color: var(--t1);
  }
  .cv-dd-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--t4);
    flex-shrink: 0;
  }
  .cv-dd-dot.on {
    background: var(--ok, #2bd97c);
  }
  .cv-dd-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cv-coll-sel {
    height: 24px;
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 0 22px 0 8px;
    font-size: 12px;
    color: var(--t1);
    font-family: var(--mono);
    outline: none;
    cursor: default;
    min-width: 110px;
    max-width: 200px;
    transition: border-color 0.15s;
    -webkit-appearance: none;
    appearance: none;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23b0b0c8' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'><polyline points='3 5 6 8 9 5'/></svg>");
    background-repeat: no-repeat;
    background-position: right 7px center;
    background-size: 9px 9px;
  }
  .cv-coll-sel:focus { border-color: var(--acc); }
  .cv-coll-sel:disabled { opacity: 0.55; }
</style>

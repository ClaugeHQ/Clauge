<script lang="ts">
  import { activeNoSqlConnection, connectedNoSqlIds, nosqlLiveConnectionIds, nosqlTabState, setNoSqlTabData, nosqlConnections, activeNoSqlConnectionId, insertNoSqlQueryText, aiExecuteNoSqlQuery } from '../stores';
  import { nosqlListDatabases, nosqlListCollections as listColls } from '../commands';
  import { showToast } from '$lib/components/shared/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { tabs, activeTabId, addTab } from '$lib/stores/tabs';
  import DocumentViewer from './DocumentViewer.svelte';
  import RedisViewer from './RedisViewer.svelte';

  // Caches: connId → databases, connId:db → collections
  let dbCache = $state<Map<string, string[]>>(new Map());
  let collCache = $state<Map<string, string[]>>(new Map());
  let loadingDbs = $state(false);
  let loadingColls = $state(false);

  const conn = $derived($activeNoSqlConnection);
  const isConnected = $derived(conn ? $connectedNoSqlIds.has(conn.id) : false);
  const isRedis = $derived(conn?.driver === 'redis');
  const liveId = $derived(conn ? $nosqlLiveConnectionIds[conn.id] ?? null : null);

  // Active NoSQL tab
  const activeNoSqlTab = $derived($tabs.find(t => t.id === $activeTabId && t.mode === 'nosql'));
  const tabData = $derived(activeNoSqlTab ? $nosqlTabState.get(activeNoSqlTab.id) : undefined);

  // Per-tab selections
  const tabConnId = $derived(tabData?.connectionId ?? '');
  const tabLiveId = $derived(tabConnId ? $nosqlLiveConnectionIds[tabConnId] ?? null : null);
  const selectedDb = $derived(tabData?.database ?? '');
  const selectedColl = $derived(tabData?.collection ?? '');

  // All connected MongoDB connections for the grouped dropdown
  interface ConnGroup {
    connId: string;
    connName: string;
    databases: string[];
  }

  const connGroups = $derived.by(() => {
    const groups: ConnGroup[] = [];
    for (const c of $nosqlConnections) {
      if (c.driver !== 'mongodb' || !$connectedNoSqlIds.has(c.id)) continue;
      const dbs = dbCache.get(c.id) ?? [];
      groups.push({ connId: c.id, connName: c.name, databases: dbs });
    }
    return groups;
  });

  // Load databases for all connected MongoDB instances
  $effect(() => {
    for (const c of $nosqlConnections) {
      if (c.driver !== 'mongodb' || !$connectedNoSqlIds.has(c.id)) continue;
      const live = $nosqlLiveConnectionIds[c.id];
      if (live && !dbCache.has(c.id)) {
        loadDatabases(c.id, live);
      }
    }
  });

  // Load collections when tab's database changes
  let prevCollKey = '';
  $effect(() => {
    const key = `${tabConnId}:${selectedDb}`;
    if (tabLiveId && selectedDb && key !== prevCollKey) {
      prevCollKey = key;
      if (!collCache.has(key)) {
        queueMicrotask(() => loadCollections(tabConnId, selectedDb));
      }
    }
  });

  const collections = $derived(collCache.get(`${tabConnId}:${selectedDb}`) ?? []);

  async function loadDatabases(connId: string, live: string) {
    loadingDbs = true;
    try {
      const dbs = await nosqlListDatabases(live);
      dbCache = new Map([...dbCache, [connId, dbs]]);
    } catch (e: any) {
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
      const colls = await listColls(live, db);
      collCache = new Map([...collCache, [key, colls]]);
      // Auto-select first collection if tab has none
      if (colls.length > 0 && !selectedColl && activeNoSqlTab) {
        setNoSqlTabData(activeNoSqlTab.id, { collection: colls[0] });
      }
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      loadingColls = false;
    }
  }

  // Listen for AI-injected queries
  $effect(() => {
    const text = $insertNoSqlQueryText;
    if (text && activeNoSqlTab) {
      setNoSqlTabData(activeNoSqlTab.id, { filterQuery: text });
      insertNoSqlQueryText.set('');
    }
  });

  // Listen for AI-triggered NoSQL execution
  $effect(() => {
    const exec = $aiExecuteNoSqlQuery;
    if (!exec) return;

    // Auto-create a NoSQL tab if none exists — defer to next tick so derived updates
    if (!activeNoSqlTab) {
      addTab('AI Query', 'nosql', null, 'var(--nosql)');
      // Effect will re-run once activeNoSqlTab updates
      return;
    }

    // The AI sends live pool IDs, but tabs store saved connection IDs.
    // Reverse-lookup: find the saved ID that maps to this live ID.
    const liveMap = $nosqlLiveConnectionIds;
    let savedConnId = exec.connectionId;
    for (const [savedId, liveId] of Object.entries(liveMap)) {
      if (liveId === exec.connectionId) {
        savedConnId = savedId;
        break;
      }
    }

    activeNoSqlConnectionId.set(savedConnId);
    setNoSqlTabData(activeNoSqlTab.id, {
      connectionId: savedConnId,
      database: exec.database,
      collection: exec.collection,
      filterQuery: exec.filter,
    });

    aiExecuteNoSqlQuery.set(null);
  });

  function handleDbSelect(connId: string, db: string) {
    if (!activeNoSqlTab) return;
    activeNoSqlConnectionId.set(connId);
    setNoSqlTabData(activeNoSqlTab.id, { connectionId: connId, database: db, collection: '', filterQuery: '{}' });
    dbDropdownOpen = false;
  }

  function handleCollChange(e: Event) {
    const val = (e.target as HTMLSelectElement).value;
    if (activeNoSqlTab) {
      setNoSqlTabData(activeNoSqlTab.id, { collection: val, filterQuery: '{}' });
    }
  }

  // Database dropdown
  let dbDropdownOpen = $state(false);

  function toggleDbDropdown() {
    dbDropdownOpen = !dbDropdownOpen;
  }

  // Close dropdown on outside click
  function handleWindowClick() {
    if (dbDropdownOpen) dbDropdownOpen = false;
  }

  // Find which connection owns the current database
  const currentConnName = $derived(
    $nosqlConnections.find(c => c.id === tabConnId)?.name ?? ''
  );

  // Any MongoDB connected?
  const anyMongoConnected = $derived(
    $nosqlConnections.some(c => c.driver === 'mongodb' && $connectedNoSqlIds.has(c.id))
  );
  // Any Redis connected?
  const anyRedisConnected = $derived(
    $nosqlConnections.some(c => c.driver === 'redis' && $connectedNoSqlIds.has(c.id))
  );
</script>

<svelte:window onclick={handleWindowClick} />

<div class="nosql-panel">
  {#if !conn}
    <div class="nosql-empty">
      <div class="nosql-empty-icon">
        <svg viewBox="0 0 24 24"><ellipse cx="12" cy="5" rx="9" ry="3" /><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" /><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" /></svg>
      </div>
      <div class="nosql-empty-text">Select or create a connection from the sidebar</div>
    </div>
  {:else if !isConnected}
    <div class="nosql-empty">
      <div class="nosql-empty-icon">
        <svg viewBox="0 0 24 24"><path d="M1 1l22 22" /><path d="M16.72 11.06A10.94 10.94 0 0119 12.55" /><path d="M5 12.55a10.94 10.94 0 015.17-2.39" /><path d="M8.53 16.11a6 6 0 016.95 0" /><line x1="12" y1="20" x2="12.01" y2="20" /></svg>
      </div>
      <div class="nosql-empty-text">
        "{conn.name}" is not connected<br />
        <span class="nosql-empty-hint">Click the connection in the sidebar to connect</span>
      </div>
    </div>
  {:else if !activeNoSqlTab}
    <div class="nosql-empty">
      <div class="nosql-empty-icon">
        <svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14" stroke="var(--t4)" fill="none" stroke-width="1.5" stroke-linecap="round"/></svg>
      </div>
      <div class="nosql-empty-text">
        Open a tab to start querying<br />
        <span class="nosql-empty-hint">Press <kbd>+</kbd> in the tab bar or click a collection in the sidebar</span>
        <div class="nosql-empty-ai"><kbd>⌘L</kbd> AI Assistant</div>
      </div>
    </div>
  {:else if isRedis && liveId}
    <RedisViewer connectionId={liveId} />
  {:else if anyMongoConnected}
    <!-- Selector bar with grouped database dropdown -->
    <div class="nosql-selector-bar">
      <!-- Database dropdown (grouped by connection) -->
      <div class="nosql-db-dropdown-wrap">
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <button class="nosql-db-pill" onclick={(e) => { e.stopPropagation(); toggleDbDropdown(); }}>
          <svg viewBox="0 0 24 24" width="12" height="12"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>
          <span class="nosql-db-pill-name">{selectedDb || 'Select database'}</span>
          {#if currentConnName}
            <span class="nosql-db-pill-conn">{currentConnName}</span>
          {/if}
          <svg class="nosql-db-pill-chevron" viewBox="0 0 24 24" width="10" height="10"><path d="M6 9l6 6 6-6"/></svg>
        </button>
        {#if dbDropdownOpen}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="nosql-db-dropdown" onclick={(e) => e.stopPropagation()}>
            {#if connGroups.length === 0}
              <div class="nosql-dd-empty">No databases available</div>
            {:else}
              {#each connGroups as group, gi}
                {#if gi > 0}
                  <div class="nosql-dd-sep"></div>
                {/if}
                <div class="nosql-dd-group-header">
                  <svg viewBox="0 0 24 24" width="10" height="10"><path d="M12 2C12 2 7 7.5 7 13a5 5 0 0010 0c0-5.5-5-11-5-11z" fill="#00ED64" stroke="none"/></svg>
                  {group.connName}
                </div>
                {#if group.databases.length === 0}
                  <div class="nosql-dd-empty-sub">Loading databases...</div>
                {:else}
                  {#each group.databases as db}
                    <button
                      class="nosql-dd-item"
                      class:active={tabConnId === group.connId && selectedDb === db}
                      onclick={() => handleDbSelect(group.connId, db)}
                    >
                      <svg viewBox="0 0 24 24" width="10" height="10"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>
                      {db}
                    </button>
                  {/each}
                {/if}
              {/each}
            {/if}
          </div>
        {/if}
      </div>

      <span class="nosql-sel-sep">/</span>

      <!-- Collection select -->
      <select class="nosql-sel" value={selectedColl} onchange={handleCollChange} disabled={loadingColls || !selectedDb}>
        {#if collections.length === 0}
          <option value="">{loadingColls ? 'Loading...' : 'No collections'}</option>
        {/if}
        {#each collections as coll}
          <option value={coll}>{coll}</option>
        {/each}
      </select>
    </div>

    {#if selectedDb && selectedColl && tabLiveId}
      <DocumentViewer connectionId={tabLiveId} database={selectedDb} collection={selectedColl} initialFilter={tabData?.filterQuery} />
    {:else}
      <div class="nosql-empty">
        <div class="nosql-empty-icon">
          <svg viewBox="0 0 24 24"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
        </div>
        <div class="nosql-empty-text">Select a database and collection</div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .nosql-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .nosql-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }
  .nosql-empty-icon svg {
    width: 36px; height: 36px;
    stroke: var(--t4); fill: none;
    stroke-width: 1; stroke-linecap: round;
    opacity: 0.5;
  }
  .nosql-empty-text {
    font-size: 12px; color: var(--t3);
    font-family: var(--mono); text-align: center; line-height: 1.6;
  }
  .nosql-empty-hint { font-size: 11px; color: var(--t4); }
  .nosql-empty-text kbd {
    background: var(--b1); padding: 1px 5px; border-radius: 3px; font-size: 10px;
  }
  .nosql-empty-ai {
    margin-top: 12px;
    padding: 5px 14px;
    border: 1px solid var(--b1);
    border-radius: 6px;
    font-size: 11px;
    color: var(--t4);
    font-family: var(--mono);
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .nosql-empty-ai kbd {
    background: var(--b1); padding: 1px 5px; border-radius: 3px; font-size: 10px; color: var(--t3);
  }

  /* Selector bar */
  .nosql-selector-bar {
    height: 38px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    padding: 0 12px;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* Database dropdown */
  .nosql-db-dropdown-wrap {
    position: relative;
  }
  .nosql-db-pill {
    height: 26px;
    padding: 0 8px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: rgba(255,255,255,0.04);
    color: var(--t1);
    font-size: 12.5px;
    font-family: var(--mono);
    cursor: default;
    display: flex;
    align-items: center;
    gap: 6px;
    transition: border-color 0.15s;
  }
  .nosql-db-pill:hover { border-color: var(--b2); }
  .nosql-db-pill svg:first-child {
    stroke: var(--acc); fill: none;
    stroke-width: 1.5; stroke-linecap: round;
    flex-shrink: 0;
  }
  .nosql-db-pill-name {
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    max-width: 150px;
  }
  .nosql-db-pill-conn {
    font-size: 9px; color: var(--t2); font-family: var(--ui);
    padding: 2px 6px; border-radius: 3px;
    background: color-mix(in srgb, var(--acc) 15%, transparent);
    font-weight: 500;
  }
  .nosql-db-pill-chevron {
    stroke: var(--t3); fill: none; stroke-width: 2;
    stroke-linecap: round; stroke-linejoin: round;
    flex-shrink: 0;
  }

  .nosql-db-dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    min-width: 220px;
    max-height: 300px;
    overflow-y: auto;
    background: var(--modal-bg, var(--n));
    border: 1px solid var(--b2);
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0,0,0,0.55);
    z-index: 500;
    padding: 4px 0;
    animation: nosqlDdIn 0.1s ease;
    backdrop-filter: none;
  }
  @keyframes nosqlDdIn {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }
  .nosql-db-dropdown::-webkit-scrollbar { width: 4px; }
  .nosql-db-dropdown::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .nosql-dd-group-header {
    padding: 6px 12px 4px;
    font-size: 11px;
    font-weight: 500;
    color: var(--t3);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .nosql-dd-sep {
    height: 1px;
    background: var(--b1);
    margin: 4px 0;
  }
  .nosql-dd-item {
    width: 100%;
    padding: 6px 12px 6px 28px;
    font-size: 12px;
    color: var(--t2);
    font-family: var(--mono);
    border: none;
    background: transparent;
    cursor: default;
    text-align: left;
    display: flex;
    align-items: center;
    gap: 6px;
    transition: background 0.08s;
  }
  .nosql-dd-item svg {
    stroke: var(--t4); fill: none;
    stroke-width: 1.5; stroke-linecap: round;
    flex-shrink: 0;
  }
  .nosql-dd-item:hover {
    background: color-mix(in srgb, var(--acc) 8%, transparent);
    color: var(--t1);
  }
  .nosql-dd-item.active {
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    color: var(--t1);
  }
  .nosql-dd-empty, .nosql-dd-empty-sub {
    padding: 8px 12px;
    font-size: 11px;
    color: var(--t4);
    font-family: var(--mono);
  }
  .nosql-dd-empty-sub { padding-left: 28px; }

  .nosql-sel-sep {
    color: var(--t4); font-size: 14px; font-family: var(--mono); flex-shrink: 0;
  }
  .nosql-sel {
    height: 26px;
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 0 8px;
    font-size: 12.5px;
    color: var(--t1);
    font-family: var(--mono);
    outline: none;
    cursor: default;
    min-width: 100px;
    transition: border-color 0.15s;
  }
  .nosql-sel:focus { border-color: var(--acc); }
</style>

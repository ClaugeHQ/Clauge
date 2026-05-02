<script lang="ts">
  import {
    nosqlConnections,
    activeNoSqlConnectionId,
    connectedNoSqlIds,
    loadNoSqlConnections,
    deleteNoSqlConnection,
    connectToNoSql,
    disconnectFromNoSql,
    showNoSqlConnectionDialog,
    editingNoSqlConnection,
    nosqlLiveConnectionIds,
    openNoSqlCollection,
    nosqlConnectionStates,
    nosqlConnectionErrors,
    resetNoSqlConnState,
  } from '../stores';
  import { nosqlListDatabases, nosqlListCollections, nosqlCreateCollection, nosqlDropDatabase, nosqlDropCollection, nosqlRenameCollection } from '../commands';
  import { showToast } from '$lib/shared/primitives/toast';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import { friendlyError } from '$lib/utils/errors';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import type { NoSqlConnection } from '../types';

  // SVG icon helpers (inline, 14x14, stroke-based)
  const icons = {
    refresh: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M23 4v6h-6"/><path d="M1 20v-6h6"/><path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/></svg>',
    disconnect: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18.36 6.64a9 9 0 11-12.73 0"/><line x1="12" y1="2" x2="12" y2="12"/></svg>',
    edit: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
    trash: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>',
    copy: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>',
    openTab: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>',
    rename: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>',
    drop: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>',
    connect: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/><path d="M12 5l7 7-7 7"/></svg>',
    addColl: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><path d="M12 11v6M9 14h6"/></svg>',
    dropDb: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/><line x1="9" y1="12" x2="15" y2="18"/><line x1="15" y1="12" x2="9" y2="18"/></svg>',
    ellipsisV: '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="5" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="12" cy="19" r="1.5"/></svg>',
    ellipsisH: '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="19" cy="12" r="1.5"/></svg>',
  };

  interface Props {
    searchQuery?: string;
  }

  let { searchQuery = '' }: Props = $props();

  let expandedConns = $state<Set<string>>(new Set());
  let expandedDbs = $state<Set<string>>(new Set());
  let collapsedDuringSearch = $state<Set<string>>(new Set());

  // Confirm dialog state
  let confirmShow = $state(false);
  let confirmTitle = $state('');
  let confirmMessage = $state('');
  let confirmDanger = $state(false);
  let confirmText = $state('Delete');
  let confirmAction: (() => Promise<void>) | null = $state(null);

  // Rename/Create dialog state
  let renameShow = $state(false);
  let renameTitle = $state('Rename Collection');
  let renameButtonLabel = $state('Rename');
  let renameValue = $state('');
  let renameAction: ((name: string) => Promise<void>) | null = $state(null);

  let prevSearch = '';
  $effect(() => {
    if (searchQuery !== prevSearch) {
      collapsedDuringSearch = new Set();
      prevSearch = searchQuery;
    }
  });
  let dbCache = $state<Map<string, string[]>>(new Map());
  let collCache = $state<Map<string, string[]>>(new Map());
  let loadingDbs = $state<Set<string>>(new Set());
  let loadingColls = $state<Set<string>>(new Set());

  loadNoSqlConnections();

  const searchLower = $derived(searchQuery.toLowerCase());

  function connMatchesSearch(conn: NoSqlConnection): boolean {
    if (!searchQuery) return true;
    if (conn.name.toLowerCase().includes(searchLower)) return true;
    const dbs = dbCache.get(conn.id) ?? [];
    for (const db of dbs) {
      if (db.toLowerCase().includes(searchLower)) return true;
      const colls = collCache.get(`${conn.id}:${db}`) ?? [];
      if (colls.some(c => c.toLowerCase().includes(searchLower))) return true;
    }
    return false;
  }

  function dbMatchesSearch(db: string): boolean {
    if (!searchQuery) return true;
    return db.toLowerCase().includes(searchLower);
  }

  function collMatchesSearch(coll: string): boolean {
    if (!searchQuery) return true;
    return coll.toLowerCase().includes(searchLower);
  }

  function dbHasMatchingColls(connId: string, db: string): boolean {
    if (!searchQuery) return false;
    const colls = collCache.get(`${connId}:${db}`) ?? [];
    return colls.some(c => c.toLowerCase().includes(searchLower));
  }

  const filtered = $derived(
    $nosqlConnections.filter(c => connMatchesSearch(c))
  );

  $effect(() => {
    if (!searchQuery) return;
    for (const conn of $nosqlConnections) {
      if (!$connectedNoSqlIds.has(conn.id)) continue;
      if (conn.driver !== 'mongodb') continue;
      if (!dbCache.has(conn.id) && !loadingDbs.has(conn.id)) loadDatabases(conn.id);
      const dbs = dbCache.get(conn.id) ?? [];
      for (const db of dbs) {
        const key = `${conn.id}:${db}`;
        if (!collCache.has(key) && !loadingColls.has(key)) loadCollections(conn.id, db);
      }
    }
  });

  export function showAddConnection() {
    editingNoSqlConnection.set(null);
    showNoSqlConnectionDialog.set(true);
  }

  async function handleClickConnection(conn: NoSqlConnection) {
    activeNoSqlConnectionId.set(conn.id);
    const state = $nosqlConnectionStates.get(conn.id) ?? 'idle';
    // Already attempting — clicking again must NOT fire a duplicate connect.
    if (state === 'connecting') return;
    if (!$connectedNoSqlIds.has(conn.id)) {
      // Treat click on an errored row as a retry: clear so the indicator
      // transitions back to `connecting` cleanly.
      if (state === 'error') resetNoSqlConnState(conn.id);
      await doConnect(conn);
      expandedConns = new Set([...expandedConns, conn.id]);
    } else {
      toggleConn(conn.id);
    }
  }

  async function doConnect(conn: NoSqlConnection) {
    // The store guards against duplicate connects, but bail early too so we
    // don't surface a toast for the no-op case.
    if (($nosqlConnectionStates.get(conn.id) ?? 'idle') === 'connecting') return;
    try {
      await connectToNoSql(conn.id);
      showToast(`Connected to ${conn.name}`, 'success');
      await loadDatabases(conn.id);
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    }
  }

  function showConfirm(title: string, message: string, danger: boolean, action: () => Promise<void>, verb?: string) {
    confirmTitle = title;
    confirmMessage = message;
    confirmDanger = danger;
    confirmText = verb ?? (danger ? 'Delete' : 'Confirm');
    confirmAction = action;
    confirmShow = true;
  }

  async function handleConfirmOk() {
    confirmShow = false;
    if (confirmAction) await confirmAction();
    confirmAction = null;
  }

  function showRename(currentName: string, action: (name: string) => Promise<void>, title = 'Rename Collection', btnLabel = 'Rename') {
    renameValue = currentName;
    renameAction = action;
    renameTitle = title;
    renameButtonLabel = btnLabel;
    renameShow = true;
  }

  async function handleRenameOk() {
    renameShow = false;
    const name = renameValue.trim();
    if (name && renameAction) await renameAction(name);
    renameAction = null;
  }

  function toggleConn(connId: string) {
    if (searchQuery) {
      const next = new Set(collapsedDuringSearch);
      if (next.has(`conn:${connId}`)) next.delete(`conn:${connId}`);
      else next.add(`conn:${connId}`);
      collapsedDuringSearch = next;
      return;
    }
    const next = new Set(expandedConns);
    if (next.has(connId)) next.delete(connId);
    else {
      next.add(connId);
      if (!dbCache.has(connId)) loadDatabases(connId);
    }
    expandedConns = next;
  }

  function toggleDb(connId: string, db: string) {
    const key = `${connId}:${db}`;
    if (!collCache.has(key) && !loadingColls.has(key)) loadCollections(connId, db);
    if (searchQuery) {
      const next = new Set(collapsedDuringSearch);
      if (next.has(`db:${key}`)) next.delete(`db:${key}`);
      else next.add(`db:${key}`);
      collapsedDuringSearch = next;
      return;
    }
    const next = new Set(expandedDbs);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedDbs = next;
  }

  async function loadDatabases(connId: string) {
    const liveId = $nosqlLiveConnectionIds[connId];
    if (!liveId) return;
    loadingDbs = new Set([...loadingDbs, connId]);
    try {
      const dbs = await nosqlListDatabases(liveId);
      dbCache = new Map([...dbCache, [connId, dbs]]);
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      loadingDbs = new Set([...loadingDbs].filter(id => id !== connId));
    }
  }

  async function loadCollections(connId: string, db: string) {
    const liveId = $nosqlLiveConnectionIds[connId];
    if (!liveId) return;
    const key = `${connId}:${db}`;
    loadingColls = new Set([...loadingColls, key]);
    try {
      const colls = await nosqlListCollections(liveId, db);
      collCache = new Map([...collCache, [key, colls]]);
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      loadingColls = new Set([...loadingColls].filter(id => id !== key));
    }
  }

  function handleCollectionClick(connId: string, db: string, coll: string) {
    activeNoSqlConnectionId.set(connId);
    openNoSqlCollection.set({ connectionId: connId, database: db, collection: coll });
  }

  // ── Context Menus ──

  function showConnMenu(e: MouseEvent, conn: NoSqlConnection) {
    e.preventDefault();
    e.stopPropagation();
    const isConnected = $connectedNoSqlIds.has(conn.id);
    const connString = conn.connectionString || `mongodb://${conn.host}:${conn.port}`;

    showContextMenu(e.clientX, e.clientY, [
      ...(isConnected ? [
        {
          label: 'Refresh',
          icon: icons.refresh,
          action: async () => {
            dbCache = new Map([...dbCache].filter(([k]) => k !== conn.id));
            collCache = new Map([...collCache].filter(([k]) => !k.startsWith(`${conn.id}:`)));
            await loadDatabases(conn.id);
            // Reload collections for expanded DBs
            const dbs = dbCache.get(conn.id) ?? [];
            for (const db of dbs) {
              const key = `${conn.id}:${db}`;
              if (expandedDbs.has(key)) loadCollections(conn.id, db);
            }
            showToast('Refreshed', 'success');
          },
        },
        { label: '', action: () => {}, separator: true },
      ] : []),
      ...(isConnected ? [{
        label: 'Disconnect',
        icon: icons.disconnect,
        action: () => showConfirm('Disconnect', `Disconnect from "${conn.name}"?`, false, async () => {
          try {
            await disconnectFromNoSql(conn.id);
            expandedConns = new Set([...expandedConns].filter(id => id !== conn.id));
            showToast(`Disconnected from ${conn.name}`, 'info');
          } catch (e: any) { showToast(friendlyError(e), 'error'); }
        }, 'Disconnect'),
      }] : [{
        label: 'Connect',
        icon: icons.connect,
        action: () => doConnect(conn),
      }]),
      { label: '', action: () => {}, separator: true },
      {
        label: 'Edit',
        icon: icons.edit,
        action: () => {
          editingNoSqlConnection.set(conn);
          showNoSqlConnectionDialog.set(true);
        },
      },
      {
        label: 'Copy Connection String',
        icon: icons.copy,
        action: async () => {
          await navigator.clipboard.writeText(connString);
          showToast('Copied to clipboard', 'success');
        },
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Delete',
        icon: icons.trash,
        danger: true,
        action: () => showConfirm('Delete Connection', `Delete "${conn.name}"? This cannot be undone.`, true, async () => {
          try {
            await deleteNoSqlConnection(conn.id);
            showToast('Connection removed', 'success');
          } catch (e: any) { showToast(friendlyError(e), 'error'); }
        }),
      },
    ]);
  }

  function showDbMenu(e: MouseEvent, connId: string, db: string) {
    e.preventDefault();
    e.stopPropagation();
    const liveId = $nosqlLiveConnectionIds[connId];
    if (!liveId) return;

    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Create Collection',
        icon: icons.addColl,
        action: () => showRename('', async (name) => {
          try {
            await nosqlCreateCollection(liveId, db, name);
            await loadCollections(connId, db);
            showToast(`Created ${name}`, 'success');
          } catch (e: any) { showToast(friendlyError(e), 'error'); }
        }, 'Create Collection', 'Create'),
      },
      {
        label: 'Refresh',
        icon: icons.refresh,
        action: async () => {
          collCache = new Map([...collCache].filter(([k]) => !k.startsWith(`${connId}:${db}`)));
          await loadCollections(connId, db);
        },
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Drop Database',
        icon: icons.dropDb,
        danger: true,
        action: () => showConfirm('Drop Database', `Drop "${db}"? All collections and documents will be permanently deleted.`, true, async () => {
          try {
            await nosqlDropDatabase(liveId, db);
            dbCache = new Map([...dbCache].filter(([k]) => k !== connId));
            collCache = new Map([...collCache].filter(([k]) => !k.startsWith(`${connId}:${db}`)));
            await loadDatabases(connId);
            showToast(`Dropped ${db}`, 'success');
          } catch (e: any) { showToast(friendlyError(e), 'error'); }
        }, 'Drop'),
      },
    ]);
  }

  function showCollMenu(e: MouseEvent, connId: string, db: string, coll: string) {
    e.preventDefault();
    e.stopPropagation();
    const liveId = $nosqlLiveConnectionIds[connId];
    if (!liveId) return;

    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Open in New Tab',
        icon: icons.openTab,
        action: () => handleCollectionClick(connId, db, coll),
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Rename Collection',
        icon: icons.rename,
        action: () => showRename(coll, async (newName) => {
          try {
            await nosqlRenameCollection(liveId, db, coll, newName);
            await loadCollections(connId, db);
            showToast(`Renamed to ${newName}`, 'success');
          } catch (e: any) { showToast(friendlyError(e), 'error'); }
        }),
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Drop Collection',
        icon: icons.drop,
        danger: true,
        action: () => showConfirm('Drop Collection', `Drop "${coll}"? All documents will be permanently deleted.`, true, async () => {
          try {
            await nosqlDropCollection(liveId, db, coll);
            await loadCollections(connId, db);
            showToast(`Dropped ${coll}`, 'success');
          } catch (e: any) { showToast(friendlyError(e), 'error'); }
        }, 'Drop'),
      },
    ]);
  }

  function driverColor(driver: string): string {
    return driver === 'redis' ? '#DC382D' : '#00ED64';
  }
</script>

<div class="nosql-nav">
  {#if filtered.length === 0}
    <div class="nav-empty">
      {#if searchQuery}
        <span>No results for "{searchQuery}"</span>
      {:else}
        <span>No connections yet</span>
        <button class="nav-empty-btn" onclick={showAddConnection}>
          + New Connection
        </button>
      {/if}
    </div>
  {:else}
    {#each filtered as conn (conn.id)}
      {@const isConnected = $connectedNoSqlIds.has(conn.id)}
      {@const connState = $nosqlConnectionStates.get(conn.id) ?? 'idle'}
      {@const isConnecting = connState === 'connecting'}
      {@const hasError = connState === 'error'}
      {@const errorMsg = $nosqlConnectionErrors.get(conn.id) ?? ''}
      {@const isExpanded = expandedConns.has(conn.id) && isConnected}

      <button
        class="tree-item tree-conn"
        class:active={$activeNoSqlConnectionId === conn.id}
        class:connected={isConnected}
        class:connecting={isConnecting}
        class:errored={hasError}
        title={hasError ? errorMsg : ''}
        onclick={() => handleClickConnection(conn)}
        oncontextmenu={(e) => showConnMenu(e, conn)}
      >
        <svg class="tree-chevron" class:open={(isExpanded || searchQuery) && !collapsedDuringSearch.has(`conn:${conn.id}`)} viewBox="0 0 24 24">
          <path d="M9 18l6-6-6-6"/>
        </svg>
        <span
          class="conn-badge-wrap"
          class:connected={isConnected}
          class:connecting={isConnecting}
          class:errored={hasError}
        >
          <span class="conn-driver" style:color={driverColor(conn.driver)} style:background="color-mix(in srgb, {driverColor(conn.driver)} 12%, transparent)">
            {conn.driver === 'mongodb' ? 'MG' : 'RD'}
          </span>
        </span>
        <div class="tree-body">
          <div class="tree-row-top">
            <span class="tree-label">{conn.name}</span>
          </div>
          <div class="tree-row-bot">
            {#if isConnecting}
              <span class="nn-connecting-text">Connecting<span class="nn-dots"></span></span>
            {:else}
              <span class="tree-sub">{conn.host}{conn.port ? `:${conn.port}` : ''}</span>
            {/if}
          </div>
        </div>
        {#if isConnected}
          <span class="conn-action" role="button" tabindex="-1" title="Refresh"
            onclick={async (e) => { e.stopPropagation(); dbCache = new Map([...dbCache].filter(([k]) => k !== conn.id)); collCache = new Map([...collCache].filter(([k]) => !k.startsWith(`${conn.id}:`))); await loadDatabases(conn.id); const dbs = dbCache.get(conn.id) ?? []; for (const db of dbs) { const key = `${conn.id}:${db}`; if (expandedDbs.has(key)) loadCollections(conn.id, db); } }}>
            {@html icons.refresh}
          </span>
        {/if}
        <span class="conn-action conn-ellipsis" role="button" tabindex="-1" title="More"
          onclick={(e) => { e.stopPropagation(); showConnMenu(e, conn); }}>
          {@html icons.ellipsisV}
        </span>
      </button>

      <!-- Tree: databases → collections -->
      {#if ((isExpanded || searchQuery) && !collapsedDuringSearch.has(`conn:${conn.id}`)) && conn.driver === 'mongodb' && isConnected}
        {@const dbs = dbCache.get(conn.id) ?? []}
        {#if loadingDbs.has(conn.id)}
          <div class="tree-loading" style="padding-left:16px">Loading databases<span class="nn-dots"></span></div>
        {:else}
          {#each dbs as db}
            {@const dbKey = `${conn.id}:${db}`}
            {@const dbExpanded = expandedDbs.has(dbKey)}
            {@const dbVisible = !searchQuery || dbMatchesSearch(db) || dbHasMatchingColls(conn.id, db)}
            {@const autoExpand = searchQuery && (dbMatchesSearch(db) || dbHasMatchingColls(conn.id, db)) && !collapsedDuringSearch.has(`db:${dbKey}`)}

            {#if dbVisible}
              <button
                class="tree-item tree-db"
                class:expanded={dbExpanded || autoExpand}
                onclick={() => toggleDb(conn.id, db)}
                oncontextmenu={(e) => showDbMenu(e, conn.id, db)}
              >
                <svg class="tree-chevron-sm" class:open={dbExpanded || autoExpand} viewBox="0 0 24 24">
                  <path d="M9 18l6-6-6-6"/>
                </svg>
                <svg class="tree-icon tree-icon-db" viewBox="0 0 24 24">
                  <ellipse cx="12" cy="5" rx="9" ry="3"/>
                  <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/>
                  <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
                </svg>
                <span class="tree-label">{db}</span>
                <span class="db-action" role="button" tabindex="-1" title="Create Collection"
                  onclick={(e) => { e.stopPropagation(); const liveId = $nosqlLiveConnectionIds[conn.id]; if (liveId) showRename('', async (name) => { try { await nosqlCreateCollection(liveId, db, name); await loadCollections(conn.id, db); showToast(`Created ${name}`, 'success'); } catch (err) { showToast(friendlyError(err), 'error'); } }, 'Create Collection', 'Create'); }}>
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
                </span>
                <span class="db-action db-action-danger" role="button" tabindex="-1" title="Drop Database"
                  onclick={(e) => { e.stopPropagation(); const liveId = $nosqlLiveConnectionIds[conn.id]; if (liveId) showConfirm('Drop Database', `Drop "${db}"? All collections and documents will be permanently deleted.`, true, async () => { try { await nosqlDropDatabase(liveId, db); dbCache = new Map([...dbCache].filter(([k]) => k !== conn.id)); collCache = new Map([...collCache].filter(([k]) => !k.startsWith(`${conn.id}:${db}`))); await loadDatabases(conn.id); showToast(`Dropped ${db}`, 'success'); } catch (err) { showToast(friendlyError(err), 'error'); } }, 'Drop'); }}>
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
                </span>
              </button>

              {#if dbExpanded || autoExpand}
                {@const colls = collCache.get(dbKey) ?? []}
                {#if loadingColls.has(dbKey)}
                  <div class="tree-loading" style="padding-left:26px">Loading<span class="nn-dots"></span></div>
                {:else if colls.length === 0}
                  <div class="tree-loading" style="padding-left:26px">No collections</div>
                {:else}
                  {#each colls as coll}
                    {#if !searchQuery || collMatchesSearch(coll)}
                      <button
                        class="tree-item tree-coll"
                        onclick={() => handleCollectionClick(conn.id, db, coll)}
                        oncontextmenu={(e) => showCollMenu(e, conn.id, db, coll)}
                      >
                        <svg class="tree-icon tree-icon-coll" viewBox="0 0 24 24">
                          <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/>
                          <polyline points="14 2 14 8 20 8"/>
                        </svg>
                        <span class="tree-label">{coll}</span>
                        <span class="coll-ellipsis" role="button" tabindex="-1"
                          onclick={(e) => { e.stopPropagation(); showCollMenu(e, conn.id, db, coll); }}>
                          {@html icons.ellipsisH}
                        </span>
                      </button>
                    {/if}
                  {/each}
                {/if}
              {/if}
            {/if}
          {/each}
        {/if}
      {/if}
    {/each}
  {/if}
</div>

<!-- Confirm Dialog — shared primitive for visual parity across all modes. -->
<ConfirmDialog
  bind:show={confirmShow}
  title={confirmTitle}
  message={confirmMessage}
  confirmText={confirmText}
  confirmColor={confirmDanger ? 'var(--err)' : 'var(--acc)'}
  onconfirm={handleConfirmOk}
/>

<!-- Rename Dialog -->
{#if renameShow}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="nn-confirm-overlay" onclick={() => renameShow = false}>
    <div class="nn-confirm" onclick={(e) => e.stopPropagation()}>
      <div class="nn-confirm-title">{renameTitle}</div>
      <input
        class="nn-rename-input"
        type="text"
        bind:value={renameValue}
        placeholder="Collection name"
        onkeydown={(e) => { if (e.key === 'Enter') handleRenameOk(); }}
        autofocus
      />
      <div class="nn-confirm-actions">
        <button class="nn-confirm-btn" onclick={() => renameShow = false}>Cancel</button>
        <button class="nn-confirm-btn primary" onclick={handleRenameOk} disabled={!renameValue.trim()}>{renameButtonLabel}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .nosql-nav {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .nosql-nav::-webkit-scrollbar { width: 3px; }
  .nosql-nav::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .nav-empty {
    padding: 24px 12px;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--ui);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  .nav-empty-btn {
    padding: 5px 12px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11px;
    font-family: var(--ui);
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .nav-empty-btn:hover { background: var(--c); border-color: var(--b2); color: var(--t1); }

  .tree-item {
    width: 100%;
    border: none;
    background: transparent;
    display: flex;
    align-items: center;
    gap: 5px;
    cursor: pointer;
    transition: background 0.08s;
    text-align: left;
    position: relative;
  }
  .tree-item:hover { background: var(--c); }

  .tree-label {
    font-family: var(--ui);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  /* Two-line connection row to match SSH/Explorer parity. */
  .tree-conn {
    min-height: 44px;
    height: auto;
    padding: 6px 8px;
    gap: 8px;
    align-items: center;
  }
  .tree-conn.active { background: color-mix(in srgb, var(--acc) 10%, transparent); }
  .tree-conn.connecting { pointer-events: none; }
  .tree-conn.errored { background: color-mix(in srgb, var(--err) 8%, transparent); }
  /* Connection status indicator — rectangular outline around the driver
     badge + 6px pulsing corner dot. Same visual language as SSH/Explorer
     (ring + dot, dialled-down 3s pulse) just shaped to fit the rectangular
     badge instead of a square icon container. Idle = both invisible. */
  .conn-badge-wrap {
    position: relative;
    display: inline-flex;
    flex-shrink: 0;
  }
  .conn-badge-wrap.connected::before,
  .conn-badge-wrap.connecting::before,
  .conn-badge-wrap.errored::before {
    content: '';
    position: absolute;
    inset: -3px;
    border-radius: 5px;
    pointer-events: none;
  }
  .conn-badge-wrap.connected::before { border: 1px solid color-mix(in srgb, var(--ok, #1dc880) 28%, transparent); }
  .conn-badge-wrap.connecting::before { border: 1px solid color-mix(in srgb, var(--warn) 35%, transparent); }
  .conn-badge-wrap.errored::before { border: 1px solid color-mix(in srgb, var(--err) 35%, transparent); }

  .conn-badge-wrap.connected::after,
  .conn-badge-wrap.connecting::after,
  .conn-badge-wrap.errored::after {
    content: '';
    position: absolute;
    top: -4px;
    right: -4px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    box-shadow: 0 0 0 1.5px var(--n);
    pointer-events: none;
  }
  .conn-badge-wrap.connected::after  { background: var(--ok, #1dc880); animation: nnBadgePulseOk 3s ease-in-out infinite; }
  .conn-badge-wrap.connecting::after { background: var(--warn);        animation: nnBadgePulseWarn 1.4s ease-in-out infinite; }
  .conn-badge-wrap.errored::after    { background: var(--err); }

  @keyframes nnBadgePulseOk {
    0%, 100% { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 2px color-mix(in srgb, var(--ok, #1dc880) 30%, transparent); }
    50%      { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 5px color-mix(in srgb, var(--ok, #1dc880) 0%, transparent); }
  }
  @keyframes nnBadgePulseWarn {
    0%, 100% { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 2px color-mix(in srgb, var(--warn) 35%, transparent); }
    50%      { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 5px color-mix(in srgb, var(--warn) 0%, transparent); }
  }
  .tree-conn .tree-label { font-size: 12.5px; color: var(--t2); font-weight: 500; }
  .tree-conn.active .tree-label { color: var(--t1); }
  .tree-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .tree-row-top, .tree-row-bot {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .tree-sub {
    font-size: 10.5px;
    font-family: var(--mono);
    color: var(--t3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tree-conn.active .tree-sub { color: var(--t2); }

  .tree-chevron {
    width: 10px; height: 10px;
    stroke: var(--t4); fill: none; stroke-width: 2; stroke-linecap: round;
    flex-shrink: 0; transition: transform 0.15s;
  }
  .tree-chevron.open { transform: rotate(90deg); }

  .tree-chevron-sm {
    width: 8px; height: 8px;
    stroke: var(--t4); fill: none; stroke-width: 2.5; stroke-linecap: round;
    flex-shrink: 0; transition: transform 0.15s;
  }
  .tree-chevron-sm.open { transform: rotate(90deg); }


  .conn-driver {
    font-size: 9px; font-weight: 700; padding: 1px 4px; border-radius: 3px;
    font-family: var(--ui); flex-shrink: 0; letter-spacing: 0.04em;
  }

  /* Action icons on connection row */
  .conn-action {
    width: 20px; height: 20px;
    display: none; align-items: center; justify-content: center;
    border-radius: 4px; flex-shrink: 0; cursor: default;
    color: var(--t3); transition: background 0.1s, color 0.1s;
  }
  .tree-conn:hover .conn-action { display: flex; }
  .conn-action:hover { background: rgba(255,255,255,0.08); color: var(--t1); }
  .conn-ellipsis { display: none; }
  .tree-conn:hover .conn-ellipsis { display: flex; }

  .nn-connecting-text { font-size: 10px; color: var(--t3); font-family: var(--ui); flex-shrink: 0; margin-left: 4px; }
  .nn-dots::after { content: ''; animation: nn-dots 1.4s steps(4, end) infinite; }
  @keyframes nn-dots { 0% { content: ''; } 25% { content: '.'; } 50% { content: '..'; } 75% { content: '...'; } }

  .tree-icon {
    width: 11px; height: 11px; stroke-width: 1.5; stroke-linecap: round; flex-shrink: 0;
  }
  .tree-icon-db { stroke: var(--acc); fill: none; opacity: 0.7; }
  .tree-icon-coll { stroke: var(--t3); fill: none; stroke-linejoin: round; }

  .tree-db {
    height: 28px; padding: 0 12px 0 16px;
    border-left: 1px solid var(--b1); margin-left: 11px;
  }
  .tree-db .tree-label { font-size: 11.5px; color: var(--t2); }

  .tree-coll {
    height: 26px; padding: 0 12px 0 26px;
    border-left: 1px solid var(--b1); margin-left: 11px;
  }
  .tree-coll .tree-label { font-size: 11px; color: var(--t2); }
  .tree-coll:hover .tree-label { color: var(--t1); }

  /* DB action icons (+ and delete) */
  .db-action {
    width: 18px; height: 18px;
    display: none; align-items: center; justify-content: center;
    border-radius: 3px; flex-shrink: 0; cursor: default;
    color: var(--t3); transition: background 0.1s, color 0.1s;
  }
  .tree-db:hover .db-action { display: flex; }
  .db-action:hover { background: rgba(255,255,255,0.08); color: var(--t1); }
  .db-action-danger:hover { background: rgba(240,68,68,0.12); color: var(--err); }

  /* Collection ellipsis */
  .coll-ellipsis {
    width: 18px; height: 18px;
    display: none; align-items: center; justify-content: center;
    border-radius: 3px; flex-shrink: 0; cursor: default;
    color: var(--t3); transition: background 0.1s, color 0.1s;
  }
  .tree-coll:hover .coll-ellipsis { display: flex; }
  .coll-ellipsis:hover { background: rgba(255,255,255,0.08); color: var(--t1); }

  .tree-loading {
    padding: 6px 0; font-size: 10px; color: var(--t4); font-family: var(--ui);
  }

  /* Rename dialog still uses .nn-confirm-* classes below — kept for it. */
  .nn-confirm-overlay {
    position: fixed; top: 0; left: 0; width: 100vw; height: 100vh;
    background: rgba(0,0,0,0.4); z-index: 9999;
    display: flex; align-items: center; justify-content: center;
  }
  .nn-confirm {
    background: var(--modal-bg, var(--n)); border: 1px solid var(--b1);
    border-radius: 12px; padding: 24px; min-width: 320px; max-width: 400px;
    box-shadow: 0 16px 48px rgba(0,0,0,0.5);
  }
  .nn-confirm-title {
    font-size: 15px; font-weight: 600; color: var(--t1); font-family: var(--ui);
    margin-bottom: 8px;
  }
  .nn-confirm-msg {
    font-size: 13px; color: var(--t2); font-family: var(--ui); line-height: 1.5;
    margin-bottom: 20px;
  }
  .nn-confirm-actions { display: flex; justify-content: flex-end; gap: 8px; }
  .nn-confirm-btn {
    padding: 7px 16px; border-radius: 8px; font-size: 12px; font-weight: 600;
    font-family: var(--ui); cursor: default; border: 1px solid var(--b1);
    background: transparent; color: var(--t2); transition: all 0.12s;
  }
  .nn-confirm-btn:hover { background: var(--c); color: var(--t1); }
  .nn-confirm-btn.primary { background: var(--acc); color: #fff; border-color: transparent; }
  .nn-confirm-btn.primary:hover { opacity: 0.9; }
  .nn-confirm-btn.danger { background: var(--err); color: #fff; border-color: transparent; }
  .nn-confirm-btn.danger:hover { opacity: 0.9; }

  .nn-rename-input {
    width: 100%; padding: 8px 12px; border-radius: 8px; margin-bottom: 16px;
    border: 1px solid var(--b1); background: rgba(255,255,255,0.04);
    color: var(--t1); font-size: 13px; font-family: var(--ui); outline: none;
  }
  .nn-rename-input:focus { border-color: var(--acc); box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 12%, transparent); }
</style>

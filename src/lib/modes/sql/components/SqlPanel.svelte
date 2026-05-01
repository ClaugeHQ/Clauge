<script lang="ts">
  import { mod } from '$lib/utils/platform';
  const m = mod();
  import QueryEditor from './QueryEditor.svelte';
  import ResultsTable from './ResultsTable.svelte';
  import {
    activeConnection, connectedIds, connectToDb, disconnectFromDb,
    loadConnections, saveConnection, showSqlConnectionDialog, getLiveId,
    selectedDatabase, databaseTables, insertQueryText, aiExecuteQuery,
    activeConnectionId, connectionDatabases, connections, dbLiveConnections,
    sqlTabState, getSqlTabData, setSqlTabData,
    getDbLiveId, connectToDatabase, updateSqlScript,
    sqlRowLimit
  } from '../stores';
  import { tabs, activeTabId, addTab } from '$lib/shared/stores/tabs';
  import { sqlExecuteQuery, sqlDescribeTable, sqlListSchemas } from '../commands';
  import type { TableInfo, SqlResultEntry, ColumnInfo } from '../types';
  import { descriptorFor } from '../dialects';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { mode } from '$lib/stores/app';
  import type { SqlQueryResult, SqlConnectionConfig } from '../types';
  import { get } from 'svelte/store';
  import { splitSqlStatements } from '../utils/splitter';

  let editorHeight = $state(45);
  let dragging = $state(false);
  let containerEl: HTMLDivElement | undefined = $state();
  let dbDropdownOpen = $state(false);

  // Load saved connections on mount
  loadConnections();

  const isConnected = $derived(
    $activeConnection ? $connectedIds.has($activeConnection.id) : false
  );

  let queryEditorRef: { handleExecute: () => void } | undefined;

  // Current active SQL tab
  const activeSqlTab = $derived($tabs.find(t => t.id === $activeTabId && t.mode === 'sql'));
  const hasActiveSqlTab = $derived(!!activeSqlTab);

  // Current tab's state
  const currentTabData = $derived.by(() => {
    if (!activeSqlTab) return null;
    return $sqlTabState.get(activeSqlTab.id) ?? { query: '', result: null, error: null, loading: false, database: $selectedDatabase || '', results: [], activeResultIdx: 0 };
  });

  const currentQuery = $derived(currentTabData?.query ?? '');
  const currentLoading = $derived(currentTabData?.loading ?? false);
  const currentDatabase = $derived(currentTabData?.database ?? $selectedDatabase ?? '');
  const currentResults = $derived(currentTabData?.results ?? []);
  const currentActiveResultIdx = $derived(currentTabData?.activeResultIdx ?? 0);

  // Active result entry (from multi-results or fallback to single result/error)
  const activeResultEntry = $derived.by(() => {
    if (currentResults.length > 0 && currentActiveResultIdx < currentResults.length) {
      return currentResults[currentActiveResultIdx];
    }
    return null;
  });

  const currentResult = $derived(activeResultEntry?.result ?? currentTabData?.result ?? null);
  const currentError = $derived(activeResultEntry?.error ?? currentTabData?.error ?? null);
  const currentLiveId = $derived.by(() => {
    if (!$activeConnection) return '';
    const dbLid = currentDatabase ? getDbLiveId($activeConnection.id, currentDatabase) : null;
    return dbLid ?? getLiveId($activeConnection.id) ?? '';
  });

  // Get tables for current selection
  const tableList = $derived.by(() => {
    if (!$activeConnection || !currentDatabase) return [] as TableInfo[];
    const key = `${$activeConnection.id}:${currentDatabase}`;
    return $databaseTables.get(key) ?? ([] as TableInfo[]);
  });

  // Column map for autocomplete: table name → column names
  let columnMap = $state<Record<string, string[]>>({});
  let columnMapKey = '';

  $effect(() => {
    const tables = tableList;
    const conn = $activeConnection;
    const db = currentDatabase;
    if (!conn || !db || tables.length === 0) {
      columnMap = {};
      return;
    }
    const key = `${conn.id}:${db}`;
    if (key === columnMapKey) return;
    columnMapKey = key;

    // Fetch columns for all tables in background
    const lid = getConnectionLid();
    if (!lid) return;
    const fetchColumns = async () => {
      const map: Record<string, string[]> = {};
      // Fetch in parallel, limit to 50 tables to avoid overload
      const batch = tables.slice(0, 50);
      const results = await Promise.allSettled(
        batch.map(async (t) => {
          const cols = await sqlDescribeTable(lid, t.name);
          return { name: t.name, cols: cols.map((c: ColumnInfo) => c.name) };
        })
      );
      for (const r of results) {
        if (r.status === 'fulfilled') {
          map[r.value.name] = r.value.cols;
        }
      }
      columnMap = map;
    };
    fetchColumns();
  });

  // Grouped databases: all connected instances with their databases
  interface DbGroup {
    connId: string;
    connName: string;
    driver: string;
    databases: string[];
  }

  const groupedDatabases = $derived.by(() => {
    const groups: DbGroup[] = [];
    const connected = $connectedIds;
    const allConns = $connections;
    const dbMap = $connectionDatabases;
    for (const conn of allConns) {
      if (!connected.has(conn.id)) continue;
      const dbs = dbMap.get(conn.id) ?? [];
      if (dbs.length > 0) {
        groups.push({ connId: conn.id, connName: conn.name, driver: conn.driver, databases: dbs });
      }
    }
    return groups;
  });

  const hasAnyDatabases = $derived(groupedDatabases.some(g => g.databases.length > 0));

  // Driver icon label helper
  function driverLabel(driver: string): string {
    return descriptorFor(driver)?.abbreviation ?? (driver ? driver.substring(0, 2).toUpperCase() : '?');
  }

  // Find which connection owns the current database
  const currentDbDriver = $derived.by(() => {
    for (const g of groupedDatabases) {
      if (g.databases.includes(currentDatabase)) {
        return g.driver;
      }
    }
    return $activeConnection?.driver ?? '';
  });

  // Listen for insert query from nav tree — append to existing query
  $effect(() => {
    const text = $insertQueryText;
    if (text && activeSqlTab) {
      const existing = currentQuery.trim();
      const newQuery = existing ? existing + '\n\n' + text : text;
      setSqlTabData(activeSqlTab.id, { query: newQuery });
      insertQueryText.set('');
    }
  });

  // Listen for AI-triggered query execution
  $effect(() => {
    const exec = $aiExecuteQuery;
    if (!exec) return;

    // Auto-create a SQL tab if none exists
    let tab = activeSqlTab;
    if (!tab) {
      tab = addTab('AI Query', 'sql', null, 'var(--sql)');
    }
    const tabId = tab.id;

    // Set the query in the active tab and sync UI state
    if (exec.database) {
      setSqlTabData(tabId, { query: exec.query, database: exec.database });
      selectedDatabase.set(exec.database);
      // Register AI's pool in dbLiveConnections so getConnectionLid() works for manual queries
      if (exec.connectionId && $activeConnection) {
        const dbKey = `${$activeConnection.id}:${exec.database}`;
        dbLiveConnections.update(m => ({ ...m, [dbKey]: exec.connectionId }));
      }
    } else {
      setSqlTabData(tabId, { query: exec.query });
    }

    // Execute the query — append as a new result tab
    const lid = exec.connectionId;
    if (lid) {
      setSqlTabData(tabId, { loading: true, error: null });
      sqlExecuteQuery(lid, applyRowLimit(exec.query))
        .then((result) => {
          const existing = getSqlTabData(tabId).results || [];
          const label = makeResultLabel(exec.query);
          const newEntry = { label, query: exec.query, result, error: null, connectionId: lid };
          const updated = [...existing, newEntry];
          setSqlTabData(tabId, {
            results: updated,
            activeResultIdx: updated.length - 1,
            loading: false,
            error: null,
          });
        })
        .catch((e: any) => {
          setSqlTabData(tabId, {
            error: friendlyError(e),
            loading: false,
          });
          showToast(friendlyError(e), 'error');
        });
    }

    aiExecuteQuery.set(null);
  });

  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  function handleQueryChange(q: string) {
    if (activeSqlTab) {
      setSqlTabData(activeSqlTab.id, { query: q });

      // Auto-save to database (debounced)
      if (activeSqlTab.key) {
        if (saveTimer) clearTimeout(saveTimer);
        const scriptId = activeSqlTab.key;
        const label = activeSqlTab.label;
        const db = currentDatabase;
        saveTimer = setTimeout(async () => {
          try {
            await updateSqlScript(scriptId, label, q, db);
          } catch {
            // Silent fail — will save on tab close
          }
        }, 1500);
      }
    }
  }

  function getConnectionLid(): string | null {
    if (!$activeConnection || !isConnected) return null;
    let lid = currentDatabase
      ? getDbLiveId($activeConnection.id, currentDatabase)
      : null;
    if (!lid) lid = getLiveId($activeConnection.id);
    return lid ?? null;
  }

  function applyRowLimit(query: string): string {
    const limit = get(sqlRowLimit);
    if (limit <= 0) return query;
    let trimmed = query.trim().replace(/;+\s*$/, '');
    // Only apply to SELECT queries that don't already have a LIMIT clause
    if (!/^\s*select\b/i.test(trimmed)) return query;
    if (/\bLIMIT\s+\d+/i.test(trimmed)) return query;
    // Strip trailing line comments so LIMIT isn't appended inside a comment
    trimmed = trimmed.replace(/--[^\n]*$/, '').trimEnd();
    return `${trimmed} LIMIT ${limit}`;
  }

  function makeResultLabel(query: string): string {
    const trimmed = query.trim().replace(/\s+/g, ' ');
    // Extract table name for a readable label
    const match = trimmed.match(/\b(?:FROM|INTO|UPDATE|TABLE|INDEX\s+(?:\w+\s+)?ON)\s+[`"']?(\w+)/i);
    if (match) return match[1];
    return trimmed.length > 30 ? trimmed.slice(0, 30) + '...' : trimmed;
  }

  async function handleExecute(q: string) {
    if (!$activeConnection || !isConnected) {
      showToast('Connect to a database first', 'error');
      return;
    }
    if (!activeSqlTab) return;

    const lid = getConnectionLid();
    if (!lid) { showToast('Connection lost -- reconnect', 'error'); return; }

    const tabId = activeSqlTab.id;
    const label = makeResultLabel(q);
    const existing = getSqlTabData(tabId).results || [];

    // Check if a result tab with the same label (table) already exists
    const existingIdx = existing.findIndex(e => e.label === label);

    setSqlTabData(tabId, { loading: true, error: null });

    try {
      const result = await sqlExecuteQuery(lid, applyRowLimit(q));
      const entry: SqlResultEntry = { label, query: q, result, error: null, connectionId: lid };

      let updated: SqlResultEntry[];
      let focusIdx: number;
      if (existingIdx >= 0) {
        // Update existing tab in place
        updated = [...existing];
        updated[existingIdx] = entry;
        focusIdx = existingIdx;
      } else {
        // Append new tab
        updated = [...existing, entry];
        focusIdx = updated.length - 1;
      }

      setSqlTabData(tabId, { result, loading: false, results: updated, activeResultIdx: focusIdx });
      showToast(`Query completed in ${result.durationMs}ms`, 'success');
    } catch (e: any) {
      const entry: SqlResultEntry = { label, query: q, result: null, error: e.toString(), connectionId: lid };

      let updated: SqlResultEntry[];
      let focusIdx: number;
      if (existingIdx >= 0) {
        updated = [...existing];
        updated[existingIdx] = entry;
        focusIdx = existingIdx;
      } else {
        updated = [...existing, entry];
        focusIdx = updated.length - 1;
      }

      setSqlTabData(tabId, { error: e.toString(), loading: false, results: updated, activeResultIdx: focusIdx });
      showToast(friendlyError(e), 'error');
    }
  }

  async function handleExecuteMulti(queries: string[]) {
    if (!$activeConnection || !isConnected) {
      showToast('Connect to a database first', 'error');
      return;
    }
    if (!activeSqlTab) return;

    const lid = getConnectionLid();
    if (!lid) { showToast('Connection lost -- reconnect', 'error'); return; }

    const tabId = activeSqlTab.id;
    const entries: SqlResultEntry[] = queries.map(q => ({
      label: makeResultLabel(q), query: q, result: null, error: null, connectionId: lid,
    }));

    setSqlTabData(tabId, { loading: true, result: null, error: null, results: entries, activeResultIdx: 0 });

    let successCount = 0;
    let errorCount = 0;

    for (let i = 0; i < queries.length; i++) {
      try {
        const result = await sqlExecuteQuery(lid, applyRowLimit(queries[i]));
        entries[i].result = result;
        successCount++;
      } catch (e: any) {
        entries[i].error = e.toString();
        errorCount++;
      }
      // Update state after each query so user sees progress
      setSqlTabData(tabId, { results: [...entries], activeResultIdx: i });
    }

    // Set final state — focus on last result tab
    const last = entries[entries.length - 1];
    setSqlTabData(tabId, {
      loading: false,
      result: last.result,
      error: last.error,
      results: entries,
      activeResultIdx: entries.length - 1,
    });

    if (errorCount === 0) {
      showToast(`${successCount} queries completed`, 'success');
    } else {
      showToast(`${successCount} succeeded, ${errorCount} failed`, errorCount === queries.length ? 'error' : 'info');
    }
  }

  function setActiveResult(idx: number) {
    if (!activeSqlTab) return;
    const entry = currentResults[idx];
    if (!entry) return;
    setSqlTabData(activeSqlTab.id, {
      activeResultIdx: idx,
      result: entry.result,
      error: entry.error,
    });
  }

  function closeResult(idx: number) {
    if (!activeSqlTab) return;
    const updated = [...currentResults];
    updated.splice(idx, 1);
    if (updated.length === 0) {
      setSqlTabData(activeSqlTab.id, { results: [], activeResultIdx: 0, result: null, error: null });
      return;
    }
    let newIdx = currentActiveResultIdx;
    if (newIdx >= updated.length) newIdx = updated.length - 1;
    const entry = updated[newIdx];
    setSqlTabData(activeSqlTab.id, {
      results: updated,
      activeResultIdx: newIdx,
      result: entry?.result ?? null,
      error: entry?.error ?? null,
    });
  }

  async function selectDatabase(connId: string, db: string) {
    dbDropdownOpen = false;
    activeConnectionId.set(connId);
    selectedDatabase.set(db);
    if (activeSqlTab) {
      setSqlTabData(activeSqlTab.id, { database: db });
    }
    // Ensure per-database connection exists
    try {
      await connectToDatabase(connId, db);
    } catch {
      // Non-fatal: will fall back to instance connection
    }
  }

  function toggleDbDropdown(e: MouseEvent) {
    e.stopPropagation();
    dbDropdownOpen = !dbDropdownOpen;
  }

  function closeDbDropdown() {
    dbDropdownOpen = false;
  }

  // Draggable divider
  function handleDividerMousedown(e: MouseEvent) {
    e.preventDefault();
    dragging = true;

    const startY = e.clientY;
    const startHeight = editorHeight;

    const onMousemove = (ev: MouseEvent) => {
      if (!containerEl) return;
      const rect = containerEl.getBoundingClientRect();
      const deltaY = ev.clientY - startY;
      const deltaPct = (deltaY / rect.height) * 100;
      editorHeight = Math.max(15, Math.min(85, startHeight + deltaPct));
    };

    const onMouseup = () => {
      dragging = false;
      window.removeEventListener('mousemove', onMousemove);
      window.removeEventListener('mouseup', onMouseup);
    };

    window.addEventListener('mousemove', onMousemove);
    window.addEventListener('mouseup', onMouseup);
  }
</script>

<svelte:window onclick={closeDbDropdown} />

{#if hasActiveSqlTab}
  <div class="sql-panel" bind:this={containerEl}>
    <!-- Action bar -->
    <div class="sql-action-bar">
      <div class="sql-action-left">
        <!-- Database target pill -->
        <div class="db-selector-wrap">
          <button class="db-pill" onclick={toggleDbDropdown} title="Select target database">
            {#if currentDbDriver}
              <span class="db-pill-driver">{driverLabel(currentDbDriver)}</span>
            {:else}
              <svg class="db-pill-icon" viewBox="0 0 24 24">
                <ellipse cx="12" cy="5" rx="9" ry="3"/>
                <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/>
                <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
              </svg>
            {/if}
            <span class="db-pill-name">{currentDatabase || 'No database'}</span>
            <svg class="db-pill-chevron" viewBox="0 0 24 24"><path d="M6 9l6 6 6-6"/></svg>
          </button>
          {#if dbDropdownOpen}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="db-dropdown" onclick={(e) => e.stopPropagation()}>
              {#if !hasAnyDatabases}
                <div class="db-dropdown-empty">No databases available</div>
              {:else}
                {#each groupedDatabases as group, gi}
                  {#if gi > 0}
                    <div class="db-dropdown-sep"></div>
                  {/if}
                  <div class="db-dropdown-group-header">
                    <span class="db-group-driver">{driverLabel(group.driver)}</span>
                    {group.connName}
                  </div>
                  {#each group.databases as db}
                    <button
                      class="db-dropdown-item"
                      class:active={db === currentDatabase && group.connId === $activeConnectionId}
                      onclick={() => selectDatabase(group.connId, db)}
                    >
                      <span class="db-dropdown-name">{db}</span>
                      {#if db === currentDatabase && group.connId === $activeConnectionId}
                        <svg class="db-dropdown-check" viewBox="0 0 24 24"><path d="M20 6L9 17l-5-5"/></svg>
                      {/if}
                    </button>
                  {/each}
                {/each}
              {/if}
            </div>
          {/if}
        </div>
      </div>
      <div class="sql-action-right">
        <button
          class="sql-execute-btn"
          onclick={() => queryEditorRef?.handleExecute()}
          disabled={currentLoading || !currentQuery.trim()}
          title={`Execute (${m}+Enter)`}
        >
          {#if currentLoading}
            Running...
          {:else}
            Execute &#9654;
          {/if}
        </button>
      </div>
    </div>

    <!-- Top: Query Editor -->
    <div class="sql-editor" style="height:{editorHeight}%">
      <QueryEditor
        bind:this={queryEditorRef}
        query={currentQuery}
        tables={tableList}
        {columnMap}
        onexecute={handleExecute}
        onexecutemulti={handleExecuteMulti}
        onquerychange={handleQueryChange}
      />
    </div>

    <!-- Draggable divider -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="sql-divider"
      class:active={dragging}
      onmousedown={handleDividerMousedown}
    ></div>

    <!-- Bottom: Results -->
    <div class="sql-results" style="height:{100 - editorHeight}%">
      {#if currentResults.length > 0}
        <div class="result-tabs">
          {#each currentResults as entry, idx}
            <button
              class="result-tab"
              class:active={idx === currentActiveResultIdx}
              class:has-error={!!entry.error}
              onclick={() => setActiveResult(idx)}
              title={entry.query}
            >
              {#if entry.error}
                <svg class="result-tab-icon err" viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.5"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
              {:else if entry.result}
                <svg class="result-tab-icon ok" viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg>
              {/if}
              <span class="result-tab-label">{entry.label}</span>
              {#if entry.result}
                <span class="result-tab-count">{entry.result.rows.length}</span>
              {/if}
              <span class="result-tab-close" role="button" tabindex="-1"
                onclick={(e) => { e.stopPropagation(); closeResult(idx); }}>
                <svg viewBox="0 0 24 24" width="8" height="8" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
              </span>
            </button>
          {/each}
        </div>
      {/if}
      <ResultsTable result={currentResult} error={currentError} loading={currentLoading} tabId={activeSqlTab?.id ?? -1} query={activeResultEntry?.query ?? currentQuery} liveConnectionId={activeResultEntry?.connectionId || currentLiveId} databaseName={currentDatabase} />
    </div>
  </div>
{:else}
  <div class="sql-empty-state">
    <div class="sql-empty-icon">
      <svg viewBox="0 0 24 24">
        <ellipse cx="12" cy="5" rx="9" ry="3"/>
        <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/>
        <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
      </svg>
    </div>
    <div class="sql-empty-text">Create a new query with the + button in the tab bar</div>
    <div class="sql-empty-hint">or press {m}+T</div>
    <div class="sql-empty-ai"><kbd>{m}+L</kbd> AI Assistant</div>
  </div>
{/if}

<style>
  .sql-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* Action bar */
  .sql-action-bar {
    height: 38px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 10px;
    gap: 8px;
  }

  .sql-action-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .sql-action-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  /* Database pill selector */
  .db-selector-wrap {
    position: relative;
  }

  .db-pill {
    height: 26px;
    padding: 0 8px 0 6px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12.5px;
    font-family: var(--mono);
    cursor: default;
    display: flex;
    align-items: center;
    gap: 5px;
    transition: border-color 0.12s, background 0.12s;
  }
  .db-pill:hover {
    border-color: var(--b2);
    background: rgba(255,255,255,0.03);
  }

  .db-pill-icon {
    width: 12px;
    height: 12px;
    stroke: var(--acc);
    fill: none;
    stroke-width: 1.5;
    stroke-linecap: round;
    flex-shrink: 0;
    opacity: 0.7;
  }

  .db-pill-name {
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .db-pill-chevron {
    width: 10px;
    height: 10px;
    stroke: var(--t3);
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex-shrink: 0;
  }

  .db-dropdown {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    min-width: 160px;
    max-height: 240px;
    overflow-y: auto;
    background: var(--n);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 4px 0;
    z-index: 100;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    animation: dbDropIn 0.1s ease;
  }
  @keyframes dbDropIn {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: none; }
  }

  .db-dropdown::-webkit-scrollbar {
    width: 4px;
  }
  .db-dropdown::-webkit-scrollbar-thumb {
    background: var(--b1);
    border-radius: 2px;
  }

  .db-pill-driver {
    font-size: 9px;
    font-weight: 700;
    color: var(--acc);
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    padding: 1px 4px;
    border-radius: 3px;
    font-family: var(--mono);
    flex-shrink: 0;
    letter-spacing: 0.04em;
  }

  .db-dropdown-empty {
    padding: 8px 14px;
    font-size: 11px;
    color: var(--t4);
    font-family: var(--mono);
  }

  .db-dropdown-group-header {
    font-size: 11px;
    font-weight: 500;
    color: var(--t3);
    text-transform: uppercase;
    padding: 6px 10px;
    font-family: var(--mono);
    font-weight: 600;
    letter-spacing: 0.04em;
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: default;
    user-select: none;
  }

  .db-group-driver {
    font-size: 8px;
    font-weight: 700;
    color: var(--acc);
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    padding: 1px 3px;
    border-radius: 2px;
    flex-shrink: 0;
  }

  .db-dropdown-sep {
    height: 1px;
    background: var(--b1);
    margin: 4px 0;
  }

  .db-dropdown-item {
    width: 100%;
    padding: 5px 10px 5px 20px;
    border: none;
    background: transparent;
    color: var(--t2);
    font-size: 11.5px;
    font-family: var(--mono);
    cursor: default;
    text-align: left;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    transition: background 0.08s, color 0.08s;
  }
  .db-dropdown-item:hover {
    background: var(--c);
    color: var(--t1);
  }
  .db-dropdown-item.active {
    color: var(--acc);
  }

  .db-dropdown-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .db-dropdown-check {
    width: 12px;
    height: 12px;
    stroke: var(--acc);
    fill: none;
    stroke-width: 2.5;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex-shrink: 0;
  }

  /* Execute button */
  .sql-execute-btn {
    height: 34px;
    padding: 0 20px;
    border-radius: 8px;
    border: none;
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 600;
    cursor: default;
    flex-shrink: 0;
    color: #fff;
    background: var(--acc);
    transition: opacity 0.12s;
  }
  .sql-execute-btn:hover:not(:disabled) {
    opacity: 0.85;
  }
  .sql-execute-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .sql-editor {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 80px;
    min-width: 0;
  }

  .sql-divider {
    height: 4px;
    flex-shrink: 0;
    background: var(--b1);
    cursor: row-resize;
    position: relative;
    transition: background 0.12s;
  }
  .sql-divider:hover,
  .sql-divider.active {
    background: var(--acc);
  }

  .sql-results {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 60px;
  }

  /* Result tabs */
  .result-tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--b1);
    background: var(--n2);
    overflow-x: auto;
    flex-shrink: 0;
  }
  .result-tabs::-webkit-scrollbar { height: 2px; }
  .result-tabs::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 1px; }

  .result-tab {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 8px;
    border-radius: 6px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--t3);
    font-size: 11px;
    font-family: var(--ui);
    cursor: default;
    white-space: nowrap;
    transition: all 0.1s;
    max-width: 200px;
  }
  .result-tab:hover {
    background: var(--c);
    color: var(--t2);
  }
  .result-tab.active {
    background: color-mix(in srgb, var(--acc) 10%, transparent);
    color: var(--t1);
    border-color: color-mix(in srgb, var(--acc) 30%, transparent);
    font-weight: 500;
  }
  .result-tab.has-error {
    color: var(--err);
  }
  .result-tab.has-error.active {
    background: color-mix(in srgb, var(--err) 8%, transparent);
    border-color: color-mix(in srgb, var(--err) 25%, transparent);
    color: var(--err);
  }

  .result-tab-icon { flex-shrink: 0; }
  .result-tab-icon.ok { stroke: var(--ok); }
  .result-tab-icon.err { stroke: var(--err); }

  .result-tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 140px;
  }

  .result-tab-count {
    font-size: 9px;
    background: rgba(255,255,255,0.08);
    padding: 1px 4px;
    border-radius: 3px;
    color: var(--t3);
    flex-shrink: 0;
  }
  .result-tab.active .result-tab-count {
    background: color-mix(in srgb, var(--acc) 15%, transparent);
    color: var(--acc);
  }

  .result-tab-close {
    display: none;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 3px;
    flex-shrink: 0;
    color: var(--t4);
    transition: all 0.1s;
  }
  .result-tab:hover .result-tab-close { display: flex; }
  .result-tab-close:hover { background: rgba(255,255,255,0.1); color: var(--t1); }


  /* Empty state */
  .sql-empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    background: transparent;
  }

  .sql-empty-icon svg {
    width: 40px;
    height: 40px;
    stroke: var(--t4);
    fill: none;
    stroke-width: 1.2;
    stroke-linecap: round;
  }

  .sql-empty-text {
    font-size: 13px;
    color: var(--t3);
    font-family: var(--mono);
  }

  .sql-empty-hint {
    font-size: 11px;
    color: var(--t4);
    font-family: var(--mono);
  }
  .sql-empty-ai {
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
  .sql-empty-ai kbd {
    background: var(--b1);
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 10px;
    color: var(--t3);
  }
</style>

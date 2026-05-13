<script lang="ts">
  import { mod } from '$lib/utils/platform';
  const m = mod();
  import QueryEditor from './QueryEditor.svelte';
  import ResultsTable from './ResultsTable.svelte';
  import {
    connections,
    sqlTabState,
    setSqlTabData,
    getSqlTabData,
    ensureConnected,
    setBinding,
    cancelQuery,
    poolStates,
    poolErrors,
    reconnectingFlash,
    poolKey,
    connectionDatabases,
    databaseTables,
    loadConnections,
    loadDatabaseList,
    loadTablesForDb,
    insertQueryText,
    aiExecuteQuery,
    sqlRowLimit,
    updateSqlScript,
    registerSqlEventListeners,
  } from '../stores';
  import { tabs, activeTabId, addTab } from '$lib/shared/stores/tabs';
  import { sqlExecuteQuery, sqlDescribeTable } from '../commands';
  import type { TableInfo, SqlResultEntry, ColumnInfo, SqlQueryResult, Binding } from '../types';
  import { descriptorFor } from '../dialects';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { get } from 'svelte/store';
  import { splitSqlStatements } from '../utils/splitter';

  // Component-local UI state
  let editorHeight = $state(45);
  let dragging = $state(false);
  let containerEl: HTMLDivElement | undefined = $state();
  let dbDropdownOpen = $state(false);
  let elapsedMs = $state(0);

  // Load saved connections + register Tauri event listeners once on mount.
  loadConnections();
  registerSqlEventListeners();

  // --- Active-tab + per-tab derived ----------------------------------------

  const activeSqlTab = $derived($tabs.find((t) => t.id === $activeTabId && t.mode === 'sql'));
  const hasActiveSqlTab = $derived(!!activeSqlTab);

  const currentTabData = $derived.by(() => {
    if (!activeSqlTab) return null;
    return (
      $sqlTabState.get(activeSqlTab.id) ?? {
        binding: null,
        query: '',
        result: null,
        error: null,
        inFlight: null,
        results: [],
        activeResultIdx: 0,
      }
    );
  });

  const binding = $derived(currentTabData?.binding ?? null);
  const currentQuery = $derived(currentTabData?.query ?? '');
  const inFlight = $derived(currentTabData?.inFlight ?? null);
  const currentResults = $derived(currentTabData?.results ?? []);
  const currentActiveResultIdx = $derived(currentTabData?.activeResultIdx ?? 0);

  const boundConnection = $derived(
    binding ? $connections.find((c) => c.id === binding.connectionId) ?? null : null,
  );
  const currentDatabase = $derived(binding?.database ?? '');
  const currentDbDriver = $derived(boundConnection?.driver ?? '');

  const currentPoolKey = $derived(binding ? poolKey(binding.connectionId, binding.database) : '');
  const poolState = $derived(currentPoolKey ? $poolStates.get(currentPoolKey) ?? 'idle' : 'idle');
  const poolError = $derived(currentPoolKey ? $poolErrors.get(currentPoolKey) ?? null : null);
  const isConnecting = $derived(poolState === 'connecting');
  const isConnected = $derived(poolState === 'connected');
  const reconnecting = $derived(currentPoolKey ? $reconnectingFlash.has(currentPoolKey) : false);

  // Active result entry shape (from multi-results or fallback to single).
  const activeResultEntry = $derived.by(() => {
    if (currentResults.length > 0 && currentActiveResultIdx < currentResults.length) {
      return currentResults[currentActiveResultIdx];
    }
    return null;
  });
  const currentResult = $derived(activeResultEntry?.result ?? currentTabData?.result ?? null);
  const currentError = $derived(activeResultEntry?.error ?? currentTabData?.error ?? null);

  // Tables for current binding — used by editor autocomplete + nav inserts.
  const tableList = $derived.by(() => {
    if (!binding) return [] as TableInfo[];
    const key = `${binding.connectionId}:${binding.database}`;
    return $databaseTables.get(key) ?? ([] as TableInfo[]);
  });

  // Column autocomplete map: table → column names.
  let columnMap = $state<Record<string, string[]>>({});
  let columnMapKey = '';

  $effect(() => {
    const tables = tableList;
    const b = binding;
    if (!b || tables.length === 0) {
      columnMap = {};
      return;
    }
    const key = `${b.connectionId}:${b.database}`;
    if (key === columnMapKey) return;
    columnMapKey = key;

    const fetchColumns = async () => {
      const map: Record<string, string[]> = {};
      const batch = tables.slice(0, 50);
      const results = await Promise.allSettled(
        batch.map(async (t) => {
          const cols = await sqlDescribeTable(b.connectionId, b.database, t.name);
          return { name: t.name, cols: cols.map((c: ColumnInfo) => c.name) };
        }),
      );
      for (const r of results) {
        if (r.status === 'fulfilled') map[r.value.name] = r.value.cols;
      }
      columnMap = map;
    };
    fetchColumns();
  });

  // --- Auto-connect on binding change --------------------------------------
  //
  // Fires ONCE per unique `(connId, db)` binding. We deliberately don't
  // re-fire when the pool's state changes — otherwise a user-initiated
  // disconnect (which transitions the pool out of `connected`) would
  // immediately re-trigger ensureConnected and silently reconnect, defeating
  // the disconnect. The user explicitly opens the pool again by pressing
  // Run, picking a DB from the pill, or switching tabs.
  let lastAutoConnectKey = $state('');
  $effect(() => {
    const b = binding;
    if (!b) {
      lastAutoConnectKey = '';
      return;
    }
    const k = poolKey(b.connectionId, b.database);
    if (k === lastAutoConnectKey) return;
    lastAutoConnectKey = k;
    ensureConnected(b.connectionId, b.database).catch(() => {
      /* surfaced via poolErrors */
    });
  });

  // Refresh database + table caches once a pool becomes connected, so
  // the DB dropdown and editor autocomplete see real data.
  $effect(() => {
    const b = binding;
    if (!b || poolState !== 'connected') return;
    if (!$connectionDatabases.has(b.connectionId)) {
      loadDatabaseList(b.connectionId);
    }
    const tableKey = `${b.connectionId}:${b.database}`;
    if (!$databaseTables.has(tableKey)) {
      loadTablesForDb(b.connectionId, b.database);
    }
  });

  // Elapsed-time counter while a query is in flight.
  $effect(() => {
    if (!inFlight) {
      elapsedMs = 0;
      return;
    }
    const startedAt = inFlight.startedAt;
    elapsedMs = Date.now() - startedAt;
    const id = setInterval(() => {
      elapsedMs = Date.now() - startedAt;
    }, 100);
    return () => clearInterval(id);
  });

  // --- DB dropdown — lists every saved connection's databases --------------

  interface DbGroup {
    connId: string;
    connName: string;
    driver: string;
    state: 'idle' | 'connecting' | 'connected' | 'error';
    databases: string[];
  }

  const dbGroups = $derived.by<DbGroup[]>(() => {
    const all = $connections;
    return all.map((conn) => {
      const cached = $connectionDatabases.get(conn.id);
      const databases = cached && cached.length > 0 ? cached : [conn.databaseName];
      // Group state = connected if ANY pool on this conn is connected.
      const prefix = `${conn.id}:`;
      let state: DbGroup['state'] = 'idle';
      for (const [k, s] of $poolStates) {
        if (k.startsWith(prefix)) {
          if (s === 'connecting') state = 'connecting';
          else if (s === 'connected' && state !== 'connecting') state = 'connected';
          else if (s === 'error' && state === 'idle') state = 'error';
        }
      }
      return {
        connId: conn.id,
        connName: conn.name,
        driver: conn.driver,
        state,
        databases,
      };
    });
  });

  function driverLabel(driver: string): string {
    return descriptorFor(driver)?.abbreviation ?? (driver ? driver.substring(0, 2).toUpperCase() : '?');
  }

  function toggleDbDropdown(e: MouseEvent) {
    e.stopPropagation();
    dbDropdownOpen = !dbDropdownOpen;
    // Lazy-load database lists for connected pools when the dropdown opens.
    if (dbDropdownOpen) {
      for (const g of dbGroups) {
        if (g.state === 'connected' && !$connectionDatabases.has(g.connId)) {
          loadDatabaseList(g.connId);
        }
      }
    }
  }
  function closeDbDropdown() {
    dbDropdownOpen = false;
  }

  async function pickBinding(connId: string, db: string) {
    dbDropdownOpen = false;
    if (!activeSqlTab) return;
    if (binding?.connectionId === connId && binding?.database === db) return;
    setBinding(activeSqlTab.id, connId, db);
    // Persist the script's saved binding IMMEDIATELY on a DB pick.
    // Typing autosave is debounced 1.5s because it fires per keystroke,
    // but picking a DB is a single deliberate gesture — waiting for the
    // next keystroke (which may never come if the user just closes the
    // app) would lose the choice. Failures are silent; the close-tab
    // autosave path will retry.
    if (activeSqlTab.key) {
      try {
        await updateSqlScript(activeSqlTab.key, activeSqlTab.label, currentQuery, db, connId);
      } catch {
        /* silent — next save will retry */
      }
    }
  }

  // --- Insert-from-nav effect ----------------------------------------------

  $effect(() => {
    const text = $insertQueryText;
    if (text && activeSqlTab) {
      const existing = currentQuery.trim();
      const newQuery = existing ? existing + '\n\n' + text : text;
      setSqlTabData(activeSqlTab.id, { query: newQuery });
      insertQueryText.set('');
    }
  });

  // --- AI-triggered execution effect ---------------------------------------

  $effect(() => {
    const exec = $aiExecuteQuery;
    if (!exec) return;
    // `triggerAiSqlExecution` already opened/focused the right tab + set
    // the binding + query. Just fire Run via handleExecute on the active
    // tab. Defer one microtask so the latest store state is committed.
    const tabIdSnapshot = activeSqlTab?.id;
    aiExecuteQuery.set(null);
    if (tabIdSnapshot != null) {
      queueMicrotask(() => handleExecute(exec.query));
    }
  });

  // --- Query change autosave -----------------------------------------------

  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  function handleQueryChange(q: string) {
    if (!activeSqlTab) return;
    setSqlTabData(activeSqlTab.id, { query: q });

    if (activeSqlTab.key) {
      if (saveTimer) clearTimeout(saveTimer);
      const scriptId = activeSqlTab.key;
      const label = activeSqlTab.label;
      // Autosave the CURRENT binding alongside name+query. Both fields
      // update atomically via COALESCE on the backend, so the script's
      // `(connection_id, database_name)` pair can never end up
      // mismatched — they always move together. Picking a different
      // DB from the pill, then typing, persists the new target.
      const bConn = binding?.connectionId;
      const bDb = binding?.database;
      saveTimer = setTimeout(async () => {
        try {
          await updateSqlScript(scriptId, label, q, bDb, bConn);
        } catch {
          /* silent — retry on close */
        }
      }, 1500);
    }
  }

  // --- LIMIT injection + result-label helpers (preserved) ------------------

  function applyRowLimit(query: string): string {
    const limit = get(sqlRowLimit);
    if (limit <= 0) return query;
    let trimmed = query.trim().replace(/;+\s*$/, '');
    if (!/^\s*select\b/i.test(trimmed)) return query;
    if (/\bLIMIT\s+\d+/i.test(trimmed)) return query;
    if (/\bFORMAT\s+\w+\s*$/i.test(trimmed)) return query;
    if (/\bSETTINGS\b/i.test(trimmed)) return query;
    trimmed = trimmed.replace(/--[^\n]*$/, '').trimEnd();
    return `${trimmed} LIMIT ${limit}`;
  }

  function makeResultLabel(query: string): string {
    const trimmed = query.trim().replace(/\s+/g, ' ');
    const match = trimmed.match(/\b(?:FROM|INTO|UPDATE|TABLE|INDEX\s+(?:\w+\s+)?ON)\s+[`"']?(\w+)/i);
    if (match) return match[1];
    return trimmed.length > 30 ? trimmed.slice(0, 30) + '...' : trimmed;
  }

  // --- Execute / Cancel ----------------------------------------------------

  let queryEditorRef: { handleExecute: () => void } | undefined = $state();

  function makeQueryId(): string {
    return (globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`);
  }

  async function handleExecute(q: string) {
    if (!activeSqlTab || !binding) {
      showToast('Pick a connection first', 'error');
      return;
    }
    if (currentTabData?.inFlight) return; // structurally blocked, defensive

    // Always re-check the pool before executing. ensureConnected is a
    // no-op when already connected. If the user clicks Run during the
    // connect handshake, this awaits the same in-flight Promise.
    try {
      await ensureConnected(binding.connectionId, binding.database);
    } catch (e: any) {
      showToast(`Couldn't connect: ${friendlyError(e)}`, 'error');
      return;
    }

    const tabId = activeSqlTab.id;
    const queryId = makeQueryId();
    const label = makeResultLabel(q);
    const existing = getSqlTabData(tabId).results || [];
    const existingIdx = existing.findIndex((e) => e.label === label);

    const startedAt = Date.now();
    setSqlTabData(tabId, {
      inFlight: { queryId, startedAt },
      error: null,
    });

    try {
      const result = await sqlExecuteQuery(
        binding.connectionId,
        binding.database,
        applyRowLimit(q),
        queryId,
      );
      const entry: SqlResultEntry = { label, query: q, result, error: null, startedAt };
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
      setSqlTabData(tabId, {
        result,
        results: updated,
        activeResultIdx: focusIdx,
        inFlight: null,
      });
      showToast(`Query completed in ${result.durationMs}ms`, 'success');
    } catch (e: any) {
      const msg = e?.toString?.() ?? String(e);
      const entry: SqlResultEntry = { label, query: q, result: null, error: msg, startedAt };
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
      setSqlTabData(tabId, {
        error: msg,
        results: updated,
        activeResultIdx: focusIdx,
        inFlight: null,
      });
      showToast(friendlyError(e), 'error');
    }
  }

  async function handleExecuteMulti(queries: string[]) {
    if (!activeSqlTab || !binding) {
      showToast('Pick a connection first', 'error');
      return;
    }
    if (currentTabData?.inFlight) return;

    try {
      await ensureConnected(binding.connectionId, binding.database);
    } catch (e: any) {
      showToast(`Couldn't connect: ${friendlyError(e)}`, 'error');
      return;
    }

    const tabId = activeSqlTab.id;
    const queryId = makeQueryId();
    const entries: SqlResultEntry[] = queries.map((q) => ({
      label: makeResultLabel(q),
      query: q,
      result: null,
      error: null,
    }));

    setSqlTabData(tabId, {
      inFlight: { queryId, startedAt: Date.now() },
      result: null,
      error: null,
      results: entries,
      activeResultIdx: 0,
    });

    let successCount = 0;
    let errorCount = 0;
    for (let i = 0; i < queries.length; i++) {
      entries[i].startedAt = Date.now();
      try {
        // Each statement gets its own queryId so cancel works mid-batch.
        const subId = makeQueryId();
        const result = await sqlExecuteQuery(
          binding.connectionId,
          binding.database,
          applyRowLimit(queries[i]),
          subId,
        );
        entries[i].result = result;
        successCount++;
      } catch (e: any) {
        entries[i].error = e?.toString?.() ?? String(e);
        errorCount++;
      }
      setSqlTabData(tabId, { results: [...entries], activeResultIdx: i });
    }

    const last = entries[entries.length - 1];
    setSqlTabData(tabId, {
      inFlight: null,
      result: last?.result ?? null,
      error: last?.error ?? null,
      results: entries,
      activeResultIdx: entries.length - 1,
    });

    if (errorCount === 0) {
      showToast(`${successCount} queries completed`, 'success');
    } else {
      showToast(
        `${successCount} succeeded, ${errorCount} failed`,
        errorCount === queries.length ? 'error' : 'info',
      );
    }
  }

  async function handleCancel() {
    if (!activeSqlTab) return;
    await cancelQuery(activeSqlTab.id);
  }

  function retryConnect() {
    if (!binding) return;
    ensureConnected(binding.connectionId, binding.database).catch(() => {
      /* surfaced via poolErrors */
    });
  }

  // --- Result tabs ---------------------------------------------------------

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

  // --- Divider drag --------------------------------------------------------

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

  function fmtElapsed(ms: number): string {
    const s = (ms / 1000).toFixed(1);
    return `${s}s`;
  }
</script>

<svelte:window onclick={closeDbDropdown} />

{#if hasActiveSqlTab}
  <div class="sql-panel" bind:this={containerEl}>
    <!-- Action bar -->
    <div class="sql-action-bar">
      <div class="sql-action-left">
        <div class="db-selector-wrap">
          <button
            class="db-pill"
            class:state-connecting={isConnecting}
            class:state-error={poolState === 'error'}
            onclick={toggleDbDropdown}
            title="Select target connection / database"
          >
            {#if currentDbDriver}
              <span class="db-pill-driver">{driverLabel(currentDbDriver)}</span>
            {:else}
              <svg class="db-pill-icon" viewBox="0 0 24 24">
                <ellipse cx="12" cy="5" rx="9" ry="3" />
                <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
                <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
              </svg>
            {/if}
            <span class="db-pill-name">
              {#if binding && boundConnection}
                {boundConnection.name} / {binding.database}
              {:else}
                Pick connection
              {/if}
            </span>
            {#if isConnecting}
              <span class="state-dot connecting" title="Connecting…"></span>
            {:else if reconnecting}
              <span class="state-dot reconnecting" title="Reconnecting…"></span>
            {:else if poolState === 'error'}
              <span class="state-dot err" title={poolError ?? 'Connection error'}></span>
            {/if}
            <svg class="db-pill-chevron" viewBox="0 0 24 24"><path d="M6 9l6 6 6-6" /></svg>
          </button>

          {#if dbDropdownOpen}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="db-dropdown" onclick={(e) => e.stopPropagation()}>
              {#if dbGroups.length === 0}
                <div class="db-dropdown-empty">No saved connections — create one from the SQL sidebar</div>
              {:else}
                {#each dbGroups as group, gi}
                  {#if gi > 0}
                    <div class="db-dropdown-sep"></div>
                  {/if}
                  <div class="db-dropdown-group-header">
                    <span class="db-group-driver">{driverLabel(group.driver)}</span>
                    <span class="db-group-name">{group.connName}</span>
                    {#if group.state === 'connected'}
                      <span class="group-state ok" title="Pool open"></span>
                    {:else if group.state === 'connecting'}
                      <span class="group-state connecting" title="Connecting…"></span>
                    {:else if group.state === 'error'}
                      <span class="group-state err" title="Connection error"></span>
                    {/if}
                  </div>
                  {#each group.databases as db}
                    {@const sel = binding?.connectionId === group.connId && binding?.database === db}
                    <button
                      class="db-dropdown-item"
                      class:active={sel}
                      onclick={() => pickBinding(group.connId, db)}
                    >
                      <span class="db-dropdown-name">{db}</span>
                      {#if sel}
                        <svg class="db-dropdown-check" viewBox="0 0 24 24"><path d="M20 6L9 17l-5-5" /></svg>
                      {/if}
                    </button>
                  {/each}
                {/each}
              {/if}
            </div>
          {/if}
        </div>

        {#if inFlight}
          <span class="elapsed">⏱ {fmtElapsed(elapsedMs)}</span>
        {/if}
      </div>

      <div class="sql-action-right">
        {#if inFlight}
          <button class="sql-cancel-btn" onclick={handleCancel} title="Cancel running query">
            Cancel
          </button>
        {/if}
        <button
          class="sql-execute-btn"
          onclick={() => queryEditorRef?.handleExecute()}
          disabled={!!inFlight || isConnecting || !binding || !currentQuery.trim()}
          title={`Execute (${m}+Enter)`}
        >
          {#if inFlight}
            Running…
          {:else if isConnecting}
            Connecting…
          {:else}
            Execute &#9654;
          {/if}
        </button>
      </div>
    </div>

    <!-- Unbound banner: new tab, or saved-connection deleted with no
         other connections to fall back to. Friendly prompt to pick a
         target from the pill above. -->
    {#if !binding}
      <div class="unbound-banner">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"
          ><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>
        <span>
          {$connections.length === 0
            ? 'No saved connections yet. Create one from the SQL sidebar to start querying.'
            : 'Pick a database from the dropdown above to start querying.'}
        </span>
      </div>
    {/if}

    <!-- Top: Query Editor -->
    <!-- Keyed by tab id so each tab owns its CodeMirror EditorView (and
         therefore its own undo history). Sharing one editor across tabs
         leaked cmd+z across tab boundaries. -->
    <div class="sql-editor" style="height:{editorHeight}%">
      {#key activeSqlTab?.id}
        <QueryEditor
          bind:this={queryEditorRef}
          query={currentQuery}
          tables={tableList}
          {columnMap}
          disabled={!!inFlight || isConnecting}
          onexecute={handleExecute}
          onexecutemulti={handleExecuteMulti}
          onquerychange={handleQueryChange}
        />
      {/key}
    </div>

    <!-- Draggable divider -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="sql-divider" class:active={dragging} onmousedown={handleDividerMousedown}></div>

    <!-- Bottom: Results -->
    <div class="sql-results" style="height:{100 - editorHeight}%">
      {#if currentResults.length > 0 && !inFlight && !isConnecting && poolState !== 'error'}
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
                <svg class="result-tab-icon err" viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.5">
                  <circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" />
                </svg>
              {:else if entry.result}
                <svg class="result-tab-icon ok" viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.5">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              {/if}
              <span class="result-tab-label">{entry.label}</span>
              {#if entry.result}
                <span class="result-tab-count">{entry.result.rows.length}</span>
              {/if}
              <span
                class="result-tab-close"
                role="button"
                tabindex="-1"
                onclick={(e) => {
                  e.stopPropagation();
                  closeResult(idx);
                }}
              >
                <svg viewBox="0 0 24 24" width="8" height="8" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round">
                  <path d="M18 6L6 18M6 6l12 12" />
                </svg>
              </span>
            </button>
          {/each}
        </div>
      {/if}
      <ResultsTable
        result={currentResult}
        error={currentError}
        loading={!!inFlight}
        tabId={activeSqlTab?.id ?? -1}
        query={activeResultEntry?.query ?? currentQuery}
        startedAt={activeResultEntry?.startedAt}
        liveConnectionId={binding ? `${binding.connectionId}:${binding.database}` : ''}
        databaseName={currentDatabase}
        poolState={isConnecting ? 'connecting' : poolState === 'error' ? 'error' : 'idle'}
        connectingLabel={boundConnection && binding ? `${boundConnection.name} / ${binding.database}` : ''}
        connectError={poolError}
        elapsedMs={elapsedMs}
        oncancel={handleCancel}
        onretry={retryConnect}
      />
    </div>
  </div>
{:else}
  <div class="sql-empty-state">
    <div class="sql-empty-icon">
      <svg viewBox="0 0 24 24">
        <ellipse cx="12" cy="5" rx="9" ry="3" />
        <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
        <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
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
  .sql-action-left { display: flex; align-items: center; gap: 8px; }
  .sql-action-right { display: flex; align-items: center; gap: 8px; }

  .db-selector-wrap { position: relative; }
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
  .db-pill:hover { border-color: var(--b2); background: rgba(255,255,255,0.03); }
  .db-pill.state-connecting { border-color: color-mix(in srgb, var(--acc) 50%, transparent); }
  .db-pill.state-error { border-color: color-mix(in srgb, var(--err) 60%, transparent); }
  .db-pill-icon { width: 12px; height: 12px; stroke: var(--acc); fill: none; stroke-width: 1.5; stroke-linecap: round; flex-shrink: 0; opacity: 0.7; }
  .db-pill-name { max-width: 240px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .db-pill-chevron { width: 10px; height: 10px; stroke: var(--t3); fill: none; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; flex-shrink: 0; }

  .state-dot {
    width: 7px; height: 7px; border-radius: 50%;
    flex-shrink: 0; display: inline-block;
  }
  .state-dot.connecting { background: var(--acc); animation: pulse 1.1s ease-in-out infinite; }
  .state-dot.reconnecting { background: #d97706; animation: pulse 0.8s ease-in-out infinite; }
  .state-dot.err { background: var(--err); }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.35; }
  }

  .db-dropdown {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    min-width: 200px;
    max-height: 300px;
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
  .db-dropdown::-webkit-scrollbar { width: 4px; }
  .db-dropdown::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .db-pill-driver {
    font-size: 9px; font-weight: 700; color: var(--acc);
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    padding: 1px 4px; border-radius: 3px; font-family: var(--mono);
    flex-shrink: 0; letter-spacing: 0.04em;
  }
  .db-dropdown-empty { padding: 10px 14px; font-size: 11px; color: var(--t4); font-family: var(--mono); }
  .db-dropdown-group-header {
    font-size: 11px; font-weight: 600; color: var(--t3);
    text-transform: uppercase; padding: 6px 10px; font-family: var(--mono);
    letter-spacing: 0.04em; display: flex; align-items: center; gap: 6px;
    cursor: default; user-select: none;
  }
  .db-group-driver {
    font-size: 8px; font-weight: 700; color: var(--acc);
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    padding: 1px 3px; border-radius: 2px; flex-shrink: 0;
  }
  .db-group-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .group-state {
    width: 6px; height: 6px; border-radius: 50%; margin-left: auto; flex-shrink: 0;
  }
  .group-state.ok { background: #10b981; }
  .group-state.connecting { background: var(--acc); animation: pulse 1.1s ease-in-out infinite; }
  .group-state.err { background: var(--err); }

  .db-dropdown-sep { height: 1px; background: var(--b1); margin: 4px 0; }
  .db-dropdown-item {
    width: 100%; padding: 5px 10px 5px 20px; border: none;
    background: transparent; color: var(--t2); font-size: 11.5px;
    font-family: var(--mono); cursor: default; text-align: left;
    display: flex; align-items: center; justify-content: space-between;
    gap: 8px; transition: background 0.08s, color 0.08s;
  }
  .db-dropdown-item:hover { background: var(--c); color: var(--t1); }
  .db-dropdown-item.active { color: var(--acc); }
  .db-dropdown-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .db-dropdown-check { width: 12px; height: 12px; stroke: var(--acc); fill: none; stroke-width: 2.5; stroke-linecap: round; stroke-linejoin: round; flex-shrink: 0; }

  .elapsed {
    font-size: 11.5px; font-family: var(--mono);
    color: var(--t3); padding-left: 6px;
    border-left: 1px solid var(--b1); margin-left: 4px;
  }

  .sql-cancel-btn {
    height: 34px; padding: 0 16px;
    border-radius: 8px; border: 1px solid color-mix(in srgb, var(--err) 50%, transparent);
    background: transparent; color: var(--err);
    font-family: var(--ui); font-size: 12px; font-weight: 600;
    cursor: default; transition: background 0.12s;
  }
  .sql-cancel-btn:hover { background: color-mix(in srgb, var(--err) 12%, transparent); }

  .sql-execute-btn {
    height: 34px; padding: 0 20px;
    border-radius: 8px; border: none;
    font-family: var(--ui); font-size: 12px; font-weight: 600;
    cursor: default; flex-shrink: 0; color: #fff;
    background: var(--acc); transition: opacity 0.12s;
  }
  .sql-execute-btn:hover:not(:disabled) { opacity: 0.85; }
  .sql-execute-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  .unbound-banner {
    display: flex; align-items: center; gap: 8px;
    padding: 8px 12px; background: color-mix(in srgb, #d97706 10%, transparent);
    border-bottom: 1px solid color-mix(in srgb, #d97706 30%, transparent);
    color: var(--t2); font-size: 12px; font-family: var(--ui);
  }
  .unbound-banner svg { color: #d97706; flex-shrink: 0; }

  .sql-editor { display: flex; flex-direction: column; overflow: hidden; min-height: 80px; min-width: 0; }

  .sql-divider {
    height: 4px; flex-shrink: 0; background: var(--b1);
    cursor: row-resize; position: relative; transition: background 0.12s;
  }
  .sql-divider:hover, .sql-divider.active { background: var(--acc); }

  .sql-results { display: flex; flex-direction: column; overflow: hidden; min-height: 60px; }

  .result-tabs {
    display: flex; align-items: center; gap: 2px;
    padding: 4px 8px; border-bottom: 1px solid var(--b1);
    background: var(--n2); overflow-x: auto; flex-shrink: 0;
  }
  .result-tabs::-webkit-scrollbar { height: 2px; }
  .result-tabs::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 1px; }

  .result-tab {
    display: flex; align-items: center; gap: 5px;
    padding: 4px 8px; border-radius: 6px;
    border: 1px solid transparent; background: transparent;
    color: var(--t3); font-size: 11px; font-family: var(--ui);
    cursor: default; white-space: nowrap;
    transition: all 0.1s; max-width: 200px;
  }
  .result-tab:hover { background: var(--c); color: var(--t2); }
  .result-tab.active {
    background: color-mix(in srgb, var(--acc) 10%, transparent);
    color: var(--t1); border-color: color-mix(in srgb, var(--acc) 30%, transparent);
    font-weight: 500;
  }
  .result-tab.has-error { color: var(--err); }
  .result-tab.has-error.active {
    background: color-mix(in srgb, var(--err) 8%, transparent);
    border-color: color-mix(in srgb, var(--err) 25%, transparent);
    color: var(--err);
  }
  .result-tab-icon { flex-shrink: 0; }
  .result-tab-icon.ok { stroke: var(--ok); }
  .result-tab-icon.err { stroke: var(--err); }
  .result-tab-label { overflow: hidden; text-overflow: ellipsis; max-width: 140px; }
  .result-tab-count {
    font-size: 9px; background: rgba(255,255,255,0.08);
    padding: 1px 4px; border-radius: 3px; color: var(--t3); flex-shrink: 0;
  }
  .result-tab.active .result-tab-count {
    background: color-mix(in srgb, var(--acc) 15%, transparent); color: var(--acc);
  }
  .result-tab-close {
    display: none; align-items: center; justify-content: center;
    width: 14px; height: 14px; border-radius: 3px;
    flex-shrink: 0; color: var(--t4); transition: all 0.1s;
  }
  .result-tab:hover .result-tab-close { display: flex; }
  .result-tab-close:hover { background: rgba(255,255,255,0.1); color: var(--t1); }

  .sql-empty-state {
    flex: 1; display: flex; flex-direction: column;
    align-items: center; justify-content: center;
    gap: 10px; background: transparent;
  }
  .sql-empty-icon svg {
    width: 40px; height: 40px; stroke: var(--t4);
    fill: none; stroke-width: 1.2; stroke-linecap: round;
  }
  .sql-empty-text { font-size: 13px; color: var(--t3); font-family: var(--mono); }
  .sql-empty-hint { font-size: 11px; color: var(--t4); font-family: var(--mono); }
  .sql-empty-ai {
    margin-top: 12px; padding: 5px 14px;
    border: 1px solid var(--b1); border-radius: 6px;
    font-size: 11px; color: var(--t4); font-family: var(--mono);
    display: inline-flex; align-items: center; gap: 6px;
  }
  .sql-empty-ai kbd {
    background: var(--b1); padding: 1px 5px;
    border-radius: 3px; font-size: 10px; color: var(--t3);
  }
</style>

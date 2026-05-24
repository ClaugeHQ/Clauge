<script lang="ts">
  import type { TableInfo, ColumnInfo } from '../types';
  import { sqlListTables, sqlDescribeTable, sqlListDatabases } from '../commands';
  import { activeConnection, connectedIds, getLiveId } from '../stores';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';

  interface Props {
    oninsertquery?: (query: string) => void;
  }

  let { oninsertquery }: Props = $props();

  let databases = $state<string[]>([]);
  let selectedDatabase = $state('');
  let tables = $state<TableInfo[]>([]);
  let expandedTable = $state<string | null>(null);
  let tableColumns = $state<Record<string, ColumnInfo[]>>({});
  let loadingTables = $state(false);
  let loadingColumns = $state<string | null>(null);

  const isConnected = $derived(
    $activeConnection ? $connectedIds.has($activeConnection.id) : false
  );

  $effect(() => {
    if (isConnected && $activeConnection) {
      loadDatabases();
      loadTables();
    } else {
      databases = [];
      tables = [];
      tableColumns = {};
      expandedTable = null;
    }
  });

  // Helper: split a `${connId}:${db}` pool key into its parts.
  function splitLid(lid: string | null): { connId: string; db: string } | null {
    if (!lid) return null;
    const i = lid.indexOf(':');
    if (i <= 0) return null;
    return { connId: lid.slice(0, i), db: lid.slice(i + 1) };
  }

  async function loadDatabases() {
    if (!$activeConnection) return;
    const parts = splitLid(getLiveId($activeConnection.id));
    if (!parts) return;
    try {
      databases = await sqlListDatabases(parts.connId, parts.db);
    } catch {
      databases = [];
    }
  }

  async function loadTables() {
    if (!$activeConnection) return;
    const parts = splitLid(getLiveId($activeConnection.id));
    if (!parts) return;
    loadingTables = true;
    try {
      tables = await sqlListTables(parts.connId, selectedDatabase || parts.db);
    } catch (err: any) {
      errorToast('Failed to load tables', err);
      tables = [];
    } finally {
      loadingTables = false;
    }
  }

  async function toggleTable(tableName: string) {
    if (expandedTable === tableName) {
      expandedTable = null;
      return;
    }
    expandedTable = tableName;
    const parts = $activeConnection ? splitLid(getLiveId($activeConnection.id)) : null;
    if (!tableColumns[tableName] && parts) {
      loadingColumns = tableName;
      try {
        const cols = await sqlDescribeTable(parts.connId, selectedDatabase || parts.db, tableName);
        tableColumns = { ...tableColumns, [tableName]: cols };
      } catch (err: any) {
        errorToast('Failed to describe table', err);
      } finally {
        loadingColumns = null;
      }
    }
  }

  function handleTableDblClick(tableName: string) {
    oninsertquery?.(`SELECT * FROM ${tableName} LIMIT 100;`);
  }

  function handleDatabaseChange(e: Event) {
    selectedDatabase = (e.target as HTMLSelectElement).value;
    loadTables();
  }
</script>

<div class="table-explorer">
  {#if !isConnected}
    <div class="te-empty">Connect to a database to explore tables</div>
  {:else}
    {#if databases.length > 1}
      <div class="te-db-select">
        <select class="te-select" value={selectedDatabase} onchange={handleDatabaseChange}>
          <option value="">Default database</option>
          {#each databases as db}
            <option value={db}>{db}</option>
          {/each}
        </select>
      </div>
    {/if}

    {#if loadingTables}
      <div class="te-empty">Loading tables...</div>
    {:else if tables.length === 0}
      <div class="te-empty">No tables found</div>
    {:else}
      <div class="te-list">
        {#each tables as table (table.name)}
          <div class="te-table-item">
            <button
              class="te-table-row"
              class:expanded={expandedTable === table.name}
              onclick={() => toggleTable(table.name)}
              ondblclick={() => handleTableDblClick(table.name)}
            >
              <svg class="te-chevron" viewBox="0 0 24 24">
                <path d="M9 18l6-6-6-6"/>
              </svg>
              <svg class="te-table-icon" viewBox="0 0 24 24">
                <rect x="3" y="3" width="18" height="18" rx="2"/>
                <path d="M3 9h18M3 15h18M9 3v18"/>
              </svg>
              <span class="te-table-name">{table.name}</span>
              {#if table.rowCount >= 0}
                <span class="te-table-count">{table.rowCount}</span>
              {/if}
            </button>

            {#if expandedTable === table.name}
              <div class="te-columns">
                {#if loadingColumns === table.name}
                  <div class="te-col-loading">Loading columns...</div>
                {:else if tableColumns[table.name]}
                  {#each tableColumns[table.name] as col}
                    <div class="te-col-row">
                      {#if col.isPrimaryKey}
                        <svg class="te-col-key" viewBox="0 0 24 24"><path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 11-7.778 7.778 5.5 5.5 0 010-7.778zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/></svg>
                      {:else}
                        <span class="te-col-dot"></span>
                      {/if}
                      <span class="te-col-name">{col.name}</span>
                      <span class="te-col-type">{col.dataType}</span>
                      {#if col.isNullable}
                        <span class="te-col-nullable">?</span>
                      {/if}
                    </div>
                  {/each}
                {/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .table-explorer {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow-y: auto;
  }
  .table-explorer::-webkit-scrollbar {
    width: 3px;
  }
  .table-explorer::-webkit-scrollbar-thumb {
    background: var(--b1);
    border-radius: 2px;
  }

  .te-empty {
    padding: 24px 12px;
    color: var(--t3);
    font-size: 11px;
    font-family: var(--mono);
    text-align: center;
  }

  .te-db-select {
    padding: 6px 8px;
    border-bottom: 1px solid var(--b1);
    flex-shrink: 0;
  }

  .te-select {
    width: 100%;
    height: 26px;
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md);
    padding: 0 8px;
    padding-right: 24px;
    font-size: 11px;
    font-family: var(--mono);
    color: var(--t1);
    outline: none;
    cursor: pointer;
    -webkit-appearance: none;
    appearance: none;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23b0b0c8' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'><polyline points='3 5 6 8 9 5'/></svg>");
    background-repeat: no-repeat;
    background-position: right 7px center;
    background-size: 9px 9px;
  }
  .te-select:focus {
    border-color: var(--acc);
  }

  .te-list {
    display: flex;
    flex-direction: column;
  }

  .te-table-row {
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--t2);
    font-size: 11.5px;
    font-family: var(--mono);
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 6px;
    transition: background 0.08s, color 0.08s;
    text-align: left;
  }
  .te-table-row:hover {
    background: var(--c);
    color: var(--t1);
  }
  .te-table-row.expanded {
    color: var(--t1);
  }

  .te-chevron {
    width: 10px;
    height: 10px;
    stroke: var(--t4);
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
    flex-shrink: 0;
    transition: transform 0.15s;
  }
  .te-table-row.expanded .te-chevron {
    transform: rotate(90deg);
  }

  .te-table-icon {
    width: 12px;
    height: 12px;
    stroke: var(--acc);
    fill: none;
    stroke-width: 1.5;
    stroke-linecap: round;
    flex-shrink: 0;
  }

  .te-table-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .te-table-count {
    font-size: 9px;
    color: var(--t4);
    background: var(--b1);
    padding: 1px 5px;
    border-radius: 8px;
    flex-shrink: 0;
  }

  .te-columns {
    padding: 2px 0 4px 0;
  }

  .te-col-loading {
    padding: 6px 32px;
    font-size: 10px;
    color: var(--t4);
    font-family: var(--mono);
  }

  .te-col-row {
    height: 24px;
    padding: 0 12px 0 32px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-family: var(--mono);
    color: var(--t2);
  }

  .te-col-key {
    width: 10px;
    height: 10px;
    stroke: var(--warn);
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
    flex-shrink: 0;
  }

  .te-col-dot {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--t4);
    flex-shrink: 0;
    margin: 0 3px;
  }

  .te-col-name {
    color: var(--t1);
    flex-shrink: 0;
  }

  .te-col-type {
    color: var(--t3);
    font-size: 10px;
    margin-left: auto;
  }

  .te-col-nullable {
    color: var(--t4);
    font-size: 10px;
    flex-shrink: 0;
  }
</style>

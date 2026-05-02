<script lang="ts">
  import {
    connections, activeConnectionId, connectedIds, expandedConnectionId,
    connectionDatabases, databaseTables,
    loadConnections, deleteConnection, saveConnection, connectToDb, disconnectFromDb,
    showSqlConnectionDialog, editingSqlConnection, getLiveId, loadTablesForDb,
    insertQueryText, connectToDatabase, getDbLiveId, dbLiveConnections,
    showSqlDisconnectConfirm, sqlDisconnectTarget,
    sqlConnectionStates, sqlConnectionErrors, resetSqlConnState
  } from '../stores';
  import { sqlListSchemas, sqlDescribeTable, sqlExecuteQuery, sqlCreateDatabase } from '../commands';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import type { SqlConnectionConfig, SqlConnection, TableInfo, ColumnInfo } from '../types';
  import { descriptorFor } from '../dialects';
  import { tabs, activeTabId } from '$lib/shared/stores/tabs';
  import { get } from 'svelte/store';

  const icons = {
    refresh: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M23 4v6h-6"/><path d="M1 20v-6h6"/><path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/></svg>',
    disconnect: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18.36 6.64a9 9 0 11-12.73 0"/><line x1="12" y1="2" x2="12" y2="12"/></svg>',
    connect: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/><path d="M12 5l7 7-7 7"/></svg>',
    edit: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
    duplicate: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>',
    copy: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>',
    trash: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>',
    newQuery: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><path d="M12 11v6M9 14h6"/></svg>',
    select: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18"/></svg>',
    count: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" y1="22" x2="4" y2="15"/></svg>',
    describe: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>',
    selectAll: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>',
    insert: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>',
    truncate: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M21 4H8l-7 8 7 8h13a2 2 0 002-2V6a2 2 0 00-2-2z"/><line x1="18" y1="9" x2="12" y2="15"/><line x1="12" y1="9" x2="18" y2="15"/></svg>',
    drop: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>',
    ellipsisV: '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="5" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="12" cy="19" r="1.5"/></svg>',
    ellipsisH: '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="19" cy="12" r="1.5"/></svg>',
  };

  interface Props {
    searchQuery?: string;
  }

  let { searchQuery = '' }: Props = $props();

  // Confirm dialog
  let confirmShow = $state(false);
  let confirmTitle = $state('');
  let confirmMessage = $state('');
  let confirmDanger = $state(false);
  /** Per-call button verb. Most destructive flows want "Delete" (the
   *  default), but DDL ops like Truncate need their own word so the
   *  button matches the title verb. */
  let confirmText = $state('Delete');
  let confirmAction: (() => Promise<void>) | null = $state(null);

  // Create database dialog
  let createDbShow = $state(false);
  let createDbName = $state('');
  let createDbConnId = $state('');
  let createDbLoading = $state(false);

  function openCreateDbDialog(connId: string) {
    createDbConnId = connId;
    createDbName = '';
    createDbLoading = false;
    createDbShow = true;
  }

  async function handleCreateDb() {
    if (!createDbName.trim() || createDbLoading) return;
    createDbLoading = true;
    try {
      const lid = getLiveId(createDbConnId);
      if (!lid) throw new Error('Not connected');
      await sqlCreateDatabase(lid, createDbName.trim());
      showToast(`Database "${createDbName.trim()}" created`, 'success');
      createDbShow = false;
      // Refresh connection's database list from server
      clearConnectionCaches(createDbConnId);
      const dbs = await (await import('../commands')).sqlListDatabases(lid);
      connectionDatabases.update(m => {
        const next = new Map(m);
        next.set(createDbConnId, dbs);
        return next;
      });
      expandedConnectionId.set(null);
      setTimeout(() => expandedConnectionId.set(createDbConnId), 50);
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      createDbLoading = false;
    }
  }

  // Track expanded state at each level
  let expandedDbs = $state<Set<string>>(new Set());
  let expandedSchemas = $state<Set<string>>(new Set());
  let expandedTables = $state<Set<string>>(new Set());

  // Loading states
  let loadingDbs = $state<Set<string>>(new Set());
  let loadingSchemas = $state<Set<string>>(new Set());
  let loadingTables = $state<Set<string>>(new Set());
  let loadingColumns = $state<Set<string>>(new Set());

  // Caches
  let schemaCache = $state<Map<string, string[]>>(new Map());
  let tableCache = $state<Map<string, TableInfo[]>>(new Map());
  let columnCache = $state<Map<string, ColumnInfo[]>>(new Map());

  const filteredConnections = $derived(
    searchQuery
      ? $connections.filter(c =>
          c.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
          c.driver.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : $connections
  );

  export function showAddConnection() {
    editingSqlConnection.set(null);
    showSqlConnectionDialog.set(true);
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

  // ── Connection handlers ──

  async function handleClickConnection(conn: SqlConnection) {
    activeConnectionId.set(conn.id);
    const alreadyConnected = $connectedIds.has(conn.id);
    const state = $sqlConnectionStates.get(conn.id) ?? 'idle';

    // Already attempting — clicking again must NOT fire a duplicate connect.
    // The store also enforces this, but bailing here keeps behavior obvious.
    if (state === 'connecting') return;

    if (!alreadyConnected) {
      // If the previous attempt errored, treat the click as a retry: clear the
      // error first so the indicator transitions back to `connecting` cleanly.
      if (state === 'error') resetSqlConnState(conn.id);
      try {
        await connectToDb(conn.id);
        showToast(`Connected to ${conn.name}`, 'success');
      } catch (e: any) {
        showToast(friendlyError(e), 'error');
      }
    } else {
      if ($expandedConnectionId === conn.id) {
        expandedConnectionId.set(null);
      } else {
        expandedConnectionId.set(conn.id);
      }
    }
  }

  function clearConnectionCaches(connId: string) {
    const prefix = `${connId}:`;
    expandedDbs = new Set([...expandedDbs].filter(k => !k.startsWith(prefix)));
    expandedSchemas = new Set([...expandedSchemas].filter(k => !k.startsWith(prefix)));
    expandedTables = new Set([...expandedTables].filter(k => !k.startsWith(prefix)));
    schemaCache = new Map([...schemaCache].filter(([k]) => !k.startsWith(prefix)));
    tableCache = new Map([...tableCache].filter(([k]) => !k.startsWith(prefix)));
    columnCache = new Map([...columnCache].filter(([k]) => !k.startsWith(prefix)));
  }

  // ── Database handlers ──

  async function handleClickDatabase(connId: string, db: string) {
    activeConnectionId.set(connId);
    const key = `${connId}:${db}`;

    if (expandedDbs.has(key)) {
      expandedDbs = new Set([...expandedDbs].filter(k => k !== key));
      return;
    }

    expandedDbs = new Set([...expandedDbs, key]);

    if (!schemaCache.has(key)) {
      loadingSchemas = new Set([...loadingSchemas, key]);
      try {
        const dbLiveId = await connectToDatabase(connId, db);
        const schemas = await sqlListSchemas(dbLiveId);
        schemaCache = new Map([...schemaCache, [key, schemas]]);
      } catch {
        schemaCache = new Map([...schemaCache, [key, ['public']]]);
      }
      loadingSchemas = new Set([...loadingSchemas].filter(k => k !== key));
    }
  }

  async function refreshDatabase(connId: string, db: string) {
    const dbKey = `${connId}:${db}`;
    schemaCache = new Map([...schemaCache].filter(([k]) => k !== dbKey));
    tableCache = new Map([...tableCache].filter(([k]) => !k.startsWith(dbKey)));
    columnCache = new Map([...columnCache].filter(([k]) => !k.startsWith(dbKey)));
    expandedSchemas = new Set([...expandedSchemas].filter(k => !k.startsWith(dbKey)));
    expandedTables = new Set([...expandedTables].filter(k => !k.startsWith(dbKey)));

    loadingSchemas = new Set([...loadingSchemas, dbKey]);
    try {
      const dbLiveId = await connectToDatabase(connId, db);
      const schemas = await sqlListSchemas(dbLiveId);
      schemaCache = new Map([...schemaCache, [dbKey, schemas]]);
      // Reload tables for expanded schemas
      for (const schema of schemas) {
        const sKey = `${dbKey}:${schema}`;
        if (expandedSchemas.has(sKey)) {
          const tables = await (await import('../commands')).sqlListTables(dbLiveId, db, schema);
          tableCache = new Map([...tableCache, [sKey, tables]]);
        }
      }
    } catch {
      schemaCache = new Map([...schemaCache, [dbKey, ['public']]]);
    }
    loadingSchemas = new Set([...loadingSchemas].filter(k => k !== dbKey));
    showToast('Refreshed', 'success');
  }

  // ── Schema handlers ──

  async function handleClickSchema(connId: string, db: string, schema: string) {
    const key = `${connId}:${db}:${schema}`;

    if (expandedSchemas.has(key)) {
      expandedSchemas = new Set([...expandedSchemas].filter(k => k !== key));
      return;
    }

    expandedSchemas = new Set([...expandedSchemas, key]);

    if (!tableCache.has(key)) {
      loadingTables = new Set([...loadingTables, key]);
      try {
        const dbLiveId = getDbLiveId(connId, db) ?? getLiveId(connId);
        if (dbLiveId) {
          const tables = await (await import('../commands')).sqlListTables(dbLiveId, db, schema);
          tableCache = new Map([...tableCache, [key, tables]]);
        }
      } catch {
        tableCache = new Map([...tableCache, [key, []]]);
      }
      loadingTables = new Set([...loadingTables].filter(k => k !== key));
    }
  }

  // ── Table handlers ──

  async function handleClickTable(connId: string, db: string, schema: string, table: string) {
    const key = `${connId}:${db}:${schema}:${table}`;

    if (expandedTables.has(key)) {
      expandedTables = new Set([...expandedTables].filter(k => k !== key));
      return;
    }

    expandedTables = new Set([...expandedTables, key]);

    if (!columnCache.has(key)) {
      loadingColumns = new Set([...loadingColumns, key]);
      try {
        const dbLiveId = getDbLiveId(connId, db) ?? getLiveId(connId);
        if (dbLiveId) {
          const columns = await sqlDescribeTable(dbLiveId, table, schema);
          columnCache = new Map([...columnCache, [key, columns]]);
        }
      } catch {
        columnCache = new Map([...columnCache, [key, []]]);
      }
      loadingColumns = new Set([...loadingColumns].filter(k => k !== key));
    }
  }

  function qualifiedName(schema: string, name: string): string {
    return (schema !== 'public' && schema !== 'default' && schema !== 'main')
      ? `${schema}.${name}` : name;
  }

  function hasActiveSqlTab(): boolean {
    const allTabs = get(tabs);
    const activeId = get(activeTabId);
    return allTabs.some(t => t.id === activeId && t.mode === 'sql');
  }

  function insertQuery(query: string) {
    if (!hasActiveSqlTab()) {
      showToast('Open a query tab first', 'info');
      return;
    }
    insertQueryText.set(query);
  }

  function handleTableDblClick(schema: string, tableName: string) {
    insertQuery(`SELECT * FROM ${qualifiedName(schema, tableName)} LIMIT 100;`);
  }

  // ── Query generators ──

  function genSelectTop100(schema: string, table: string) {
    insertQuery(`SELECT * FROM ${qualifiedName(schema, table)} LIMIT 100;`);
  }

  function genSelectCount(schema: string, table: string) {
    insertQuery(`SELECT COUNT(*) FROM ${qualifiedName(schema, table)};`);
  }

  function genDescribeTable(schema: string, table: string) {
    insertQuery(`SELECT column_name, data_type, is_nullable, column_default
FROM information_schema.columns
WHERE table_name = '${table}' AND table_schema = '${schema}'
ORDER BY ordinal_position;`);
  }

  function genCopySelect(connId: string, db: string, schema: string, table: string) {
    const key = `${connId}:${db}:${schema}:${table}`;
    const columns = columnCache.get(key);
    const cols = columns && columns.length > 0
      ? columns.map(c => c.name).join(', ')
      : '*';
    const q = `SELECT ${cols} FROM ${qualifiedName(schema, table)};`;
    writeText(q).then(() => showToast('SELECT copied', 'success')).catch(() => showToast('Failed to copy', 'error'));
  }

  function genCopyInsert(connId: string, db: string, schema: string, table: string) {
    const key = `${connId}:${db}:${schema}:${table}`;
    const columns = columnCache.get(key);
    const cols = columns && columns.length > 0
      ? columns.filter(c => !c.isPrimaryKey).map(c => c.name)
      : ['column1', 'column2'];
    const placeholders = cols.map(() => '?').join(', ');
    const q = `INSERT INTO ${qualifiedName(schema, table)} (${cols.join(', ')}) VALUES (${placeholders});`;
    writeText(q).then(() => showToast('INSERT template copied', 'success')).catch(() => showToast('Failed to copy', 'error'));
  }

  // ── Context menus ──

  function showConnMenu(e: MouseEvent, conn: SqlConnection) {
    e.preventDefault();
    e.stopPropagation();
    const isConnected = $connectedIds.has(conn.id);
    const connString = `${conn.driver}://${conn.username ? conn.username + '@' : ''}${conn.host}:${conn.port}/${conn.databaseName}`;

    showContextMenu(e.clientX, e.clientY, [
      ...(isConnected ? [{
        label: 'Refresh',
        icon: icons.refresh,
        action: async () => {
          clearConnectionCaches(conn.id);
          // Re-fetch database list from server
          try {
            const lid = getLiveId(conn.id);
            if (lid) {
              const dbs = await (await import('../commands')).sqlListDatabases(lid);
              connectionDatabases.update(m => {
                const next = new Map(m);
                next.set(conn.id, dbs);
                return next;
              });
            }
          } catch (e: any) {
            showToast(friendlyError(e), 'error');
          }
          expandedConnectionId.set(null);
          setTimeout(() => expandedConnectionId.set(conn.id), 50);
        },
      }] : []),
      ...(isConnected ? [
        { label: '', action: () => {}, separator: true },
        {
          label: 'Disconnect',
          icon: icons.disconnect,
          action: () => {
            sqlDisconnectTarget.set(conn);
            showSqlDisconnectConfirm.set(true);
          },
        },
      ] : [{
        label: 'Connect',
        icon: icons.connect,
        action: () => handleClickConnection(conn),
      }]),
      { label: '', action: () => {}, separator: true },
      {
        label: 'Edit',
        icon: icons.edit,
        action: () => {
          editingSqlConnection.set(conn);
          showSqlConnectionDialog.set(true);
        },
      },
      {
        label: 'Duplicate',
        icon: icons.duplicate,
        action: async () => {
          try {
            await saveConnection({
              name: conn.name + ' (copy)',
              driver: conn.driver,
              host: conn.host,
              port: conn.port,
              database: conn.databaseName,
              username: conn.username,
              password: conn.password,
              ssl: !!conn.ssl,
            });
            showToast('Connection duplicated', 'success');
          } catch (err: any) { showToast(friendlyError(err), 'error'); }
        },
      },
      {
        label: 'Copy Connection String',
        icon: icons.copy,
        action: async () => {
          await writeText(connString);
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
            await deleteConnection(conn.id);
            clearConnectionCaches(conn.id);
            showToast('Connection removed', 'success');
          } catch (err: any) { showToast(friendlyError(err), 'error'); }
        }),
      },
    ]);
  }

  function showDbMenu(e: MouseEvent, connId: string, db: string) {
    e.preventDefault();
    e.stopPropagation();

    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'New Query',
        icon: icons.newQuery,
        action: () => {
          insertQuery(`-- Query on ${db}\n`);
        },
      },
      {
        label: 'Refresh',
        icon: icons.refresh,
        action: () => refreshDatabase(connId, db),
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Copy Name',
        icon: icons.copy,
        action: async () => {
          await writeText(db);
          showToast('Copied', 'success');
        },
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Drop Database',
        icon: icons.drop,
        danger: true,
        action: () => showConfirm('Drop Database', `Drop "${db}"? This will permanently delete the database and all its data.`, true, async () => {
          try {
            const lid = getLiveId(connId);
            if (!lid) throw new Error('Not connected');
            const conn = $connections.find(c => c.id === connId);
            const q = descriptorFor(conn?.driver ?? '')?.identifierQuote ?? '"';
            const dropStmt = `DROP DATABASE ${q}${db}${q}`;
            await sqlExecuteQuery(lid, dropStmt);
            showToast(`Dropped database "${db}"`, 'success');
            // Refresh connection's database list from server
            clearConnectionCaches(connId);
            const dbs = await (await import('../commands')).sqlListDatabases(lid);
            connectionDatabases.update(m => {
              const next = new Map(m);
              next.set(connId, dbs);
              return next;
            });
            expandedConnectionId.set(null);
            setTimeout(() => expandedConnectionId.set(connId), 50);
          } catch (e: any) { showToast(friendlyError(e), 'error'); }
        }, 'Drop'),
      },
    ]);
  }

  function showTableMenu(e: MouseEvent, connId: string, db: string, schema: string, table: string) {
    e.preventDefault();
    e.stopPropagation();

    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'SELECT TOP 100',
        icon: icons.select,
        action: () => genSelectTop100(schema, table),
      },
      {
        label: 'SELECT COUNT(*)',
        icon: icons.count,
        action: () => genSelectCount(schema, table),
      },
      {
        label: 'Describe Table',
        icon: icons.describe,
        action: () => genDescribeTable(schema, table),
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Copy SELECT Statement',
        icon: icons.selectAll,
        action: () => genCopySelect(connId, db, schema, table),
      },
      {
        label: 'Copy INSERT Template',
        icon: icons.insert,
        action: () => genCopyInsert(connId, db, schema, table),
      },
      {
        label: 'Copy Name',
        icon: icons.copy,
        action: async () => {
          await writeText(table);
          showToast('Copied', 'success');
        },
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Truncate Table',
        icon: icons.truncate,
        danger: true,
        action: () => showConfirm('Truncate Table', `Truncate "${table}"? All rows will be permanently deleted. The table structure will remain.`, true, async () => {
          try {
            const lid = getDbLiveId(connId, db) ?? getLiveId(connId);
            if (lid) {
              await sqlExecuteQuery(lid, `TRUNCATE TABLE ${qualifiedName(schema, table)}`);
              showToast(`Truncated ${table}`, 'success');
            }
          } catch (e: any) { showToast(friendlyError(e), 'error'); }
        }, 'Truncate'),
      },
      {
        label: 'Drop Table',
        icon: icons.drop,
        danger: true,
        action: () => showConfirm('Drop Table', `Drop "${table}"? The table and all its data will be permanently deleted.`, true, async () => {
          try {
            const lid = getDbLiveId(connId, db) ?? getLiveId(connId);
            if (lid) {
              await sqlExecuteQuery(lid, `DROP TABLE ${qualifiedName(schema, table)}`);
              // Refresh the schema to remove the table from nav
              const sKey = `${connId}:${db}:${schema}`;
              tableCache = new Map([...tableCache].filter(([k]) => k !== sKey));
              const tables = await (await import('../commands')).sqlListTables(lid, db, schema);
              tableCache = new Map([...tableCache, [sKey, tables]]);
              showToast(`Dropped ${table}`, 'success');
            }
          } catch (e: any) { showToast(friendlyError(e), 'error'); }
        }, 'Drop'),
      },
    ]);
  }

  function showColumnMenu(e: MouseEvent, col: ColumnInfo) {
    e.preventDefault();
    e.stopPropagation();
    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Copy Name',
        icon: icons.copy,
        action: async () => {
          await writeText(col.name);
          showToast('Copied', 'success');
        },
      },
    ]);
  }

  // ── Helpers ──

  function driverLabel(driver: string): string {
    return descriptorFor(driver)?.abbreviation ?? (driver ? driver.substring(0, 2).toUpperCase() : '?');
  }

  // Brand-accurate identity colors for each SQL engine. Hardcoded by design —
  // these are visual identifiers tied to the *thing*, not the theme. See
  // accompanying NoSQL `driverColor` (Mongo green / Redis red) for the same
  // pattern.
  function driverColor(driver: string): string {
    switch (driver) {
      case 'postgresql': return '#336791';
      case 'mysql':      return '#00758F';
      case 'sqlite':     return '#909090';
      case 'clickhouse': return '#FFCC01';
      default:           return 'var(--t3)';
    }
  }

  function columnLabel(col: ColumnInfo): string {
    let label = col.dataType;
    const tags: string[] = [];
    if (col.isPrimaryKey) tags.push('PK');
    if (col.isNullable) tags.push('null');
    if (tags.length > 0) label += ', ' + tags.join(', ');
    return label;
  }
</script>

<div class="sql-nav">
  {#if filteredConnections.length === 0}
    <div class="nav-empty">
      {#if searchQuery}
        <span>No results for "{searchQuery}"</span>
      {:else}
        <span>No connections yet</span>
        <button class="nav-empty-btn" onclick={() => showAddConnection()}>
          + New Connection
        </button>
      {/if}
    </div>
  {:else}
    {#each filteredConnections as conn (conn.id)}
      {@const isConnected = $connectedIds.has(conn.id)}
      {@const isExpanded = $expandedConnectionId === conn.id && isConnected}
      {@const databases = $connectionDatabases.get(conn.id) ?? []}
      {@const connState = $sqlConnectionStates.get(conn.id) ?? 'idle'}
      {@const isConnecting = connState === 'connecting'}
      {@const hasError = connState === 'error'}
      {@const errorMsg = $sqlConnectionErrors.get(conn.id) ?? ''}

      <!-- Connection Row — two-line layout matching SSH/Explorer for visual
           parity. Top line: chevron + driver-badge (with corner status
           dot) + connection name. Bottom line: user@host:port (or "Local
           file" for SQLite) + status text when connecting. -->
      <button
        class="tree-item tree-conn"
        class:active={$activeConnectionId === conn.id}
        class:connected={isConnected}
        class:connecting={isConnecting}
        class:errored={hasError}
        title={hasError ? errorMsg : ''}
        onclick={() => handleClickConnection(conn)}
        oncontextmenu={(e) => showConnMenu(e, conn)}
      >
        <svg class="tree-chevron" class:open={isExpanded} viewBox="0 0 24 24">
          <path d="M9 18l6-6-6-6"/>
        </svg>
        <span
          class="conn-badge-wrap"
          class:connected={isConnected}
          class:connecting={isConnecting}
          class:errored={hasError}
        >
          <span
            class="conn-driver"
            style:color={driverColor(conn.driver)}
            style:background="color-mix(in srgb, {driverColor(conn.driver)} 12%, transparent)"
          >{driverLabel(conn.driver)}</span>
        </span>
        <div class="tree-body">
          <div class="tree-row-top">
            <span class="tree-label">{conn.name}</span>
          </div>
          <div class="tree-row-bot">
            {#if isConnecting}
              <span class="conn-state-text">Connecting<span class="conn-dots"></span></span>
            {:else if conn.driver === 'sqlite'}
              <span class="tree-sub">Local file</span>
            {:else}
              <span class="tree-sub">{conn.username}{conn.username ? '@' : ''}{conn.host}{conn.port ? `:${conn.port}` : ''}</span>
            {/if}
          </div>
        </div>
        {#if isConnected}
          <span class="conn-action" role="button" tabindex="-1" title="Create Database"
            onclick={(e) => { e.stopPropagation(); openCreateDbDialog(conn.id); }}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
          </span>
        {/if}
        <span class="conn-action conn-ellipsis" role="button" tabindex="-1" title="More"
          onclick={(e) => { e.stopPropagation(); showConnMenu(e, conn); }}>
          {@html icons.ellipsisV}
        </span>
      </button>

      <!-- Database Tree -->
      {#if isExpanded && databases.length > 0}
        {#each databases as db}
          {@const dbKey = `${conn.id}:${db}`}
          {@const isDbExpanded = expandedDbs.has(dbKey)}
          {@const schemas = schemaCache.get(dbKey) ?? []}
          {@const isLoadingSchema = loadingSchemas.has(dbKey)}

          <button
            class="tree-item tree-db"
            class:expanded={isDbExpanded}
            onclick={() => handleClickDatabase(conn.id, db)}
            oncontextmenu={(e) => showDbMenu(e, conn.id, db)}
          >
            <svg class="tree-chevron-sm" class:open={isDbExpanded} viewBox="0 0 24 24">
              <path d="M9 18l6-6-6-6"/>
            </svg>
            <svg class="tree-icon tree-icon-db" viewBox="0 0 24 24">
              <ellipse cx="12" cy="5" rx="9" ry="3"/>
              <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/>
              <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
            </svg>
            <span class="tree-label">{db}</span>
            <span class="db-ellipsis" role="button" tabindex="-1" title="More"
              onclick={(e) => { e.stopPropagation(); showDbMenu(e, conn.id, db); }}>
              {@html icons.ellipsisH}
            </span>
          </button>

          {#if isDbExpanded}
            {#if isLoadingSchema}
              <div class="tree-loading" style="padding-left:28px">Loading schemas...</div>
            {:else}
              {#each schemas as schema}
                {@const schemaKey = `${conn.id}:${db}:${schema}`}
                {@const isSchemaExpanded = expandedSchemas.has(schemaKey)}
                {@const tables = tableCache.get(schemaKey) ?? []}
                {@const isLoadingTbl = loadingTables.has(schemaKey)}

                <button
                  class="tree-item tree-schema"
                  class:expanded={isSchemaExpanded}
                  onclick={() => handleClickSchema(conn.id, db, schema)}
                >
                  <svg class="tree-chevron-sm" class:open={isSchemaExpanded} viewBox="0 0 24 24">
                    <path d="M9 18l6-6-6-6"/>
                  </svg>
                  <svg class="tree-icon tree-icon-schema" viewBox="0 0 24 24">
                    <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
                  </svg>
                  <span class="tree-label">{schema}</span>
                </button>

                {#if isSchemaExpanded}
                  {#if isLoadingTbl}
                    <div class="tree-loading" style="padding-left:38px">Loading tables...</div>
                  {:else if tables.length === 0}
                    <div class="tree-loading" style="padding-left:38px">No tables</div>
                  {:else}
                    {#each tables as table (table.name)}
                      {@const tableKey = `${conn.id}:${db}:${schema}:${table.name}`}
                      {@const isTableExpanded = expandedTables.has(tableKey)}
                      {@const columns = columnCache.get(tableKey) ?? []}
                      {@const isLoadingCol = loadingColumns.has(tableKey)}

                      <button
                        class="tree-item tree-table"
                        class:expanded={isTableExpanded}
                        onclick={() => handleClickTable(conn.id, db, schema, table.name)}
                        ondblclick={() => handleTableDblClick(schema, table.name)}
                        oncontextmenu={(e) => showTableMenu(e, conn.id, db, schema, table.name)}
                      >
                        <svg class="tree-chevron-sm" class:open={isTableExpanded} viewBox="0 0 24 24">
                          <path d="M9 18l6-6-6-6"/>
                        </svg>
                        <svg class="tree-icon tree-icon-table" viewBox="0 0 24 24">
                          <rect x="3" y="3" width="18" height="18" rx="2"/>
                          <path d="M3 9h18M3 15h18M9 3v18"/>
                        </svg>
                        <span class="tree-label">{table.name}</span>
                        <span class="table-ellipsis" role="button" tabindex="-1"
                          onclick={(e) => { e.stopPropagation(); showTableMenu(e, conn.id, db, schema, table.name); }}>
                          {@html icons.ellipsisH}
                        </span>
                      </button>

                      {#if isTableExpanded}
                        {#if isLoadingCol}
                          <div class="tree-loading" style="padding-left:48px">Loading columns...</div>
                        {:else if columns.length === 0}
                          <div class="tree-loading" style="padding-left:48px">No columns</div>
                        {:else}
                          {#each columns as col (col.name)}
                            <!-- svelte-ignore a11y_click_events_have_key_events -->
                            <!-- svelte-ignore a11y_no_static_element_interactions -->
                            <div class="tree-item tree-column" oncontextmenu={(e) => showColumnMenu(e, col)}>
                              {#if col.isPrimaryKey}
                                <svg class="tree-icon tree-icon-pk" viewBox="0 0 24 24">
                                  <path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 11-7.778 7.778 5.5 5.5 0 017.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/>
                                </svg>
                              {:else}
                                <svg class="tree-icon tree-icon-col" viewBox="0 0 24 24">
                                  <rect x="8" y="4" width="8" height="16" rx="1.5" />
                                </svg>
                              {/if}
                              <span class="tree-label col-name">{col.name}</span>
                              <span class="col-type">{columnLabel(col)}</span>
                            </div>
                          {/each}
                        {/if}
                      {/if}
                    {/each}
                  {/if}
                {/if}
              {/each}
            {/if}
          {/if}
        {/each}
      {/if}
    {/each}
  {/if}
</div>

<!-- Confirm Dialog — shared primitive so SSH/Explorer/SQL/NoSQL all
     read identically (header bar, body, footer, teleported to body). -->
<ConfirmDialog
  bind:show={confirmShow}
  title={confirmTitle}
  message={confirmMessage}
  confirmText={confirmText}
  confirmColor={confirmDanger ? 'var(--err)' : 'var(--acc)'}
  onconfirm={handleConfirmOk}
/>

<!-- Create Database — input prompt; can't use ConfirmDialog (no input
     slot) so keeps its own markup, but re-uses the same dialog look. -->
{#if createDbShow}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="sql-prompt-overlay" onclick={() => createDbShow = false}>
    <div class="sql-prompt" onclick={(e) => e.stopPropagation()}>
      <div class="sql-prompt-title">Create Database</div>
      <div class="sql-prompt-msg">Enter a name for the new database.</div>
      <input
        class="create-db-input"
        type="text"
        placeholder="database_name"
        bind:value={createDbName}
        onkeydown={(e) => { if (e.key === 'Enter') handleCreateDb(); if (e.key === 'Escape') createDbShow = false; }}
        autofocus
      />
      <div class="sql-prompt-actions">
        <button class="sql-prompt-btn" onclick={() => createDbShow = false}>Cancel</button>
        <button
          class="sql-prompt-btn primary"
          disabled={!createDbName.trim() || createDbLoading}
          onclick={handleCreateDb}
        >
          {createDbLoading ? 'Creating...' : 'Create'}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .sql-nav {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .sql-nav::-webkit-scrollbar { width: 3px; }
  .sql-nav::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

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

  /* Connection row — two-line layout (top: name, bottom: user@host:port).
     Bumped from 32px to 44px min-height to match SSH/Explorer parity and
     to give the new corner-status-dot on the driver badge room to breathe. */
  .tree-conn {
    min-height: 44px;
    height: auto;
    padding: 6px 8px;
    gap: 8px;
    align-items: center;
  }
  .tree-conn.active { background: color-mix(in srgb, var(--acc) 10%, transparent); }
  .tree-conn.connecting { pointer-events: none; }
  /* Connection status indicator — outline ring around the driver badge
     (matches its rectangular shape) plus a 6px pulsing dot at the
     top-right corner. Mirrors the SSH/Explorer icon treatment but with
     a rectangular outline since the badge isn't square. Idle = no ring,
     no dot. Goal: zero extra horizontal space (no standalone dot). */
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
  .conn-badge-wrap.connected::after  { background: var(--ok, #1dc880); animation: badgePulseOk 3s ease-in-out infinite; }
  .conn-badge-wrap.connecting::after { background: var(--warn);        animation: badgePulseWarn 1.4s ease-in-out infinite; }
  .conn-badge-wrap.errored::after    { background: var(--err); }

  @keyframes badgePulseOk {
    0%, 100% { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 2px color-mix(in srgb, var(--ok, #1dc880) 30%, transparent); }
    50%      { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 5px color-mix(in srgb, var(--ok, #1dc880) 0%, transparent); }
  }
  @keyframes badgePulseWarn {
    0%, 100% { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 2px color-mix(in srgb, var(--warn) 35%, transparent); }
    50%      { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 5px color-mix(in srgb, var(--warn) 0%, transparent); }
  }
  .tree-conn.errored { background: color-mix(in srgb, var(--err) 8%, transparent); }
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


  .conn-state-text {
    font-size: 10px; color: var(--t3); font-family: var(--ui);
    flex-shrink: 0; margin-left: 4px;
  }
  /* Animated "Connecting..." dots — pure CSS, no asset, no JS timer. */
  .conn-dots::after { content: ''; animation: sql-conn-dots 1.4s steps(4, end) infinite; }
  @keyframes sql-conn-dots {
    0% { content: ''; } 25% { content: '.'; } 50% { content: '..'; } 75% { content: '...'; }
  }

  .conn-driver {
    /* color + background set inline via driverColor(driver) — see template. */
    font-size: 9px; font-weight: 700;
    padding: 1px 4px; border-radius: 3px;
    font-family: var(--ui); flex-shrink: 0; letter-spacing: 0.04em;
  }

  /* Connection action icons */
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

  /* DB ellipsis icon */
  .db-ellipsis {
    width: 18px; height: 18px;
    display: none; align-items: center; justify-content: center;
    border-radius: 3px; flex-shrink: 0; cursor: default;
    color: var(--t3); transition: background 0.1s, color 0.1s;
  }
  .tree-db:hover .db-ellipsis { display: flex; }
  .db-ellipsis:hover { background: rgba(255,255,255,0.08); color: var(--t1); }

  /* Table ellipsis */
  .table-ellipsis {
    width: 18px; height: 18px;
    display: none; align-items: center; justify-content: center;
    border-radius: 3px; flex-shrink: 0; cursor: default;
    color: var(--t3); transition: background 0.1s, color 0.1s;
  }
  .tree-table:hover .table-ellipsis { display: flex; }
  .table-ellipsis:hover { background: rgba(255,255,255,0.08); color: var(--t1); }

  .tree-icon {
    width: 11px; height: 11px; stroke-width: 1.5; stroke-linecap: round; flex-shrink: 0;
  }
  .tree-icon-db { stroke: var(--acc); fill: none; opacity: 0.7; }
  .tree-icon-schema { stroke: var(--t3); fill: none; opacity: 0.7; }
  .tree-icon-table { stroke: var(--t3); fill: none; }
  .tree-icon-pk { stroke: var(--acc); fill: none; width: 10px; height: 10px; }
  .tree-icon-col { stroke: var(--t4); fill: none; width: 10px; height: 10px; stroke-width: 1.5; }

  .tree-db {
    height: 28px; padding: 0 14px 0 16px;
    border-left: 1px solid var(--b1); margin-left: 11px;
  }
  .tree-db .tree-label { font-size: 11.5px; color: var(--t2); }

  .tree-schema {
    height: 26px; padding: 0 14px 0 28px;
    border-left: 1px solid var(--b1); margin-left: 11px;
  }
  .tree-schema .tree-label { font-size: 11px; color: var(--t3); }

  .tree-table {
    height: 26px; padding: 0 14px 0 38px;
    border-left: 1px solid var(--b1); margin-left: 11px;
  }
  .tree-table .tree-label { font-size: 11px; color: var(--t2); }

  .tree-column {
    height: 24px; padding: 0 14px 0 48px;
    border-left: 1px solid var(--b1); margin-left: 11px;
    cursor: default;
  }
  .tree-column .col-name { font-size: 10.5px; color: var(--t2); }
  .col-type {
    font-size: 9px; color: var(--t4); font-family: var(--ui);
    flex-shrink: 0; margin-left: auto; padding-left: 6px;
  }

  .tree-loading {
    padding: 6px 0; font-size: 10px; color: var(--t4); font-family: var(--ui);
  }


  /* Create-database prompt — has an input field so it can't ride the
     shared ConfirmDialog primitive. Visually matches the rest of the
     app's small dialogs. */
  .sql-prompt-overlay {
    position: fixed; top: 0; left: 0; width: 100vw; height: 100vh;
    background: rgba(0,0,0,0.6); z-index: 9999;
    display: flex; align-items: center; justify-content: center;
  }
  .sql-prompt {
    width: 360px;
    background: var(--modal-bg, var(--n));
    border: 1px solid var(--b1);
    border-radius: 10px;
    padding: 20px 22px;
    box-shadow: 0 20px 60px rgba(0,0,0,0.5);
  }
  .sql-prompt-title { font-size: 16px; font-weight: 600; color: var(--t1); font-family: var(--ui); margin-bottom: 6px; }
  .sql-prompt-msg { font-size: 13px; color: var(--t2); font-family: var(--ui); line-height: 1.5; margin-bottom: 14px; }
  .sql-prompt-actions { display: flex; justify-content: flex-end; gap: 8px; }
  .sql-prompt-btn {
    height: 30px; padding: 0 16px;
    border-radius: 8px; border: 1px solid var(--b1);
    background: transparent; color: var(--t2);
    font-size: 12px; font-family: var(--ui); cursor: pointer;
    transition: border-color 0.1s, color 0.1s;
  }
  .sql-prompt-btn:hover { border-color: var(--b2); color: var(--t1); }
  .sql-prompt-btn.primary { background: var(--acc); color: #fff; border-color: transparent; }
  .sql-prompt-btn.primary:hover { opacity: 0.9; }
  .sql-prompt-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  .create-db-input {
    width: 100%;
    padding: 8px 12px;
    border-radius: 8px;
    border: 1px solid var(--b1);
    background: var(--c, rgba(255,255,255,0.05));
    color: var(--t1);
    font-size: 13px;
    font-family: var(--ui);
    outline: none;
    margin-bottom: 16px;
    transition: border-color 0.15s;
    box-sizing: border-box;
  }
  .create-db-input:focus { border-color: var(--acc); }
  .create-db-input::placeholder { color: var(--t4); }
</style>

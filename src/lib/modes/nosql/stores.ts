import { writable, derived, get } from 'svelte/store';
import type { NoSqlConnection, NoSqlConnectionConfig } from './types';
import {
  nosqlListSavedConnections,
  nosqlSaveConnection,
  nosqlDeleteSavedConnection,
  nosqlUpdateSavedConnection,
  nosqlConnect,
  nosqlDisconnect,
} from './commands';

// --- NoSQL Tab State (per global tab) ---
export interface NoSqlTabData {
  connectionId: string;
  database: string;
  collection: string;
  filterQuery: string;
  sortQuery: string;
}

export const nosqlTabState = writable<Map<number, NoSqlTabData>>(new Map());

export function getNoSqlTabData(tabId: number): NoSqlTabData {
  const map = get(nosqlTabState);
  return map.get(tabId) ?? { connectionId: '', database: '', collection: '', filterQuery: '{}', sortQuery: '{}' };
}

export function setNoSqlTabData(tabId: number, data: Partial<NoSqlTabData>) {
  nosqlTabState.update(m => {
    const next = new Map(m);
    const existing = m.get(tabId) ?? { connectionId: '', database: '', collection: '', filterQuery: '{}', sortQuery: '{}' };
    next.set(tabId, { ...existing, ...data });
    return next;
  });
}

export function clearNoSqlTabData(tabId: number) {
  nosqlTabState.update(m => {
    const next = new Map(m);
    next.delete(tabId);
    return next;
  });
}

export function initNoSqlTab(tabId: number) {
  // Use active connection if connected, otherwise first connected MongoDB
  let connId = get(activeNoSqlConnectionId) || '';
  if (connId && !get(connectedNoSqlIds).has(connId)) {
    // Active connection is not connected — find first connected MongoDB
    const allConns = get(nosqlConnections);
    const connected = get(connectedNoSqlIds);
    const firstConnected = allConns.find(c => c.driver === 'mongodb' && connected.has(c.id));
    connId = firstConnected?.id ?? '';
  }
  setNoSqlTabData(tabId, { connectionId: connId, database: '', collection: '', filterQuery: '{}', sortQuery: '{}' });
}

// Signal from nav to open a collection in a tab
export const openNoSqlCollection = writable<{ connectionId: string; database: string; collection: string } | null>(null);

// AI helper — insert query into active NoSQL editor
export const insertNoSqlQueryText = writable<string>('');

export function applyAiNoSqlQuery(query: string) {
  insertNoSqlQueryText.set(query);
}

export const nosqlConnections = writable<NoSqlConnection[]>([]);
export const activeNoSqlConnectionId = writable<string | null>(null);
export const connectedNoSqlIds = writable<Set<string>>(new Set());
// Maps saved connection ID -> live session ID (from Rust pool)
export const nosqlLiveConnectionIds = writable<Record<string, string>>({});
export const showNoSqlConnectionDialog = writable(false);
export const editingNoSqlConnection = writable<NoSqlConnection | null>(null);

export async function handleNoSqlConnectionSave(config: NoSqlConnectionConfig) {
  const editing = get(editingNoSqlConnection);
  let connId: string;
  if (editing) {
    // Disconnect old connection if it was connected
    if (get(connectedNoSqlIds).has(editing.id)) {
      try { await disconnectFromNoSql(editing.id); } catch { /* ignore */ }
    }
    await updateNoSqlConnection(editing.id, config);
    connId = editing.id;
  } else {
    const conn = await saveNoSqlConnection(config);
    connId = conn.id;
  }
  // Close dialog immediately — don't wait for auto-connect
  showNoSqlConnectionDialog.set(false);
  editingNoSqlConnection.set(null);
  // Auto-connect in background
  connectToNoSql(connId).catch(() => {
    // Saved but couldn't auto-connect — that's ok
  });
}

export const activeNoSqlConnection = derived(
  [nosqlConnections, activeNoSqlConnectionId],
  ([$connections, $activeId]) =>
    $activeId ? $connections.find((c) => c.id === $activeId) ?? null : null
);

export async function loadNoSqlConnections(): Promise<void> {
  try {
    const data = await nosqlListSavedConnections();
    nosqlConnections.set(data);
  } catch (err) {
    console.error('Failed to load NoSQL connections:', err);
  }
}

export async function saveNoSqlConnection(config: NoSqlConnectionConfig): Promise<NoSqlConnection> {
  try {
    const conn = await nosqlSaveConnection(config);
    nosqlConnections.update((c) => [...c, conn]);
    activeNoSqlConnectionId.set(conn.id);
    return conn;
  } catch (err) {
    console.error('Failed to save NoSQL connection:', err);
    throw err;
  }
}

export async function updateNoSqlConnection(id: string, config: NoSqlConnectionConfig): Promise<NoSqlConnection> {
  const updated = await nosqlUpdateSavedConnection(id, config);
  nosqlConnections.update((c) => c.map((x) => (x.id === id ? updated : x)));
  return updated;
}

export async function deleteNoSqlConnection(id: string): Promise<void> {
  // Disconnect first if connected
  if (get(connectedNoSqlIds).has(id)) {
    try { await disconnectFromNoSql(id); } catch { /* ignore */ }
  }
  await nosqlDeleteSavedConnection(id);
  nosqlConnections.update((c) => c.filter((x) => x.id !== id));
  if (get(activeNoSqlConnectionId) === id) {
    activeNoSqlConnectionId.set(null);
  }
}

export async function connectToNoSql(id: string): Promise<string> {
  const allConns = get(nosqlConnections);
  const conn = allConns.find((c) => c.id === id);
  if (!conn) throw new Error('Connection not found');
  const liveId = await nosqlConnect({
    name: conn.name,
    driver: conn.driver as any,
    connectionString: conn.connectionString,
    host: conn.host,
    port: conn.port,
    database: conn.databaseName || undefined,
    username: conn.username || undefined,
    password: conn.password || undefined,
    ssl: !!conn.ssl,
    directConnection: !!conn.directConnection,
  });
  connectedNoSqlIds.update((s) => {
    const next = new Set(s);
    next.add(id);
    return next;
  });
  nosqlLiveConnectionIds.update((m) => ({ ...m, [id]: liveId }));
  activeNoSqlConnectionId.set(id);
  return liveId;
}

export function getNoSqlLiveId(savedId: string): string | null {
  return get(nosqlLiveConnectionIds)[savedId] ?? null;
}

export async function disconnectFromNoSql(id: string): Promise<void> {
  const liveId = getNoSqlLiveId(id);
  if (liveId) {
    await nosqlDisconnect(liveId);
  }
  connectedNoSqlIds.update((s) => {
    const next = new Set(s);
    next.delete(id);
    return next;
  });
  nosqlLiveConnectionIds.update((m) => {
    const next = { ...m };
    delete next[id];
    return next;
  });
}

// AI execution trigger — AI writes query + target, NoSqlPanel/DocumentViewer auto-executes
export interface AiNoSqlExecution {
  filter: string;
  connectionId: string;
  database: string;
  collection: string;
}
export const aiExecuteNoSqlQuery = writable<AiNoSqlExecution | null>(null);

export function triggerAiNoSqlExecution(filter: string, connectionId: string, database: string, collection: string) {
  aiExecuteNoSqlQuery.set({ filter, connectionId, database, collection });
}

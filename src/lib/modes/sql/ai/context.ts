import { get } from 'svelte/store';
import { activeTabId } from '$lib/shared/stores/tabs';
import { activeConnectionId, getSqlTabData, connections as sqlConnections, databaseTables, getLiveId, connectedIds, dbLiveConnections } from '../stores';
import type { ChatContext, ContextRequest, ContextResponse, ContextEnvVar } from '$lib/types/ai';

export async function gatherSqlContext(): Promise<ChatContext> {
  const tabId = get(activeTabId);
  const tabData = getSqlTabData(tabId);
  const connId = get(activeConnectionId);
  const conns = get(sqlConnections);
  const activeConn = conns.find((c: any) => c.id === connId);

  let currentRequest: ContextRequest | null = null;
  if (tabData.query) {
    currentRequest = {
      method: 'SQL', url: '', headers: [], params: [],
      body: tabData.query,
      bodyType: 'sql',
      authType: 'none', authData: '{}',
    };
  }

  let currentResponse: ContextResponse | null = null;
  if (tabData.results && tabData.results.length > 0) {
    const activeResult = tabData.results[tabData.activeResultIdx || 0];
    if (activeResult?.result) {
      const r = activeResult.result;
      const rowCount = r.rows?.length || r.affectedRows || 0;
      const preview = `${rowCount} rows. Columns: ${(r.columns || []).join(', ')}`;
      currentResponse = {
        status: 200, statusText: `${rowCount} rows`,
        headers: (r.columns || []).map((c: string) => [c, 'column'] as [string, string]),
        body: preview,
        durationMs: r.durationMs || 0, sizeBytes: 0,
      };
    } else if (activeResult?.error) {
      currentResponse = {
        status: 500, statusText: 'Error',
        headers: [], body: activeResult.error,
        durationMs: 0, sizeBytes: 0,
      };
    }
  }

  const envVars: ContextEnvVar[] = [];
  if (activeConn) {
    // Check if actually connected — don't send connection_id if disconnected
    const isConnected = get(connectedIds).has(activeConn.id);
    if (!isConnected) {
      envVars.push({ key: 'connection_status', value: 'disconnected', isSecret: false });
      envVars.push({ key: 'connection_name', value: activeConn.name, isSecret: false });
      return { mode: 'sql', currentRequest, currentResponse, envVars };
    }

    // Send pool key that both UI and AI use (savedId:dbName format)
    const db = tabData.database || activeConn.databaseName || '';
    const dbKey = `${activeConn.id}:${db}`;
    const poolId = get(dbLiveConnections)[dbKey] || getLiveId(activeConn.id) || activeConn.id;
    envVars.push({ key: 'connection_id', value: poolId, isSecret: false });
    envVars.push({ key: 'saved_connection_id', value: activeConn.id, isSecret: false });
    envVars.push({ key: 'connection_name', value: activeConn.name, isSecret: false });
    envVars.push({ key: 'driver', value: activeConn.driver, isSecret: false });
    envVars.push({ key: 'database', value: db, isSecret: false });

    // Include table schema for active database so AI can write correct queries
    const tables = get(databaseTables);
    const tableList = tables.get(`${connId}:${db}`);
    if (tableList && tableList.length > 0) {
      const schema = tableList.slice(0, 30).map((t: any) => {
        const cols = (t.columns || []).map((c: any) => c.name).join(', ');
        return `${t.name}(${cols})`;
      }).join('\n');
      envVars.push({ key: 'schema', value: schema, isSecret: false });
    }
  }

  // List other connected SQL instances so AI can target them
  const allConnected = get(connectedIds);
  for (const c of conns.filter((c: any) => c.id !== connId && allConnected.has(c.id)).slice(0, 3)) {
    const otherLiveId = getLiveId(c.id) || c.id;
    envVars.push({ key: `other_sql_connection`, value: `${c.name} (saved_id: ${c.id}, connection_id: ${otherLiveId}, driver: ${c.driver})`, isSecret: false });
  }

  return { mode: 'sql', currentRequest, currentResponse, envVars };
}

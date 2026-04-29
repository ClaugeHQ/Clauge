import { get } from 'svelte/store';
import { nosqlConnections, connectedNoSqlIds, activeNoSqlConnectionId, getNoSqlTabData, nosqlLiveConnectionIds } from '../stores';
import { activeTabId } from '$lib/stores/tabs';
import type { ChatContext, ContextRequest, ContextEnvVar } from '$lib/types/ai';

export async function gatherNosqlContext(): Promise<ChatContext> {
  const conns = get(nosqlConnections);
  const connected = get(connectedNoSqlIds);
  const tabId = get(activeTabId);
  const tabData = getNoSqlTabData(tabId);
  const liveIds = get(nosqlLiveConnectionIds);

  // Use tab's connection if available, fall back to global active
  const tabConnId = tabData.connectionId || get(activeNoSqlConnectionId) || '';
  const activeConn = conns.find(c => c.id === tabConnId);

  const envVars: ContextEnvVar[] = [];

  if (activeConn) {
    const isConnected = connected.has(activeConn.id);
    if (!isConnected) {
      envVars.push({ key: 'connection_status', value: 'disconnected', isSecret: false });
      envVars.push({ key: 'connection_name', value: activeConn.name, isSecret: false });
      return { mode: 'nosql', currentRequest: null, currentResponse: null, envVars };
    }

    const liveId = liveIds[activeConn.id] || activeConn.id;
    envVars.push({ key: 'connection_id', value: liveId, isSecret: false });
    envVars.push({ key: 'connection_name', value: activeConn.name, isSecret: false });
    envVars.push({ key: 'driver', value: activeConn.driver, isSecret: false });
    envVars.push({ key: 'connected', value: 'yes', isSecret: false });
  }

  // Active database + collection from tab state
  if (tabData.database) {
    envVars.push({ key: 'database', value: tabData.database, isSecret: false });
  }
  if (tabData.collection) {
    envVars.push({ key: 'collection', value: tabData.collection, isSecret: false });
  }

  // Current filter query
  let currentRequest = null;
  if (tabData.filterQuery && tabData.filterQuery !== '{}') {
    currentRequest = {
      method: activeConn?.driver === 'redis' ? 'REDIS' : 'MONGO',
      url: tabData.collection || '',
      headers: [] as { key: string; value: string; enabled: boolean }[],
      params: [] as { key: string; value: string; enabled: boolean }[],
      body: tabData.filterQuery,
      bodyType: 'json',
      authType: 'none',
      authData: '{}',
    };
  }

  // List other connected instances for context
  for (const c of conns.filter(c => c.id !== tabConnId && connected.has(c.id)).slice(0, 3)) {
    const otherLiveId = liveIds[c.id] || c.id;
    envVars.push({ key: `other_connection_${c.driver}`, value: `${c.name} (id: ${otherLiveId})`, isSecret: false });
  }

  return { mode: 'nosql', currentRequest, currentResponse: null, envVars };
}

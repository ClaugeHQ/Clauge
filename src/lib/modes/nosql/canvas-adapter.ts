import { mount, unmount } from 'svelte';
import { get } from 'svelte/store';
import type { CanvasTabAdapter } from '$lib/modes/canvas/adapter';
import { tabs, activateTab } from '$lib/shared/stores/tabs';
import { setMode } from '$lib/stores/app';
import {
  nosqlTabState,
  nosqlConnections,
  activeNoSqlConnectionId,
} from './stores';
import NoSqlQueryTileBody from './components/NoSqlQueryTileBody.svelte';

type Driver = 'mongodb' | 'redis';

function driverOfTab(tabId: number): Driver | null {
  const data = get(nosqlTabState).get(tabId);
  if (!data) return null;
  const conn = get(nosqlConnections).find((c) => c.id === data.connectionId);
  return (conn?.driver as Driver | undefined) ?? null;
}

function listNoSqlTabsForDriver(driver: Driver): { id: string; title: string }[] {
  const out: { id: string; title: string }[] = [];
  const allTabs = get(tabs);
  const conns = get(nosqlConnections);
  const state = get(nosqlTabState);
  for (const t of allTabs) {
    if (t.mode !== 'nosql') continue;
    const data = state.get(t.id);
    const conn = data ? conns.find((c) => c.id === data.connectionId) : undefined;
    if (conn) {
      if (conn.driver !== driver) continue;
    } else {
      // Unbound tab (no connection chosen yet, or connection vanished) —
      // default to the Mongo adapter so the user can pick a connection
      // from the in-tile picker. Skip from the Redis listing.
      if (driver !== 'mongodb') continue;
    }
    out.push({ id: String(t.id), title: t.label });
  }
  return out;
}

function metaFor(tabId: number): { title: string } {
  const data = get(nosqlTabState).get(tabId);
  const allTabs = get(tabs);
  const tab = allTabs.find((t) => t.id === tabId);
  const fallback = tab?.label ?? `Tab ${tabId}`;
  if (!data) return { title: fallback };
  const conn = get(nosqlConnections).find((c) => c.id === data.connectionId);
  if (!conn) return { title: fallback };
  if (conn.driver === 'redis') {
    return { title: `Redis / ${conn.name}` };
  }
  const dbColl = data.database && data.collection
    ? `${data.database}.${data.collection}`
    : data.database || conn.name;
  return { title: `MongoDB / ${dbColl}` };
}

function renderTile(
  tabId: string,
  slot: HTMLElement,
): { destroy: () => void } {
  const numericId = Number(tabId);
  const component = mount(NoSqlQueryTileBody, {
    target: slot,
    props: { tabId: numericId },
  });
  return {
    destroy: () => {
      try {
        unmount(component);
      } catch {
        // Already unmounted (e.g., HMR) — ignore.
      }
    },
  };
}

function openHomeMode(tabId: string): void {
  const numericId = Number(tabId);
  const tab = get(tabs).find((t) => t.id === numericId && t.mode === 'nosql');
  if (!tab) return;
  const data = get(nosqlTabState).get(numericId);
  if (data?.connectionId) {
    activeNoSqlConnectionId.set(data.connectionId);
  }
  activateTab(tab.id);
  void setMode('nosql');
}

function closeViaCanvas(tabId: string): void {
  const numericId = Number(tabId);
  const tab = get(tabs).find((t) => t.id === numericId && t.mode === 'nosql');
  if (!tab) return;
  window.dispatchEvent(
    new CustomEvent('canvas:request-tab-close', { detail: { tabId: tab.id } }),
  );
}

function subscribeToNoSql(onChange: () => void): () => void {
  const u1 = tabs.subscribe(() => onChange());
  const u2 = nosqlTabState.subscribe(() => onChange());
  const u3 = nosqlConnections.subscribe(() => onChange());
  return () => {
    u1();
    u2();
    u3();
  };
}

export const mongoQueryAdapter: CanvasTabAdapter = {
  tabKind: 'mongo_query',
  mountStrategy: 'remount',
  defaultSpawnSize: { width: 800, height: 600 },

  listOpenTabs(_workspaceId) {
    return listNoSqlTabsForDriver('mongodb');
  },

  subscribe(_workspaceId, onChange) {
    return subscribeToNoSql(onChange);
  },

  render(tabId, slot, _ctx) {
    return renderTile(tabId, slot);
  },

  getMeta(tabId) {
    return metaFor(Number(tabId));
  },

  openInHomeMode(tabId) {
    openHomeMode(tabId);
  },

  closeTab(tabId) {
    closeViaCanvas(tabId);
  },
};

export const redisQueryAdapter: CanvasTabAdapter = {
  tabKind: 'redis_query',
  mountStrategy: 'remount',
  defaultSpawnSize: { width: 750, height: 520 },

  listOpenTabs(_workspaceId) {
    return listNoSqlTabsForDriver('redis');
  },

  subscribe(_workspaceId, onChange) {
    return subscribeToNoSql(onChange);
  },

  render(tabId, slot, _ctx) {
    return renderTile(tabId, slot);
  },

  getMeta(tabId) {
    return metaFor(Number(tabId));
  },

  openInHomeMode(tabId) {
    openHomeMode(tabId);
  },

  closeTab(tabId) {
    closeViaCanvas(tabId);
  },
};

import { get } from 'svelte/store';
import { mount, unmount } from 'svelte';
import type { CanvasTabAdapter } from '$lib/modes/canvas/adapter';
import { tabs, activateTab } from '$lib/shared/stores/tabs';
import {
  explorerConnections,
  explorerConnStates,
  activeExplorerConnection,
} from '$lib/modes/explorer/stores';
import { connectionIdFromTabKey } from '$lib/modes/explorer/tabkey';
import { setMode } from '$lib/stores/app';
import FileBrowserTileBody from '$lib/modes/explorer/components/FileBrowserTileBody.svelte';

function statusDotFor(state: 'connecting' | 'connected' | 'error' | 'disconnected' | undefined): string {
  switch (state) {
    case 'connected':
      return 'var(--ok, #2ecc71)';
    case 'connecting':
      return 'var(--warn, #f39c12)';
    case 'error':
      return 'var(--err, #e74c3c)';
    case 'disconnected':
    default:
      return 'var(--t4, #888)';
  }
}

export const explorerFileBrowserAdapter: CanvasTabAdapter = {
  tabKind: 'explorer_file_browser',
  mountStrategy: 'remount',
  defaultSpawnSize: { width: 700, height: 480 },

  listOpenTabs(_workspaceId) {
    return get(tabs)
      .filter((t) => t.mode === 'explorer' && t.key)
      .map((t) => ({ id: t.key as string, title: t.label }));
  },

  subscribe(_workspaceId, onChange) {
    const u1 = tabs.subscribe(() => onChange());
    const u2 = explorerConnStates.subscribe(() => onChange());
    return () => {
      u1();
      u2();
    };
  },

  render(tabId, slot, _ctx) {
    const component = mount(FileBrowserTileBody, {
      target: slot,
      props: { tabId },
    });
    return {
      destroy: () => {
        void unmount(component);
      },
    };
  },

  getMeta(tabId) {
    const connId = connectionIdFromTabKey(tabId);
    const conn = get(explorerConnections).find((c) => c.id === connId);
    const baseName = conn?.name ?? connId;
    const hashIdx = tabId.indexOf('#');
    const suffix = hashIdx !== -1 ? tabId.slice(hashIdx + 1) : null;
    const title = suffix ? `${baseName} — ${suffix}` : baseName;

    const state = get(explorerConnStates).get(tabId);
    return {
      title,
      statusDot: statusDotFor(state),
    };
  },

  openInHomeMode(tabId) {
    const connId = connectionIdFromTabKey(tabId);
    const conn = get(explorerConnections).find((c) => c.id === connId) ?? null;

    const match = get(tabs).find((t) => t.mode === 'explorer' && t.key === tabId);
    if (match) activateTab(match.id);

    activeExplorerConnection.set(conn);
    void setMode('explorer');
  },

  closeTab(tabId) {
    const topbarTab = get(tabs).find((t) => t.mode === 'explorer' && t.key === tabId);
    if (!topbarTab) return;
    window.dispatchEvent(
      new CustomEvent('canvas:request-tab-close', { detail: { tabId: topbarTab.id } }),
    );
  },
};

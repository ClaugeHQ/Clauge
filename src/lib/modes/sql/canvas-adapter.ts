import type { CanvasTabAdapter } from '$lib/modes/canvas/adapter';
import { get } from 'svelte/store';
import { mount, unmount } from 'svelte';
import { sqlTabState, sqlPendingChanges, connections } from './stores';
import { tabs, activateTab } from '$lib/shared/stores/tabs';
import { setMode } from '$lib/stores/app';
import SqlEditorTileBody from './components/SqlEditorTileBody.svelte';

/**
 * Canvas adapter for SQL editor tabs. Uses the `remount` strategy: the
 * tile body is a freshly mounted `SqlEditorTileBody` that hosts the
 * singleton CodeMirror EditorView (via the reparent registry), a Run
 * button strip, and the same ResultsTable the home panel uses. Tab
 * state is read straight off `sqlTabState[tabId]` so home + tile stay
 * in sync.
 *
 * Canvas id is `tab.key` when the script is saved, or `draft:<tab.id>`
 * for unsaved drafts. Drafts won't survive an app restart (their
 * numeric tab id is session-local), but they do show on canvas while
 * the session lives so users can edit them side by side with anything
 * else.
 */
const DRAFT_PREFIX = 'draft:';

function canvasIdFor(t: { id: number; key: string | null }): string {
  return t.key ?? `${DRAFT_PREFIX}${t.id}`;
}

function findSqlTabByCanvasId(canvasId: string) {
  if (canvasId.startsWith(DRAFT_PREFIX)) {
    const numeric = Number(canvasId.slice(DRAFT_PREFIX.length));
    return get(tabs).find((t) => t.mode === 'sql' && t.id === numeric);
  }
  return get(tabs).find((t) => t.mode === 'sql' && t.key === canvasId);
}

function bindingTitle(tabId: number, fallback: string): string {
  const data = get(sqlTabState).get(tabId);
  const binding = data?.binding;
  if (!binding) return fallback;
  const conn = get(connections).find((c) => c.id === binding.connectionId);
  const connLabel = conn?.name ?? binding.connectionId;
  return `${fallback} — ${connLabel} / ${binding.database}`;
}

export const sqlEditorAdapter: CanvasTabAdapter = {
  tabKind: 'sql_editor',
  mountStrategy: 'remount',
  defaultSpawnSize: { width: 900, height: 650 },

  listOpenTabs(_workspaceId) {
    return get(tabs)
      .filter((t) => t.mode === 'sql')
      .map((t) => ({ id: canvasIdFor(t), title: t.label }));
  },

  subscribe(_workspaceId, onChange) {
    const unsubTabs = tabs.subscribe(() => onChange());
    const unsubState = sqlTabState.subscribe(() => onChange());
    return () => {
      unsubTabs();
      unsubState();
    };
  },

  render(tabId, slot) {
    const tab = findSqlTabByCanvasId(tabId);
    if (!tab) return { destroy: () => {} };
    let component: ReturnType<typeof mount> | null = null;
    try {
      component = mount(SqlEditorTileBody, {
        target: slot,
        props: { tabId: tab.id },
      });
    } catch (err) {
      console.error('[atlas] failed to mount SQL tile body', { tabId, err });
      return { destroy: () => {} };
    }
    return {
      destroy: () => {
        if (!component) return;
        try {
          void unmount(component);
        } catch (err) {
          console.error('[atlas] failed to unmount SQL tile body', { tabId, err });
        }
      },
    };
  },

  getMeta(tabId) {
    const tab = findSqlTabByCanvasId(tabId);
    if (!tab) return { title: tabId };
    const title = bindingTitle(tab.id, tab.label);
    const dirty = get(sqlPendingChanges).has(tab.id);
    return { title, dirty };
  },

  openInHomeMode(tabId) {
    const tab = findSqlTabByCanvasId(tabId);
    if (tab) activateTab(tab.id);
    void setMode('sql');
  },

  closeTab(tabId) {
    const tab = findSqlTabByCanvasId(tabId);
    if (!tab) return;
    window.dispatchEvent(
      new CustomEvent('canvas:request-tab-close', { detail: { tabId: tab.id } }),
    );
  },
};

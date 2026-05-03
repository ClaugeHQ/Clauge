import { writable, get } from 'svelte/store';
import type { KVInput } from '$lib/types';

export interface Tab {
  id: number;
  label: string;
  mode: 'rest' | 'sql' | 'nosql' | 'agent' | 'ssh' | 'explorer';
  key: string | null;
  dot: string;
  dirty: boolean;
  unsaved: boolean;
}

export interface DraftRequest {
  method: string;
  url: string;
  body: string;
  bodyType: string;
  headers: KVInput[];
  params: KVInput[];
  authType: string;
  authData: string;
  preScript: string;
}

let nextId = 0;

export const tabs = writable<Tab[]>([]);
export const activeTabId = writable<number>(-1);
export const draftRequests = writable<Map<number, Partial<DraftRequest>>>(new Map());

export function addTab(label: string, mode: 'rest' | 'sql' | 'nosql' | 'agent' | 'ssh' | 'explorer', key: string | null, dot: string): Tab {
  const isNew = key === null;
  const tab: Tab = { id: nextId++, label, mode, key, dot, dirty: false, unsaved: isNew };
  tabs.update(t => [...t, tab]);
  activeTabId.set(tab.id);
  return tab;
}

export function closeTab(id: number) {
  tabs.update(t => {
    const closingTab = t.find(x => x.id === id);
    const filtered = t.filter(x => x.id !== id);
    if (get(activeTabId) === id && filtered.length > 0) {
      // Prefer the most recent tab of the same mode as the closing tab —
      // keeps the user in their current mental context. Fall back to the
      // most recent tab overall (cross-mode); callers that care about
      // the resulting mode should realign $mode after this returns.
      const sameMode = closingTab
        ? filtered.filter(x => x.mode === closingTab.mode)
        : [];
      const next = sameMode.length > 0
        ? sameMode[sameMode.length - 1]
        : filtered[filtered.length - 1];
      activeTabId.set(next.id);
    } else if (filtered.length === 0) {
      activeTabId.set(-1);
    }
    return filtered;
  });
  clearDraft(id);
}

export function activateTab(id: number) {
  activeTabId.set(id);
}

export function updateTab(id: number, updates: Partial<Tab>) {
  tabs.update(t => t.map(x => x.id === id ? { ...x, ...updates } : x));
}

export function setDraft(tabId: number, data: Partial<DraftRequest>) {
  draftRequests.update(m => {
    const existing = m.get(tabId) ?? {};
    const updated = { ...existing, ...data };
    const next = new Map(m);
    next.set(tabId, updated);
    return next;
  });
}

export function getDraft(tabId: number): Partial<DraftRequest> | undefined {
  return get(draftRequests).get(tabId);
}

export function clearDraft(tabId: number) {
  draftRequests.update(m => {
    const next = new Map(m);
    next.delete(tabId);
    return next;
  });
}

export function markDirty(tabId: number) {
  tabs.update(t => t.map(x => x.id === tabId ? { ...x, dirty: true } : x));
}

export function markClean(tabId: number) {
  tabs.update(t => t.map(x => x.id === tabId ? { ...x, dirty: false } : x));
}

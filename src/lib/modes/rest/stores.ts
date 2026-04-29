// REST mode runtime state — collections / requests, environments,
// per-request env overrides, and request history. Consolidated from
// former $lib/stores/{collections,environments,history}.ts.

import { writable, get } from 'svelte/store';
import type {
  Collection,
  Request,
  RequestWithDetails,
  RequestUpdate,
  KVInput,
  HttpResponse,
  Environment,
  EnvVariable,
  HistoryEntry,
} from './types';
import * as cmd from './commands';
import { STORAGE_KEYS } from '$lib/shared/constants/storage';

// ── Collections / requests ─────────────────────────────────────────────

export const collections = writable<Collection[]>([]);
export const collectionsRefreshTrigger = writable(0);
export const activeCollectionId = writable<string | null>(null);
export const activeRequestId = writable<string | null>(null);
export const activeRequest = writable<RequestWithDetails | null>(null);

export const currentRestResponse = writable<HttpResponse | null>(null);

/** Per-request environment overrides (requestId/tabId -> envId) */
const savedOverrides = typeof localStorage !== 'undefined' ? JSON.parse(localStorage.getItem(STORAGE_KEYS.REQUEST_ENV_OVERRIDES) || '{}') : {};
export const requestEnvOverrides = writable<Record<string, string>>(savedOverrides);
// Keep old name as alias for backward compatibility during migration
export const collectionEnvOverrides = requestEnvOverrides;

export async function loadCollections() {
  try {
    const data = await cmd.listCollections();
    collections.set(data);
    collectionsRefreshTrigger.update(n => n + 1);
  } catch (err) {
    console.error('Failed to load collections:', err);
  }
}

export async function createCollection(name: string) {
  const coll = await cmd.createCollection(name);
  collections.update(c => [...c, coll]);
  return coll;
}

export async function deleteCollection(id: string) {
  await cmd.deleteCollection(id);
  collections.update(c => c.filter(x => x.id !== id));
  if (get(activeCollectionId) === id) {
    activeCollectionId.set(null);
  }
}

export async function updateCollection(id: string, name: string, envId: string | null) {
  const updated = await cmd.updateCollection(id, name, envId);
  collections.update(c => c.map(x => x.id === id ? updated : x));
}

export async function loadRequest(id: string) {
  const req = await cmd.getRequest(id);
  activeRequestId.set(id);
  activeRequest.set(req);
  currentRestResponse.set(null); // Clear stale response when switching requests
}

export function clearActiveRequest() {
  activeRequestId.set(null);
  activeRequest.set(null);
  currentRestResponse.set(null);
}

export async function createRequest(collectionId: string, name: string, method: string) {
  const req = await cmd.createRequest(collectionId, name, method);
  return req;
}

export async function deleteRequest(id: string) {
  await cmd.deleteRequest(id);
  activeRequest.update(r => r?.id === id ? null : r);
  if (get(activeRequestId) === id) {
    activeRequestId.set(null);
  }
}

export async function saveRequest(id: string, data: RequestUpdate) {
  await cmd.updateRequest(id, data);
}

export async function saveHeaders(requestId: string, headers: KVInput[]) {
  await cmd.updateRequestHeaders(requestId, headers);
}

export async function saveParams(requestId: string, params: KVInput[]) {
  await cmd.updateRequestParams(requestId, params);
}

export async function commitRequest(requestId: string, draft: { method?: string; url?: string; body?: string; bodyType?: string; authType?: string; authData?: string; preScript?: string; headers?: { key: string; value: string; enabled: number }[]; params?: { key: string; value: string; enabled: number }[] }) {
  const { headers, params, ...requestData } = draft;
  const hasRequestData = Object.keys(requestData).length > 0;
  if (hasRequestData) {
    await cmd.updateRequest(requestId, requestData);
  }
  if (headers) {
    await cmd.updateRequestHeaders(requestId, headers);
  }
  if (params) {
    await cmd.updateRequestParams(requestId, params);
  }
  // Reload so activeRequest and sidebar reflect saved state
  await loadRequest(requestId);
  await loadCollections();
}

export function setRequestEnv(requestOrTabId: string, envId: string | null) {
  requestEnvOverrides.update(map => {
    let next: Record<string, string>;
    if (envId === null) {
      const { [requestOrTabId]: _, ...rest } = map;
      next = rest;
    } else {
      next = { ...map, [requestOrTabId]: envId };
    }
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(STORAGE_KEYS.REQUEST_ENV_OVERRIDES, JSON.stringify(next));
    }
    return next;
  });
}

// Backward compatibility alias
export const setCollectionEnv = setRequestEnv;

// ── Environments ───────────────────────────────────────────────────────

export const environments = writable<Environment[]>([]);

// Persist active env selection
const savedEnvId = typeof localStorage !== 'undefined' ? localStorage.getItem(STORAGE_KEYS.ACTIVE_ENV_ID) : null;
export const activeEnvId = writable<string | null>(savedEnvId);

export async function loadEnvironments() {
  try {
    const envs = await cmd.listEnvironments();
    environments.set(envs);
    // Read current activeEnvId from localStorage (not the stale module-level snapshot)
    const current = typeof localStorage !== 'undefined' ? localStorage.getItem(STORAGE_KEYS.ACTIVE_ENV_ID) : null;
    const currentExists = current && envs.some(e => e.id === current);
    if (!currentExists && envs.length > 0) {
      const def = envs.find(e => e.isDefault === 1);
      if (def) setActiveEnv(def.id);
      else setActiveEnv(envs[0].id);
    } else if (!currentExists && envs.length === 0) {
      activeEnvId.set(null);
      if (typeof localStorage !== 'undefined') {
        localStorage.removeItem(STORAGE_KEYS.ACTIVE_ENV_ID);
      }
    }
  } catch (err) {
    console.error('Failed to load environments:', err);
  }
}

export async function createEnvironment(name: string, color: string) {
  const env = await cmd.createEnvironment(name, color);
  environments.update(e => [...e, env]);
  // Auto-activate if it's the first (and now default) environment
  if (env.isDefault === 1) {
    setActiveEnv(env.id);
  }
  return env;
}

export async function updateEnvironment(id: string, name: string, color: string) {
  const env = await cmd.updateEnvironment(id, name, color);
  environments.update(e => e.map(x => x.id === id ? env : x));
}

export async function deleteEnvironment(id: string) {
  await cmd.deleteEnvironment(id);
  environments.update(e => e.filter(x => x.id !== id));
  // Clear activeEnvId if the deleted env was the active one
  const currentActive = typeof localStorage !== 'undefined' ? localStorage.getItem(STORAGE_KEYS.ACTIVE_ENV_ID) : null;
  if (currentActive === id) {
    activeEnvId.set(null);
    if (typeof localStorage !== 'undefined') {
      localStorage.removeItem(STORAGE_KEYS.ACTIVE_ENV_ID);
    }
  }
  // Remove all per-request overrides pointing to the deleted env (revert to global)
  if (typeof localStorage !== 'undefined') {
    const overridesRaw = localStorage.getItem(STORAGE_KEYS.REQUEST_ENV_OVERRIDES);
    if (overridesRaw) {
      try {
        const overrides = JSON.parse(overridesRaw);
        const cleaned: Record<string, string> = {};
        for (const [key, val] of Object.entries(overrides)) {
          if (val !== id) cleaned[key] = val as string;
        }
        localStorage.setItem(STORAGE_KEYS.REQUEST_ENV_OVERRIDES, JSON.stringify(cleaned));
        requestEnvOverrides.set(cleaned);
      } catch {}
    }
  }
}

export async function setDefaultEnv(id: string) {
  await cmd.setDefaultEnvironment(id);
  activeEnvId.set(id);
  await loadEnvironments();
}

export async function setActiveEnv(id: string) {
  activeEnvId.set(id);
  if (typeof localStorage !== 'undefined') {
    localStorage.setItem(STORAGE_KEYS.ACTIVE_ENV_ID, id);
  }
}

export async function loadEnvVariables(envId: string): Promise<EnvVariable[]> {
  return cmd.listEnvVariables(envId);
}

export async function setEnvVariable(envId: string, key: string, value: string, isSecret: number) {
  return cmd.setEnvVariable(envId, key, value, isSecret);
}

export async function updateEnvVariable(id: string, key: string, value: string, isSecret: number) {
  return cmd.updateEnvVariable(id, key, value, isSecret);
}

export async function deleteEnvVariable(id: string) {
  return cmd.deleteEnvVariable(id);
}

export function getEffectiveEnvId(
  requestOrTabId: string | null,
  overrides: Record<string, string>,
  globalEnvId: string | null,
): string | null {
  if (requestOrTabId && overrides[requestOrTabId]) {
    return overrides[requestOrTabId];
  }
  return globalEnvId;
}

// ── History ────────────────────────────────────────────────────────────

export const history = writable<HistoryEntry[]>([]);
export const historyOpen = writable<boolean>(false);
export const activeHistoryEntry = writable<HistoryEntry | null>(null);

export async function loadHistory(limit: number = 50) {
  try {
    const entries = await cmd.listHistory(limit);
    history.set(entries);
  } catch (err) {
    console.error('Failed to load history:', err);
  }
}

export async function clearHistory() {
  await cmd.clearHistory();
  history.set([]);
}

export async function deleteHistoryEntry(id: string) {
  await cmd.deleteHistoryEntry(id);
  history.update(h => h.filter(x => x.id !== id));
}

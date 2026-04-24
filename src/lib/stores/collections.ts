import { writable, get } from 'svelte/store';
import type { Collection, Request, RequestWithDetails, RequestUpdate, KVInput, HttpResponse } from '$lib/types';
import * as cmd from '$lib/commands';

export const collections = writable<Collection[]>([]);
export const collectionsRefreshTrigger = writable(0);
export const activeCollectionId = writable<string | null>(null);
export const activeRequestId = writable<string | null>(null);
export const activeRequest = writable<RequestWithDetails | null>(null);

export const currentRestResponse = writable<HttpResponse | null>(null);

/** Per-request environment overrides (requestId/tabId -> envId) */
const savedOverrides = typeof localStorage !== 'undefined' ? JSON.parse(localStorage.getItem('clauge_request_env_overrides') || '{}') : {};
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
      localStorage.setItem('clauge_request_env_overrides', JSON.stringify(next));
    }
    return next;
  });
}

// Backward compatibility alias
export const setCollectionEnv = setRequestEnv;

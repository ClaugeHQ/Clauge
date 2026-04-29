import { get } from 'svelte/store';
import { activeRequest, requestEnvOverrides, currentRestResponse, collections, activeCollectionId, activeEnvId, getEffectiveEnvId, loadEnvVariables } from '../stores';
import { activeTabId, getDraft } from '$lib/shared/stores/tabs';
import type { ChatContext, ContextRequest, ContextResponse, ContextEnvVar } from '$lib/types/ai';

export async function gatherRestContext(): Promise<ChatContext> {
  const req = get(activeRequest);
  const tabId = get(activeTabId);
  const draft = getDraft(tabId);
  const response = get(currentRestResponse);

  let currentRequest: ContextRequest | null = null;
  if (req || draft) {
    const r = req;
    const d = draft;
    currentRequest = {
      method: d?.method || r?.method || 'GET',
      url: d?.url || r?.url || '',
      headers: (d?.headers || r?.headers || []).map((h: any) => ({
        key: h.key, value: h.value, enabled: h.enabled === 1 || h.enabled === true,
      })),
      params: (d?.params || r?.params || []).map((p: any) => ({
        key: p.key, value: p.value, enabled: p.enabled === 1 || p.enabled === true,
      })),
      body: d?.body || r?.body || '',
      bodyType: d?.bodyType || r?.bodyType || 'json',
      authType: d?.authType || r?.authType || 'none',
      authData: d?.authData || r?.authData || '{}',
    };
  }

  let currentResponse: ContextResponse | null = null;
  if (response) {
    currentResponse = {
      status: response.status,
      statusText: response.status_text,
      headers: response.headers,
      body: response.body,
      durationMs: response.duration_ms,
      sizeBytes: response.size_bytes,
    };
  }

  let envVars: ContextEnvVar[] = [];
  const req2 = get(activeRequest);
  const tabId2 = get(activeTabId);
  const overrideKey = req2?.id ?? String(tabId2);
  const overrides = get(requestEnvOverrides);
  const globalEnv = get(activeEnvId);
  const envId = getEffectiveEnvId(overrideKey, overrides, globalEnv);
  const collId = get(activeCollectionId);

  // Include active collection name for context
  if (collId) {
    const colls = get(collections);
    const activeCollection = colls.find(c => c.id === collId);
    if (activeCollection) {
      envVars.push({ key: 'collection_name', value: activeCollection.name, isSecret: false });
      envVars.push({ key: 'collection_id', value: activeCollection.id, isSecret: false });
    }
  }
  if (envId) {
    try {
      const vars = await loadEnvVariables(envId);
      envVars.push(...vars.map((v: any) => ({
        key: v.key, value: v.value, isSecret: v.isSecret === 1,
      })));
    } catch { /* ignore */ }
  }

  return { mode: 'rest', currentRequest, currentResponse, envVars };
}

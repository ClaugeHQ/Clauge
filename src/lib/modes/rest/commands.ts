// REST mode invoke wrappers — consolidated from former
// $lib/commands/{collections,requests,environments,http,history,import_export}.ts.

import { invoke } from '@tauri-apps/api/core';
import type {
  Collection,
  Request,
  RequestWithDetails,
  RequestUpdate,
  RequestHeader,
  RequestParam,
  KVInput,
  Environment,
  EnvVariable,
  HttpResponse,
  HistoryEntry,
  ImportResult,
} from './types';

// ── Collections ────────────────────────────────────────────────────────

export async function listCollections(): Promise<Collection[]> {
  return invoke('list_collections');
}

export async function createCollection(name: string): Promise<Collection> {
  return invoke('create_collection', { name });
}

export async function updateCollection(id: string, name: string, envId: string | null): Promise<Collection> {
  return invoke('update_collection', { id, name, envId });
}

export async function deleteCollection(id: string): Promise<void> {
  return invoke('delete_collection', { id });
}

export async function reorderCollections(ids: string[]): Promise<void> {
  return invoke('reorder_collections', { ids });
}

// ── Requests ───────────────────────────────────────────────────────────

export async function listRequests(collectionId: string): Promise<Request[]> {
  return invoke('list_requests', { collectionId });
}

export async function getRequest(id: string): Promise<RequestWithDetails> {
  return invoke('get_request', { id });
}

export async function createRequest(collectionId: string, name: string, method: string): Promise<Request> {
  return invoke('create_request', { collectionId, name, method });
}

export async function updateRequest(id: string, data: RequestUpdate): Promise<Request> {
  return invoke('update_request', { id, data });
}

export async function deleteRequest(id: string): Promise<void> {
  return invoke('delete_request', { id });
}

export async function duplicateRequest(id: string): Promise<Request> {
  return invoke('duplicate_request', { id });
}

export async function moveRequest(id: string, targetCollectionId: string): Promise<Request> {
  return invoke('move_request', { id, targetCollectionId });
}

export async function updateRequestHeaders(requestId: string, headers: KVInput[]): Promise<RequestHeader[]> {
  return invoke('update_request_headers', { requestId, headers });
}

export async function updateRequestParams(requestId: string, params: KVInput[]): Promise<RequestParam[]> {
  return invoke('update_request_params', { requestId, params });
}

// ── Environments ───────────────────────────────────────────────────────

export async function listEnvironments(): Promise<Environment[]> {
  return invoke('list_environments');
}

export async function createEnvironment(name: string, color: string): Promise<Environment> {
  return invoke('create_environment', { name, color });
}

export async function updateEnvironment(id: string, name: string, color: string): Promise<Environment> {
  return invoke('update_environment', { id, name, color });
}

export async function deleteEnvironment(id: string): Promise<void> {
  return invoke('delete_environment', { id });
}

export async function setDefaultEnvironment(id: string): Promise<void> {
  return invoke('set_default_environment', { id });
}

export async function listEnvVariables(environmentId: string): Promise<EnvVariable[]> {
  return invoke('list_env_variables', { environmentId });
}

export async function setEnvVariable(environmentId: string, key: string, value: string, isSecret: number): Promise<EnvVariable> {
  return invoke('set_env_variable', { environmentId, key, value, isSecret });
}

export async function updateEnvVariable(id: string, key: string, value: string, isSecret: number): Promise<EnvVariable> {
  return invoke('update_env_variable', { id, key, value, isSecret });
}

export async function deleteEnvVariable(id: string): Promise<void> {
  return invoke('delete_env_variable', { id });
}

export async function getEnvVariablesForResolution(environmentId: string): Promise<Record<string, string>> {
  return invoke('get_env_variables_for_resolution', { environmentId });
}

// ── HTTP execution ─────────────────────────────────────────────────────

export async function executeRequest(requestId: string, environmentId: string): Promise<HttpResponse> {
  return invoke('execute_request', { requestId, environmentId });
}

export async function quickExecute(method: string, url: string, body: string = '', headers: [string, string][] = [], environmentId: string = '', authType: string = 'none', authData: string = '{}', bodyType: string = 'json'): Promise<HttpResponse> {
  return invoke('quick_execute', { method, url, body, headers, environmentId, authType, authData, bodyType });
}

// ── History ────────────────────────────────────────────────────────────

export async function listHistory(limit: number): Promise<HistoryEntry[]> {
  return invoke('list_history', { limit });
}

export async function clearHistory(): Promise<void> {
  return invoke('clear_history');
}

export async function deleteHistoryEntry(id: string): Promise<void> {
  return invoke('delete_history_entry', { id });
}

// ── Import / Export ────────────────────────────────────────────────────

export async function exportCollection(collectionId: string): Promise<string> {
  return invoke('export_collection', { collectionId });
}

export async function exportAllCollections(): Promise<string> {
  return invoke('export_all_collections');
}

export async function importClauge(json: string): Promise<ImportResult> {
  return invoke('import_clauge', { json });
}

export async function importPostman(json: string): Promise<ImportResult> {
  return invoke('import_postman', { json });
}

export async function importCurl(curlCommand: string, collectionId?: string): Promise<string> {
  return invoke('import_curl', { curlCommand, collectionId: collectionId ?? null });
}

export async function exportAsCurl(requestId: string, environmentId?: string): Promise<string> {
  return invoke('export_as_curl', { requestId, environmentId: environmentId ?? null });
}

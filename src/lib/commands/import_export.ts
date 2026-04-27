import { invoke } from '@tauri-apps/api/core';
import type { ImportResult } from '$lib/types';

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

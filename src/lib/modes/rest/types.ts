// REST mode types — collections, requests, environments, HTTP, history.
// Consolidated from former $lib/types/{collection,environment,http}.ts and
// the HistoryEntry slice of $lib/types/settings.ts (which is REST-only).

export interface Collection {
  id: string;
  name: string;
  description: string;
  sortOrder: number;
  envId: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface Request {
  id: string;
  collectionId: string;
  name: string;
  description: string;
  method: string;
  url: string;
  body: string;
  bodyType: string;
  authType: string;
  authData: string;
  preScript: string;
  sortOrder: number;
  createdAt: string;
  updatedAt: string;
}

export interface RequestHeader {
  id: string;
  requestId: string;
  key: string;
  value: string;
  enabled: number;
  sortOrder: number;
}

export interface RequestParam {
  id: string;
  requestId: string;
  key: string;
  value: string;
  enabled: number;
  sortOrder: number;
}

export interface RequestWithDetails extends Request {
  headers: RequestHeader[];
  params: RequestParam[];
}

export interface RequestUpdate {
  name?: string;
  method?: string;
  url?: string;
  body?: string;
  bodyType?: string;
  authType?: string;
  authData?: string;
  preScript?: string;
}

export interface KVInput {
  key: string;
  value: string;
  enabled: number;
}

export interface ImportResult {
  collectionsCount: number;
  requestsCount: number;
  message: string;
}

export interface Environment {
  id: string;
  name: string;
  color: string;
  isDefault: number;
  sortOrder: number;
  createdAt: string;
  updatedAt: string;
}

export interface EnvVariable {
  id: string;
  environmentId: string;
  key: string;
  value: string;
  isSecret: number;
  sortOrder: number;
}

export interface HttpResponse {
  status: number;
  status_text: string;
  headers: [string, string][];
  body: string;
  duration_ms: number;
  size_bytes: number;
}

export interface HistoryEntry {
  id: string;
  requestId: string | null;
  method: string;
  url: string;
  resolvedUrl: string;
  requestBody: string;
  requestHeaders: string;
  responseStatus: number | null;
  responseBody: string | null;
  responseHeaders: string | null;
  responseSizeBytes: number | null;
  durationMs: number | null;
  environmentId: string | null;
  createdAt: string;
}

// --- Chat types (sent to/from Rust) ---

export interface ChatMessage {
  role: 'user' | 'assistant';
  content: string;
}

export interface ChatContext {
  mode: string;
  currentRequest: ContextRequest | null;
  currentResponse: ContextResponse | null;
  envVars: ContextEnvVar[];
}

export interface ContextRequest {
  method: string;
  url: string;
  headers: ContextKV[];
  params: ContextKV[];
  body: string;
  bodyType: string;
  authType: string;
  authData: string;
}

export interface ContextResponse {
  status: number;
  statusText: string;
  headers: [string, string][];
  body: string;
  durationMs: number;
  sizeBytes: number;
}

export interface ContextKV {
  key: string;
  value: string;
  enabled: boolean;
}

export interface ContextEnvVar {
  key: string;
  value: string;
  isSecret: boolean;
}

// --- UI message types (rendered in AIPanel) ---

export interface AIMessage {
  role: 'user' | 'assistant';
  content: string;
  actions?: AIActionBlock[];
  toolIndicator?: string;
  isStreaming?: boolean;
  error?: { type: 'rate_limit' | 'auth' | 'cloud_auth' | 'generic'; message: string };
  timestamp: number;
}

export interface AIActionBlock {
  type: 'apply_request' | 'execute_result' | 'request_created' | 'collection_executed' | 'apply_query' | 'request_modified' | 'collection_created' | 'switch_environment' | string;
  data: any;
  applied?: boolean;
}

// --- Usage stats ---

export interface AiUsageStat {
  mode: string;
  totalCalls: number;
  inputTokens: number;
  outputTokens: number;
  totalToolRounds: number;
  maxToolRounds: number;
}

export interface AiProviderStat {
  model: string;
  totalCalls: number;
  inputTokens: number;
  outputTokens: number;
  totalToolRounds: number;
  maxToolRounds: number;
}

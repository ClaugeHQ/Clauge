import { invoke } from '@tauri-apps/api/core';
import type { AiUsageStat, AiProviderStat, ChatMessage, ChatContext } from '$lib/types/ai';

export async function testAiKey(apiKey: string, provider: string = 'claude'): Promise<string> {
  return invoke('test_ai_key', { apiKey, provider });
}

export async function getAiUsageStats(): Promise<AiUsageStat[]> {
  return invoke('get_ai_usage_stats');
}

export async function getAiProviderStats(): Promise<AiProviderStat[]> {
  return invoke('get_ai_provider_stats');
}

export async function resetAiUsage(): Promise<void> {
  return invoke('reset_ai_usage');
}

export async function recordAiUsage(
  mode: string,
  model: string,
  inputTokens: number,
  outputTokens: number,
  toolRounds: number = 0,
): Promise<void> {
  return invoke('record_ai_usage', { mode, model, inputTokens, outputTokens, toolRounds });
}

export async function aiChat(
  apiKey: string,
  messages: ChatMessage[],
  context: ChatContext,
  sessionId: string,
  systemPrompt: string,
  tools: any[],
  provider: string = 'claude',
  // Extra HTTP headers to attach upstream. Used by the `clauge` provider
  // to send `X-Provider: github|google` so the worker can pick the right
  // JWKS to validate the bearer.
  extraHeaders?: Record<string, string>,
): Promise<void> {
  return invoke('ai_chat', {
    apiKey,
    messages,
    context,
    sessionId,
    systemPrompt,
    tools,
    provider,
    extraHeaders,
  });
}

/// Returns [token, providerSlug] for the currently signed-in cloud user,
/// or null if not signed in. The token is the Bearer used with the Clauge
/// worker; providerSlug ("github" | "google") tells the worker which JWKS
/// to validate against.
export async function cloudGetActiveToken(): Promise<[string, string] | null> {
  return invoke('cloud_get_active_token');
}

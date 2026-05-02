import { writable } from 'svelte/store';
import type { AIMessage } from '$lib/types/ai';
import { STORAGE_KEYS } from '$lib/shared/constants/storage';

export type AppMode = 'agent' | 'rest' | 'sql' | 'nosql' | 'ssh' | 'explorer' | 'history';

const VALID_MODES: AppMode[] = ['agent', 'rest', 'sql', 'nosql', 'ssh', 'explorer'];

function loadInitialMode(): AppMode {
  try {
    const saved = localStorage.getItem(STORAGE_KEYS.LAST_MODE);
    if (saved && (VALID_MODES as string[]).includes(saved)) return saved as AppMode;
  } catch { /* ignore */ }
  return 'agent';
}

export const mode = writable<AppMode>(loadInitialMode());

// Persist mode changes (skip 'history' — it's a transient view, not a primary mode)
mode.subscribe(v => {
  if ((VALID_MODES as string[]).includes(v)) {
    try { localStorage.setItem(STORAGE_KEYS.LAST_MODE, v); } catch { /* ignore */ }
  }
});
export const navOpen = writable<boolean>(true);
export const aiPanelOpen = writable<boolean>(false);
export const aiPanelOpenPerMode = writable<Record<string, boolean>>({});
export const activeModal = writable<string | null>(null);

// Per-mode AI chat history — persisted to localStorage
function loadChatHistory(): Record<string, AIMessage[]> {
  try {
    const saved = localStorage.getItem(STORAGE_KEYS.AI_CHAT_HISTORY);
    return saved ? JSON.parse(saved) : {};
  } catch { return {}; }
}

function saveChatHistory(history: Record<string, AIMessage[]>) {
  try {
    localStorage.setItem(STORAGE_KEYS.AI_CHAT_HISTORY, JSON.stringify(history));
  } catch { /* ignore */ }
}

export const aiChatHistory = writable<Record<string, AIMessage[]>>(loadChatHistory());

aiChatHistory.subscribe(v => saveChatHistory(v));

export function getModeChatMessages(currentMode: string): AIMessage[] {
  const history = loadChatHistory();
  return history[currentMode] || [];
}

export function setModeChatMessages(currentMode: string, messages: AIMessage[]) {
  aiChatHistory.update(h => {
    h[currentMode] = messages;
    return { ...h };
  });
}

export function clearModeChatMessages(currentMode: string) {
  aiChatHistory.update(h => {
    delete h[currentMode];
    return { ...h };
  });
}

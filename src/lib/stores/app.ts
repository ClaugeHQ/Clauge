import { writable } from 'svelte/store';
import type { AIMessage } from '$lib/types/ai';

export type AppMode = 'agent' | 'rest' | 'sql' | 'nosql' | 'ssh' | 'history';

const MODE_STORAGE_KEY = 'clauge_last_mode';
const VALID_MODES: AppMode[] = ['agent', 'rest', 'sql', 'nosql', 'ssh'];

function loadInitialMode(): AppMode {
  try {
    const saved = localStorage.getItem(MODE_STORAGE_KEY);
    if (saved && (VALID_MODES as string[]).includes(saved)) return saved as AppMode;
  } catch { /* ignore */ }
  return 'agent';
}

export const mode = writable<AppMode>(loadInitialMode());

// Persist mode changes (skip 'history' — it's a transient view, not a primary mode)
mode.subscribe(v => {
  if ((VALID_MODES as string[]).includes(v)) {
    try { localStorage.setItem(MODE_STORAGE_KEY, v); } catch { /* ignore */ }
  }
});
export const navOpen = writable<boolean>(true);
export const aiPanelOpen = writable<boolean>(false);
export const aiPanelOpenPerMode = writable<Record<string, boolean>>({});
export const activeModal = writable<string | null>(null);

// Per-mode AI chat history — persisted to localStorage
const CHAT_STORAGE_KEY = 'clauge_ai_chat_history';

function loadChatHistory(): Record<string, AIMessage[]> {
  try {
    const saved = localStorage.getItem(CHAT_STORAGE_KEY);
    return saved ? JSON.parse(saved) : {};
  } catch { return {}; }
}

function saveChatHistory(history: Record<string, AIMessage[]>) {
  try {
    localStorage.setItem(CHAT_STORAGE_KEY, JSON.stringify(history));
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

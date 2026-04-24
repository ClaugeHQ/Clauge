import { writable } from 'svelte/store';
import type { AIMessage } from '$lib/types/ai';

export type AppMode = 'agent' | 'rest' | 'sql' | 'nosql' | 'history';

export const mode = writable<AppMode>('rest');
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

import { writable, derived, get } from 'svelte/store';
import type { AIMessage } from '$lib/types/ai';
import { STORAGE_KEYS } from '$lib/shared/constants/storage';
import { focusedTileMode } from '$lib/modes/canvas/stores/canvasStore';

export type AppMode = 'agent' | 'canvas' | 'rest' | 'sql' | 'nosql' | 'ssh' | 'explorer' | 'workspace' | 'history';

const VALID_MODES: AppMode[] = ['agent', 'canvas', 'rest', 'sql', 'nosql', 'ssh', 'explorer', 'workspace'];

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
/**
 * The mode that hotkeys, AI panel context, and topbar highlight should follow.
 * On Atlas it follows the focused tile's source mode so a Cmd+L on a focused
 * SQL tile opens the SQL AI panel, not a canvas-flavoured one. Falls back to
 * the actual `$mode` everywhere else.
 */
export const effectiveMode = derived(
  [mode, focusedTileMode],
  ([$m, $tile]) => ($m === 'canvas' && $tile ? $tile : $m),
);

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

/** Clear AI chat history across every mode. Used by Settings → General →
 *  Chat History → Clear, alongside the REST history table. */
export function clearAllChatMessages() {
  aiChatHistory.set({});
}

/** Total AI chat messages (across all modes). */
export function countAllChatMessages(): number {
  const h = loadChatHistory();
  let n = 0;
  for (const k in h) n += h[k]?.length ?? 0;
  return n;
}

/**
 * Switch the active mode safely. When leaving Canvas, flushes pending
 * viewport and tile-geometry writes BEFORE the Canvas panel unmounts so
 * the last drag/zoom is persisted.
 *
 * Callers should use `await setMode(...)` in async contexts. In sync
 * handlers (e.g. button click in Sidebar), use `void setMode(...)`.
 */
export async function setMode(next: AppMode): Promise<void> {
  const prev = get(mode);
  if (prev === next) return;
  if (prev === 'canvas') {
    try {
      const { flushViewportNow, flushDirtyTilesNow, flushDirtyRegionsNow } = await import('$lib/modes/canvas/stores/canvasStore');
      await flushViewportNow();
      await flushDirtyTilesNow();
      await flushDirtyRegionsNow();
    } catch {
      // Network/IPC errors from the flush should not block mode switch.
    }
  }
  mode.set(next);
}

/** Approx. byte size of the AI chat localStorage payload (JSON length). */
export function chatHistorySizeBytes(): number {
  try {
    const raw = localStorage.getItem(STORAGE_KEYS.AI_CHAT_HISTORY);
    return raw ? new Blob([raw]).size : 0;
  } catch { return 0; }
}

/** Drop AI chat messages older than `retentionMs` from now (each mode
 *  filtered independently). Used on app/settings load. */
export function purgeOldChatMessages(retentionMs: number) {
  if (!Number.isFinite(retentionMs) || retentionMs <= 0) return;
  const cutoff = Date.now() - retentionMs;
  aiChatHistory.update(h => {
    const next: Record<string, AIMessage[]> = {};
    for (const k of Object.keys(h)) {
      const filtered = (h[k] ?? []).filter(m => (m.timestamp ?? 0) >= cutoff);
      if (filtered.length > 0) next[k] = filtered;
    }
    return next;
  });
}

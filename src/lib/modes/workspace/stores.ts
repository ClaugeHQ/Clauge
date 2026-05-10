// Workspace mode runtime state — list, active selection, and thin
// helpers around the invoke wrappers in `commands.ts` so components
// don't have to thread the actor argument every time.

import { writable, derived, get } from 'svelte/store';
import type {
  Workspace,
  WorkspaceBoard,
  WorkspaceBoardCard,
  WorkspaceBoardColumn,
  WorkspaceNote,
} from './types';
import type { WorkspaceCoworker } from './types';
import * as cmd from './commands';
import { currentUserActor } from './attribution';

// ── List + active selection ───────────────────────────────────────────

export const workspaces = writable<Workspace[]>([]);
export const activeWorkspaceId = writable<string | null>(null);

/** All coworker rows — loaded on app boot, refreshed on CRUD. */
export const coworkers = writable<WorkspaceCoworker[]>([]);

export async function loadCoworkers() {
  try {
    coworkers.set(await cmd.workspaceCoworkerList());
  } catch (e) { console.warn('Failed to load coworkers:', e); }
}

/** MCP server status — kept in a writable so the footer indicators
 *  (WorkspaceNav, AgentNav) can subscribe and re-render whenever the
 *  user toggles from Settings. Refreshed on app start + after toggle. */
export const mcpStatus = writable<{ running: boolean; port: number | null }>({
  running: false,
  port: null,
});

/** In-flight @-mention map: cardId → provider slug ("claude", …).
 *  BoardView reads this to render the per-card spinner+icon while the
 *  agent CLI is running. Multiple cards can be in-flight simultaneously
 *  (different sessions, no shared state) so a Map fits better than a
 *  single nullable. Components mutate via the helpers below; never
 *  reach into the Map directly so add/remove stays balanced. */
/** Per-card in-flight info. Drives both the kanban-tile pulsing
 *  chip and (when the drawer is reopened mid-flight) the thinking
 *  bubble inside the thread. Promoting this to a global store
 *  means closing + reopening the drawer doesn't lose the indicator. */
export interface InflightMention {
  /** Provider slug for the kanban-tile icon ('claude' / etc). */
  provider: string;
  /** Coworker driving the chat — used to render the thinking
   *  bubble's avatar + name in the drawer when reopened. */
  coworkerId: string;
  coworkerName: string;
  /** Wall-clock start time for the thinking-bubble copy escalation
   *  ("is thinking" → "still working"). */
  startedAt: string;
}

export const inflightMentions = writable<Map<string, InflightMention>>(new Map());

export function markMentionStart(cardId: string, info: InflightMention) {
  inflightMentions.update((m) => {
    const next = new Map(m);
    next.set(cardId, info);
    return next;
  });
}

export function markMentionEnd(cardId: string) {
  inflightMentions.update((m) => {
    if (!m.has(cardId)) return m;
    const next = new Map(m);
    next.delete(cardId);
    return next;
  });
}

export async function loadMcpStatus() {
  try {
    const s = await cmd.workspaceMcpStatus();
    mcpStatus.set(s);
  } catch { /* ignore */ }
}

// ── Inbox unread tracking ────────────────────────────────────────────
// "Unread" = items whose updated_at is newer than the timestamp the
// user last saw the inbox. Persisted in localStorage so the count
// survives app restarts. Marking read just bumps the timestamp to now.

const INBOX_LAST_READ_KEY = 'clauge.workspace.inbox.lastReadAt';

export const inboxLastReadAt = writable<number>(loadInboxLastReadAt());
export const inboxUnreadCount = writable<number>(0);

function loadInboxLastReadAt(): number {
  try {
    const raw = localStorage.getItem(INBOX_LAST_READ_KEY);
    const n = raw ? parseInt(raw, 10) : 0;
    return Number.isFinite(n) ? n : 0;
  } catch { return 0; }
}

export function markInboxRead() {
  const now = Date.now();
  inboxLastReadAt.set(now);
  inboxUnreadCount.set(0);
  try { localStorage.setItem(INBOX_LAST_READ_KEY, String(now)); } catch { /* ignore */ }
}

// ── Per-card unread tracking ─────────────────────────────────────────
// Cards mutated by an agent are "unread" until the user opens their
// drawer. Persisted as `{ cardId → updatedAt-when-last-seen }`.
// Comparing card.updatedAt > cardLastSeen[id] yields the unread state.
// Stored in one localStorage blob to keep writes cheap (a Map gets
// flattened to JSON; size cap by trimming the oldest 200 entries).

const CARD_LAST_SEEN_KEY = 'clauge.workspace.card.lastSeenAt';
const CARD_LAST_SEEN_MAX = 500;

export const cardLastSeenAt = writable<Record<string, string>>(loadCardLastSeenAt());

function loadCardLastSeenAt(): Record<string, string> {
  try {
    const raw = localStorage.getItem(CARD_LAST_SEEN_KEY);
    if (!raw) return {};
    const obj = JSON.parse(raw);
    return obj && typeof obj === 'object' ? (obj as Record<string, string>) : {};
  } catch { return {}; }
}

function persistCardLastSeen(map: Record<string, string>) {
  // Trim to the most-recently-seen N if we've grown past the cap.
  const keys = Object.keys(map);
  if (keys.length > CARD_LAST_SEEN_MAX) {
    const trimmed: [string, string][] = keys
      .map((k) => [k, map[k]] as [string, string])
      .sort((a, b) => (a[1] < b[1] ? 1 : -1))
      .slice(0, CARD_LAST_SEEN_MAX);
    map = Object.fromEntries(trimmed);
  }
  try { localStorage.setItem(CARD_LAST_SEEN_KEY, JSON.stringify(map)); } catch { /* ignore */ }
}

/** Mark a card as seen at the given updated_at timestamp. Use the
 *  card's own updatedAt rather than `now` so we measure against the
 *  exact mutation the user just consumed. */
export function markCardSeen(cardId: string, updatedAt: string) {
  cardLastSeenAt.update((m) => {
    if (m[cardId] === updatedAt) return m;
    const next = { ...m, [cardId]: updatedAt };
    persistCardLastSeen(next);
    return next;
  });
}

/** True if the card was last mutated by an agent AND that mutation is
 *  newer than the user's recorded "seen" timestamp. New cards default
 *  to "seen at created_at" so freshly-imported issues don't all start
 *  with red dots. */
export function isCardUnread(card: {
  id: string;
  updatedAt: string;
  updatedBy: string;
  createdAt: string;
}, lastSeen: Record<string, string>): boolean {
  if (!card.updatedBy || card.updatedBy === 'user' || card.updatedBy.startsWith('user:')) {
    return false;
  }
  const seen = lastSeen[card.id] ?? card.createdAt;
  return card.updatedAt > seen;
}

/** Recompute the unread count by fetching the inbox and counting
 *  items that haven't been seen yet. An item is considered read if
 *  either: (a) the inbox was opened after this update, OR (b) for
 *  card items, the card drawer was opened at or after this update
 *  (reuses the per-card cardLastSeenAt map, which markCardSeen()
 *  populates whenever a user opens a card drawer). Notes don't have
 *  per-item tracking so they only clear via (a). */
export async function refreshInboxUnread() {
  try {
    const items = await cmd.workspaceInboxList(50);
    const since = get(inboxLastReadAt);
    const seenMap = get(cardLastSeenAt);
    const count = items.filter(it => {
      const t = new Date(it.updatedAt).getTime();
      if (!Number.isFinite(t) || t <= since) return false;
      if (it.kind === 'card') {
        const seen = seenMap[it.id];
        if (seen && seen >= it.updatedAt) return false;
      }
      return true;
    }).length;
    inboxUnreadCount.set(count);
  } catch { /* ignore */ }
}

export const activeWorkspace = derived(
  [workspaces, activeWorkspaceId],
  ([$ws, $id]) => $ws.find(w => w.id === $id) ?? null,
);

export async function loadWorkspaces() {
  try {
    const list = await cmd.workspaceList();
    workspaces.set(list);
    // If the active id was deleted elsewhere (or this is first load), pick
    // the most-recent one — keeps the UI from showing an empty pane.
    const cur = get(activeWorkspaceId);
    if (cur && !list.some(w => w.id === cur)) {
      activeWorkspaceId.set(list[0]?.id ?? null);
    } else if (!cur && list.length > 0) {
      activeWorkspaceId.set(list[0].id);
    }
  } catch (e) {
    console.error('Failed to load workspaces:', e);
  }
}

export async function createWorkspace(params: {
  name: string;
  projectPath?: string | null;
  color?: string | null;
}): Promise<Workspace> {
  const ws = await cmd.workspaceCreate({
    name: params.name,
    projectPath: params.projectPath ?? null,
    color: params.color ?? null,
    actor: currentUserActor(),
  });
  // Refresh list + activate the new workspace immediately.
  await loadWorkspaces();
  activeWorkspaceId.set(ws.id);
  return ws;
}

export async function updateWorkspace(params: {
  id: string;
  name: string;
  projectPath?: string | null;
  color?: string | null;
}) {
  await cmd.workspaceUpdate({
    ...params,
    projectPath: params.projectPath ?? null,
    color: params.color ?? null,
    actor: currentUserActor(),
  });
  await loadWorkspaces();
}

export async function deleteWorkspace(id: string, deleteWorktrees: boolean = true) {
  await cmd.workspaceDelete(id, deleteWorktrees);
  // Clear active before the list refresh so the UI doesn't briefly
  // render a workspace that no longer exists.
  if (get(activeWorkspaceId) === id) {
    activeWorkspaceId.set(null);
  }
  await loadWorkspaces();
}

// ── Notes (per-workspace caches) ──────────────────────────────────────
// Lazy-loaded as the user opens a workspace. Map<workspaceId, Note[]>.

export const notesByWorkspace = writable<Map<string, WorkspaceNote[]>>(new Map());

export async function loadNotes(workspaceId: string) {
  try {
    const list = await cmd.workspaceNoteList(workspaceId);
    notesByWorkspace.update(m => {
      const next = new Map(m);
      next.set(workspaceId, list);
      return next;
    });
  } catch (e) {
    console.error('Failed to load notes:', e);
  }
}

export async function createNote(
  workspaceId: string,
  title: string,
  linkedSessionId: string | null = null,
): Promise<WorkspaceNote> {
  const note = await cmd.workspaceNoteCreate({
    workspaceId,
    title,
    content: '',
    tags: [],
    linkedSessionId,
    actor: currentUserActor(),
  });
  await loadNotes(workspaceId);
  return note;
}

export async function saveNote(note: WorkspaceNote, content: string) {
  // `tags` is JSON in the wire format; tolerate already-parsed callers.
  let tags: string[];
  try {
    tags = Array.isArray(note.tags) ? (note.tags as unknown as string[]) : JSON.parse(note.tags);
  } catch { tags = []; }
  await cmd.workspaceNoteUpdate({
    id: note.id,
    title: note.title,
    content,
    tags,
    linkedSessionId: note.linkedSessionId,
    actor: currentUserActor(),
  });
  await loadNotes(note.workspaceId);
}

export async function deleteNote(note: WorkspaceNote) {
  await cmd.workspaceNoteDelete(note.id);
  await loadNotes(note.workspaceId);
}

// ── Boards (per-workspace caches) ─────────────────────────────────────

export const boardsByWorkspace = writable<Map<string, WorkspaceBoard[]>>(new Map());
export const columnsByBoard = writable<Map<string, WorkspaceBoardColumn[]>>(new Map());
export const cardsByBoard = writable<Map<string, WorkspaceBoardCard[]>>(new Map());

export async function loadBoards(workspaceId: string) {
  try {
    const list = await cmd.workspaceBoardList(workspaceId);
    boardsByWorkspace.update(m => {
      const next = new Map(m);
      next.set(workspaceId, list);
      return next;
    });
  } catch (e) {
    console.error('Failed to load boards:', e);
  }
}

export async function loadBoardContents(boardId: string) {
  try {
    const [cols, cards] = await Promise.all([
      cmd.workspaceColumnList(boardId),
      cmd.workspaceCardList(boardId),
    ]);
    columnsByBoard.update(m => {
      const next = new Map(m);
      next.set(boardId, cols);
      return next;
    });
    cardsByBoard.update(m => {
      const next = new Map(m);
      next.set(boardId, cards);
      return next;
    });
  } catch (e) {
    console.error('Failed to load board contents:', e);
  }
}

export async function createBoard(workspaceId: string, name: string): Promise<WorkspaceBoard> {
  const board = await cmd.workspaceBoardCreate(workspaceId, name);
  await loadBoards(workspaceId);
  return board;
}

export async function deleteBoard(boardId: string, workspaceId: string) {
  await cmd.workspaceBoardDelete(boardId);
  await loadBoards(workspaceId);
  // Drop cached columns/cards for this board so a future re-create gets
  // a clean slot.
  columnsByBoard.update(m => {
    const next = new Map(m);
    next.delete(boardId);
    return next;
  });
  cardsByBoard.update(m => {
    const next = new Map(m);
    next.delete(boardId);
    return next;
  });
}

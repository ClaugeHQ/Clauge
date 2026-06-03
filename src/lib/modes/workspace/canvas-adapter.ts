import type { CanvasTabAdapter } from '$lib/modes/canvas/adapter';
import { mount, unmount } from 'svelte';
import { get } from 'svelte/store';
import { tabs, activateTab } from '$lib/shared/stores/tabs';
import { setMode } from '$lib/stores/app';
import { activeWorkspaceId, notesByWorkspace, boardsByWorkspace } from './stores';
import {
  attachNoteEditor,
  detachNoteEditor,
  isNoteEditorDirty,
} from './services/noteEditorReparent';
import BoardTileBody from './components/BoardTileBody.svelte';
import type { WorkspaceNote, WorkspaceBoard } from './types';

function noteIdFromTabKey(tabId: string): string | null {
  if (!tabId.startsWith('note:')) return null;
  return tabId.slice('note:'.length) || null;
}

function findNote(noteId: string): { note: WorkspaceNote; workspaceId: string } | null {
  for (const [workspaceId, list] of get(notesByWorkspace)) {
    const note = list.find((n) => n.id === noteId);
    if (note) return { note, workspaceId };
  }
  return null;
}

export const workspaceNoteAdapter: CanvasTabAdapter = {
  tabKind: 'workspace_note',
  mountStrategy: 'reparent',
  defaultSpawnSize: { width: 600, height: 500 },

  listOpenTabs(_workspaceId) {
    return get(tabs)
      .filter((t) => t.mode === 'workspace' && typeof t.key === 'string' && t.key.startsWith('note:'))
      .map((t) => ({ id: t.key as string, title: t.label }));
  },

  subscribe(_workspaceId, onChange) {
    const unsubTabs = tabs.subscribe(() => onChange());
    const unsubNotes = notesByWorkspace.subscribe(() => onChange());
    return () => {
      unsubTabs();
      unsubNotes();
    };
  },

  attach(tabId, slot) {
    const noteId = noteIdFromTabKey(tabId);
    if (!noteId) return;
    void attachNoteEditor(noteId, slot);
  },

  detach(tabId, slot) {
    const noteId = noteIdFromTabKey(tabId);
    if (!noteId) return;
    detachNoteEditor(noteId, slot);
  },

  getMeta(tabId) {
    const noteId = noteIdFromTabKey(tabId);
    const match = noteId ? findNote(noteId) : null;
    const topbarLabel = get(tabs).find((t) => t.mode === 'workspace' && t.key === tabId)?.label;
    const title = match?.note.title || topbarLabel || 'Untitled';
    return {
      title,
      dirty: noteId ? isNoteEditorDirty(noteId) : false,
    };
  },

  openInHomeMode(tabId) {
    const allTabs = get(tabs);
    const topbarTab = allTabs.find((t) => t.mode === 'workspace' && t.key === tabId);
    if (topbarTab) {
      const noteId = noteIdFromTabKey(tabId);
      if (noteId) {
        const match = findNote(noteId);
        if (match) activeWorkspaceId.set(match.workspaceId);
      }
      activateTab(topbarTab.id);
    }
    void setMode('workspace');
  },

  closeTab(tabId) {
    const allTabs = get(tabs);
    const topbarTab = allTabs.find((t) => t.mode === 'workspace' && t.key === tabId);
    if (!topbarTab) return;
    window.dispatchEvent(
      new CustomEvent('canvas:request-tab-close', { detail: { tabId: topbarTab.id } }),
    );
  },
};

function boardIdFromTabKey(tabId: string): string | null {
  if (!tabId.startsWith('board:')) return null;
  return tabId.slice('board:'.length) || null;
}

function findBoard(boardId: string): { board: WorkspaceBoard; workspaceId: string } | null {
  for (const [workspaceId, list] of get(boardsByWorkspace)) {
    const board = list.find((b) => b.id === boardId);
    if (board) return { board, workspaceId };
  }
  return null;
}

export const workspaceBoardAdapter: CanvasTabAdapter = {
  tabKind: 'workspace_board',
  mountStrategy: 'remount',
  defaultSpawnSize: { width: 950, height: 700 },

  listOpenTabs(_workspaceId) {
    return get(tabs)
      .filter((t) => t.mode === 'workspace' && typeof t.key === 'string' && t.key.startsWith('board:'))
      .map((t) => ({ id: t.key as string, title: t.label }));
  },

  subscribe(_workspaceId, onChange) {
    const unsubTabs = tabs.subscribe(() => onChange());
    const unsubBoards = boardsByWorkspace.subscribe(() => onChange());
    return () => {
      unsubTabs();
      unsubBoards();
    };
  },

  render(tabId, slot) {
    const boardId = boardIdFromTabKey(tabId);
    if (!boardId) return { destroy: () => {} };
    const component = mount(BoardTileBody, {
      target: slot,
      props: { boardId },
    });
    return {
      destroy: () => {
        void unmount(component);
      },
    };
  },

  getMeta(tabId) {
    const boardId = boardIdFromTabKey(tabId);
    const match = boardId ? findBoard(boardId) : null;
    const topbarLabel = get(tabs).find((t) => t.mode === 'workspace' && t.key === tabId)?.label;
    return { title: match?.board.name || topbarLabel || 'Board' };
  },

  openInHomeMode(tabId) {
    const allTabs = get(tabs);
    const topbarTab = allTabs.find((t) => t.mode === 'workspace' && t.key === tabId);
    if (topbarTab) {
      const boardId = boardIdFromTabKey(tabId);
      if (boardId) {
        const match = findBoard(boardId);
        if (match) activeWorkspaceId.set(match.workspaceId);
      }
      activateTab(topbarTab.id);
    }
    void setMode('workspace');
  },

  closeTab(tabId) {
    const allTabs = get(tabs);
    const topbarTab = allTabs.find((t) => t.mode === 'workspace' && t.key === tabId);
    if (!topbarTab) return;
    window.dispatchEvent(
      new CustomEvent('canvas:request-tab-close', { detail: { tabId: topbarTab.id } }),
    );
  },
};

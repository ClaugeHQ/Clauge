import { get } from 'svelte/store';
import { Crepe } from '@milkdown/crepe';
import { editorViewCtx } from '@milkdown/kit/core';
import { workspaceNoteGet, workspaceNoteUpdate } from '../commands';
import { notesByWorkspace, loadNotes } from '../stores';
import { currentUserActor } from '../attribution';
import { tabs as sharedTabs, updateTab } from '$lib/shared/stores/tabs';
import type { WorkspaceNote } from '../types';

interface NoteEditorEntry {
  crepe: Crepe;
  /** The wrapper div we reparent. Crepe's editor lives inside this. */
  container: HTMLDivElement;
  /** Live markdown — kept current via Crepe's `markdownUpdated`. */
  currentContent: string;
  /** Last persisted markdown — guards against phantom autosaves. */
  baseline: string | null;
  /** Pending debounced flush. */
  saveTimer: ReturnType<typeof setTimeout> | null;
  /** In-flight save guard. */
  saving: boolean;
  /** External subscribers (NoteView) wanting markdownUpdated callbacks. */
  listeners: Set<(markdown: string) => void>;
  /** True when a markdown change has been accepted but not yet persisted. */
  dirty: boolean;
  /** Pending base64 image uploads, keyed for cancel-on-destroy bookkeeping. */
  uploadPromises: Set<Promise<string>>;
}

const registry = new Map<string, NoteEditorEntry>();
/** Notes that have a Crepe instance but no DOM consumer right now. */
const detachedNoteIds = new Set<string>();

const SAVE_DEBOUNCE_MS = 600;

function fileToDataUrl(noteId: string, file: File): Promise<string> {
  const promise = new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error ?? new Error('image read failed'));
    reader.readAsDataURL(file);
  });
  const entry = registry.get(noteId);
  if (entry) {
    entry.uploadPromises.add(promise);
    promise.finally(() => entry.uploadPromises.delete(promise));
  }
  return promise;
}

async function flushSave(noteId: string): Promise<void> {
  const entry = registry.get(noteId);
  if (!entry || entry.saving || !entry.dirty) return;
  entry.saving = true;
  try {
    const note = await workspaceNoteGet(noteId);
    let tags: string[];
    try {
      tags = JSON.parse(note.tags);
    } catch {
      tags = [];
    }
    await workspaceNoteUpdate({
      id: noteId,
      title: note.title,
      content: entry.currentContent,
      tags,
      linkedSessionId: note.linkedSessionId,
      actor: currentUserActor(),
    });
    entry.baseline = entry.currentContent;
    entry.dirty = false;

    const cached = get(notesByWorkspace).get(note.workspaceId) ?? [];
    if (cached.some((n) => n.id === noteId)) {
      void loadNotes(note.workspaceId);
    }

    const myTab = get(sharedTabs).find(
      (t) => t.mode === 'workspace' && t.key === `note:${noteId}`,
    );
    if (myTab && note.title && myTab.label !== note.title) {
      updateTab(myTab.id, { label: note.title });
    }
  } catch (err) {
    console.warn('[noteEditor] autosave failed:', err);
  } finally {
    entry.saving = false;
  }
}

function scheduleSave(noteId: string): void {
  const entry = registry.get(noteId);
  if (!entry) return;
  if (entry.saveTimer) clearTimeout(entry.saveTimer);
  entry.saveTimer = setTimeout(() => {
    entry.saveTimer = null;
    void flushSave(noteId);
  }, SAVE_DEBOUNCE_MS);
}

async function createEntry(noteId: string): Promise<NoteEditorEntry> {
  const note: WorkspaceNote = await workspaceNoteGet(noteId);

  const container = document.createElement('div');
  container.className = 'md-host';

  const crepe = new Crepe({
    root: container,
    defaultValue: note.content ?? '',
    featureConfigs: {
      [Crepe.Feature.Placeholder]: { text: "Type '/' for commands", mode: 'block' },
      [Crepe.Feature.ImageBlock]: {
        onUpload: (file: File) => fileToDataUrl(noteId, file),
        blockOnUpload: (file: File) => fileToDataUrl(noteId, file),
        inlineOnUpload: (file: File) => fileToDataUrl(noteId, file),
      },
    },
  });

  const entry: NoteEditorEntry = {
    crepe,
    container,
    currentContent: note.content ?? '',
    baseline: null,
    saveTimer: null,
    saving: false,
    listeners: new Set(),
    dirty: false,
    uploadPromises: new Set(),
  };
  registry.set(noteId, entry);

  await crepe.create();
  crepe.on((listener) => {
    listener.markdownUpdated((_ctx, markdown) => {
      const live = registry.get(noteId);
      if (!live) return;
      if (live.baseline === null) {
        live.baseline = markdown;
        live.currentContent = markdown;
        for (const cb of live.listeners) cb(markdown);
        return;
      }
      if (markdown === live.baseline) return;
      live.currentContent = markdown;
      live.dirty = true;
      for (const cb of live.listeners) cb(markdown);
      // When a Svelte host (NoteView) is subscribed it owns saves —
      // it edits title/tags/linkedSession alongside content, so the
      // registry must not race those writes. Fall back to internal
      // autosave only when no host is around (canvas-only mode).
      if (live.listeners.size === 0) {
        scheduleSave(noteId);
      }
    });
  });

  return entry;
}

/**
 * Mount the editor into `slot`. Lazy-creates the Crepe instance on first
 * call; subsequent calls move the same DOM via `appendChild`, preserving
 * cursor, undo stack, and inline formatting state.
 */
export async function attachNoteEditor(noteId: string, slot: HTMLElement): Promise<void> {
  let entry = registry.get(noteId);
  if (!entry) {
    entry = await createEntry(noteId);
  }
  slot.appendChild(entry.container);
  detachedNoteIds.delete(noteId);

  requestAnimationFrame(() => {
    try {
      entry?.crepe.editor.action((ctx) => {
        ctx.get(editorViewCtx).focus();
      });
    } catch (err) {
      console.warn('[noteEditor] focus restore failed:', err);
    }
  });
}

/**
 * Remove the editor's DOM from `slot` without destroying state. Flushes
 * any pending debounced save first so autosave fires even when no Svelte
 * component is around to call `saveNow`.
 */
export function detachNoteEditor(noteId: string, slot: HTMLElement): void {
  const entry = registry.get(noteId);
  if (!entry) return;

  if (entry.saveTimer) {
    clearTimeout(entry.saveTimer);
    entry.saveTimer = null;
  }
  if (entry.dirty) {
    void flushSave(noteId);
  }

  try {
    entry.crepe.editor.action((ctx) => {
      ctx.get(editorViewCtx).dom.blur();
    });
  } catch {
    // Editor may already be torn down.
  }

  if (entry.container.parentElement === slot) {
    slot.removeChild(entry.container);
  }
  detachedNoteIds.add(noteId);
}

/**
 * Permanently destroy the editor — used when the note is closed everywhere
 * (last tab closed, note deleted). Flushes one final save synchronously
 * before disposing Crepe.
 */
export async function destroyNoteEditor(noteId: string): Promise<void> {
  const entry = registry.get(noteId);
  if (!entry) return;

  if (entry.saveTimer) {
    clearTimeout(entry.saveTimer);
    entry.saveTimer = null;
  }
  if (entry.dirty) {
    await flushSave(noteId);
  }

  if (entry.container.parentElement) {
    entry.container.parentElement.removeChild(entry.container);
  }
  entry.listeners.clear();
  entry.uploadPromises.clear();

  try {
    entry.crepe.destroy();
  } catch (err) {
    console.warn('[noteEditor] destroy failed:', err);
  }
  registry.delete(noteId);
  detachedNoteIds.delete(noteId);
}

/** Subscribe to markdown changes for a note. Returns unsubscribe. */
export function subscribeNoteMarkdown(
  noteId: string,
  cb: (markdown: string) => void,
): () => void {
  const entry = registry.get(noteId);
  if (entry) entry.listeners.add(cb);
  return () => {
    const live = registry.get(noteId);
    live?.listeners.delete(cb);
  };
}

/** Force a save flush — used by NoteView before its component unmounts. */
export async function saveNoteEditorNow(noteId: string): Promise<void> {
  await flushSave(noteId);
}

/** True if the entry has unsaved changes. */
export function isNoteEditorDirty(noteId: string): boolean {
  return registry.get(noteId)?.dirty ?? false;
}

/** True if a Crepe instance exists for this note (mounted or detached). */
export function hasNoteEditor(noteId: string): boolean {
  return registry.has(noteId);
}

/** Note ids currently registered (any state: attached or detached). */
export function listOpenNoteEditors(): string[] {
  return [...registry.keys()];
}

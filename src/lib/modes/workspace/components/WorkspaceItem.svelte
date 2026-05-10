<script lang="ts">
  // One workspace row in the side panel — mirrors CollectionItem from
  // RestNav for a uniform look across modes. Header has:
  //   drag-handle · folder-icon · name (or InlineInput when renaming)
  //   · sub ("N notes · M boards") · (+) add · (⋯) menu · chevron
  // Body expands to render boards then notes, each as a mini-row.

  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import {
    activeWorkspaceId,
    notesByWorkspace,
    boardsByWorkspace,
    loadNotes,
    loadBoards,
    deleteWorkspace,
    deleteNote,
    deleteBoard,
    updateWorkspace,
  } from '../stores';
  import {
    workspaceNoteCreate,
    workspaceBoardCreate,
    workspaceDeletePreview,
    type WorkspaceDeletePreviewResult,
  } from '../commands';
  import { currentUserActor } from '../attribution';
  import type { Workspace, WorkspaceNote, WorkspaceBoard } from '../types';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import { showToast } from '$lib/shared/primitives/toast';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import InlineInput from '$lib/components/nav/InlineInput.svelte';
  import { tabs as sharedTabs, addTab, activateTab, updateTab, closeTab } from '$lib/shared/stores/tabs';
  import { mode } from '$lib/stores/app';
  import { WORKSPACE_EVENT } from '$lib/shared/constants/events';

  interface Props {
    workspace: Workspace;
    searchQuery?: string;
  }

  let { workspace, searchQuery = '' }: Props = $props();

  let expanded = $state(false);
  let loaded = $state(false);
  let addingKind = $state<'note' | 'board' | null>(null);
  let renaming = $state(false);
  let showDeleteConfirm = $state(false);
  let deletePreview = $state<WorkspaceDeletePreviewResult | null>(null);
  let deleteWorktrees = $state(true);

  const isActive = $derived($activeWorkspaceId === workspace.id);
  const notes = $derived($notesByWorkspace.get(workspace.id) ?? []);
  const boards = $derived($boardsByWorkspace.get(workspace.id) ?? []);

  const filteredNotes = $derived(
    searchQuery
      ? notes.filter(n => n.title.toLowerCase().includes(searchQuery.toLowerCase()))
      : notes,
  );
  const filteredBoards = $derived(
    searchQuery
      ? boards.filter(b => b.name.toLowerCase().includes(searchQuery.toLowerCase()))
      : boards,
  );

  async function loadChildren() {
    if (loaded) return;
    await Promise.all([loadNotes(workspace.id), loadBoards(workspace.id)]);
    loaded = true;
  }

  async function toggle() {
    expanded = !expanded;
    if (expanded) await loadChildren();
  }

  function openNote(n: WorkspaceNote) {
    activeWorkspaceId.set(workspace.id);
    const key = `note:${n.id}`;
    const existing = get(sharedTabs).find(t => t.mode === 'workspace' && t.key === key);
    if (existing) activateTab(existing.id);
    else addTab(n.title || 'Untitled', 'workspace', key, 'var(--acc)');
    mode.set('workspace');
  }

  function openBoard(b: WorkspaceBoard) {
    activeWorkspaceId.set(workspace.id);
    const key = `board:${b.id}`;
    const existing = get(sharedTabs).find(t => t.mode === 'workspace' && t.key === key);
    if (existing) activateTab(existing.id);
    else addTab(b.name, 'workspace', key, '#a78bfa');
    mode.set('workspace');
  }

  function handleAddBtn(e: MouseEvent) {
    e.stopPropagation();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    showContextMenu(rect.left, rect.bottom + 4, [
      {
        label: 'New Note',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>',
        action: () => beginAdd('note'),
      },
      {
        label: 'New Board',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="6" height="16" rx="1"/><rect x="11" y="4" width="6" height="10" rx="1"/><rect x="19" y="4" width="2" height="14" rx="1"/></svg>',
        action: () => beginAdd('board'),
      },
    ]);
  }

  async function beginAdd(kind: 'note' | 'board') {
    if (!expanded) {
      expanded = true;
      await loadChildren();
    }
    addingKind = kind;
  }

  async function commitAdd(name: string) {
    const kind = addingKind;
    addingKind = null;
    const trimmed = name.trim();
    if (!trimmed || !kind) return;
    try {
      if (kind === 'note') {
        const n = await workspaceNoteCreate({
          workspaceId: workspace.id,
          title: trimmed,
          content: '',
          tags: [],
          actor: currentUserActor(),
        });
        await loadNotes(workspace.id);
        openNote(n);
      } else {
        const b = await workspaceBoardCreate(workspace.id, trimmed);
        await loadBoards(workspace.id);
        openBoard(b);
      }
    } catch (e) {
      showToast(`Failed to create ${kind}: ${e}`, 'error');
    }
  }

  function cancelAdd() {
    addingKind = null;
  }

  function handleMenuBtn(e: MouseEvent) {
    e.stopPropagation();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    showContextMenu(rect.left, rect.bottom + 4, buildMenuItems());
  }

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    showContextMenu(e.clientX, e.clientY, buildMenuItems());
  }

  function buildMenuItems() {
    return [
      {
        label: 'Rename',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
        action: () => { renaming = true; },
      },
      {
        label: 'New Note',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>',
        action: () => beginAdd('note'),
      },
      {
        label: 'New Board',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="6" height="16" rx="1"/><rect x="11" y="4" width="6" height="10" rx="1"/><rect x="19" y="4" width="2" height="14" rx="1"/></svg>',
        action: () => beginAdd('board'),
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Change project',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/><path d="M9 13l2 2 4-4"/></svg>',
        action: () => window.dispatchEvent(new CustomEvent(WORKSPACE_EVENT.EDIT_WORKSPACE, { detail: { workspace } })),
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Delete Workspace',
        danger: true,
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/></svg>',
        action: openDeleteConfirm,
      },
    ];
  }

  async function handleRename(newName: string) {
    renaming = false;
    const trimmed = newName.trim();
    if (!trimmed || trimmed === workspace.name) return;
    try {
      await updateWorkspace({
        id: workspace.id,
        name: trimmed,
        projectPath: workspace.projectPath,
        color: workspace.color,
      });
    } catch (e) {
      showToast(`Rename failed: ${e}`, 'error');
    }
  }

  async function openDeleteConfirm() {
    // Reset the checkbox to default-on every time the dialog opens —
    // a previous "off" toggle on a different workspace shouldn't carry
    // over to this one.
    deleteWorktrees = true;
    deletePreview = null;
    showDeleteConfirm = true;
    try {
      deletePreview = await workspaceDeletePreview(workspace.id);
    } catch (e) {
      // Preview failure is non-fatal — the dialog falls back to the
      // generic message and still allows confirm.
      console.warn('workspace delete preview failed', e);
    }
  }

  async function confirmDelete() {
    try {
      // Close any open tabs for items in this workspace.
      const myKeys = new Set([
        ...notes.map(n => `note:${n.id}`),
        ...boards.map(b => `board:${b.id}`),
      ]);
      const tabsToClose = get(sharedTabs).filter(t => t.mode === 'workspace' && t.key && myKeys.has(t.key));
      tabsToClose.forEach(t => closeTab(t.id));
      const hasWorktrees = (deletePreview?.activeWorktrees.length ?? 0) > 0;
      // Only forward the flag when there's actually something to clean —
      // skips the no-op worktree-walk path on empty workspaces.
      await deleteWorkspace(workspace.id, hasWorktrees && deleteWorktrees);
      showToast(`Deleted "${workspace.name}"`, 'success');
    } catch (e) {
      showToast(`Delete failed: ${e}`, 'error');
    }
  }

  async function handleDeleteItem(kind: 'note' | 'board', item: WorkspaceNote | WorkspaceBoard) {
    const label = kind === 'note' ? (item as WorkspaceNote).title : (item as WorkspaceBoard).name;
    try {
      if (kind === 'note') await deleteNote(item as WorkspaceNote);
      else await deleteBoard((item as WorkspaceBoard).id, workspace.id);
      // Close its tab if open.
      const key = `${kind}:${item.id}`;
      const t = get(sharedTabs).find(x => x.mode === 'workspace' && x.key === key);
      if (t) closeTab(t.id);
      showToast(`Deleted "${label}"`, 'success');
    } catch (e) {
      showToast(`Delete failed: ${e}`, 'error');
    }
  }

  function showLeafMenu(e: MouseEvent, kind: 'note' | 'board', item: WorkspaceNote | WorkspaceBoard) {
    e.preventDefault();
    e.stopPropagation();
    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Open',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M15 3h6v6"/><path d="M10 14L21 3"/><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/></svg>',
        action: () => kind === 'note' ? openNote(item as WorkspaceNote) : openBoard(item as WorkspaceBoard),
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Delete',
        danger: true,
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/></svg>',
        action: () => handleDeleteItem(kind, item),
      },
    ]);
  }

  onMount(() => {
    // Always load this workspace's notes + boards on mount so the
    // header sub-line ("3 notes · 1 board") is accurate without the
    // user having to expand each row first. Cost is one tiny SELECT
    // per workspace at app start; the data is then cached in the
    // shared store so subsequent expand toggles are instant.
    loadChildren();
  });

  function selectMe() {
    activeWorkspaceId.set(workspace.id);
  }

  const subText = $derived(
    `${notes.length} note${notes.length === 1 ? '' : 's'} · ${boards.length} board${boards.length === 1 ? '' : 's'}${workspace.projectName ? ` · ${workspace.projectName}` : ''}`,
  );
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="ncoll">
  <div
    class="ncoll-hdr"
    class:active={isActive}
    onclick={() => { selectMe(); toggle(); }}
    oncontextmenu={handleContextMenu}
  >
    <div class="drag-handle" title="Drag to reorder">
      <svg viewBox="0 0 24 24" width="10" height="10">
        <circle cx="8" cy="6" r="1.5" fill="currentColor"/><circle cx="16" cy="6" r="1.5" fill="currentColor"/>
        <circle cx="8" cy="12" r="1.5" fill="currentColor"/><circle cx="16" cy="12" r="1.5" fill="currentColor"/>
        <circle cx="8" cy="18" r="1.5" fill="currentColor"/><circle cx="16" cy="18" r="1.5" fill="currentColor"/>
      </svg>
    </div>
    <div class="coll-icon coll-icon-accent">
      <!-- 2×2 grid — workspace's own identity, matches the sidebar. -->
      <svg viewBox="0 0 24 24"><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg>
    </div>
    <div class="ncoll-text">
      {#if renaming}
        <InlineInput
          value={workspace.name}
          placeholder="Workspace name…"
          onsubmit={handleRename}
          oncancel={() => renaming = false}
        />
      {:else}
        <div class="ncoll-row-top">
          <span class="ncoll-name">{workspace.name}</span>
        </div>
        <div class="ncoll-row-bot">
          <span class="ncoll-sub">{subText}</span>
        </div>
      {/if}
    </div>
    <button class="coll-add" title="Add note or board" onclick={handleAddBtn}>
      <svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>
    </button>
    <button class="coll-menu" title="Options" onclick={handleMenuBtn}>
      <svg viewBox="0 0 24 24" width="13" height="13" fill="currentColor">
        <circle cx="12" cy="5" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="12" cy="19" r="1.5"/>
      </svg>
    </button>
    <svg class="ncoll-arr" class:open={expanded} viewBox="0 0 24 24">
      <path d="M9 18l6-6-6-6" stroke="currentColor" fill="none" stroke-width="1.8" stroke-linecap="round"/>
    </svg>
  </div>

  <div
    class="ncoll-body"
    style="max-height:{expanded ? (filteredBoards.length + filteredNotes.length + (addingKind ? 1 : 0)) * 38 + 64 + 'px' : '0'}"
  >
    {#if addingKind}
      <div class="inline-add-row">
        <InlineInput
          placeholder={addingKind === 'note' ? 'Note title…' : 'Board name…'}
          onsubmit={commitAdd}
          oncancel={cancelAdd}
        />
      </div>
    {/if}

    {#each filteredBoards as b (b.id)}
      <div class="ws-leaf" onclick={() => openBoard(b)} oncontextmenu={(e) => showLeafMenu(e, 'board', b)}>
        <span class="ws-leaf-ico ws-leaf-board" aria-hidden="true">
          <!-- Kanban — 3 stroked columns of varying heights. Stroked
               (not filled) so the glyph reads cleanly at 16px against
               any theme background; matches the 1.8 stroke used by
               drag-handle / chevron / ellipsis elsewhere in the row. -->
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="5" height="16" rx="1"/><rect x="9.5" y="4" width="5" height="9" rx="1"/><rect x="16" y="4" width="5" height="13" rx="1"/></svg>
        </span>
        <span class="ws-leaf-name">{b.name}</span>
      </div>
    {/each}

    {#each filteredNotes as n (n.id)}
      <div class="ws-leaf" onclick={() => openNote(n)} oncontextmenu={(e) => showLeafMenu(e, 'note', n)}>
        <span class="ws-leaf-ico ws-leaf-note" aria-hidden="true">
          <!-- Document with folded corner + 2 text lines. Standard
               note glyph; reads instantly against either light or
               dark themes. -->
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><polyline points="14 3 14 9 20 9"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="13" y2="17"/></svg>
        </span>
        <span class="ws-leaf-name">{n.title || 'Untitled'}</span>
      </div>
    {/each}

    {#if expanded && filteredBoards.length === 0 && filteredNotes.length === 0 && !addingKind}
      <div class="ws-leaf-empty">Empty — use the + above</div>
    {/if}
  </div>
</div>

<ConfirmDialog
  bind:show={showDeleteConfirm}
  title="Delete workspace"
  message={deletePreview
    ? `Delete "${workspace.name}"? This will remove ${deletePreview.noteCount} note${deletePreview.noteCount === 1 ? '' : 's'}, ${deletePreview.boardCount} board${deletePreview.boardCount === 1 ? '' : 's'}, and ${deletePreview.cardCount} card${deletePreview.cardCount === 1 ? '' : 's'}. This cannot be undone.`
    : `Delete "${workspace.name}"? All notes and boards inside will be removed. This cannot be undone.`}
  confirmText="Delete"
  onconfirm={confirmDelete}
>
  {#snippet extra()}
    {#if deletePreview && deletePreview.activeWorktrees.length > 0}
      <label class="ws-del-wt">
        <input type="checkbox" bind:checked={deleteWorktrees} />
        <span>
          Remove the {deletePreview.activeWorktrees.length} active git worktree{deletePreview.activeWorktrees.length === 1 ? '' : 's'} from disk too
        </span>
      </label>
      <ul class="ws-del-wt-list">
        {#each deletePreview.activeWorktrees as wt (wt.cardId)}
          <li>
            <span class="ws-del-wt-card">{wt.cardTitle}</span>
            {#if wt.worktreeBranch}
              <span class="ws-del-wt-branch">· {wt.worktreeBranch}</span>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  {/snippet}
</ConfirmDialog>

<style>
  .ncoll {
    border-bottom: 1px solid var(--b1);
    transition: opacity 0.15s;
  }
  .ncoll-hdr {
    min-height: 44px;
    padding: 6px 8px;
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    transition: background 0.1s;
    user-select: none;
    position: relative;
  }
  .ncoll-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ncoll-row-top, .ncoll-row-bot {
    display: flex;
    align-items: center;
    min-width: 0;
  }
  .ncoll-sub {
    font-size: 10.5px;
    font-family: var(--mono);
    color: var(--t4);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ncoll-hdr.active .ncoll-sub { color: var(--t3); }
  .ncoll-hdr:hover { background: var(--n2); }
  .ncoll-hdr.active { background: var(--n2); }

  .drag-handle {
    width: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t4);
    flex-shrink: 0;
    cursor: grab;
    opacity: 0.3;
    transition: opacity 0.15s, color 0.15s;
  }
  .ncoll-hdr:hover .drag-handle { opacity: 0.7; }
  .drag-handle:hover { color: var(--t2); }
  .drag-handle:active { cursor: grabbing; }

  .coll-icon {
    width: 22px;
    height: 22px;
    border-radius: 5px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .coll-icon-accent {
    background: color-mix(in srgb, var(--acc) 18%, transparent);
    color: var(--acc);
  }
  .coll-icon svg {
    width: 13px;
    height: 13px;
    stroke: currentColor;
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
  }

  .ncoll-name {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--t2);
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ncoll-hdr.active .ncoll-name { color: var(--t1); }

  .coll-add,
  .coll-menu {
    width: 18px;
    height: 18px;
    border-radius: 4px;
    border: none;
    background: transparent;
    display: none;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    flex-shrink: 0;
    color: var(--t3);
    transition: background 0.1s, color 0.1s;
    padding: 0;
  }
  .ncoll-hdr:hover .coll-add,
  .ncoll-hdr:hover .coll-menu { display: flex; }
  .coll-add:hover,
  .coll-menu:hover { background: var(--b1); color: var(--t1); }
  .coll-add svg {
    width: 12px;
    height: 12px;
    stroke: currentColor;
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
  }

  .ncoll-arr {
    width: 12px;
    height: 12px;
    stroke: var(--t3);
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
    flex-shrink: 0;
    transition: transform 0.18s;
  }
  .ncoll-arr.open { transform: rotate(90deg); }

  .ncoll-body {
    overflow: hidden;
    background: var(--e);
    transition: max-height 0.2s ease;
  }

  .inline-add-row {
    display: flex;
    gap: 4px;
    padding: 6px 10px;
    align-items: center;
    background: var(--e);
    border-bottom: 1px solid var(--b1);
  }

  .ws-leaf {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 12px 9px 38px;
    cursor: pointer;
    transition: background 0.08s;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12.5px;
    min-height: 34px;
  }
  .ws-leaf:hover {
    background: var(--n2);
    color: var(--t1);
  }
  .ws-leaf-ico {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--t3);
    flex-shrink: 0;
    width: 18px;
    height: 18px;
    /* Lock SVG to the container so a 16x16 icon never gets cropped or
       padded oddly by the parent flex row. */
  }
  .ws-leaf-ico svg {
    width: 16px;
    height: 16px;
    display: block;
  }
  /* Item-type tints — accent for notes, distinct purple for boards
     so the glyph itself communicates kind even before reading the
     name. Both stay full-opacity on hover (no fade). */
  .ws-leaf-board { color: #a78bfa; }
  .ws-leaf-note  { color: var(--acc); }
  .ws-leaf-name {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ws-leaf-empty {
    padding: 8px 10px 12px 32px;
    color: var(--t4);
    font-size: 11px;
    font-style: italic;
    font-family: var(--ui);
  }

  .ws-del-wt {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: 12px;
    color: var(--t1);
  }
  .ws-del-wt input { margin: 0; }
  .ws-del-wt-list {
    margin: 8px 0 0;
    padding: 0 0 0 24px;
    list-style: disc;
    font-size: 11px;
    color: var(--t3);
    max-height: 120px;
    overflow-y: auto;
  }
  .ws-del-wt-list li { padding: 2px 0; }
  .ws-del-wt-card { color: var(--t2); }
  .ws-del-wt-branch {
    font-family: var(--mono);
    color: var(--t4);
    margin-left: 4px;
  }
</style>

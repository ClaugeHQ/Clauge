<script lang="ts">
  // Workspace mode main panel — pure router. Active Topbar tab whose
  // mode is 'workspace' decides what we show:
  //   key starts with 'note:'  → NoteView
  //   key starts with 'board:' → BoardView
  //   no workspace tab active  → empty pane
  //
  // The actual content lives in NoteView / BoardView; this file is just
  // the entry switch + onboarding state.

  import { tabs as sharedTabs, activeTabId } from '$lib/shared/stores/tabs';
  import { activeWorkspace, workspaces } from '../stores';
  import NoteView from './NoteView.svelte';
  import BoardView from './BoardView.svelte';
  import InboxView from './InboxView.svelte';
  import CoworkersView from './CoworkersView.svelte';
  import { WORKSPACE_EVENT } from '$lib/shared/constants/events';

  const activeWorkspaceTab = $derived($sharedTabs.find(t => t.id === $activeTabId && t.mode === 'workspace'));
  const activeKind = $derived.by(() => {
    const k = activeWorkspaceTab?.key ?? '';
    if (k === 'inbox') return 'inbox' as const;
    if (k === 'coworkers') return 'coworkers' as const;
    if (k.startsWith('note:')) return 'note' as const;
    if (k.startsWith('board:')) return 'board' as const;
    return null;
  });
  const activeId = $derived.by(() => {
    const k = activeWorkspaceTab?.key ?? '';
    const idx = k.indexOf(':');
    return idx > 0 ? k.slice(idx + 1) : null;
  });

  function newWorkspace() {
    window.dispatchEvent(new CustomEvent(WORKSPACE_EVENT.NEW_WORKSPACE));
  }
  function newNote() {
    if (!$activeWorkspace) return;
    window.dispatchEvent(new CustomEvent(WORKSPACE_EVENT.NEW_NOTE, { detail: { workspaceId: $activeWorkspace.id } }));
  }
  function newBoard() {
    if (!$activeWorkspace) return;
    window.dispatchEvent(new CustomEvent(WORKSPACE_EVENT.NEW_BOARD, { detail: { workspaceId: $activeWorkspace.id } }));
  }
</script>

{#if activeKind === 'inbox'}
  <InboxView />
{:else if activeKind === 'coworkers'}
  <CoworkersView />
{:else if activeKind === 'note' && activeId}
  <NoteView noteId={activeId} />
{:else if activeKind === 'board' && activeId}
  <BoardView boardId={activeId} />
{:else if $workspaces.length === 0}
  <div class="ws-empty-pane">
    <svg viewBox="0 0 24 24" width="42" height="42" fill="none" stroke="var(--t4)" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <rect x="4" y="3" width="16" height="18" rx="2"/><line x1="8" y1="8" x2="16" y2="8"/><line x1="8" y1="12" x2="16" y2="12"/><line x1="8" y1="16" x2="13" y2="16"/>
    </svg>
    <h2>Welcome to Workspaces</h2>
    <p>Organize notes and Kanban boards around your projects. Your agents can read and write here through the MCP server, keeping everything in sync with your work.</p>
    <button class="ws-cta" onclick={newWorkspace}>+ Create your first workspace</button>
  </div>
{:else}
  <div class="ws-empty-pane">
    <svg viewBox="0 0 24 24" width="42" height="42" fill="none" stroke="var(--t4)" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <rect x="4" y="3" width="16" height="18" rx="2"/><line x1="8" y1="8" x2="16" y2="8"/><line x1="8" y1="12" x2="16" y2="12"/><line x1="8" y1="16" x2="13" y2="16"/>
    </svg>
    <h2>{$activeWorkspace?.name ?? 'No workspace selected'}</h2>
    <p>Select a note or board from the side panel, or create something new below.</p>
    <div class="ws-cta-row">
      <button class="ws-cta-secondary" onclick={newNote}>+ New Note</button>
      <button class="ws-cta-secondary" onclick={newBoard}>+ New Board</button>
    </div>
    <p class="ws-hint">Cards moved to <strong>Review</strong> by an agent show a pending badge — approve them before they advance to Done.</p>
  </div>
{/if}

<style>
  .ws-empty-pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 40px;
    color: var(--t3);
    text-align: center;
  }
  .ws-empty-pane h2 {
    margin: 8px 0 0;
    font-size: 17px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }
  .ws-empty-pane p {
    margin: 0;
    max-width: 440px;
    font-size: 12.5px;
    color: var(--t3);
    font-family: var(--ui);
    line-height: 1.6;
  }
  .ws-cta {
    margin-top: 8px;
    padding: 8px 18px;
    border-radius: 8px;
    border: 1px solid var(--acc);
    background: color-mix(in srgb, var(--acc) 18%, transparent);
    color: var(--t1);
    font-size: 12.5px;
    font-family: var(--ui);
    font-weight: 500;
    cursor: default;
    transition: background 0.12s;
  }
  .ws-cta:hover { background: color-mix(in srgb, var(--acc) 28%, transparent); }
  .ws-cta-row { display: flex; gap: 8px; margin-top: 6px; }
  .ws-cta-secondary {
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: border-color 0.12s, color 0.12s;
  }
  .ws-cta-secondary:hover { border-color: var(--acc); color: var(--t1); }
  .ws-hint {
    margin-top: 10px;
    font-size: 11.5px;
    color: var(--t4);
    max-width: 460px;
  }
  .ws-hint strong { color: var(--t2); font-weight: 600; }
</style>

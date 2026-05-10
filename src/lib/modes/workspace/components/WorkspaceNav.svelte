<script lang="ts">
  // Workspace side panel — flat list of WorkspaceItem rows. All visual
  // cues (drag-handle / icon / ellipsis menu / chevron) come from
  // WorkspaceItem so behaviour stays uniform with REST collections.
  // Creation flows through NewWorkspaceModal (NOT inline) — workspaces
  // accept an optional project link, so a name-only inline path drops
  // the project field. NavPanel + button dispatches NEW_WORKSPACE.

  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import {
    workspaces, loadWorkspaces,
    refreshInboxUnread, markInboxRead, inboxUnreadCount,
    coworkers, loadCoworkers,
  } from '../stores';
  import WorkspaceItem from './WorkspaceItem.svelte';
  import { WORKSPACE_EVENT } from '$lib/shared/constants/events';
  import { mode } from '$lib/stores/app';
  import { tabs as sharedTabs, activeTabId, addTab, activateTab } from '$lib/shared/stores/tabs';

  interface Props {
    searchQuery?: string;
  }

  let { searchQuery = '' }: Props = $props();

  const inboxActive = $derived(
    $mode === 'workspace' && !!$sharedTabs.find(t => t.id === $activeTabId && t.key === 'inbox'),
  );
  const coworkersActive = $derived(
    $mode === 'workspace' && !!$sharedTabs.find(t => t.id === $activeTabId && t.key === 'coworkers'),
  );

  function openInbox() {
    const key = 'inbox';
    const existing = get(sharedTabs).find(t => t.mode === 'workspace' && t.key === key);
    if (existing) activateTab(existing.id);
    else addTab('Inbox', 'workspace', key, 'var(--acc)');
    mode.set('workspace');
    markInboxRead();
  }

  function openCoworkers() {
    const key = 'coworkers';
    const existing = get(sharedTabs).find(t => t.mode === 'workspace' && t.key === key);
    if (existing) activateTab(existing.id);
    else addTab('Co-workers', 'workspace', key, 'var(--acc)');
    mode.set('workspace');
  }

  onMount(() => {
    loadWorkspaces();
    refreshInboxUnread();
    loadCoworkers();
  });

  const filtered = $derived(
    searchQuery.trim()
      ? $workspaces.filter(w => w.name.toLowerCase().includes(searchQuery.toLowerCase()))
      : $workspaces,
  );

  function openModal() {
    window.dispatchEvent(new CustomEvent(WORKSPACE_EVENT.NEW_WORKSPACE));
  }
</script>

<div class="ws-nav">
  <!-- Pinned Inbox row — agent-touched items across all workspaces.
       Right-side shows a count of unread items (anything mutated by an
       agent since the user last opened the inbox). Clicking the row
       opens the inbox AND marks read, clearing the badge. -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="ws-inbox-row"
    class:active={inboxActive}
    onclick={openInbox}
  >
    <span class="ws-inbox-ico">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/></svg>
    </span>
    <span class="ws-inbox-name">Inbox</span>
    {#if $inboxUnreadCount > 0}
      <span class="ws-inbox-badge" title="{$inboxUnreadCount} unread">{$inboxUnreadCount > 99 ? '99+' : $inboxUnreadCount}</span>
    {:else}
      <span class="ws-inbox-dot" aria-hidden="true"></span>
    {/if}
  </div>

  <!-- Pinned Co-workers row — sits below Inbox. Click opens the
       CoworkersView in the main panel; mutually exclusive with Inbox. -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="ws-inbox-row"
    class:active={coworkersActive}
    onclick={openCoworkers}
  >
    <span class="ws-inbox-ico">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="9" cy="8" r="3.2"/>
        <path d="M2.5 19a6.5 6.5 0 0 1 13 0"/>
        <circle cx="17" cy="6" r="2.4"/>
        <path d="M14 13a4.5 4.5 0 0 1 8.5 2"/>
      </svg>
    </span>
    <span class="ws-inbox-name">Co-workers</span>
    {#if $coworkers.length > 0}
      <span class="ws-inbox-badge ws-coworker-count" title="{$coworkers.length} {$coworkers.length === 1 ? 'persona' : 'personas'}">
        {$coworkers.length}
      </span>
    {:else}
      <span class="ws-inbox-dot" aria-hidden="true"></span>
    {/if}
  </div>

  {#if filtered.length === 0}
    <div class="nav-empty">
      {#if searchQuery}
        <span>No results for "{searchQuery}"</span>
      {:else}
        <span>No workspaces yet</span>
        <button class="nav-empty-btn" onclick={openModal}>+ New Workspace</button>
      {/if}
    </div>
  {:else}
    {#each filtered as w (w.id)}
      <WorkspaceItem workspace={w} {searchQuery} />
    {/each}
  {/if}
</div>

<style>
  .ws-nav {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .nav-empty {
    padding: 24px 12px;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  .nav-empty-btn {
    padding: 5px 12px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11px;
    font-family: var(--mono);
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .nav-empty-btn:hover {
    background: var(--c);
    border-color: var(--b2);
    color: var(--t1);
  }

  /* Inbox row — pinned at the top, distinct from the workspace tree. */
  .ws-inbox-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 11px 14px;
    cursor: default;
    border-bottom: 1px solid var(--b1);
    transition: background 0.08s;
  }
  .ws-inbox-row:hover { background: var(--n2); }
  .ws-inbox-row.active { background: var(--n2); }
  .ws-inbox-row.active .ws-inbox-name { color: var(--acc); }
  .ws-inbox-ico {
    color: var(--acc);
    display: inline-flex;
    flex-shrink: 0;
  }
  .ws-inbox-name {
    font-family: var(--ui);
    font-size: 12.5px;
    font-weight: 600;
    color: var(--t1);
    flex: 1;
  }
  /* Right-side state: dot when caught up, count badge when unread. */
  .ws-inbox-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--t4);
    flex-shrink: 0;
    opacity: 0.5;
  }
  .ws-inbox-badge {
    flex-shrink: 0;
    min-width: 18px;
    height: 18px;
    padding: 0 6px;
    border-radius: 9px;
    background: var(--acc);
    color: #fff;
    font-family: var(--ui);
    font-size: 10px;
    font-weight: 700;
    line-height: 18px;
    text-align: center;
  }
  /* Co-worker count is informational, not action-required — quieter
     style than the unread inbox badge. */
  .ws-coworker-count {
    background: rgba(255, 255, 255, 0.08);
    color: var(--t2);
    font-weight: 600;
  }
</style>

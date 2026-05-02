<script lang="ts">
  import { agentSessions, activeAgentSession, agentContextUsage, agentSessionActivity, agentClaudePlan } from '../stores';
  import { mode } from '$lib/stores/app';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import { showToast } from '$lib/shared/primitives/toast';
  import type { AgentSession } from '../types';
  import { tabs, addTab, activateTab } from '$lib/shared/stores/tabs';
  import { get } from 'svelte/store';
  import { AGENT_EVENT } from '$lib/shared/constants/events';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';

  interface Props {
    searchQuery?: string;
  }

  let { searchQuery = '' }: Props = $props();

  // Confirm dialog
  let confirmShow = $state(false);
  let confirmTitle = $state('');
  let confirmMessage = $state('');
  let confirmDanger = $state(false);
  let confirmText = $state('Confirm');
  let confirmAction: (() => Promise<void>) | null = $state(null);

  /** Set up + show the shared ConfirmDialog. Centralised so the Reset
   *  vs Delete branches don't have to duplicate state-write boilerplate. */
  function showConfirm(opts: { title: string; message: string; danger: boolean; confirmText: string; action: () => Promise<void> }) {
    confirmTitle = opts.title;
    confirmMessage = opts.message;
    confirmDanger = opts.danger;
    confirmText = opts.confirmText;
    confirmAction = opts.action;
    confirmShow = true;
  }

  // Collapsed project groups — persisted across app reloads so the user's
  // organisation choices survive a restart. localStorage (sync, instant)
  // is fine here since this is purely device-local UI state.
  const COLLAPSED_KEY = 'clauge.agent.collapsedProjects';
  function loadCollapsed(): Set<string> {
    try {
      const raw = localStorage.getItem(COLLAPSED_KEY);
      if (!raw) return new Set();
      const arr = JSON.parse(raw);
      if (Array.isArray(arr)) return new Set(arr.filter((v): v is string => typeof v === 'string'));
    } catch { /* corrupt entry — ignore and start fresh */ }
    return new Set();
  }
  function saveCollapsed(set: Set<string>) {
    try { localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...set])); } catch { /* quota / private mode — silent */ }
  }
  let collapsedProjects = $state<Set<string>>(loadCollapsed());

  const purposeColors: Record<string, string> = {
    'Brainstorming': '#d2a8ff',
    'Development': '#3fb950',
    'Code Review': '#58a6ff',
    'PR Review': '#d29922',
    'Debugging': '#f85149',
    'Custom': '#8b949e',
  };

  function purposeColor(purpose: string): string {
    return purposeColors[purpose] ?? '#8b949e';
  }

  const filteredSessions = $derived(
    searchQuery
      ? $agentSessions.filter(s =>
          s.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
          s.projectName.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : $agentSessions
  );

  const groupedByProject = $derived.by(() => {
    const groups = new Map<string, AgentSession[]>();
    for (const s of filteredSessions) {
      const key = s.projectName || 'Untitled';
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)!.push(s);
    }
    // Sort sessions within each group by lastUsedAt descending
    for (const [, sessions] of groups) {
      sessions.sort((a, b) => b.lastUsedAt.localeCompare(a.lastUsedAt));
    }
    return groups;
  });

  function handleNewSession() {
    window.dispatchEvent(new CustomEvent(AGENT_EVENT.NEW_SESSION));
  }

  function handleSelectSession(session: AgentSession) {
    // Open or focus the tab for this session
    const currentTabs = get(tabs);
    const existing = currentTabs.find(t => t.mode === 'agent' && t.key === session.id);
    if (existing) {
      activateTab(existing.id);
    } else {
      addTab(session.title, 'agent', session.id, purposeColor(session.purpose));
    }

    // Don't re-select the already active session
    if ($activeAgentSession?.id === session.id) return;
    activeAgentSession.set(session);
    window.dispatchEvent(new CustomEvent(AGENT_EVENT.SELECT_SESSION, { detail: { session } }));
  }

  function toggleProject(name: string) {
    const next = new Set(collapsedProjects);
    if (next.has(name)) {
      next.delete(name);
    } else {
      next.add(name);
    }
    collapsedProjects = next;
    saveCollapsed(next);
  }

  function contextPercent(sessionId: string): number | null {
    const usage = $agentContextUsage.get(sessionId);
    return usage ? Math.round(usage.fillPercent) : null;
  }

  function contextClass(pct: number): string {
    if (pct >= 85) return 'ctx-red';
    if (pct >= 70) return 'ctx-yellow';
    return 'ctx-green';
  }

  function activityStatus(sessionId: string): 'running' | 'done' | null {
    return $agentSessionActivity.get(sessionId) ?? null;
  }

  function relativeTime(iso: string): string {
    const diff = Date.now() - new Date(iso).getTime();
    if (diff < 60000) return 'just now';
    if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`;
    if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`;
    return `${Math.floor(diff / 86400000)}d ago`;
  }

  function showSessionMenu(e: MouseEvent, session: AgentSession) {
    e.preventDefault();
    e.stopPropagation();

    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Edit',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
        action: () => {
          window.dispatchEvent(new CustomEvent(AGENT_EVENT.EDIT_SESSION, { detail: { session } }));
        },
      },
      {
        label: 'Reset Session',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M23 4v6h-6"/><path d="M1 20v-6h6"/><path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/></svg>',
        action: () => showConfirm({
          title: 'Reset Session',
          message: `Reset "${session.title}"? This will clear the Claude session ID and start fresh.`,
          danger: false,
          confirmText: 'Reset',
          action: async () => {
            window.dispatchEvent(new CustomEvent(AGENT_EVENT.RESET_SESSION, { detail: { session } }));
          },
        }),
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Delete',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>',
        danger: true,
        action: () => showConfirm({
          title: 'Delete Session',
          message: `Delete "${session.title}"? This cannot be undone.`,
          danger: true,
          confirmText: 'Delete',
          action: async () => {
            window.dispatchEvent(new CustomEvent(AGENT_EVENT.DELETE_SESSION, { detail: { session } }));
          },
        }),
      },
    ]);
  }

  async function handleConfirmOk() {
    confirmShow = false;
    if (confirmAction) await confirmAction();
    confirmAction = null;
  }
</script>

<div class="agent-nav">
  {#if filteredSessions.length === 0}
    <div class="nav-empty">
      {#if searchQuery}
        <span>No results for "{searchQuery}"</span>
      {:else}
        <span>No sessions yet</span>
        <button class="nav-empty-btn" onclick={handleNewSession}>
          + New Session
        </button>
      {/if}
    </div>
  {:else}
    {#each [...groupedByProject] as [projectName, sessions] (projectName)}
      {@const isCollapsed = collapsedProjects.has(projectName)}
      <!-- Project group header -->
      <button class="project-header" onclick={() => toggleProject(projectName)}>
        <svg class="tree-chevron" class:open={!isCollapsed} viewBox="0 0 24 24">
          <path d="M9 18l6-6-6-6"/>
        </svg>
        <span class="project-name">{projectName}</span>
        <span class="project-count">{sessions.length}</span>
      </button>

      {#if !isCollapsed}
        {#each sessions as session (session.id)}
          {@const pct = contextPercent(session.id)}
          {@const activity = activityStatus(session.id)}
          <button
            class="session-item"
            class:active={$activeAgentSession?.id === session.id}
            onclick={() => handleSelectSession(session)}
            oncontextmenu={(e) => showSessionMenu(e, session)}
          >
            <span class="session-icon">
              {#if activity === 'running'}
                <img src="/code-in-action.svg" alt="" width="36" height="26" />
              {:else}
                <img src="/code-no-action.svg" alt="" width="22" height="22" />
              {/if}
            </span>
            <div class="session-body">
              <div class="session-row-top">
                <span class="session-title">{session.title}</span>
                {#if pct !== null}
                  <span class="ctx-badge {contextClass(pct)}" title="{pct}% context window used">{pct}%</span>
                {/if}
              </div>
              <div class="session-row-bot">
                <span class="purpose-badge" style="color:{purposeColor(session.purpose)};background:{purposeColor(session.purpose)}22">{session.purpose}</span>
                {#if session.worktreePath}
                  <span class="wt-badge" title="Isolated worktree: {session.worktreeBranch}">WT</span>
                {/if}
                <span class="session-time-spacer"></span>
                <span class="session-time">{relativeTime(session.lastUsedAt)}</span>
              </div>
            </div>
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <span
              class="session-ellipsis"
              role="button"
              tabindex="-1"
              title="More"
              onclick={(e) => { e.stopPropagation(); showSessionMenu(e, session); }}
            >
              <svg viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="5" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="12" cy="19" r="1.5"/></svg>
            </span>
          </button>
        {/each}
      {/if}
    {/each}
  {/if}
</div>

<!-- Confirm Dialog — shared primitive across all modes (header bar, body,
     footer with proper dividers; teleports to body so nav stacking
     contexts can't clip it). -->
<ConfirmDialog
  bind:show={confirmShow}
  title={confirmTitle}
  message={confirmMessage}
  confirmText={confirmText}
  confirmColor={confirmDanger ? 'var(--err)' : 'var(--acc)'}
  onconfirm={handleConfirmOk}
/>

<style>
  .agent-nav {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .agent-nav::-webkit-scrollbar { width: 3px; }
  .agent-nav::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .nav-empty {
    padding: 24px 12px;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--ui);
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
    font-family: var(--ui);
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .nav-empty-btn:hover { background: var(--c); border-color: var(--b2); color: var(--t1); }

  /* Project group header */
  .project-header {
    width: 100%;
    height: 28px;
    border: none;
    background: transparent;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    cursor: pointer;
    transition: background 0.08s;
    text-align: left;
  }
  .project-header:hover { background: var(--c); }

  .project-name {
    font-family: var(--ui);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--t3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .project-count {
    font-size: 9px;
    font-family: var(--ui);
    color: var(--t4);
    background: rgba(255,255,255,0.06);
    padding: 1px 5px;
    border-radius: 8px;
    flex-shrink: 0;
  }

  .tree-chevron {
    width: 10px; height: 10px;
    stroke: var(--t4); fill: none; stroke-width: 2; stroke-linecap: round;
    flex-shrink: 0; transition: transform 0.15s;
  }
  .tree-chevron.open { transform: rotate(90deg); }

  /* Session item */
  .session-item {
    width: 100%;
    min-height: 46px;
    border: none;
    background: transparent;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px 5px 8px;
    cursor: pointer;
    transition: background 0.08s;
    text-align: left;
    position: relative;
    /* Divider between sessions inside a project group. The :last-child
       rule below clears it on the trailing item so groups don't run into
       the project header underneath. */
    border-bottom: 1px solid var(--b-subtle);
  }
  .session-item:last-child { border-bottom: none; }
  .session-item:hover { background: var(--c); }
  .session-item.active { background: color-mix(in srgb, var(--agent, var(--acc)) 10%, transparent); }

  .session-icon {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .session-icon img {
    display: block;
  }

  .session-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .session-row-top {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .session-title {
    font-family: var(--ui);
    font-size: 12px;
    color: var(--t2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .session-item.active .session-title { color: var(--t1); }

  .session-row-bot {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .purpose-badge {
    font-size: 10px;
    font-family: var(--ui);
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 4px;
    white-space: nowrap;
    line-height: 1.4;
  }

  .session-time-spacer {
    flex: 1;
  }

  .session-time {
    font-family: var(--ui);
    font-size: 9px;
    color: var(--t4);
    white-space: nowrap;
  }

  /* WT (worktree) is a single fixed identity tag — like the brand badges
     (Postgres, S3, Mongo). Kept theme-independent so it doesn't follow the
     user's accent. */
  .wt-badge {
    font-size: 8px;
    font-family: var(--mono);
    font-weight: 700;
    color: #7c5cf8;
    background: rgba(124, 92, 248, 0.12);
    padding: 1px 4px;
    border-radius: 3px;
    flex-shrink: 0;
  }

  /* Context usage badge */
  .ctx-badge {
    font-size: 9px;
    font-family: var(--mono);
    font-weight: 600;
    padding: 1px 5px;
    border-radius: 8px;
    flex-shrink: 0;
  }
  .ctx-green { color: #3fb950; background: rgba(63, 185, 80, 0.12); }
  .ctx-yellow { color: #d29922; background: rgba(210, 153, 34, 0.12); }
  .ctx-red { color: #f85149; background: rgba(248, 81, 73, 0.12); animation: ctx-pulse 1.5s ease-in-out infinite; }
  @keyframes ctx-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.6; }
  }

  /* Ellipsis button */
  .session-ellipsis {
    width: 18px; height: 18px;
    display: none; align-items: center; justify-content: center;
    border-radius: 3px; flex-shrink: 0; cursor: default;
    color: var(--t3); transition: background 0.1s, color 0.1s;
  }
  .session-ellipsis svg { width: 14px; height: 14px; }
  .session-item:hover .session-ellipsis { display: flex; }
  .session-ellipsis:hover { background: rgba(255,255,255,0.08); color: var(--t1); }


  /* Claude plan badge */
  .plan-badge-row {
    padding: 8px 10px 4px;
    margin-top: auto;
  }
  .plan-badge {
    font-size: 9px;
    font-family: var(--ui);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--acc, #6366f1);
    background: rgba(99, 102, 241, 0.10);
    padding: 2px 7px;
    border-radius: 4px;
  }
</style>

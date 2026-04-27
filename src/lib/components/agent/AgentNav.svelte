<script lang="ts">
  import { agentSessions, activeAgentSession, agentContextUsage, agentSessionActivity, agentClaudePlan } from '$lib/stores/agent';
  import { mode } from '$lib/stores/app';
  import { showContextMenu } from '$lib/components/shared/contextmenu';
  import { showToast } from '$lib/components/shared/toast';
  import type { AgentSession } from '$lib/types/agent';
  import { tabs, addTab, activateTab } from '$lib/stores/tabs';
  import { get } from 'svelte/store';

  // Teleport action: moves element to document.body to escape stacking context
  function teleport(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentElement === document.body) node.remove();
      }
    };
  }

  interface Props {
    searchQuery?: string;
  }

  let { searchQuery = '' }: Props = $props();

  // Confirm dialog
  let confirmShow = $state(false);
  let confirmTitle = $state('');
  let confirmMessage = $state('');
  let confirmAction: (() => Promise<void>) | null = $state(null);

  // Collapsed project groups
  let collapsedProjects = $state<Set<string>>(new Set());

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
    window.dispatchEvent(new CustomEvent('agent:new-session'));
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
    window.dispatchEvent(new CustomEvent('agent:select-session', { detail: { session } }));
  }

  function toggleProject(name: string) {
    const next = new Set(collapsedProjects);
    if (next.has(name)) {
      next.delete(name);
    } else {
      next.add(name);
    }
    collapsedProjects = next;
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
          window.dispatchEvent(new CustomEvent('agent:edit-session', { detail: { session } }));
        },
      },
      {
        label: 'Reset Session',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M23 4v6h-6"/><path d="M1 20v-6h6"/><path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/></svg>',
        action: () => {
          confirmTitle = 'Reset Session';
          confirmMessage = `Reset "${session.title}"? This will clear the Claude session ID and start fresh.`;
          confirmAction = async () => {
            window.dispatchEvent(new CustomEvent('agent:reset-session', { detail: { session } }));
          };
          confirmShow = true;
        },
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Delete',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>',
        danger: true,
        action: () => {
          confirmTitle = 'Delete Session';
          confirmMessage = `Delete "${session.title}"? This cannot be undone.`;
          confirmAction = async () => {
            window.dispatchEvent(new CustomEvent('agent:delete-session', { detail: { session } }));
          };
          confirmShow = true;
        },
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

<!-- Confirm Dialog — teleported to body to avoid nav panel stacking context clipping -->
{#if confirmShow}
  <div class="confirm-portal" use:teleport>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="confirm-overlay" onclick={() => confirmShow = false}>
      <div class="confirm-dialog" onclick={(e) => e.stopPropagation()}>
        <div class="confirm-title">{confirmTitle}</div>
        <div class="confirm-msg">{confirmMessage}</div>
        <div class="confirm-actions">
          <button class="confirm-btn" onclick={() => confirmShow = false}>Cancel</button>
          <button class="confirm-btn danger" onclick={handleConfirmOk}>{confirmTitle === 'Delete Session' ? 'Delete' : 'Reset'}</button>
        </div>
      </div>
    </div>
  </div>
{/if}

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
  }
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

  .wt-badge {
    font-size: 8px;
    font-family: var(--mono);
    font-weight: 700;
    color: var(--acc, #7c5cf8);
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

  /* Confirm dialog */
  .confirm-overlay {
    position: fixed; top: 0; left: 0; width: 100vw; height: 100vh;
    background: rgba(0,0,0,0.4); z-index: 9999;
    display: flex; align-items: center; justify-content: center;
  }
  .confirm-dialog {
    background: var(--modal-bg, var(--n)); border: 1px solid var(--b1);
    border-radius: 12px; padding: 24px; min-width: 320px; max-width: 400px;
    box-shadow: 0 16px 48px rgba(0,0,0,0.5);
  }
  .confirm-title {
    font-size: 15px; font-weight: 600; color: var(--t1); font-family: var(--ui);
    margin-bottom: 8px;
  }
  .confirm-msg {
    font-size: 13px; color: var(--t2); font-family: var(--ui); line-height: 1.5;
    margin-bottom: 20px;
  }
  .confirm-actions { display: flex; justify-content: flex-end; gap: 8px; }
  .confirm-btn {
    padding: 7px 16px; border-radius: 8px; font-size: 12px; font-weight: 600;
    font-family: var(--ui); cursor: default; border: 1px solid var(--b1);
    background: transparent; color: var(--t2); transition: all 0.12s;
  }
  .confirm-btn:hover { background: var(--c); color: var(--t1); }
  .confirm-btn.danger { background: var(--err); color: #fff; border-color: transparent; }
  .confirm-btn.danger:hover { opacity: 0.9; }

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

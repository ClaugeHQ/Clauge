<script lang="ts">
  import { get } from 'svelte/store';
  import { activeAgentSession, agentGitFiles, agentGitBranchName, agentGitAhead, agentGitBehind, refreshAgentGitStatus } from '$lib/stores/agent';
  import {
    agentGitStatus, agentGitCommit, agentGitPush, agentGitPull,
    agentGitDiffFile, agentGitStageFile, agentGitUnstageFile,
    agentGitLog, agentGitStash, agentGitStashPop,
    agentGitListBranches, agentGitSwitchBranch,
  } from '$lib/commands/agent';
  import type { GitFileChange } from '$lib/types/agent';

  let { open = $bindable(false) } = $props();

  // Teleport to body to escape stacking context
  function teleport(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy() { if (node.parentElement === document.body) node.remove(); } };
  }

  function getProjectPath(): string {
    const session = get(activeAgentSession);
    return session?.worktreePath || session?.projectPath || '';
  }

  // Tab state
  let activeTab = $state<'changes' | 'history' | 'branches'>('changes');

  // Changes tab state
  let stagedFiles = $state<Set<string>>(new Set());
  let commitMsg = $state('');
  let diffFile = $state('');
  let diffContent = $state('');

  // History tab state
  let commits = $state<any[]>([]);

  // Branches tab state
  let branches = $state<any[]>([]);

  // Loading / status
  let loading = $state('');
  let statusMsg = $state('');
  let statusTimer: ReturnType<typeof setTimeout> | null = null;

  // Confirmation dialog
  let confirmAction = $state<{ label: string; description: string; action: () => void } | null>(null);

  function showStatus(msg: string) {
    statusMsg = msg;
    if (statusTimer) clearTimeout(statusTimer);
    statusTimer = setTimeout(() => { statusMsg = ''; }, 3000);
  }

  function confirmGitAction(label: string, description: string, action: () => void) {
    confirmAction = { label, description, action };
  }

  function executeConfirmed() {
    if (confirmAction?.action) confirmAction.action();
    confirmAction = null;
  }

  // Changes actions
  async function viewDiff(file: GitFileChange) {
    const pp = getProjectPath();
    if (!pp) return;
    diffFile = file.path;
    try {
      diffContent = await agentGitDiffFile(pp, file.path);
    } catch { diffContent = 'Failed to load diff'; }
  }

  function closeDiff() {
    diffFile = '';
    diffContent = '';
  }

  async function toggleStageFile(file: GitFileChange) {
    const pp = getProjectPath();
    if (!pp) return;
    const isStaged = stagedFiles.has(file.path);
    try {
      if (isStaged) {
        await agentGitUnstageFile(pp, file.path);
        stagedFiles.delete(file.path);
      } else {
        await agentGitStageFile(pp, file.path);
        stagedFiles.add(file.path);
      }
      stagedFiles = new Set(stagedFiles);
    } catch { /* ignore */ }
  }

  async function stageAll() {
    const pp = getProjectPath();
    if (!pp) return;
    const files = get(agentGitFiles);
    for (const f of files) {
      if (!stagedFiles.has(f.path)) {
        try { await agentGitStageFile(pp, f.path); stagedFiles.add(f.path); } catch { /* ignore */ }
      }
    }
    stagedFiles = new Set(stagedFiles);
  }

  async function unstageAll() {
    const pp = getProjectPath();
    if (!pp) return;
    for (const path of stagedFiles) {
      try { await agentGitUnstageFile(pp, path); } catch { /* ignore */ }
    }
    stagedFiles = new Set();
  }

  async function doCommit() {
    const pp = getProjectPath();
    if (!pp || !commitMsg.trim()) return;
    loading = 'commit';
    try {
      await agentGitCommit(pp, commitMsg.trim());
      commitMsg = '';
      stagedFiles = new Set();
      showStatus('Committed');
      await refreshAgentGitStatus();
    } catch { showStatus('Commit failed'); }
    loading = '';
  }

  // History
  async function loadHistory() {
    const pp = getProjectPath();
    if (!pp) return;
    try { commits = await agentGitLog(pp, 20); } catch { commits = []; }
  }

  // Branches
  async function loadBranches() {
    const pp = getProjectPath();
    if (!pp) return;
    try { branches = await agentGitListBranches(pp); } catch { branches = []; }
  }

  async function switchBranch(name: string) {
    const pp = getProjectPath();
    if (!pp) return;
    confirmGitAction('Switch Branch', `Switch to branch "${name}"? Uncommitted changes may be lost.`, async () => {
      loading = 'branch';
      try {
        await agentGitSwitchBranch(pp, name);
        showStatus(`Switched to ${name}`);
        await refreshAgentGitStatus();
        await loadBranches();
      } catch { showStatus('Switch failed'); }
      loading = '';
    });
  }

  // Action buttons
  async function doStash() {
    const pp = getProjectPath();
    if (!pp) return;
    loading = 'stash';
    try {
      await agentGitStash(pp);
      showStatus('Stashed');
      await refreshAgentGitStatus();
    } catch { showStatus('Stash failed'); }
    loading = '';
  }

  async function doStashPop() {
    const pp = getProjectPath();
    if (!pp) return;
    loading = 'stashpop';
    try {
      await agentGitStashPop(pp);
      showStatus('Stash popped');
      await refreshAgentGitStatus();
    } catch { showStatus('Pop failed'); }
    loading = '';
  }

  async function doPull() {
    const pp = getProjectPath();
    if (!pp) return;
    loading = 'pull';
    try {
      await agentGitPull(pp);
      showStatus('Pulled');
      await refreshAgentGitStatus();
    } catch { showStatus('Pull failed'); }
    loading = '';
  }

  async function doPush() {
    const pp = getProjectPath();
    if (!pp) return;
    loading = 'push';
    try {
      await agentGitPush(pp);
      showStatus('Pushed');
      await refreshAgentGitStatus();
    } catch { showStatus('Push failed'); }
    loading = '';
  }

  function handleTabClick(tab: 'changes' | 'history' | 'branches') {
    activeTab = tab;
    if (tab === 'history') loadHistory();
    if (tab === 'branches') loadBranches();
  }
</script>

{#if open}
<div use:teleport>
<!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
<div class="gp-overlay" onclick={() => open = false}>
  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
  <div class="gp-panel" onclick={(e) => e.stopPropagation()}>
    <!-- Tabs + Actions -->
    <div class="gp-tabs">
      <button class="gp-tab" class:active={activeTab === 'changes'} onclick={() => handleTabClick('changes')}>
        Changes{$agentGitFiles.length > 0 ? ` (${$agentGitFiles.length})` : ''}
      </button>
      <button class="gp-tab" class:active={activeTab === 'history'} onclick={() => handleTabClick('history')}>History</button>
      <button class="gp-tab" class:active={activeTab === 'branches'} onclick={() => handleTabClick('branches')}>Branches</button>
      <div class="gp-tab-actions">
        <button class="gp-action has-tooltip" disabled={!!loading} onclick={() => confirmGitAction('Stash', 'Stash all uncommitted changes?', doStash)}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h14a2 2 0 012 2v14a2 2 0 01-2 2z"/><path d="M17 21v-8H7v8"/><path d="M7 3v5h8"/></svg>
          <span class="gp-tooltip">Stash</span>
        </button>
        <button class="gp-action has-tooltip" disabled={!!loading} onclick={() => confirmGitAction('Pop Stash', 'Restore previously stashed changes?', doStashPop)}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h14a2 2 0 012 2v14a2 2 0 01-2 2z"/><polyline points="12 8 12 16"/><polyline points="8 12 12 8 16 12"/></svg>
          <span class="gp-tooltip">Pop Stash</span>
        </button>
        <button class="gp-action has-tooltip" disabled={loading === 'pull'} onclick={() => confirmGitAction('Pull', 'Pull latest changes from remote?', doPull)}>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><path d="M8 12a.75.75 0 01-.75-.75V4.56L5.03 6.78a.75.75 0 01-1.06-1.06l3.5-3.5a.75.75 0 011.06 0l3.5 3.5a.75.75 0 01-1.06 1.06L8.75 4.56v6.69A.75.75 0 018 12z"/><path d="M2.75 13a.75.75 0 000 1.5h10.5a.75.75 0 000-1.5H2.75z"/></svg>
          <span class="gp-tooltip">{loading === 'pull' ? 'Pulling...' : 'Pull'}</span>
        </button>
        <button class="gp-action has-tooltip" disabled={loading === 'push'} onclick={() => confirmGitAction('Push', 'Push local commits to remote?', doPush)}>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><path d="M8 4a.75.75 0 01.75.75v6.69l2.22-2.22a.75.75 0 011.06 1.06l-3.5 3.5a.75.75 0 01-1.06 0l-3.5-3.5a.75.75 0 011.06-1.06l2.22 2.22V4.75A.75.75 0 018 4z"/><path d="M2.75 1.5a.75.75 0 000 1.5h10.5a.75.75 0 000-1.5H2.75z"/></svg>
          <span class="gp-tooltip">{loading === 'push' ? 'Pushing...' : 'Push'}</span>
        </button>
      </div>
    </div>

    <!-- Status message -->
    {#if statusMsg}
      <div class="gp-status">{statusMsg}</div>
    {/if}

    <!-- Changes Tab -->
    {#if activeTab === 'changes'}
      {#if diffFile}
        <div class="gp-diff-header">
          <button class="gp-diff-back" onclick={closeDiff}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="15 18 9 12 15 6"/></svg>
          </button>
          <span class="gp-diff-filename">{diffFile}</span>
        </div>
        <div class="gp-diff-view">
          {#each diffContent.split('\n') as line}
            <div class="gp-diff-line" class:diff-add={line.startsWith('+')} class:diff-del={line.startsWith('-')} class:diff-hunk={line.startsWith('@@')}>{line}</div>
          {/each}
        </div>
      {:else if $agentGitFiles.length > 0}
        <div class="gp-stage-actions">
          <button class="gp-stage-btn" onclick={stageAll}>Stage All</button>
          <button class="gp-stage-btn" onclick={unstageAll}>Unstage All</button>
        </div>
        <div class="gp-file-list">
          {#each $agentGitFiles as file}
            <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
            <div class="gp-file-item" onclick={() => viewDiff(file)}>
              <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
              <span class="gp-file-stage" onclick={(e) => { e.stopPropagation(); toggleStageFile(file); }} title={stagedFiles.has(file.path) ? 'Unstage' : 'Stage'}>
                {#if stagedFiles.has(file.path)}
                  <svg width="12" height="12" viewBox="0 0 16 16" fill="var(--acc, #7c5cf8)"><path d="M13.78 4.22a.75.75 0 010 1.06l-7.25 7.25a.75.75 0 01-1.06 0L2.22 9.28a.75.75 0 011.06-1.06L6 10.94l6.72-6.72a.75.75 0 011.06 0z"/></svg>
                {:else}
                  <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="var(--t3)" stroke-width="1.5"><rect x="2" y="2" width="12" height="12" rx="2"/></svg>
                {/if}
              </span>
              <span class="gp-file-status" class:modified={file.status === 'M'} class:added={file.status === 'A' || file.status === '??'} class:deleted={file.status === 'D'} class:renamed={file.status === 'R'}>{file.status === '??' ? 'U' : file.status}</span>
              <span class="gp-file-path">{file.path}</span>
            </div>
          {/each}
        </div>
        <div class="gp-commit-row">
          <textarea class="gp-commit-input" bind:value={commitMsg} placeholder="Commit message..." rows="2" onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey && commitMsg.trim()) { e.preventDefault(); doCommit(); } }}></textarea>
          <button class="gp-commit-btn" disabled={!commitMsg.trim() || loading === 'commit'} onclick={doCommit}>
            {loading === 'commit' ? '...' : 'Commit'}
          </button>
        </div>
      {:else}
        <div class="gp-empty">Working tree clean</div>
      {/if}

    <!-- History Tab -->
    {:else if activeTab === 'history'}
      <div class="gp-history-list">
        {#each commits as commit}
          <div class="gp-commit-item">
            <span class="gp-commit-hash">{commit.short || commit.hash?.slice(0, 7) || ''}</span>
            <span class="gp-commit-message">{commit.message || ''}</span>
            <span class="gp-commit-meta">{commit.author || ''} {commit.date || ''}</span>
          </div>
        {:else}
          <div class="gp-empty">No commits</div>
        {/each}
      </div>

    <!-- Branches Tab -->
    {:else if activeTab === 'branches'}
      <div class="gp-branch-list">
        {#each branches as branch}
          <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
          <div class="gp-branch-item" class:current={branch.current} onclick={() => { if (!branch.current) switchBranch(branch.name); }}>
            {#if branch.current}
              <svg width="10" height="10" viewBox="0 0 16 16" fill="var(--acc, #7c5cf8)"><path d="M13.78 4.22a.75.75 0 010 1.06l-7.25 7.25a.75.75 0 01-1.06 0L2.22 9.28a.75.75 0 011.06-1.06L6 10.94l6.72-6.72a.75.75 0 011.06 0z"/></svg>
            {/if}
            <span class="gp-branch-name">{branch.name}</span>
          </div>
        {:else}
          <div class="gp-empty">No branches</div>
        {/each}
      </div>
    {/if}
  </div>
</div>

{#if confirmAction}
  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
  <div class="gp-confirm-overlay" onclick={() => confirmAction = null}>
    <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
    <div class="gp-confirm" onclick={(e) => e.stopPropagation()}>
      <p class="gp-confirm-text">{confirmAction.description}</p>
      <div class="gp-confirm-actions">
        <button class="gp-confirm-cancel" onclick={() => confirmAction = null}>Cancel</button>
        <button class="gp-confirm-ok" onclick={executeConfirmed}>{confirmAction.label}</button>
      </div>
    </div>
  </div>
{/if}
</div>
{/if}

<style>
  /* Overlay positioned above status bar — blocks clicks and scroll to terminal behind */
  .gp-overlay {
    position: fixed;
    inset: 0;
    z-index: 9000;
    background: rgba(0,0,0,0.25);
    overscroll-behavior: contain;
    cursor: default;
  }
  .gp-panel {
    position: absolute;
    bottom: 32px;
    left: 12px;
    width: 440px;
    max-height: 420px;
    background: var(--n, var(--s, #0d1117));
    border: 1px solid var(--b1, #30363d);
    border-radius: 10px;
    box-shadow: 0 12px 40px rgba(0,0,0,0.5);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: gpIn 0.15s ease;
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    cursor: default;
  }
  @keyframes gpIn { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: none; } }

  /* Tabs */
  .gp-tabs {
    display: flex;
    align-items: center;
    border-bottom: 1px solid var(--b1, #30363d);
    padding: 0 4px;
    flex-shrink: 0;
    overflow: visible;
    position: relative;
    z-index: 1;
  }
  .gp-tab {
    padding: 8px 10px;
    border: none;
    background: transparent;
    color: var(--t3, #8b949e);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    transition: all 0.1s;
    font-family: inherit;
  }
  .gp-tab.active { color: var(--acc, #7c5cf8); border-bottom-color: var(--acc, #7c5cf8); }
  .gp-tab:hover { color: var(--t1, #e6edf3); }

  .gp-tab-actions {
    margin-left: auto;
    display: flex;
    gap: 1px;
    padding-right: 4px;
  }

  /* Action buttons */
  .gp-action {
    border: none;
    background: transparent;
    color: var(--t3, #8b949e);
    cursor: pointer;
    padding: 3px 5px;
    border-radius: 4px;
    display: flex;
    align-items: center;
    transition: all 0.1s;
    position: relative;
  }
  .gp-action:hover:not(:disabled) { background: rgba(255,255,255,0.06); color: var(--t1, #e6edf3); }
  .gp-action:disabled { opacity: 0.4; cursor: wait; }
  .gp-tooltip {
    display: none;
    position: absolute;
    top: calc(100% + 6px);
    left: 50%;
    transform: translateX(-50%);
    background: var(--n, #1c2128);
    border: 1px solid var(--b1, #30363d);
    border-radius: 4px;
    padding: 3px 8px;
    font-size: 10px;
    color: var(--t1, #e6edf3);
    white-space: nowrap;
    z-index: 600;
    pointer-events: none;
  }
  .gp-action.has-tooltip:hover .gp-tooltip { display: block; }

  /* Status message */
  .gp-status {
    padding: 4px 10px;
    font-size: 10px;
    color: var(--t3, #8b949e);
    font-style: italic;
    border-bottom: 1px solid var(--b1, #30363d);
    animation: gpMsgIn 0.2s ease;
  }
  @keyframes gpMsgIn { from { opacity: 0; } to { opacity: 1; } }

  /* Diff view */
  .gp-diff-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--b1, #30363d);
    background: rgba(255,255,255,0.02);
    flex-shrink: 0;
  }
  .gp-diff-back {
    border: none;
    background: transparent;
    color: var(--t3, #8b949e);
    cursor: pointer;
    padding: 2px;
    border-radius: 3px;
    display: flex;
  }
  .gp-diff-back:hover { background: rgba(255,255,255,0.06); color: var(--t1, #e6edf3); }
  .gp-diff-filename { font-size: 11px; font-family: var(--mono, monospace); color: var(--t1, #e6edf3); }
  .gp-diff-view {
    max-height: 300px;
    overflow: auto;
    padding: 4px 0;
    font-family: var(--mono, monospace);
    font-size: 11px;
    line-height: 1.5;
    overscroll-behavior: contain;
  }
  .gp-diff-line { padding: 0 12px; white-space: pre; }
  .gp-diff-line.diff-add { background: rgba(63,185,80,0.12); color: #3fb950; }
  .gp-diff-line.diff-del { background: rgba(248,81,73,0.12); color: #f85149; }
  .gp-diff-line.diff-hunk { color: var(--acc, #7c5cf8); font-weight: 600; }

  /* Stage actions */
  .gp-stage-actions {
    display: flex;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid rgba(255,255,255,0.04);
    flex-shrink: 0;
  }
  .gp-stage-btn {
    padding: 3px 10px;
    border-radius: 4px;
    border: 1px solid var(--b1, #30363d);
    background: transparent;
    color: var(--t3, #8b949e);
    font-size: 10px;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.1s;
  }
  .gp-stage-btn:hover { background: rgba(255,255,255,0.06); color: var(--t1, #e6edf3); }

  /* File list */
  .gp-file-list { max-height: 220px; overflow-y: auto; padding: 4px 0; overscroll-behavior: contain; }
  .gp-file-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 14px;
    font-size: 11px;
    cursor: pointer;
    transition: background 0.08s;
  }
  .gp-file-item:hover { background: rgba(255,255,255,0.03); }
  .gp-file-stage {
    cursor: pointer;
    display: flex;
    align-items: center;
    flex-shrink: 0;
    padding: 1px;
    border-radius: 3px;
  }
  .gp-file-stage:hover { background: rgba(255,255,255,0.08); }
  .gp-file-status {
    width: 16px;
    font-size: 10px;
    font-weight: 700;
    font-family: var(--mono, monospace);
    text-align: center;
    flex-shrink: 0;
  }
  .gp-file-status.modified { color: #d29922; }
  .gp-file-status.added { color: #3fb950; }
  .gp-file-status.deleted { color: #f85149; }
  .gp-file-status.renamed { color: #58a6ff; }
  .gp-file-path {
    color: var(--t3, #8b949e);
    font-family: var(--mono, monospace);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Commit row */
  .gp-commit-row {
    display: flex;
    gap: 6px;
    padding: 6px 10px;
    border-top: 1px solid var(--b1, #30363d);
    flex-shrink: 0;
    align-items: flex-end;
  }
  .gp-commit-input {
    flex: 1;
    padding: 5px 8px;
    border-radius: 4px;
    border: 1px solid var(--b1, #30363d);
    background: transparent;
    color: var(--t1, #e6edf3);
    font-size: 11px;
    font-family: inherit;
    resize: none;
  }
  .gp-commit-input:focus { border-color: var(--acc, #7c5cf8); outline: none; }
  .gp-commit-input::placeholder { color: var(--t3, #8b949e); }
  .gp-commit-btn {
    padding: 5px 12px;
    border-radius: 4px;
    border: none;
    background: var(--acc, #7c5cf8);
    color: #fff;
    font-size: 11px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: opacity 0.1s;
    flex-shrink: 0;
    align-self: stretch;
  }
  .gp-commit-btn:hover:not(:disabled) { opacity: 0.85; }
  .gp-commit-btn:disabled { opacity: 0.4; cursor: default; }

  /* Empty state */
  .gp-empty { padding: 24px; text-align: center; font-size: 11px; color: var(--t3, #8b949e); }

  /* History list */
  .gp-history-list { max-height: 340px; overflow-y: auto; overscroll-behavior: contain; }
  .gp-commit-item {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 6px 12px;
    font-size: 11px;
    border-bottom: 1px solid rgba(255,255,255,0.03);
  }
  .gp-commit-item:hover { background: rgba(255,255,255,0.03); }
  .gp-commit-hash {
    font-family: var(--mono, monospace);
    color: var(--acc, #7c5cf8);
    font-weight: 600;
    flex-shrink: 0;
    font-size: 10px;
  }
  .gp-commit-message {
    color: var(--t1, #e6edf3);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .gp-commit-meta {
    color: var(--t4, var(--t3, #8b949e));
    flex-shrink: 0;
    font-size: 10px;
    white-space: nowrap;
  }

  /* Branches list */
  .gp-branch-list { max-height: 340px; overflow-y: auto; overscroll-behavior: contain; }
  .gp-branch-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    font-size: 12px;
    cursor: pointer;
    transition: background 0.1s;
  }
  .gp-branch-item:hover { background: rgba(255,255,255,0.04); }
  .gp-branch-item.current { color: var(--acc, #7c5cf8); cursor: default; }
  .gp-branch-item:not(.current) { color: var(--t3, #8b949e); }
  .gp-branch-name { font-family: var(--mono, monospace); font-size: 11px; }

  /* Confirmation dialog */
  .gp-confirm-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0,0,0,0.5);
    z-index: 500;
    display: flex;
    align-items: center;
    justify-content: center;
    animation: fadeIn 0.1s ease-out;
  }
  @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }
  .gp-confirm {
    background: var(--n, var(--modal-bg, #0d1117));
    border: 1px solid var(--b1, #30363d);
    border-radius: 10px;
    padding: 16px 20px;
    width: 320px;
    box-shadow: 0 12px 36px rgba(0,0,0,0.5);
    animation: gpIn 0.12s ease-out;
  }
  .gp-confirm-text {
    font-size: 13px;
    color: var(--t2, var(--t3, #8b949e));
    line-height: 1.5;
    margin: 0 0 14px;
  }
  .gp-confirm-actions { display: flex; gap: 8px; justify-content: flex-end; }
  .gp-confirm-cancel, .gp-confirm-ok {
    padding: 6px 14px;
    border-radius: 6px;
    font-size: 12px;
    font-family: inherit;
    cursor: pointer;
    border: none;
    font-weight: 500;
  }
  .gp-confirm-cancel {
    background: transparent;
    color: var(--t3, #8b949e);
    border: 1px solid var(--b1, #30363d);
  }
  .gp-confirm-cancel:hover { background: rgba(255,255,255,0.04); }
  .gp-confirm-ok { background: var(--acc, #7c5cf8); color: #fff; }
  .gp-confirm-ok:hover { filter: brightness(1.1); }
</style>

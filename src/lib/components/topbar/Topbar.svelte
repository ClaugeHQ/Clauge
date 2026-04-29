<script lang="ts">
  import { navOpen, aiPanelOpen, aiPanelOpenPerMode, mode } from '$lib/stores/app';
  import { tabs, activeTabId, addTab, closeTab, activateTab, getDraft, markClean, clearDraft } from '$lib/shared/stores/tabs';
  import { activeRequestId, loadRequest, clearActiveRequest, commitRequest } from '$lib/modes/rest/stores';
  import { sqlIsConnected, activeConnection, disconnectFromDb, initSqlTab, clearSqlTabData, setSqlTabData, sqlScripts, saveSqlScript, updateSqlScript, deleteSqlScript, getSqlTabData, activeConnectionId, selectedDatabase, connectToDatabase, sqlPendingChanges, connectToDb, connectedIds } from '$lib/modes/sql/stores';
  import { clearNoSqlTabData, initNoSqlTab, openNoSqlCollection, setNoSqlTabData, activeNoSqlConnectionId } from '$lib/modes/nosql/stores';
  import { showToast } from '$lib/components/shared/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { get } from 'svelte/store';
  import { onMount, onDestroy } from 'svelte';
  import EnvPill from './EnvPill.svelte';
  import { agentSessions, activeAgentSession, agentShellOpen, agentTerminalIds, agentShellIds } from '$lib/stores/agent';
  import { agentKillTerminal } from '$lib/commands/agent';
  import { sshProfiles, activeSshProfile, sshTerminalIds } from '$lib/modes/ssh/stores';
  import { sshKillTerminal } from '$lib/modes/ssh/commands';
  import { SSH_EVENT, AGENT_EVENT, APP_EVENT } from '$lib/shared/constants/events';

  // SQL disconnect
  async function handleSqlDisconnect() {
    const conn = get(activeConnection);
    if (!conn) return;
    try {
      await disconnectFromDb(conn.id);
      showToast('Disconnected', 'success');
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    }
  }

  // REST save prompt state
  let showCloseConfirm = $state(false);
  let closeConfirmTabId = $state(-1);


  const filteredTabs = $derived($tabs.filter(t => t.mode === $mode));

  function handleTabClick(tabId: number) {
    activateTab(tabId);
    const allTabs = get(tabs);
    const tab = allTabs.find(t => t.id === tabId);
    if (tab?.mode === 'rest') {
      if (tab.key) loadRequest(tab.key);
      else clearActiveRequest();
    }
    if (tab?.mode === 'agent' && tab.key) {
      const sessions = get(agentSessions);
      const session = sessions.find((s: any) => s.id === tab.key);
      if (session) activeAgentSession.set(session);
    }
    if (tab?.mode === 'ssh' && tab.key) {
      const profiles = get(sshProfiles);
      const profile = profiles.find((p) => p.id === tab.key);
      if (profile) activeSshProfile.set(profile);
    }
    // SQL/NoSQL tabs manage their own state via stores
  }

  function handleTabClose(e: MouseEvent, tabId: number) {
    e.stopPropagation();
    const allTabs = get(tabs);
    const tab = allTabs.find(t => t.id === tabId);
    if (!tab) return;

    if (tab.mode === 'agent') {
      closeConfirmTabId = tabId;
      showCloseConfirm = true;
      return;
    }

    if (tab.mode === 'ssh') {
      closeConfirmTabId = tabId;
      showCloseConfirm = true;
      return;
    }

    if (tab.mode === 'rest' && (tab.dirty || tab.unsaved)) {
      // REST: prompt save to collection
      closeConfirmTabId = tabId;
      showCloseConfirm = true;
    } else if (tab.mode === 'sql' && get(sqlPendingChanges).has(tabId)) {
      // SQL: has unsaved result edits
      closeConfirmTabId = tabId;
      showCloseConfirm = true;
    } else {
      doCloseTab(tabId);
    }
  }

  async function doCloseTab(tabId: number) {
    const allTabsBefore = get(tabs);
    const closingTab = allTabsBefore.find(t => t.id === tabId);

    // Auto-save SQL script on close
    if (closingTab?.mode === 'sql' && closingTab.key) {
      try {
        const tabData = getSqlTabData(tabId);
        await updateSqlScript(closingTab.key, closingTab.label, tabData.query, tabData.database);
      } catch (e) {
        console.error('Failed to auto-save SQL script:', e);
      }
    }

    // Clean up mode-specific state
    if (closingTab?.mode === 'sql') clearSqlTabData(tabId);
    if (closingTab?.mode === 'rest') clearDraft(tabId);
    if (closingTab?.mode === 'nosql') clearNoSqlTabData(tabId);

    if (closingTab?.mode === 'ssh' && closingTab.key) {
      // Kill SSH terminal (fire-and-forget)
      const sIds = get(sshTerminalIds);
      const termId = sIds.get(closingTab.key);
      if (termId) sshKillTerminal(termId).catch(() => {});

      // Let SshPanel clean up its xterm + maps via window event
      window.dispatchEvent(new CustomEvent(SSH_EVENT.CLOSE_TAB, { detail: { tabKey: closingTab.key } }));

      // Promote next SSH tab if any, else clear active profile
      const remaining = get(tabs).filter((t) => t.id !== tabId);
      const nextSshTab = remaining.find((t) => t.mode === 'ssh');
      if (nextSshTab?.key) {
        const profiles = get(sshProfiles);
        const profile = profiles.find((p) => p.id === nextSshTab.key);
        if (profile) activeSshProfile.set(profile);
      } else {
        activeSshProfile.set(null);
      }
    }

    if (closingTab?.mode === 'agent' && closingTab.key) {
      // Kill terminal + shell PTYs (fire-and-forget)
      const tIds = get(agentTerminalIds);
      const termId = tIds.get(closingTab.key);
      if (termId) agentKillTerminal(termId).catch(() => {});
      const sIds = get(agentShellIds);
      const shellId = sIds.get(closingTab.key);
      if (shellId) agentKillTerminal(shellId).catch(() => {});

      // Let AgentPanel clean up terminal entries
      window.dispatchEvent(new CustomEvent(AGENT_EVENT.CLOSE_TAB_SESSION, { detail: { sessionId: closingTab.key } }));

      const remaining = get(tabs).filter(t => t.id !== tabId);
      const nextAgentTab = remaining.find(t => t.mode === 'agent');
      if (nextAgentTab?.key) {
        const sessions = get(agentSessions);
        const session = sessions.find((s: any) => s.id === nextAgentTab.key);
        if (session) activeAgentSession.set(session);
      } else {
        agentShellOpen.set(false);
        activeAgentSession.set(null);
      }
    }

    closeTab(tabId);

    // After close, update REST state if needed
    const remaining = get(tabs);
    const currentTabId = get(activeTabId);
    const activeTab = remaining.find(t => t.id === currentTabId);
    if (activeTab?.mode === 'rest') {
      if (activeTab.key) loadRequest(activeTab.key);
      else clearActiveRequest();
    } else if (!activeTab) {
      clearActiveRequest();
    }
  }

  // REST-only save prompt handlers
  async function handleSaveAndClose() {
    const allTabs = get(tabs);
    const tab = allTabs.find(t => t.id === closeConfirmTabId);
    if (!tab) { showCloseConfirm = false; return; }

    if (tab.unsaved && !tab.key) {
      window.dispatchEvent(new CustomEvent(APP_EVENT.SAVE_NEW_REQUEST, { detail: { tabId: closeConfirmTabId } }));
    } else if (tab.dirty && tab.key) {
      const draft = getDraft(closeConfirmTabId);
      if (draft) {
        try {
          await commitRequest(tab.key, draft);
          markClean(closeConfirmTabId);
        } catch (err) {
          console.error('Failed to save:', err);
        }
      }
      doCloseTab(closeConfirmTabId);
    }
    showCloseConfirm = false;
  }

  function handleDiscardAndClose() {
    clearDraft(closeConfirmTabId);
    doCloseTab(closeConfirmTabId);
    showCloseConfirm = false;
  }

  // SQL script modal state
  let showSqlScriptModal = $state(false);
  let sqlScriptName = $state('');

  // "+" button
  function handleAddTab(btn?: HTMLElement) {
    const m = get(mode) as 'rest' | 'sql' | 'nosql' | 'agent' | 'ssh';
    if (m === 'ssh') {
      // Mirrors agent: no profiles → open create modal; otherwise show picker.
      // The +layout.svelte handler decides which based on profiles count.
      const rect = btn?.getBoundingClientRect();
      window.dispatchEvent(new CustomEvent(SSH_EVENT.ADD_TAB, { detail: { x: rect?.left ?? 290, y: rect?.bottom ?? 48 } }));
      return;
    }
    if (m === 'agent') {
      const rect = btn?.getBoundingClientRect();
      window.dispatchEvent(new CustomEvent(AGENT_EVENT.ADD_TAB, { detail: { x: rect?.left ?? 290, y: rect?.bottom ?? 48 } }));
      return;
    }
    if (m === 'sql') {
      sqlScriptName = '';
      showSqlScriptModal = true;
      return;
    }
    if (m === 'nosql') {
      const tab = addTab('New Query', 'nosql', null, 'var(--nosql)');
      initNoSqlTab(tab.id);
      return;
    }
    addTab('New Request', 'rest', null, 'var(--rest)');
    clearActiveRequest();
  }

  async function handleCreateSqlScript() {
    const name = sqlScriptName.trim() || 'Untitled Query';
    try {
      const connId = get(activeConnectionId) || null;
      const dbName = get(selectedDatabase) || '';
      const script = await saveSqlScript(name, connId, dbName, '');
      const tab = addTab(name, 'sql', script.id, 'var(--sql)');
      initSqlTab(tab.id);
    } catch (e) {
      console.error('Failed to save SQL script:', e);
      const tab = addTab(name, 'sql', null, 'var(--sql)');
      initSqlTab(tab.id);
    }
    showSqlScriptModal = false;
    sqlScriptName = '';
  }

  async function handleOpenScript(script: import('$lib/modes/sql/types').SqlScript) {
    // Check if already open in a tab
    const allTabs = get(tabs);
    const existing = allTabs.find(t => t.mode === 'sql' && t.key === script.id);
    if (existing) {
      activateTab(existing.id);
      showSqlScriptModal = false;
      return;
    }
    const tab = addTab(script.name, 'sql', script.id, 'var(--sql)');
    initSqlTab(tab.id);
    setSqlTabData(tab.id, { query: script.query, database: script.databaseName });
    showSqlScriptModal = false;

    // Ensure connection exists — connect main instance first if needed, then per-database
    if (script.connectionId && script.databaseName) {
      try {
        activeConnectionId.set(script.connectionId);
        selectedDatabase.set(script.databaseName);

        // Connect main instance if not already connected
        if (!get(connectedIds).has(script.connectionId)) {
          await connectToDb(script.connectionId);
        }

        // Connect to specific database
        await connectToDatabase(script.connectionId, script.databaseName);
      } catch {
        // Connection will be established when user selects from dropdown
      }
    }
  }

  async function handleDeleteScript(e: MouseEvent, scriptId: string) {
    e.stopPropagation();
    try {
      await deleteSqlScript(scriptId);
    } catch (err) {
      console.error('Failed to delete script:', err);
    }
  }

  function handleSqlScriptKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      handleCreateSqlScript();
    } else if (e.key === 'Escape') {
      showSqlScriptModal = false;
    }
  }

  // NoSQL: open collection in tab (triggered from nav)
  $effect(() => {
    const req = $openNoSqlCollection;
    if (!req) return;
    openNoSqlCollection.set(null);

    // Check if already open
    const allTabs = get(tabs);
    const key = `${req.connectionId}:${req.database}:${req.collection}`;
    const existing = allTabs.find(t => t.mode === 'nosql' && t.key === key);
    if (existing) {
      activateTab(existing.id);
      return;
    }

    const label = `${req.collection}`;
    const tab = addTab(label, 'nosql', key, 'var(--nosql)');
    setNoSqlTabData(tab.id, {
      connectionId: req.connectionId,
      database: req.database,
      collection: req.collection,
      filterQuery: '{}',
      sortQuery: '{}',
    });
  });

  // Shortcuts events
  function handleTabClosePromptEvent(e: Event) {
    const detail = (e as CustomEvent).detail;
    const tabId = detail?.tabId;
    if (tabId === undefined) return;
    const allTabs = get(tabs);
    const tab = allTabs.find(t => t.id === tabId);
    if (tab?.mode === 'agent') {
      closeConfirmTabId = tabId;
      showCloseConfirm = true;
    } else if (tab?.mode === 'ssh') {
      closeConfirmTabId = tabId;
      showCloseConfirm = true;
    } else if (tab?.mode === 'rest' && (tab.dirty || tab.unsaved)) {
      closeConfirmTabId = tabId;
      showCloseConfirm = true;
    } else {
      doCloseTab(tabId);
    }
  }

  onMount(() => {
    window.addEventListener(APP_EVENT.TAB_CLOSE_PROMPT, handleTabClosePromptEvent);
  });
  onDestroy(() => {
    window.removeEventListener(APP_EVENT.TAB_CLOSE_PROMPT, handleTabClosePromptEvent);
  });
</script>

<header class="topbar">
  {#if $mode !== 'history'}
    <div class="tabs">
      {#each filteredTabs as tab (tab.id)}
        <button
          class="tab"
          class:on={$activeTabId === tab.id}
          onclick={() => handleTabClick(tab.id)}
        >
          {#if tab.mode === 'rest' && (tab.dirty || tab.unsaved)}
            <span class="dirty-dot"></span>
          {/if}
          {#if tab.mode === 'agent'}
            <img src="/code-no-action.svg" alt="" class="tab-agent-icon" />
          {/if}
          <span class="td" style="background:{tab.dot}"></span>
          <span class="tab-label">{tab.label}</span>
          <span
            class="tab-close"
            onclick={(e: MouseEvent) => { if (e.detail < 2) handleTabClose(e, tab.id); }}
            role="button"
            tabindex="-1"
          >&times;</span>
        </button>
      {/each}
    </div>

    <button class="tab-add" title="New tab" onclick={(e) => { handleAddTab(e.currentTarget as HTMLElement); }}>+</button>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="drag-spacer" data-drag-region></div>

  <div class="tbar-right">
    {#if $mode === 'rest'}
      <EnvPill />
    {/if}
    {#if $mode !== 'agent'}
      <button class="ai-toggle-btn" class:active={$aiPanelOpen} onclick={() => { aiPanelOpen.update(v => { const next = !v; aiPanelOpenPerMode.update(m => ({ ...m, [$mode]: next })); return next; }); }} title="AI Assistant">
        <svg viewBox="0 0 24 24"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/></svg>
      </button>
    {/if}
  </div>
</header>

<!-- SQL Script modal -->
{#if showSqlScriptModal}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="confirm-overlay" onclick={() => showSqlScriptModal = false}>
    <div class="sql-script-modal" onclick={(e: MouseEvent) => e.stopPropagation()}>
      <div class="ssm-title">SQL Script</div>
      <div class="ssm-section">
        <label class="ssm-label">Script Name</label>
        <input
          class="ssm-input"
          type="text"
          placeholder="Untitled Query"
          bind:value={sqlScriptName}
          onkeydown={handleSqlScriptKeydown}
        />
        <button class="ssm-btn primary" onclick={handleCreateSqlScript}>
          New Script
        </button>
      </div>
      {#if $sqlScripts.length > 0}
        <div class="ssm-divider"></div>
        <div class="ssm-section">
          <label class="ssm-label">Open Existing</label>
          <div class="ssm-list">
            {#each $sqlScripts as script (script.id)}
              <button class="ssm-list-item" onclick={() => handleOpenScript(script)}>
                <svg viewBox="0 0 24 24"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><path d="M14 2v6h6"/></svg>
                <span class="ssm-item-info">
                  <span class="ssm-item-name">{script.name}</span>
                  <span class="ssm-item-meta">{script.databaseName || 'No database'} &middot; {new Date(script.updatedAt).toLocaleDateString()}</span>
                </span>
                <span
                  class="ssm-item-delete"
                  onclick={(e: MouseEvent) => handleDeleteScript(e, script.id)}
                  role="button"
                  tabindex="-1"
                >&times;</span>
              </button>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<!-- REST-only close confirmation -->
{#if showCloseConfirm}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="confirm-overlay" onclick={() => showCloseConfirm = false}>
    <div class="confirm-dialog" onclick={(e: MouseEvent) => e.stopPropagation()}>
      {#if $tabs.find(t => t.id === closeConfirmTabId)?.mode === 'agent'}
        <div class="confirm-title">Close this tab?</div>
        <div class="confirm-body">This agent session tab will be closed.</div>
        <div class="confirm-actions">
          <button class="confirm-btn discard" onclick={() => showCloseConfirm = false}>Cancel</button>
          <button class="confirm-btn save" onclick={handleDiscardAndClose}>Close</button>
        </div>
      {:else if $tabs.find(t => t.id === closeConfirmTabId)?.mode === 'ssh'}
        <div class="confirm-title">Disconnect SSH session?</div>
        <div class="confirm-body">This will close the connection and the tab.</div>
        <div class="confirm-actions">
          <button class="confirm-btn discard" onclick={() => showCloseConfirm = false}>Cancel</button>
          <button class="confirm-btn save" onclick={handleDiscardAndClose}>Disconnect</button>
        </div>
      {:else}
        <div class="confirm-title">Unsaved Changes</div>
        <div class="confirm-body">Do you want to save changes before closing?</div>
        <div class="confirm-actions">
          <button class="confirm-btn discard" onclick={handleDiscardAndClose}>Don't Save</button>
          <button class="confirm-btn save" onclick={handleSaveAndClose}>Save</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .topbar {
    height: 46px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    display: flex;
    align-items: center;
    padding: 0 12px;
    -webkit-app-region: drag;
  }
  .tabs {
    display: flex;
    align-items: center;
    -webkit-app-region: no-drag;
    gap: 4px;
    height: 100%;
    overflow-x: auto;
    padding: 0 4px;
    flex-shrink: 1;
    min-width: 0;
  }
  .tabs::-webkit-scrollbar { display: none; }
  .tab {
    height: 30px;
    padding: 6px 14px;
    border-radius: 7px;
    border: none;
    background: transparent;
    color: var(--t3);
    font-size: 12.5px;
    font-family: var(--mono);
    cursor: default;
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    transition: background 0.08s, color 0.08s;
    flex-shrink: 0;
    -webkit-app-region: no-drag;
  }
  .tab:hover { color: var(--t2); }
  .tab.on {
    background: rgba(255,255,255,0.06);
    color: var(--t1);
  }
  .tab .td {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .tab-agent-icon {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
    opacity: 0.7;
  }
  .dirty-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--acc);
    flex-shrink: 0;
    margin-right: -2px;
  }
  .tab-label {
    max-width: 150px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab-close {
    font-size: 14px;
    color: var(--t3);
    cursor: default;
    opacity: 0;
    transition: opacity 0.1s, color 0.1s;
    line-height: 1;
  }
  .tab:hover .tab-close { opacity: 1; }
  .tab-close:hover { color: var(--t1); }

  .tab-add-wrap {
    position: relative;
    -webkit-app-region: no-drag;
  }
  .tab-add {
    height: 34px;
    width: 32px;
    border: none;
    background: transparent;
    color: var(--t3);
    font-size: 20px;
    cursor: default;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    transition: background 0.1s, color 0.1s;
    flex-shrink: 0;
    -webkit-app-region: no-drag;
  }
  .tab-add:hover {
    background: rgba(255,255,255,0.04);
    color: var(--t1);
  }

  .sql-dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    background: var(--modal-bg, #101016);
    border: 1px solid var(--b1);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0,0,0,0.4);
    z-index: 100;
    min-width: 180px;
    padding: 4px;
    animation: dropIn 0.12s ease;
  }
  @keyframes dropIn {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: none; }
  }
  .sql-dropdown-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border: none;
    background: transparent;
    color: var(--t1);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    border-radius: 5px;
    transition: background 0.08s;
  }
  .sql-dropdown-item:hover {
    background: rgba(255,255,255,0.06);
  }
  .sql-dropdown-item svg {
    width: 14px;
    height: 14px;
    stroke: var(--t2);
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex-shrink: 0;
  }

  .drag-spacer {
    flex: 1;
    height: 100%;
    min-width: 40px;
  }
  .tbar-right {
    display: flex;
    align-items: center;
    gap: 6px;
    -webkit-app-region: no-drag;
    flex-shrink: 0;
  }
  .sql-disconnect-btn {
    width: 30px;
    height: 30px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: default;
    transition: border-color 0.15s;
    flex-shrink: 0;
    -webkit-app-region: no-drag;
  }
  .sql-disconnect-btn:hover { border-color: var(--err); }
  .sql-disconnect-btn svg {
    width: 14px;
    height: 14px;
    stroke: var(--t2);
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
  }
  .sql-disconnect-btn:hover svg { stroke: var(--err); }
  .ai-toggle-btn {
    width: 30px;
    height: 30px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: default;
    transition: border-color 0.15s, background 0.15s;
    flex-shrink: 0;
    -webkit-app-region: no-drag;
  }
  .ai-toggle-btn:hover { border-color: var(--b2); background: var(--n2); }
  .ai-toggle-btn.active { border-color: var(--acc); background: var(--n2); }
  .ai-toggle-btn svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: var(--t2);
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .ai-toggle-btn.active svg { stroke: var(--acc); }

  /* REST-only confirmation dialog */
  .confirm-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0,0,0,0.4);
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .confirm-dialog {
    background: var(--modal-bg, #101016);
    border: 1px solid var(--b1);
    border-radius: 10px;
    box-shadow: 0 16px 48px rgba(0,0,0,0.5);
    width: 340px;
    padding: 20px;
  }
  .confirm-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
    margin-bottom: 8px;
  }
  .confirm-body {
    font-size: 12px;
    color: var(--t2);
    font-family: var(--ui);
    margin-bottom: 16px;
  }
  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .confirm-btn {
    height: 30px;
    padding: 0 14px;
    border-radius: 6px;
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
  }
  .confirm-btn.discard {
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
  }
  .confirm-btn.discard:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .confirm-btn.save {
    border: none;
    background: var(--acc);
    color: #fff;
    font-weight: 600;
  }
  .confirm-btn.save:hover { opacity: 0.85; }

  /* SQL Script Modal */
  .sql-script-modal {
    background: var(--modal-bg, #101016);
    border: 1px solid var(--b1);
    border-radius: 10px;
    box-shadow: 0 16px 48px rgba(0,0,0,0.5);
    width: 360px;
    padding: 20px;
    animation: dropIn 0.15s ease;
  }
  @keyframes dropIn {
    from { opacity: 0; transform: translateY(-8px); }
    to { opacity: 1; transform: none; }
  }
  .ssm-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
    margin-bottom: 16px;
  }
  .ssm-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .ssm-label {
    font-size: 10px;
    font-weight: 600;
    color: var(--t3);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .ssm-input {
    height: 32px;
    background: transparent;
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 0 10px;
    font-size: 12px;
    font-family: var(--mono);
    color: var(--t1);
    outline: none;
  }
  .ssm-input:focus {
    border-color: var(--acc);
  }
  .ssm-input::placeholder {
    color: var(--t4);
  }
  .ssm-btn {
    height: 30px;
    border-radius: 6px;
    font-size: 12px;
    font-family: var(--ui);
    font-weight: 600;
    cursor: default;
  }
  .ssm-btn.primary {
    border: none;
    background: var(--acc);
    color: #fff;
  }
  .ssm-btn.primary:hover { opacity: 0.85; }
  .ssm-divider {
    height: 1px;
    background: var(--b1);
    margin: 16px 0;
  }
  .ssm-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 200px;
    overflow-y: auto;
  }
  .ssm-list-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border: none;
    background: transparent;
    color: var(--t1);
    font-size: 12px;
    font-family: var(--mono);
    cursor: default;
    border-radius: 5px;
    transition: background 0.08s;
  }
  .ssm-list-item:hover {
    background: rgba(255,255,255,0.06);
  }
  .ssm-list-item svg {
    width: 14px;
    height: 14px;
    stroke: var(--acc);
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
    flex-shrink: 0;
  }
  .ssm-item-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
    text-align: left;
  }
  .ssm-item-name {
    font-size: 12px;
    color: var(--t1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ssm-item-meta {
    font-size: 10px;
    color: var(--t3);
    font-family: var(--ui);
  }
  .ssm-item-delete {
    font-size: 16px;
    color: var(--t3);
    cursor: default;
    opacity: 0;
    transition: opacity 0.1s, color 0.1s;
    line-height: 1;
    flex-shrink: 0;
    padding: 0 2px;
  }
  .ssm-list-item:hover .ssm-item-delete { opacity: 1; }
  .ssm-item-delete:hover { color: var(--err); }
</style>

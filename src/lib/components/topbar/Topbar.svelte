<script lang="ts">
  import { navOpen, aiPanelOpen, aiPanelOpenPerMode, mode, setMode } from '$lib/stores/app';
  import { tabs, activeTabId, addTab, closeTab, activateTab, getDraft, markClean, clearDraft } from '$lib/shared/stores/tabs';
  import { activeRequestId, loadRequest, clearActiveRequest, commitRequest } from '$lib/modes/rest/stores';
  import { sqlIsConnected, activeConnection, disconnectFromDb, initSqlTab, clearSqlTabData, setSqlTabData, sqlScripts, saveSqlScript, updateSqlScript, deleteSqlScript, getSqlTabData, activeConnectionId, selectedDatabase, connectToDatabase, sqlPendingChanges, connectToDb, connectedIds, connections, loadConnections } from '$lib/modes/sql/stores';
  import { clearNoSqlTabData, initNoSqlTab, openNoSqlCollection, setNoSqlTabData, activeNoSqlConnectionId, connectedNoSqlIds, nosqlConnections } from '$lib/modes/nosql/stores';
  import { showToast } from '$lib/shared/primitives/toast';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import { friendlyError } from '$lib/utils/errors';
  import { get } from 'svelte/store';
  import { onMount, onDestroy } from 'svelte';
  import EnvPill from './EnvPill.svelte';
  import { agentSessions, activeAgentSession, agentShellOpen, agentTerminalIds, agentShellIds } from '$lib/modes/agent/stores';
  import { agentKillTerminal } from '$lib/modes/agent/commands';
  import { sshProfiles, activeSshProfile, sshTerminalIds, sshConnStates } from '$lib/modes/ssh/stores';
  import { sshKillTerminal } from '$lib/modes/ssh/commands';
  import { profileIdFromTabKey } from '$lib/modes/ssh/tabkey';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import { SSH_EVENT, AGENT_EVENT, APP_EVENT, WORKSPACE_EVENT } from '$lib/shared/constants/events';
  import { activateTabAcrossMode } from '$lib/utils/tabActivation';

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
  let closeConfirmKeepMode = $state(false);

  /** DOM ref to the scrolling tabs container. Used by the effect below
   *  to auto-scroll a newly-created or newly-activated tab into view
   *  when the strip overflows. Without this, opening a new tab via "+"
   *  while the strip is full appends the tab off-screen and the user
   *  has to scroll right to find it. */
  let tabsContainer: HTMLDivElement | undefined = $state();

  // Topbar shows ALL tabs across modes. Click flips mode + activates +
  // runs mode-specific side effects via the shared helper.
  const filteredTabs = $derived($tabs);

  // Auto-scroll the active tab into view whenever:
  //   • a new tab is created (addTab activates it → $activeTabId changes)
  //   • the user clicks a tab that's currently off-screen
  //   • a tab is closed and another becomes active off-screen
  // Uses scrollIntoView with `inline: 'nearest'` so the container only
  // scrolls when the tab isn't already visible — no jitter for tabs
  // already in view. requestAnimationFrame defers until after Svelte
  // has painted the new tab DOM, otherwise querySelector would miss it.
  $effect(() => {
    // Track both signals so a focus change OR a tab list change retriggers.
    void $activeTabId;
    void filteredTabs.length;
    if (!tabsContainer) return;
    requestAnimationFrame(() => {
      const active = tabsContainer?.querySelector('.tab.on') as HTMLElement | null;
      if (!active) return;
      active.scrollIntoView({ inline: 'nearest', block: 'nearest', behavior: 'smooth' });
    });
  });

  function handleTabClick(tabId: number) {
    activateTabAcrossMode(tabId);
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

    if (tab.mode === 'explorer') {
      // Closing an explorer tab tears down the Rust-side session (see
      // doCloseTab) — same kill-on-close model as SSH/Agent, so prompt
      // the same way. SQL/NoSQL deliberately don't prompt because their
      // sessions outlive tabs.
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
        // Persist the tab's CURRENT binding alongside query/name so
        // the script reopens at the same (connection, database) the
        // user left it on. Backend COALESCE keeps them atomic — they
        // can never get out of sync from this path.
        await updateSqlScript(
          closingTab.key,
          closingTab.label,
          tabData.query,
          tabData.binding?.database,
          tabData.binding?.connectionId,
        );
      } catch (e) {
        console.error('Failed to auto-save SQL script:', e);
      }
    }

    // Clean up mode-specific state
    if (closingTab?.mode === 'sql') clearSqlTabData(tabId);
    if (closingTab?.mode === 'rest') clearDraft(tabId);
    if (closingTab?.mode === 'nosql') clearNoSqlTabData(tabId);
    if (closingTab?.mode === 'history') {
      const { clearHistoryEntryForTab } = await import('$lib/modes/rest/stores');
      clearHistoryEntryForTab(tabId);
    }
    if (closingTab?.mode === 'explorer' && closingTab.key) {
      // Tear down the Rust-side session for this Explorer tab.
      // Fire-and-forget — the user is closing the tab regardless of result.
      import('$lib/modes/explorer/commands').then(({ closeSession }) => {
        closeSession(closingTab.key as string).catch(() => { /* ignore */ });
      });
    }

    if (closingTab?.mode === 'ssh' && closingTab.key) {
      // Kill SSH terminal (fire-and-forget)
      const sIds = get(sshTerminalIds);
      const termId = sIds.get(closingTab.key);
      if (termId) sshKillTerminal(termId).catch(() => {});

      // Let SshPanel clean up its xterm + maps via window event
      window.dispatchEvent(new CustomEvent(SSH_EVENT.CLOSE_TAB, { detail: { tabKey: closingTab.key } }));

      // If no SSH tabs will remain, clear active profile so the panel
      // resets cleanly. When a sibling SSH tab remains, closeTab below
      // promotes it and SshPanel's activeTabId subscriber sets the
      // active profile from there.
      const remaining = get(tabs).filter((t) => t.id !== tabId);
      const anySshLeft = remaining.some((t) => t.mode === 'ssh');
      if (!anySshLeft) activeSshProfile.set(null);
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

      const remaining = get(tabs).filter((t) => t.id !== tabId);
      const anyAgentLeft = remaining.some((t) => t.mode === 'agent');
      if (!anyAgentLeft) {
        agentShellOpen.set(false);
        activeAgentSession.set(null);
      }
    }

    closeTab(tabId);

    // After close, realign $mode + run side effects if the new active
    // tab is in a different mode than current $mode. closeTab prefers
    // a same-mode sibling, so this only fires when the closed tab was
    // the last of its mode — at which point we cross over to whatever
    // tab took its place. activateTabAcrossMode handles REST loadRequest
    // / Agent activeAgentSession; SSH/SQL/NoSQL/Explorer panels self-heal
    // via their own activeTabId subscribers.
    //
    // Skip realignment when the close was triggered from Canvas — the
    // user is already on Canvas and should stay there.
    if (!closeConfirmKeepMode) {
      const newActiveId = get(activeTabId);
      if (newActiveId === -1) {
        closeConfirmKeepMode = false;
        clearActiveRequest();
        return;
      }
      const newActive = get(tabs).find(t => t.id === newActiveId);
      if (newActive && newActive.mode !== get(mode)) {
        activateTabAcrossMode(newActive.id);
      } else if (newActive?.mode === 'rest') {
        // Same-mode REST close: ensure the editor loads the new active
        // request (closeTab doesn't run side effects on its own).
        if (newActive.key) loadRequest(newActive.key);
        else clearActiveRequest();
      } else if (newActive?.mode === 'agent' && newActive.key) {
        // Same-mode agent close: switch the active session to the promoted tab.
        // closeTab only updates the tab bar; activeAgentSession must be set
        // explicitly or the panel stays blank until the user clicks the tab.
        const sessions = get(agentSessions);
        const nextSession = sessions.find(s => s.id === newActive.key);
        if (nextSession) activeAgentSession.set(nextSession);
      }
    }
    closeConfirmKeepMode = false;  // Always reset.
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

  /** Prompt copy + button labels for the tab-close ConfirmDialog. Driven
   *  off the closing tab's mode so SSH/Agent/Explorer (sessions die with
   *  the tab) share a "Disconnect" dialog while REST/SQL (unsaved data)
   *  show a 3-button "Don't Save / Cancel / Save" variant. */
  const closeConfirmProps = $derived.by(() => {
    const tab = $tabs.find((t) => t.id === closeConfirmTabId);
    const mode = tab?.mode;
    if (mode === 'agent') {
      return { title: 'Close this tab?', message: 'This agent session tab will be closed.', confirmText: 'Close', confirmColor: 'var(--acc)', discardText: undefined as string | undefined };
    }
    if (mode === 'ssh') {
      return { title: 'Disconnect SSH session?', message: 'This will close the connection and the tab.', confirmText: 'Disconnect', confirmColor: 'var(--acc)', discardText: undefined as string | undefined };
    }
    if (mode === 'explorer') {
      return { title: 'Close file browser tab?', message: 'The remote connection for this tab will be closed.', confirmText: 'Disconnect', confirmColor: 'var(--acc)', discardText: undefined as string | undefined };
    }
    // REST / SQL "save before close" — 3-button.
    return { title: 'Unsaved changes', message: 'Do you want to save changes before closing?', confirmText: 'Save', confirmColor: 'var(--acc)', discardText: "Don't Save" as string | undefined };
  });

  // SQL script modal state
  let showSqlScriptModal = $state(false);
  let sqlScriptName = $state('');

  // Right-click menu on a tab. Currently only SSH tabs have an entry
  // ("Duplicate Session"); other modes get no menu (just suppress the
  // browser default context menu).
  function handleTabContextMenu(e: MouseEvent, tab: { id: number; mode: string; key: string | null }) {
    e.preventDefault();
    if (tab.mode !== 'ssh' || !tab.key) return;
    const profileId = profileIdFromTabKey(tab.key);
    const profile = get(sshProfiles).find((p) => p.id === profileId);
    if (!profile) return;
    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Duplicate Session',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>',
        action: () => {
          window.dispatchEvent(new CustomEvent(SSH_EVENT.DUPLICATE_SESSION, { detail: profile }));
        },
      },
    ]);
  }

  // "+" button
  function handleAddTab(btn?: HTMLElement) {
    const m = get(mode) as 'rest' | 'sql' | 'nosql' | 'agent' | 'ssh' | 'explorer' | 'workspace';
    if (m === 'workspace') {
      const rect = btn?.getBoundingClientRect();
      window.dispatchEvent(new CustomEvent(WORKSPACE_EVENT.ADD_TAB, { detail: { x: rect?.left ?? 290, y: rect?.bottom ?? 48 } }));
      return;
    }
    if (m === 'ssh') {
      // Mirrors agent: no profiles → open create modal; otherwise show picker.
      // The +layout.svelte handler decides which based on profiles count.
      const rect = btn?.getBoundingClientRect();
      window.dispatchEvent(new CustomEvent(SSH_EVENT.ADD_TAB, { detail: { x: rect?.left ?? 290, y: rect?.bottom ?? 48 } }));
      return;
    }
    if (m === 'explorer') {
      // Same shape as SSH: no connections → kind picker; otherwise show
      // the connections picker. +layout.svelte makes the call.
      const rect = btn?.getBoundingClientRect();
      window.dispatchEvent(new CustomEvent('explorer:add-tab', { detail: { x: rect?.left ?? 290, y: rect?.bottom ?? 48 } }));
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
      if (get(connectedNoSqlIds).size === 0) {
        showToast('Connect to a database first', 'info');
        return;
      }
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
      // Brand-new scripts start UNBOUND. The user picks the target
      // connection + database from the pill on the new tab; their
      // selection is then atomically persisted via the autosave path
      // (which writes connection_id and database_name together).
      const script = await saveSqlScript(name, null, '', '');
      const tab = addTab(name, 'sql', script.id, 'var(--sql)');
      setSqlTabData(tab.id, {
        binding: null,
        query: '',
        results: [],
        activeResultIdx: 0,
        inFlight: null,
        error: null,
        result: null,
      });
    } catch (e) {
      console.error('Failed to save SQL script:', e);
      const tab = addTab(name, 'sql', null, 'var(--sql)');
      initSqlTab(tab.id);
    }
    showSqlScriptModal = false;
    sqlScriptName = '';
  }

  async function handleOpenScript(script: import('$lib/modes/sql/types').SqlScript) {
    // Already open? Activate it (and switch mode if needed).
    const allTabs = get(tabs);
    const existing = allTabs.find(t => t.mode === 'sql' && t.key === script.id);
    if (existing) {
      await activateTabAcrossMode(existing.id);
      showSqlScriptModal = false;
      return;
    }

    // Create the tab; activeTabId flips inside addTab.
    const tab = addTab(script.name, 'sql', script.id, 'var(--sql)');
    showSqlScriptModal = false;
    await setMode('sql');

    // Resolve binding:
    //   - If the script's saved connection still exists, bind to its
    //     `(connId, db)`.
    //   - If the script's connection has been deleted, fall back to the
    //     first available saved connection's default DB and toast.
    //   - If there are zero saved connections, leave binding null and
    //     show the unbound banner in SqlPanel.
    await loadConnections();
    const allConns = get(connections);

    // Resolve the binding:
    //
    //   1. Script intentionally has no connection_id (new scripts are
    //      created unbound) → open unbound, user picks from pill.
    //   2. Script has a connection_id AND that connection still exists →
    //      bind to (script.connectionId, script.databaseName).
    //   3. Script has a connection_id but that connection was DELETED →
    //      toast + fall back to first available connection.
    //   4. Script has a connection_id, connection is gone, and no other
    //      connections exist → leave unbound.
    let nextBinding: { connectionId: string; database: string } | null = null;
    if (!script.connectionId) {
      // Case 1 — intentionally unbound. No toast.
      nextBinding = null;
    } else {
      const targetConn = allConns.find(c => c.id === script.connectionId);
      if (targetConn) {
        // Case 2.
        nextBinding = { connectionId: targetConn.id, database: script.databaseName };
      } else if (allConns.length > 0) {
        // Case 3 — saved conn was deleted. Toast + fallback.
        const fb = allConns[0];
        nextBinding = { connectionId: fb.id, database: fb.databaseName };
        showToast(
          `Original connection deleted — switched to ${fb.name} / ${fb.databaseName}.`,
          'info',
        );
      }
      // Case 4 falls through with nextBinding still null.
    }

    setSqlTabData(tab.id, {
      binding: nextBinding,
      query: script.query,
      results: [],
      activeResultIdx: 0,
      inFlight: null,
      error: null,
      result: null,
    });
    // SqlPanel's binding-watcher effect triggers ensureConnected on its own;
    // the loader appears in the editor area while the pool opens.
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

  // NoSQL: open collection (Mongo) or connection viewer (Redis) in a tab.
  // Mongo requests include database+collection; Redis sends connectionId only.
  $effect(() => {
    const req = $openNoSqlCollection;
    if (!req) return;
    openNoSqlCollection.set(null);

    const allTabs = get(tabs);
    const isRedisReq = !req.database && !req.collection;
    const key = isRedisReq
      ? `${req.connectionId}:__redis__`
      : `${req.connectionId}:${req.database}:${req.collection}`;
    const existing = allTabs.find(t => t.mode === 'nosql' && t.key === key);
    if (existing) {
      activateTab(existing.id);
      return;
    }

    let label: string;
    if (isRedisReq) {
      const conn = get(nosqlConnections).find(c => c.id === req.connectionId);
      label = conn?.name ?? 'Redis';
    } else {
      label = `${req.collection}`;
    }
    const tab = addTab(label, 'nosql', key, 'var(--nosql)');
    setNoSqlTabData(tab.id, {
      connectionId: req.connectionId,
      database: req.database ?? '',
      collection: req.collection ?? '',
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
    if (tab?.mode === 'agent' || tab?.mode === 'ssh' || tab?.mode === 'explorer') {
      // Modes whose remote sessions die when the tab closes — always prompt.
      closeConfirmTabId = tabId;
      showCloseConfirm = true;
    } else if (tab?.mode === 'rest' && (tab.dirty || tab.unsaved)) {
      closeConfirmTabId = tabId;
      showCloseConfirm = true;
    } else {
      doCloseTab(tabId);
    }
  }

  function handleNewTabShortcut() {
    // No anchor element: pickers/popovers fall back to the default x/y
    // already baked into handleAddTab for that branch.
    handleAddTab();
  }

  const onCanvasCloseRequest = (e: Event) => {
    const tabId = (e as CustomEvent<{ tabId: number }>).detail?.tabId;
    if (typeof tabId !== 'number') return;
    closeConfirmKeepMode = true;  // Skip mode realignment — user is on Canvas.
    const synthetic = new MouseEvent('click');
    handleTabClose(synthetic, tabId);
  };

  onMount(() => {
    window.addEventListener(APP_EVENT.TAB_CLOSE_PROMPT, handleTabClosePromptEvent);
    window.addEventListener(APP_EVENT.NEW_TAB, handleNewTabShortcut);
    window.addEventListener('canvas:request-tab-close', onCanvasCloseRequest);
  });
  onDestroy(() => {
    window.removeEventListener(APP_EVENT.TAB_CLOSE_PROMPT, handleTabClosePromptEvent);
    window.removeEventListener(APP_EVENT.NEW_TAB, handleNewTabShortcut);
    window.removeEventListener('canvas:request-tab-close', onCanvasCloseRequest);
  });
</script>

<header class="topbar">
  <div class="tabs" bind:this={tabsContainer}>
    {#each filteredTabs as tab (tab.id)}
      <button
        class="tab"
        class:on={$activeTabId === tab.id && (tab.mode === $mode || tab.mode === 'settings')}
        class:tab-dirty={tab.mode === 'rest' && (tab.dirty || tab.unsaved)}
        onclick={() => handleTabClick(tab.id)}
        oncontextmenu={(e: MouseEvent) => handleTabContextMenu(e, tab)}
      >
        {#if tab.mode === 'agent'}
          {@const _sess = $agentSessions.find((s) => s.id === tab.key)}
          {@const _prov = _sess?.provider ?? 'claude'}
          {#if _prov === 'codex'}
            <img src="/codex.svg" alt="" class="tab-agent-icon" />
          {:else if _prov === 'gemini'}
            <img src="/gemini.svg" alt="" class="tab-agent-icon" />
          {:else if _prov === 'opencode'}
            <img src="/opencode-dark.svg" alt="" class="tab-agent-icon" />
          {:else}
            <img src="/code-no-action.svg" alt="" class="tab-agent-icon" />
          {/if}
        {:else if tab.mode === 'rest'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z"/></svg>
        {:else if tab.mode === 'sql'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="8" ry="2.5"/><path d="M4 5v14c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5V5"/><path d="M4 12c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5"/></svg>
        {:else if tab.mode === 'nosql'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3a2 2 0 00-2 2v4a2 2 0 01-2 2H3a1 1 0 000 2h1a2 2 0 012 2v4a2 2 0 002 2"/><path d="M16 3a2 2 0 012 2v4a2 2 0 002 2h1a1 1 0 010 2h-1a2 2 0 00-2 2v4a2 2 0 01-2 2"/></svg>
        {:else if tab.mode === 'ssh'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>
        {:else if tab.mode === 'explorer'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>
        {:else if tab.mode === 'history'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
        {:else if tab.mode === 'workspace' && tab.key?.startsWith('board:')}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="6" height="16" rx="1"/><rect x="11" y="4" width="6" height="10" rx="1"/><rect x="19" y="4" width="2" height="14" rx="1"/></svg>
        {:else if tab.mode === 'workspace' && tab.key === 'inbox'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11L2 12v6a2 2 0 002 2h16a2 2 0 002-2v-6l-3.45-6.89A2 2 0 0016.76 4H7.24a2 2 0 00-1.79 1.11z"/></svg>
        {:else if tab.mode === 'workspace' && tab.key === 'coworkers'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="9" cy="8" r="3.2"/><path d="M2.5 19a6.5 6.5 0 0113 0"/><circle cx="17" cy="6" r="2.4"/><path d="M14 13a4.5 4.5 0 018.5 2"/></svg>
        {:else if tab.mode === 'workspace'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H6a2 2 0 00-2 2v14a2 2 0 002 2h12a2 2 0 002-2V9z"/><polyline points="14 3 14 9 20 9"/></svg>
        {:else if tab.mode === 'settings'}
          <svg class="tab-mode-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 01-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06a1.65 1.65 0 00.33-1.82 1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06a1.65 1.65 0 001.82.33H9a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06a1.65 1.65 0 00-.33 1.82V9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/></svg>
        {/if}
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

  {#if $mode !== 'history'}
    <button
      class="tab-add"
      title="New tab"
      onclick={(e) => {
        // stopPropagation: same click event reaches the global
        // ContextMenu's window.click listener and immediately closes
        // the menu we're about to open. Symptom: + opens the menu
        // once, then "becomes unresponsive" after the user dismisses
        // it by clicking outside. Killing propagation here keeps
        // showContextMenu's open state intact across re-clicks.
        e.stopPropagation();
        handleAddTab(e.currentTarget as HTMLElement);
      }}
    >+</button>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="drag-spacer" data-drag-region></div>

  <div class="tbar-right">
    {#if $mode === 'rest'}
      <EnvPill />
    {/if}
    {#if $mode !== 'agent' && $mode !== 'workspace' && $mode !== 'history' && $mode !== 'canvas'}
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

<!-- Tab-close confirmation — single primitive for every mode. Props are
     computed reactively from the closing tab's mode (see `closeConfirmProps`).
     Replaces four bespoke branches that diverged from the manual-disconnect
     dialog used elsewhere; now everything reads identically. -->
<ConfirmDialog
  bind:show={showCloseConfirm}
  title={closeConfirmProps.title}
  message={closeConfirmProps.message}
  confirmText={closeConfirmProps.confirmText}
  confirmColor={closeConfirmProps.confirmColor}
  discardText={closeConfirmProps.discardText}
  onconfirm={() => {
    if (closeConfirmProps.discardText) {
      handleSaveAndClose();
    } else {
      handleDiscardAndClose();
    }
  }}
  ondiscard={() => {
    handleDiscardAndClose();
  }}
  oncancel={() => {
    closeConfirmKeepMode = false;
  }}
/>

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
    gap: 2px;
    height: 100%;
    overflow-x: auto;
    padding: 0 6px;
    flex-shrink: 1;
    min-width: 0;
  }
  .tabs::-webkit-scrollbar { display: none; }
  .tab {
    width: 200px;
    height: 34px;
    padding: 0 6px 0 12px;
    border-radius: 8px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--t3);
    font-size: 12.5px;
    font-family: var(--ui);
    font-weight: 500;
    cursor: default;
    display: flex;
    align-items: center;
    gap: 8px;
    white-space: nowrap;
    position: relative;
    transition: background 0.12s ease, color 0.12s ease, border-color 0.12s ease;
    flex-shrink: 0;
    -webkit-app-region: no-drag;
  }
  .tab:hover {
    color: var(--t2);
    background: var(--surface-hover);
  }
  /* Active tab — accent-tinted fill + hairline border for a quiet
     "selected pill" look. No underline; the icon picks up the accent
     instead (rule below). Uses color-mix so it respects the active
     theme accent. */
  .tab.on {
    background: color-mix(in srgb, var(--acc) 14%, transparent);
    border-color: color-mix(in srgb, var(--acc) 28%, transparent);
    color: var(--t1);
  }
  .tab-agent-icon {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
    opacity: 0.75;
  }
  .tab-mode-icon {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
    opacity: 0.75;
    color: var(--t3);
    transition: color 0.12s ease, opacity 0.12s ease;
  }
  .tab.on .tab-mode-icon,
  .tab.on .tab-agent-icon {
    color: var(--acc);
    opacity: 1;
  }
  .tab-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    letter-spacing: 0.005em;
  }
  .tab.tab-dirty .tab-label { font-style: italic; }
  .tab-close {
    width: 20px;
    height: 20px;
    border-radius: 5px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 15px;
    line-height: 1;
    color: var(--t3);
    cursor: default;
    flex-shrink: 0;
    margin-left: 2px;
    opacity: 0;
    transition: opacity 0.12s ease, background 0.12s ease, color 0.12s ease;
  }
  /* Keep the close button visible while the tab is active so the X
     doesn't vanish the moment the cursor moves away. */
  .tab:hover .tab-close,
  .tab.on .tab-close { opacity: 1; }
  .tab-close:hover {
    background: var(--surface-hover);
    color: var(--t1);
  }

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
    background: var(--surface-hover);
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
    background: var(--surface-hover);
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

  /* Backdrop reused by the SQL Script modal — the close-confirm dialog
     itself moved to the shared ConfirmDialog primitive. */
  .confirm-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0,0,0,0.4);
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
  }

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
    background: var(--surface-hover);
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

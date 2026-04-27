import { get } from 'svelte/store';
import { mode } from '$lib/stores/app';
import { navOpen, aiPanelOpen, aiPanelOpenPerMode, activeModal } from '$lib/stores/app';
import { tabs, activeTabId, closeTab, getDraft, markClean } from '$lib/stores/tabs';
import { commitRequest } from '$lib/stores/collections';

export function setupGlobalShortcuts() {
  document.addEventListener('keydown', handleKeydown);
}

export function teardownGlobalShortcuts() {
  document.removeEventListener('keydown', handleKeydown);
}

function handleKeydown(e: KeyboardEvent) {
  const meta = e.metaKey || e.ctrlKey;
  const target = e.target as HTMLElement;
  const isInput = target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable;

  // Escape: close any open modal or overlay
  if (e.key === 'Escape') {
    const modal = get(activeModal);
    if (modal) {
      activeModal.set(null);
      e.preventDefault();
      return;
    }
    if (get(aiPanelOpen)) {
      aiPanelOpen.set(false);
      e.preventDefault();
      return;
    }
  }

  // Cmd+W: close active tab (with prompt if dirty/unsaved)
  if (meta && e.key === 'w') {
    e.preventDefault();
    const tabId = get(activeTabId);
    if (tabId === -1) return;
    const allTabs = get(tabs);
    const tab = allTabs.find(t => t.id === tabId);
    if (!tab) return;
    // SSH and Agent tabs need PTY/connection cleanup beyond a plain closeTab —
    // route them through Topbar's prompt handler which calls doCloseTab and
    // runs the proper teardown (kill terminal, switch active profile, reset
    // spawning state). REST tabs only need the prompt when dirty.
    if (tab.mode === 'agent' || tab.mode === 'ssh' || tab.dirty || tab.unsaved) {
      window.dispatchEvent(new CustomEvent('clauge:tab-close-prompt', { detail: { tabId } }));
    } else {
      closeTab(tabId);
    }
    return;
  }

  // Cmd+S: save active request
  if (meta && e.key === 's') {
    e.preventDefault();
    const tabId = get(activeTabId);
    if (tabId === -1) return;
    const allTabs = get(tabs);
    const tab = allTabs.find(t => t.id === tabId);
    if (!tab) return;
    if (tab.mode === 'sql') {
      // SQL: trigger save for pending result edits
      window.dispatchEvent(new CustomEvent('clauge:sql-save'));
    } else if (tab.unsaved && tab.key === null) {
      // New unsaved request — show save dialog
      window.dispatchEvent(new CustomEvent('clauge:save-new-request', { detail: { tabId } }));
    } else if (tab.dirty && tab.key !== null) {
      // Existing dirty request — persist draft to backend
      const draft = getDraft(tabId);
      if (draft) {
        commitRequest(tab.key, draft).then(() => {
          markClean(tabId);
        }).catch(err => {
          console.error('Failed to save request:', err);
        });
      }
    }
    return;
  }

  // Cmd+1-9: switch tabs (like browser tabs)
  if (meta && !isInput && e.key >= '1' && e.key <= '9') {
    e.preventDefault();
    const currentMode = get(mode);
    const allTabs = get(tabs);
    const modeTabs = allTabs.filter(t => t.mode === currentMode);
    const idx = parseInt(e.key) - 1;
    if (idx < modeTabs.length) {
      const tab = modeTabs[idx];
      import('$lib/stores/tabs').then(({ activateTab }) => {
        activateTab(tab.id);
        // For agent tabs, also set active session
        if (tab.mode === 'agent' && tab.key) {
          import('$lib/stores/agent').then(({ agentSessions, activeAgentSession }) => {
            const sessions = get(agentSessions);
            const session = sessions.find((s: any) => s.id === tab.key);
            if (session) activeAgentSession.set(session);
          });
        }
      });
    }
  }

  // Cmd+B: toggle nav
  if (meta && e.key === 'b' && !isInput) {
    navOpen.update(v => !v);
    e.preventDefault();
  }

  // Cmd+L: toggle AI panel (or shell in agent mode)
  if (meta && e.key === 'l' && !e.shiftKey) {
    const currentMode = get(mode);
    if (currentMode === 'agent') {
      // In agent mode, Cmd+L toggles the shell panel (only if a session is active)
      import('$lib/stores/agent').then(({ agentShellOpen, activeAgentSession }) => {
        let hasSession = false;
        const unsub = activeAgentSession.subscribe(s => { hasSession = !!s; });
        unsub();
        if (hasSession) agentShellOpen.update(v => !v);
      });
      e.preventDefault();
      return;
    }
    aiPanelOpen.update(v => {
      const next = !v;
      aiPanelOpenPerMode.update(m => ({ ...m, [currentMode]: next }));
      return next;
    });
    e.preventDefault();
  }

  // Cmd+/ or ?: show shortcuts overlay
  if ((meta && e.key === '/') || (e.key === '?' && !isInput)) {
    activeModal.set(get(activeModal) === 'shortcuts' ? null : 'shortcuts');
    e.preventDefault();
  }

  // Ctrl+Cmd+F: toggle fullscreen
  if (e.metaKey && e.ctrlKey && e.key === 'f') {
    e.preventDefault();
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      const win = getCurrentWindow();
      win.isFullscreen().then(fs => win.setFullscreen(!fs));
    });
  }

  // Cmd+M: minimize (only when not in fullscreen)
  if (e.metaKey && !e.ctrlKey && e.key === 'm' && !isInput) {
    e.preventDefault();
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      const win = getCurrentWindow();
      win.isFullscreen().then(fs => { if (!fs) win.minimize(); });
    });
  }
}

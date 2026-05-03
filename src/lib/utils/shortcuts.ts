import { get } from 'svelte/store';
import { mode } from '$lib/stores/app';
import { navOpen, aiPanelOpen, aiPanelOpenPerMode, activeModal } from '$lib/stores/app';
import { tabs, activeTabId, closeTab, getDraft, markClean } from '$lib/shared/stores/tabs';
import { commitRequest } from '$lib/modes/rest/stores';
import { APP_EVENT } from '$lib/shared/constants/events';
import { isMac } from '$lib/utils/platform';

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
    // SSH, Agent, and Explorer tabs need session cleanup beyond a plain
    // closeTab — route them through Topbar's prompt handler which calls
    // doCloseTab and runs the proper teardown (kill terminal, switch
    // active profile, close Rust-side session, reset spawning state).
    // REST tabs only need the prompt when dirty.
    if (tab.mode === 'agent' || tab.mode === 'ssh' || tab.mode === 'explorer' || tab.dirty || tab.unsaved) {
      window.dispatchEvent(new CustomEvent(APP_EVENT.TAB_CLOSE_PROMPT, { detail: { tabId } }));
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
      window.dispatchEvent(new CustomEvent(APP_EVENT.SQL_SAVE));
    } else if (tab.unsaved && tab.key === null) {
      // New unsaved request — show save dialog
      window.dispatchEvent(new CustomEvent(APP_EVENT.SAVE_NEW_REQUEST, { detail: { tabId } }));
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

  // Cmd+1-9: switch to Nth tab in the topbar (global, across all modes).
  // Activation flips mode + runs side effects via the shared helper, so
  // the user lands on the correct panel regardless of current mode.
  if (meta && !isInput && e.key >= '1' && e.key <= '9') {
    e.preventDefault();
    const allTabs = get(tabs);
    const idx = parseInt(e.key) - 1;
    if (idx < allTabs.length) {
      const tab = allTabs[idx];
      import('$lib/utils/tabActivation').then(({ activateTabAcrossMode }) => {
        activateTabAcrossMode(tab.id);
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
      import('$lib/modes/agent/stores').then(({ agentShellOpen, activeAgentSession }) => {
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

  // Fullscreen: Ctrl+Cmd+F on macOS, F11 elsewhere
  const isFullscreenShortcut = isMac()
    ? e.metaKey && e.ctrlKey && e.key === 'f'
    : e.key === 'F11' && !meta;
  if (isFullscreenShortcut) {
    e.preventDefault();
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      const win = getCurrentWindow();
      win.isFullscreen().then(fs => win.setFullscreen(!fs));
    });
  }

  // Cmd+M: minimize (exits fullscreen first if needed)
  if (e.metaKey && !e.ctrlKey && e.key === 'm' && !isInput) {
    e.preventDefault();
    Promise.all([
      import('@tauri-apps/api/window'),
      import('$lib/shared/utils/window'),
    ]).then(async ([{ getCurrentWindow }, { ensureNotFullscreen }]) => {
      await ensureNotFullscreen();
      await getCurrentWindow().minimize();
    });
  }
}

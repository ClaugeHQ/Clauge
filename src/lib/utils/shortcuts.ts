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
    if (tab.dirty || tab.unsaved) {
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

  // Cmd+1/2/3/4: switch modes (don't intercept if user is typing in input)
  if (meta && !isInput) {
    if (e.key === '1') { mode.set('agent'); e.preventDefault(); }
    if (e.key === '2') { mode.set('rest'); e.preventDefault(); }
    if (e.key === '3') { mode.set('sql'); e.preventDefault(); }
    if (e.key === '4') { mode.set('nosql'); e.preventDefault(); }
  }

  // Cmd+B: toggle nav
  if (meta && e.key === 'b' && !isInput) {
    navOpen.update(v => !v);
    e.preventDefault();
  }

  // Cmd+Shift+L: toggle agent shell panel
  if (meta && e.shiftKey && e.key === 'l') {
    const currentMode = get(mode);
    if (currentMode === 'agent') {
      // Dynamic import to avoid circular dependency
      import('$lib/stores/agent').then(({ agentShellOpen }) => {
        agentShellOpen.update(v => !v);
      });
    }
    e.preventDefault();
  }

  // Cmd+L: toggle AI panel
  if (meta && e.key === 'l' && !e.shiftKey) {
    const currentMode = get(mode);
    if (currentMode === 'agent') { e.preventDefault(); return; }
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
}

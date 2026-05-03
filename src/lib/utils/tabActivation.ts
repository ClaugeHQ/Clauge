import { get } from 'svelte/store';
import { mode } from '$lib/stores/app';
import { tabs, activateTab } from '$lib/shared/stores/tabs';

// Switches to the tab's mode, activates it, and runs mode-specific
// side effects so the panel reflects the tab. SSH/SQL/NoSQL/Explorer
// panels re-derive their active state from $activeTabId on their own;
// REST and Agent require explicit setters because their editors bind
// to $activeRequest / $activeAgentSession respectively.
export async function activateTabAcrossMode(tabId: number) {
  const tab = get(tabs).find((t) => t.id === tabId);
  if (!tab) return;
  mode.set(tab.mode);
  activateTab(tabId);
  if (tab.mode === 'rest') {
    const { loadRequest, clearActiveRequest } = await import('$lib/modes/rest/stores');
    if (tab.key) await loadRequest(tab.key);
    else clearActiveRequest();
  } else if (tab.mode === 'agent' && tab.key) {
    const { agentSessions, activeAgentSession } = await import('$lib/modes/agent/stores');
    const session = get(agentSessions).find((s: any) => s.id === tab.key);
    if (session) activeAgentSession.set(session);
  }
}

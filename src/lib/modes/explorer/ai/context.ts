/** Build the chat context for Explorer mode. */

import { get } from 'svelte/store';
import { activeTabId, tabs } from '$lib/shared/stores/tabs';
import { activeExplorerConnection } from '$lib/modes/explorer/stores';
import type { ChatContext } from '$lib/types/ai';

export async function gatherExplorerContext(): Promise<ChatContext> {
  const allTabs = get(tabs);
  const tab = allTabs.find((t) => t.id === get(activeTabId));
  const conn = get(activeExplorerConnection);

  // Stuff a hint into envVars so the system-prompt-aware model knows the
  // current tabKey + connection kind without needing a custom field on
  // ChatContext (keeps the cross-mode shape uniform).
  const envVars = [];
  if (tab?.key) envVars.push({ key: 'EXPLORER_TAB_KEY', value: tab.key });
  if (conn) {
    envVars.push({ key: 'EXPLORER_CONNECTION_NAME', value: conn.name });
    envVars.push({ key: 'EXPLORER_KIND', value: conn.kind });
  }

  return {
    mode: 'explorer',
    currentRequest: null,
    currentResponse: null,
    envVars,
  } as unknown as ChatContext;
}

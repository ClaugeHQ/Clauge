import type { CanvasTabAdapter } from '$lib/modes/canvas/adapter';
import { get } from 'svelte/store';
import { sshTerminalIds, sshProfiles, activeSshProfile } from '$lib/modes/ssh/stores';
import { profileIdFromTabKey } from '$lib/modes/ssh/tabkey';
import { setMode } from '$lib/stores/app';
import { tabs, activateTab } from '$lib/shared/stores/tabs';
import {
  attachSshTerminal,
  detachSshTerminal,
  listOpenSshTerminals,
} from '$lib/modes/canvas/services/sshTerminalReparent';

export const sshTerminalAdapter: CanvasTabAdapter = {
  tabKind: 'ssh_terminal',
  mountStrategy: 'reparent',

  listOpenTabs(_workspaceId) {
    return listOpenSshTerminals();
  },

  subscribe(_workspaceId, onChange) {
    return sshTerminalIds.subscribe(() => onChange());
  },

  attach(tabId, slot) {
    attachSshTerminal(tabId, slot);
  },

  detach(tabId, slot) {
    detachSshTerminal(tabId, slot);
  },

  getMeta(tabId) {
    const profileId = profileIdFromTabKey(tabId);
    const profile = get(sshProfiles).find((p) => p.id === profileId);
    const baseName = profile?.name ?? profileId;
    const hashIdx = tabId.indexOf('#');
    const suffix = hashIdx !== -1 ? tabId.slice(hashIdx + 1) : null;
    return {
      title: suffix ? `${baseName} — ${suffix}` : baseName,
    };
  },

  openInHomeMode(tabId) {
    const profileId = profileIdFromTabKey(tabId);
    const profile = get(sshProfiles).find((p) => p.id === profileId) ?? null;

    // Activate the matching SSH tab in the shared tabs bar if present.
    const allTabs = get(tabs);
    const match = allTabs.find((t) => t.mode === 'ssh' && t.key === tabId);
    if (match) {
      activateTab(match.id);
    }

    // Set the active SSH profile so SshPanel focuses the right terminal.
    activeSshProfile.set(profile);

    void setMode('ssh');
  },
};

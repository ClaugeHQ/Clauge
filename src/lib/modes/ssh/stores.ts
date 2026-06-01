import { writable } from 'svelte/store';
import type { SshProfile } from './types';
import { sshListProfiles } from './commands';

// Profiles registered by the user (DB-backed).
export const sshProfiles = writable<SshProfile[]>([]);

// Currently focused profile (drives status bar / AI prompt context).
export const activeSshProfile = writable<SshProfile | null>(null);

// tab.key (== profile.id) → backend terminal id. One xterm per SSH tab.
export const sshTerminalIds = writable<Map<string, string>>(new Map());

// tab.key → 'connecting' | 'connected' | 'disconnected' for banner UI.
export type SshConnState = 'connecting' | 'connected' | 'disconnected';
export const sshConnStates = writable<Map<string, SshConnState>>(new Map());

// Per-tab xterm entries for SSH terminals. Key = tabKey (`profileId#timestamp-N`).
// Lifted from SshPanel.svelte for Canvas mode's cross-surface xterm reparenting.
// Shape mirrors SshPanel's TermEntry (typed as `any` to avoid importing @xterm/xterm here):
//   { term: Terminal; fitAddon: FitAddon; searchAddon: SearchAddon;
//     container: HTMLDivElement; terminalId: string | null;
//     profileId: string; tabKey: string; generation: number; capture: ... | null }
export const sshTerminalMap = writable<Map<string, any>>(new Map());

export async function loadSshProfiles() {
  try {
    const profiles = await sshListProfiles();
    sshProfiles.set(profiles);
  } catch (e) {
    console.error('Failed to load SSH profiles:', e);
  }
}

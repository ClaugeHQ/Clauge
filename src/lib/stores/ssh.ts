import { writable } from 'svelte/store';
import type { SshProfile } from '$lib/types/ssh';
import { sshListProfiles } from '$lib/commands/ssh';

// Profiles registered by the user (DB-backed).
export const sshProfiles = writable<SshProfile[]>([]);

// Currently focused profile (drives status bar / AI prompt context).
export const activeSshProfile = writable<SshProfile | null>(null);

// tab.key (== profile.id) → backend terminal id. One xterm per SSH tab.
export const sshTerminalIds = writable<Map<string, string>>(new Map());

// tab.key → 'connecting' | 'connected' | 'disconnected' for banner UI.
export type SshConnState = 'connecting' | 'connected' | 'disconnected';
export const sshConnStates = writable<Map<string, SshConnState>>(new Map());

export async function loadSshProfiles() {
  try {
    const profiles = await sshListProfiles();
    sshProfiles.set(profiles);
  } catch (e) {
    console.error('Failed to load SSH profiles:', e);
  }
}

import { writable } from 'svelte/store';
import { getSetting, setSetting } from '$lib/commands/settings';

export type ShellCwdChoice = 'home' | 'workspace' | string; // 'string' = absolute path (custom)

const KEY = 'canvas.shellTerminal.defaultCwd';

export const shellDefaultCwd = writable<ShellCwdChoice>('home');

/**
 * Load the persisted default cwd into the store. Call on app boot or
 * Canvas mode mount.
 */
export async function loadCanvasSettings(): Promise<void> {
  try {
    const v = await getSetting(KEY);
    if (v) shellDefaultCwd.set(v as ShellCwdChoice);
  } catch {
    // Setting not present yet — keep default 'home'.
  }
}

/**
 * Persist the user's cwd choice and update the in-memory store.
 */
export async function setShellDefaultCwd(v: ShellCwdChoice): Promise<void> {
  shellDefaultCwd.set(v);
  await setSetting(KEY, v);
}

import { writable } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/**
 * Set of terminalIds whose PTY size is currently being driven by a paired
 * phone (phone-authoritative sizing). The desktop renders an ambient
 * "Controlled from phone" hint for the on-screen terminal while it's in here.
 */
export const phoneOwnedTerminals = writable<Set<string>>(new Set());

let unlisten: UnlistenFn | null = null;
let started = false;

/**
 * Register the single `terminal-size-owner` listener. The backend emits this
 * whenever a terminal's size ownership flips between phone-driven and
 * desktop-driven. Guarded so repeated calls (HMR, double-mount) don't stack
 * listeners. Returns a teardown that removes the listener.
 */
export function startSizeOwnerListener(): () => void {
  if (started) return () => {};
  started = true;

  listen<{ terminalId: string; phoneOwned: boolean }>('terminal-size-owner', (event) => {
    const { terminalId, phoneOwned } = event.payload;
    if (!terminalId) return;
    phoneOwnedTerminals.update((set) => {
      const next = new Set(set);
      if (phoneOwned) next.add(terminalId);
      else next.delete(terminalId);
      return next;
    });
  })
    .then((fn) => {
      unlisten = fn;
    })
    .catch((e) => console.warn('[size-owner] listener failed:', e));

  return () => {
    unlisten?.();
    unlisten = null;
    started = false;
    phoneOwnedTerminals.set(new Set());
  };
}

import { writable } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/**
 * Set of terminalIds whose PTY size is currently being driven by a paired
 * phone (phone-authoritative sizing). The desktop renders an ambient
 * "Controlled from phone" hint for the on-screen terminal while it's in here.
 */
export const phoneOwnedTerminals = writable<Set<string>>(new Set());

/**
 * Map of terminalId → the size (cols × rows) the phone is currently driving.
 * Populated while a terminal is phone-owned and cleared when it stops being
 * phone-owned. The Agent/SSH panels adopt this size onto their own xterm so
 * the desktop renders a tidy narrow terminal that matches the PTY, and gate
 * their fit-addon PTY-resize drive on `!phoneDrivenSizes.has(terminalId)` so
 * the desktop never fights the phone (no resize war).
 */
export const phoneDrivenSizes = writable<Map<string, { cols: number; rows: number }>>(new Map());

let unlisten: UnlistenFn | null = null;
let started = false;
let tearingDown = false;

/**
 * Register the single `terminal-size` listener. The backend emits this on
 * every applied size change with the resulting cols/rows and whether the
 * phone owns the size. Guarded so repeated calls (HMR, double-mount) don't
 * stack listeners. Returns a teardown that removes the listener.
 */
export function startSizeOwnerListener(): () => void {
  if (started) return () => {};
  started = true;
  tearingDown = false;

  listen<{ terminalId: string; cols: number; rows: number; phoneOwned: boolean }>(
    'terminal-size',
    (event) => {
      const { terminalId, cols, rows, phoneOwned } = event.payload;
      if (!terminalId) return;
      phoneOwnedTerminals.update((set) => {
        const next = new Set(set);
        if (phoneOwned) next.add(terminalId);
        else next.delete(terminalId);
        return next;
      });
      phoneDrivenSizes.update((map) => {
        const next = new Map(map);
        if (phoneOwned) next.set(terminalId, { cols, rows });
        else next.delete(terminalId);
        return next;
      });
    },
  )
    .then((fn) => {
      // If teardown ran before listen() resolved, the listener would leak
      // (and `started` was reset, so a re-subscribe stacks a second one).
      // Unlisten immediately in that case; otherwise store the handle.
      if (tearingDown) fn();
      else unlisten = fn;
    })
    .catch((e) => {
      started = false;
      console.warn('[size-owner] listener failed:', e);
    });

  return () => {
    tearingDown = true;
    unlisten?.();
    unlisten = null;
    started = false;
    phoneOwnedTerminals.set(new Set());
    phoneDrivenSizes.set(new Map());
  };
}

import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { Channel } from '@tauri-apps/api/core';
import { get } from 'svelte/store';
import {
  canvasShellTerminalSpawn,
  type ShellOutput,
} from '$lib/modes/canvas/commands';
import {
  shellTerminals,
  type ShellTerminalEntry,
} from '$lib/modes/canvas/stores/shellTerminalsStore';

/**
 * Spawn a new Canvas shell terminal. Adds a placeholder entry to the store
 * synchronously, then awaits the PTY spawn. xterm DOM creation is deferred
 * to the first `attachShellTerminal` call to avoid opening into a detached
 * container (a known WebGL gotcha from Cate).
 *
 * Returns the new tab id.
 */
export async function spawnShellTerminal(workspaceId: string, cwd: string): Promise<string> {
  const id = crypto.randomUUID();
  const entry: ShellTerminalEntry = { id, cwd, terminalId: null };
  shellTerminals.update((m) => {
    const next = new Map(m);
    next.set(id, entry);
    return next;
  });

  const channel = new Channel<ShellOutput>();
  channel.onmessage = (msg) => {
    const e = get(shellTerminals).get(id);
    if (e?.internal && msg.data) {
      try {
        const binary = atob(msg.data);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
        e.internal.term.write(bytes);
      } catch {
        e.internal.term.write(msg.data);
      }
    }
  };

  const terminalId = await canvasShellTerminalSpawn(workspaceId, cwd, channel);
  shellTerminals.update((m) => {
    const next = new Map(m);
    const cur = next.get(id);
    if (cur) next.set(id, { ...cur, terminalId });
    return next;
  });
  return id;
}

/**
 * Move the xterm container for `id` into `slot`. If this is the first
 * attach for this entry, the xterm is created here (`term.open(container)`)
 * and the container is parented to the slot.
 */
export function attachShellTerminal(id: string, slot: HTMLElement): void {
  const entry = get(shellTerminals).get(id);
  if (!entry) return;

  if (!entry.internal) {
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: 'ui-monospace, monospace',
      fontSize: 13,
      theme: { background: '#0d0a22' },
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    const container = document.createElement('div');
    container.style.width = '100%';
    container.style.height = '100%';
    slot.appendChild(container);
    term.open(container);
    fitAddon.fit();
    shellTerminals.update((m) => {
      const next = new Map(m);
      const cur = next.get(id);
      if (cur) next.set(id, { ...cur, internal: { term, fitAddon, container } });
      return next;
    });
  } else {
    slot.appendChild(entry.internal.container);
    try {
      entry.internal.fitAddon.fit();
    } catch {
      // First resize tick will fit.
    }
  }
}

/**
 * Move the xterm container OUT of `slot` if it currently sits there.
 * Does not destroy the xterm or kill the PTY.
 */
export function detachShellTerminal(id: string, slot: HTMLElement): void {
  const entry = get(shellTerminals).get(id);
  if (!entry?.internal) return;
  if (entry.internal.container.parentElement === slot) {
    slot.removeChild(entry.internal.container);
  }
}

import { writable } from 'svelte/store';
import type { Terminal } from '@xterm/xterm';
import type { FitAddon } from '@xterm/addon-fit';

export interface ShellTerminalInternal {
  term: Terminal;
  fitAddon: FitAddon;
  container: HTMLDivElement;
}

export interface ShellTerminalEntry {
  id: string;           // tab id (uuid)
  cwd: string;
  terminalId: string | null;
  internal?: ShellTerminalInternal;
}

export const shellTerminals = writable<Map<string, ShellTerminalEntry>>(new Map());

import type { CanvasTabAdapter } from '$lib/modes/canvas/adapter';
import { get } from 'svelte/store';
import { shellTerminals } from '$lib/modes/canvas/stores/shellTerminalsStore';
import {
  attachShellTerminal,
  detachShellTerminal,
} from '$lib/modes/canvas/services/shellTerminalLifecycle';

export const shellTerminalAdapter: CanvasTabAdapter = {
  tabKind: 'shell_terminal',
  mountStrategy: 'reparent',

  listOpenTabs(_workspaceId) {
    return [...get(shellTerminals).values()].map((e) => ({
      id: e.id,
      title: `Terminal — ${e.cwd}`,
    }));
  },

  subscribe(_workspaceId, onChange) {
    return shellTerminals.subscribe(() => onChange());
  },

  attach(id, slot) {
    attachShellTerminal(id, slot);
  },

  detach(id, slot) {
    detachShellTerminal(id, slot);
  },

  getMeta(id) {
    const e = get(shellTerminals).get(id);
    return { title: e ? `Terminal — ${e.cwd}` : id };
  },

  openInHomeMode(_id) {
    // Shell terminals are Canvas-native; no home mode equivalent.
  },
};

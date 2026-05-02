import { writable, get } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { ExplorerConnection, Transfer } from './types';
import { listConnections } from './commands';

/** All known connection rows (sorted by lastUsedAt desc by the backend). */
export const explorerConnections = writable<ExplorerConnection[]>([]);

/** The connection backing the currently-active Explorer tab, if any. */
export const activeExplorerConnection = writable<ExplorerConnection | null>(null);

/** tabKey → 'connecting' | 'connected' | 'error' | 'disconnected'. */
export const explorerConnStates = writable<Map<string, 'connecting' | 'connected' | 'error' | 'disconnected'>>(new Map());

/** Long-running transfers (uploads / downloads). Updated by the global
 *  `explorer:transfer` Tauri event listener wired in `setupTransferListener`. */
export const explorerTransfers = writable<Transfer[]>([]);

export async function loadExplorerConnections() {
  try {
    const data = await listConnections();
    explorerConnections.set(data);
  } catch (e) {
    console.error('Failed to load explorer connections:', e);
  }
}

/** Backend event payload for `explorer:transfer`. Mirrors the Rust
 *  TransferEvent struct (serde camelCase). */
interface TransferEventPayload {
  id: string;
  direction: 'upload' | 'download';
  state: 'running' | 'completed' | 'failed' | 'cancelled';
  bytesDone: number;
  bytesTotal: number | null;
  error: string | null;
  name: string;
  remotePath: string;
  localPath: string;
}

let transferUnlisten: UnlistenFn | null = null;

/** Register the global Tauri event listener that drives the transfers
 *  store. Idempotent — repeat calls are no-ops. Called once from the
 *  layout / explorer panel mount. */
export async function setupTransferListener(): Promise<void> {
  if (transferUnlisten) return;
  transferUnlisten = await listen<TransferEventPayload>('explorer:transfer', (event) => {
    const p = event.payload;
    explorerTransfers.update((list) => {
      const idx = list.findIndex((t) => t.id === p.id);
      const isTerminal = p.state !== 'running';
      const next: Transfer = {
        id: p.id,
        direction: p.direction,
        name: p.name,
        localPath: p.localPath,
        remotePath: p.remotePath,
        bytesTotal: p.bytesTotal,
        bytesDone: p.bytesDone,
        state: p.state,
        error: p.error,
        startedAt: idx >= 0 ? list[idx].startedAt : new Date().toISOString(),
        completedAt: isTerminal ? new Date().toISOString() : null,
      };
      if (idx >= 0) {
        const copy = list.slice();
        copy[idx] = next;
        return copy;
      }
      return [...list, next];
    });
    // Auto-evict completed transfers after 4s so the panel clears itself.
    if (p.state !== 'running') {
      setTimeout(() => {
        explorerTransfers.update((list) => list.filter((t) => t.id !== p.id));
      }, 4000);
    }
  });
}

/** Eagerly read whether any transfer is currently running. Cheaper than
 *  a derived store for our handful of consumers. */
export function hasActiveTransfer(): boolean {
  return get(explorerTransfers).some((t) => t.state === 'running');
}

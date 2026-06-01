import type { CanvasTabAdapter } from '$lib/modes/canvas/adapter';
import type { TabKind } from '$lib/modes/canvas/commands';

const adapters = new Map<TabKind, CanvasTabAdapter>();

export const canvasAdapterRegistry = {
  register(adapter: CanvasTabAdapter): void {
    if (adapters.has(adapter.tabKind)) {
      console.warn(`[canvas] adapter for "${adapter.tabKind}" overwritten`);
    }
    adapters.set(adapter.tabKind, adapter);
  },

  get(kind: TabKind): CanvasTabAdapter | undefined {
    return adapters.get(kind);
  },

  has(kind: TabKind): boolean {
    return adapters.has(kind);
  },

  list(): CanvasTabAdapter[] {
    return [...adapters.values()];
  },

  /** Phase 3 dev-only: clear all registered adapters. Useful for HMR. */
  clear(): void {
    adapters.clear();
  },
};

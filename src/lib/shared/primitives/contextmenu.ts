import { writable } from 'svelte/store';

export interface ContextMenuItem {
  label: string;
  sub?: string;
  icon?: string;
  action: () => void;
  danger?: boolean;
  separator?: boolean;
}

export interface ContextMenuState {
  show: boolean;
  x: number;
  y: number;
  items: ContextMenuItem[];
  scrollable?: boolean;
  stickyFooter?: ContextMenuItem;
}

export const contextMenu = writable<ContextMenuState>({
  show: false,
  x: 0,
  y: 0,
  items: []
});

export function showContextMenu(
  x: number,
  y: number,
  items: ContextMenuItem[],
  opts?: { scrollable?: boolean; stickyFooter?: ContextMenuItem },
): void {
  contextMenu.set({ show: true, x, y, items, ...opts });
}

export function closeContextMenu(): void {
  contextMenu.update((s) => ({ ...s, show: false }));
}

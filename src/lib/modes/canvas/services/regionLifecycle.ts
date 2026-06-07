import { get } from 'svelte/store';
import {
  regionsById,
  pendingRenameRegionId,
} from '$lib/modes/canvas/stores/canvasStore';
import { canvasUpsertRegion, type CanvasRegion } from '$lib/modes/canvas/commands';

const REGION_PALETTE = [
  'hsl(220 70% 60%)', // blue
  'hsl(280 65% 65%)', // purple
  'hsl(330 65% 60%)', // pink
  'hsl(10 75% 60%)',  // orange-red
  'hsl(40 80% 55%)',  // gold
  'hsl(140 55% 50%)', // green
  'hsl(180 60% 50%)', // teal
];

export function pickRegionColor(): string {
  const existing = new Set(
    [...get(regionsById).values()].map((r) => r.color),
  );
  for (const c of REGION_PALETTE) {
    if (!existing.has(c)) return c;
  }
  return REGION_PALETTE[Math.floor(Math.random() * REGION_PALETTE.length)];
}

export interface CreateRegionOptions {
  workspaceId: string;
  x: number;
  y: number;
  width: number;
  height: number;
  color?: string;
  /** When true (default), the CanvasRegion component for the new region
   *  auto-focuses its inline name editor on mount. */
  focusName?: boolean;
}

/**
 * Create a region, persist it to the backend, and (by default) signal
 * the rendered component to focus the inline name editor so the user
 * can type a project name immediately.
 */
export async function createRegion(opts: CreateRegionOptions): Promise<CanvasRegion> {
  const region: CanvasRegion = {
    workspaceId: opts.workspaceId,
    regionId: crypto.randomUUID(),
    name: `Region ${get(regionsById).size + 1}`,
    x: opts.x,
    y: opts.y,
    width: opts.width,
    height: opts.height,
    color: opts.color ?? pickRegionColor(),
    zOrder: 0,
    createdAt: '',
    updatedAt: '',
  };
  regionsById.update((m) => {
    const next = new Map(m);
    next.set(region.regionId, region);
    return next;
  });
  if (opts.focusName !== false) {
    pendingRenameRegionId.set(region.regionId);
  }
  try {
    await canvasUpsertRegion(region);
  } catch (err) {
    console.error('[atlas] failed to persist region', err);
  }
  return region;
}

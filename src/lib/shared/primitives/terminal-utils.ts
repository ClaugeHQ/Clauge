// Pure terminal helpers shared between AgentPanel and SshPanel.
// These are stateless utilities — no xterm lifecycle, no mode-specific assumptions.
// If you need anything stateful (entry maps, capture state, channel handlers),
// keep it in the panel; this module is intentionally narrow.

import type { Terminal } from '@xterm/xterm';
import type { AppearanceConfig } from '$lib/types';
import { isLinux } from '$lib/utils/platform';

/**
 * Default xterm.js fontFamily stack — used when the user hasn't picked a
 * specific terminal font in Settings → Appearance. Must stay in sync with
 * `DEFAULT_TERMINAL_FONT_FAMILY` in src-tauri/src/appearance/vibrancy.rs so
 * a fresh install renders identically before the SQLite default lands.
 */
export const DEFAULT_TERMINAL_FONT_FAMILY =
  '"JetBrains Mono", "Fira Code", "Cascadia Code", "SF Mono", "Menlo", monospace';

/**
 * Curated monospace fonts offered in the Appearance picker. Each preset is a
 * full CSS `font-family` value with monospace as the last fallback so the
 * terminal renders even when the named face isn't installed. `id` is the
 * stable key persisted in settings; an empty `id` means "use the default
 * stack" and is what fresh installs see.
 */
export interface TerminalFontPreset {
  id: string;
  label: string;
  value: string;
}

export const TERMINAL_FONT_FAMILY_PRESETS: readonly TerminalFontPreset[] = [
  { id: '', label: 'Default (JetBrains Mono stack)', value: DEFAULT_TERMINAL_FONT_FAMILY },
  { id: 'jetbrains-mono', label: 'JetBrains Mono', value: '"JetBrains Mono", monospace' },
  { id: 'fira-code', label: 'Fira Code', value: '"Fira Code", monospace' },
  { id: 'cascadia-code', label: 'Cascadia Code', value: '"Cascadia Code", monospace' },
  { id: 'sf-mono', label: 'SF Mono', value: '"SF Mono", monospace' },
  { id: 'menlo', label: 'Menlo', value: 'Menlo, monospace' },
  { id: 'monaco', label: 'Monaco', value: 'Monaco, monospace' },
  { id: 'consolas', label: 'Consolas', value: 'Consolas, monospace' },
  { id: 'courier-new', label: 'Courier New', value: '"Courier New", monospace' },
  { id: 'ibm-plex-mono', label: 'IBM Plex Mono', value: '"IBM Plex Mono", monospace' },
  { id: 'source-code-pro', label: 'Source Code Pro', value: '"Source Code Pro", monospace' },
  { id: 'system-monospace', label: 'System monospace', value: 'monospace' },
] as const;

/**
 * Resolve the CSS `font-family` value to pass to `new Terminal({ fontFamily })`
 * based on the current appearance config. Empty / missing values fall back to
 * the default stack so terminals always have a usable fontFamily.
 */
export function getTerminalFontFamily(app: AppearanceConfig | null | undefined): string {
  const raw = app?.terminalFontFamily?.trim();
  return raw && raw.length > 0 ? raw : DEFAULT_TERMINAL_FONT_FAMILY;
}

/**
 * Decode the base64-encoded PTY chunk emitted by the Rust backend into a
 * Uint8Array suitable for `Terminal.write()`. The PTY reader thread base64s
 * raw bytes (not UTF-8) so that arbitrary control sequences round-trip
 * through the Tauri Channel as a string.
 */
export function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/**
 * Run `callback` after two successive animation frames.
 *
 * Why 2-rAF and not 1: when the first PTY chunk arrives, calling
 * `term.write(bytes)` does not paint synchronously — xterm batches writes
 * into the next frame. If we hide the loader on rAF #1, xterm has only
 * just *scheduled* the paint, so the user sees a blank gap before the
 * terminal appears. Waiting one more frame lets that batched paint commit
 * before we flip `spawning = false` / `termReady = true`.
 */
export function deferUntilFrame(callback: () => void): void {
  requestAnimationFrame(() => requestAnimationFrame(callback));
}

/**
 * Lazily load and attach the WebGL renderer addon to an xterm terminal.
 *
 * Falls back silently to xterm's canvas renderer if WebGL is unavailable
 * (e.g., GPU blacklisted, software rendering, headless CI). Also disposes
 * the addon on context loss so the canvas fallback can take over without
 * leaking a dead WebGL context.
 *
 * Fire-and-forget: callers don't await this. Loading is async only because
 * the addon module is dynamically imported to keep it out of the main bundle.
 */
export async function loadWebGLAddon(term: Terminal): Promise<void> {
  // WebGL crashes the WebKitGTK GPU/compositor process on Linux (Wayland and X11),
  // leaving a blank window with no JS console error. Canvas renderer is stable.
  if (isLinux()) return;
  try {
    const { WebglAddon } = await import('@xterm/addon-webgl');
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => webgl.dispose());
    term.loadAddon(webgl);
  } catch {
    // Falls back to canvas renderer silently
  }
}

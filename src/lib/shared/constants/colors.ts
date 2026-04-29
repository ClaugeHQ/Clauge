// Centralized hex/rgba color constants used outside of CSS.
//
// CSS-level theming lives in `src/app.css` and `src/lib/utils/theme.ts`.
// Color values that appear in TS expressions (palette arrays, status thresholds,
// theme previews, defaults) live here so they aren't sprinkled across components.

// --- Brand / default accent ---
// Default for stored AppearanceConfig before the user picks one.
export const DEFAULT_ACCENT_COLOR = '#6366f1';
// Display fallback used in $derived expressions when appearance store is empty.
export const FALLBACK_ACCENT_COLOR = '#7c5cf8';

// --- Selectable accent palette (Settings → Appearance) ---
export const ACCENT_PALETTE = [
  { name: 'Purple', value: '#7c5cf8' },
  { name: 'Blue', value: '#4f94d4' },
  { name: 'Green', value: '#1dc880' },
  { name: 'Orange', value: '#f06830' },
  { name: 'Red', value: '#f04444' },
  { name: 'Pink', value: '#f472b6' },
  { name: 'Cyan', value: '#22d3ee' },
  { name: 'White', value: '#e0e0e0' },
] as const;

// --- Theme preview swatches (Settings → Appearance) ---
export const THEME_PREVIEW_COLORS: Record<string, readonly string[]> = {
  'dark-glass': ['rgba(7,7,15,0.55)', 'rgba(13,13,24,0.72)', 'rgba(19,19,32,0.82)'],
  'dark-solid': ['#0a0a14', '#0f0f1a', '#16162a'],
  'midnight': ['#000000', '#080808', '#0a0a0a'],
  'nord': ['#2e3440', '#3b4252', '#353d4b'],
  'light': ['#f0f0ec', '#f5f5f2', '#fafaf8'],
};

// --- Usage / status thresholds (StatusBar + SettingsModal usage tiles) ---
// Used by `usageColor(pct)` style helpers — kept consistent across components.
export const USAGE_DANGER = '#f85149';
export const USAGE_WARN = '#d29922';

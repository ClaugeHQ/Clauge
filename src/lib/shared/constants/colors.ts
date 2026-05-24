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
  'dark-glass': ['rgba(15,15,25,0.55)', 'rgba(22,22,34,0.72)', 'rgba(30,30,46,0.82)'],
  'dark-solid': ['#0a0a14', '#0f0f1a', '#16162a'],
  'midnight': ['#000000', '#080808', '#0a0a0a'],
  'rose-pine-moon': ['#232136', '#2a273f', '#393552'],
  'rose-pine-dawn': ['#faf4ed', '#f2e9e1', '#dfdad9'],
  'atelier': ['#fdf2f5', '#fbe8eb', '#e6c4cd'],
  'hearth': ['#14100c', '#1c1812', '#3a3024'],
  'petal': ['#14091e', '#221730', '#3d2a4d'],
  'celeste': ['#0a0814', '#0d0a1a', '#25204a'],
  'carbon-grain': ['#0c0c0e', '#14141a', '#2a2a32'],
  'crt-phosphor': ['#050805', '#0a100a', '#1a2a1a'],
  'aurora-drift': ['#1a1245', '#1f2a55', '#3d2860'],
};

// --- Usage / status thresholds (StatusBar + SettingsModal usage tiles) ---
// Used by `usageColor(pct)` style helpers — kept consistent across components.
export const USAGE_DANGER = '#f85149';
export const USAGE_WARN = '#d29922';

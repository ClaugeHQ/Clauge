import { invoke } from '@tauri-apps/api/core';
import { brandConfig, type BrandKey, type BrandOverride, type BrandDisplayMode } from '$lib/shared/theme/brands';

export interface Theme {
  id: string;
  name: string;
  description: string;
  // Surface colors
  sidebar: string;
  nav: string;
  navHeader: string;
  content: string;
  editor: string;
  // Border colors
  border: string;
  borderHover: string;
  borderSubtle: string;
  // Text colors
  textPrimary: string;
  textSecondary: string;
  textMuted: string;
  textFaint: string;
  // Modal (always opaque)
  modalBg: string;
  // Elevated surface for cards, kanban items, list rows, tile grids.
  // MUST be readable on its own — on translucent themes this means it
  // must be opaque enough (>= ~0.85 alpha) so text on top doesn't bleed
  // into the wallpaper / vibrancy behind the window. Replaces the
  // anti-pattern of writing `rgba(255,255,255,0.025)` in component CSS.
  surfaceCard: string;
  // Hover state for cards, list rows, menu items. Same opacity contract
  // as surfaceCard — guaranteed visible on every theme.
  surfaceHover: string;
  // Status colors (--ok / --warn / --err). Per-theme so a theme can pick a
  // palette that reads well on its own surfaces (e.g. soft pinks on a
  // violet base) instead of falling back to one global dark/light pair.
  ok: string;
  warn: string;
  err: string;
  // Syntax token colors for the homemade JSON / document highlighters
  // (`json-highlight.ts`, NoSQL `DocumentViewer`). Applied as CSS variables
  // on `:root` and consumed by `.str / .num / .key / .boo` rules in
  // `app.css`. `.pu` (punctuation) and `.cm` (comment, when re-introduced)
  // already cascade off `--t2 / --t3`, so only the 4 distinct token types
  // need per-theme overrides.
  tokens: {
    str: string;
    num: string;
    key: string;
    boo: string;
  };
  // Glass properties
  glass: boolean;
  // Optional visual effect class. When set, `applyTheme` adds it to <body>;
  // CSS rules in app.css under that class paint animations or textures
  // behind the app shell (visible through rgba surfaces). Used by themes
  // that go beyond flat colors. Themes that want a static look leave this
  // undefined.
  effectClass?: string;
  // Per-brand color/icon overrides (Postgres / MongoDB / S3 / …). When the
  // brand registry's defaults clash with the theme palette, a theme can
  // dim them via `intensity` or replace them entirely. See
  // `$lib/shared/theme/brands.ts`.
  brandOverrides?: Partial<Record<BrandKey, BrandOverride>>;
  // Global preference for how brand badges render. 'text' keeps the
  // current PG/MY/MG abbreviations; 'icon' swaps in the brand glyph.
  // 'auto' currently maps to 'text'.
  brandDisplay?: BrandDisplayMode;
  // Theme-defined accent. When `lockAccent` is true, `applyTheme` forces
  // `--acc` to this value and the Appearance UI disables the accent
  // picker. Free themes leave both undefined and the user-picked accent
  // wins.
  accent?: string;
  lockAccent?: boolean;
  // Pro-only themes. When true the theme tile shows a PRO badge and
  // clicking it opens the upgrade modal instead of applying the theme.
  premium?: boolean;
}

const themes: Record<string, Theme> = {
  'dark-glass': {
    id: 'dark-glass',
    name: 'Dark Glass',
    description: 'Translucent with native blur',
    // Surface alphas tuned aggressively for "wallpaper reads through,
    // text stays legible". 0.40 puts 60% of the vibrancy / desktop
    // bleed at the surface — visibly glassy even on a dark wallpaper.
    // Text variables (--t*) stay at full alpha so legibility is
    // unaffected. navHeader sits a touch denser (0.55) so the top
    // strip reads as a deliberate header rather than blending into
    // the content body. Borders bumped to 0.20 / 0.28 so panel
    // separation reads through the high transparency.
    sidebar: 'rgba(22,22,34,0.40)',
    nav: 'rgba(22,22,34,0.40)',
    navHeader: 'rgba(28,28,42,0.55)',
    content: 'rgba(22,22,34,0.40)',
    editor: 'rgba(22,22,34,0.40)',
    border: 'rgba(255,255,255,0.20)',
    borderHover: 'rgba(255,255,255,0.28)',
    borderSubtle: 'rgba(255,255,255,0.12)',
    textPrimary: '#e8e8f4',
    textSecondary: '#d0d0e4',
    textMuted: '#bcbcd5',
    textFaint: '#8a8ab2',
    // Modals on glass theme: low alpha (0.50) so the vibrancy / wallpaper
    // reads through clearly. `.modal-card` adds backdrop-filter blur on
    // top — the combination gives a native macOS glass-card look. Text
    // legibility comes from --t* foreground variables which are at full
    // alpha, not from a heavy modal background.
    modalBg: 'rgba(26,26,44,0.50)',
    // Same near-invisible white-wash as the solid themes — preserves
    // the original look the codebase had everywhere. The handful of
    // components that genuinely need an opaque container on glass
    // (kanban cards on a translucent column) should target
    // `body.glass-mode .X` explicitly instead of forcing all surfaces
    // to a solid color globally.
    surfaceCard: 'rgba(255,255,255,0.025)',
    surfaceHover: 'rgba(255,255,255,0.05)',
    ok: '#1dc880',
    warn: '#f5a623',
    err: '#f04444',
    tokens: { str: '#9ccfd8', num: '#f6c177', key: '#8cb4e0', boo: '#eb6f92' },
    glass: true,
  },
  'dark-solid': {
    id: 'dark-solid',
    name: 'Dark Solid',
    description: 'Opaque dark with purple tints',
    sidebar: '#12121f',
    nav: '#161629',
    navHeader: '#1c1c35',
    content: '#1e1e32',
    editor: '#151528',
    border: '#2d2d48',
    borderHover: '#3e3e62',
    borderSubtle: '#222238',
    textPrimary: '#e4e4f0',
    textSecondary: '#d0d0e4',
    textMuted: '#b0b0c8',
    textFaint: '#7878a0',
    modalBg: '#151528',
    // Opaque themes get a near-invisible white-wash — same look as the
    // original `rgba(255,255,255,0.025)` / `0.04` inlines that were
    // scattered across components. The bulk refactor routed every one
    // through these tokens; keeping the values as rgba whites means the
    // visual end-state is identical to before on solid themes. Only
    // dark-glass uses an actual opaque colour, where the original
    // approach broke (invisible cards on vibrancy).
    surfaceCard: 'rgba(255,255,255,0.025)',
    surfaceHover: 'rgba(255,255,255,0.05)',
    ok: '#1dc880',
    warn: '#f5a623',
    err: '#f04444',
    tokens: { str: '#9ccfd8', num: '#f6c177', key: '#8cb4e0', boo: '#eb6f92' },
    glass: false,
  },
  'midnight': {
    id: 'midnight',
    name: 'Midnight',
    description: 'Pure black, zero distraction',
    sidebar: '#000000',
    nav: '#0a0a0a',
    navHeader: '#121212',
    content: '#0e0e0e',
    editor: '#080808',
    border: '#2a2a2a',
    borderHover: '#3a3a3a',
    borderSubtle: '#1c1c1c',
    textPrimary: '#e8e8e8',
    textSecondary: '#cccccc',
    textMuted: '#999999',
    textFaint: '#666666',
    modalBg: '#0e0e0e',
    surfaceCard: 'rgba(255,255,255,0.025)',
    surfaceHover: 'rgba(255,255,255,0.05)',
    ok: '#1dc880',
    warn: '#f5a623',
    err: '#f04444',
    tokens: { str: '#9ccfd8', num: '#f6c177', key: '#8cb4e0', boo: '#eb6f92' },
    glass: false,
  },
  // Rose Pine Moon — palette taken from the official VSCode theme
  // (https://github.com/rose-pine/vscode). Surfaces: Base / Surface /
  // Overlay. Tokens: Gold (string), Rose (number/bool), Foam (property),
  // Pine (success). Cyan is intentionally Rose per Rose Pine convention —
  // it warms the terminal palette so even ANSI output reads on-theme.
  'rose-pine-moon': {
    id: 'rose-pine-moon',
    name: 'Rose Pine Moon',
    description: 'Warm pastel pinks and lavenders',
    sidebar: '#232136',
    nav: '#232136',
    navHeader: '#2a273f',
    content: '#232136',
    editor: '#232136',
    border: '#393552',
    borderHover: '#44415a',
    borderSubtle: '#2a273f',
    textPrimary: '#e0def4',
    textSecondary: '#c5c1d8',
    textMuted: '#908caa',
    textFaint: '#6e6a86',
    modalBg: '#2a273f',
    surfaceCard: 'rgba(255,255,255,0.025)',
    surfaceHover: 'rgba(255,255,255,0.05)',
    ok: '#3e8fb0',
    warn: '#f6c177',
    err: '#eb6f92',
    tokens: { str: '#f6c177', num: '#ea9a97', key: '#9ccfd8', boo: '#ea9a97' },
    glass: false,
  },
  // Rose Pine Dawn — official light sister of Rose Pine Moon. Warm cream
  // base instead of harsh #ffffff. Palette is straight from the upstream
  // Rose Pine spec (base/surface/overlay/text/subtle/muted + accent roles
  // pine/foam/iris/rose/love/gold) and matches its VSCode/Helix/Neovim
  // ports so anything that uses the Rose Pine convention reads correctly.
  // The surfaceCard/Hover use rgba(0,0,0,…) rather than rgba(255,255,255,…)
  // because the dark-theme "lighten" wash needs to invert to "darken" on
  // a light base — the alpha contract still holds (legible at any depth).
  'rose-pine-dawn': {
    id: 'rose-pine-dawn',
    name: 'Rose Pine Dawn',
    description: 'Warm cream light — Rose Pine sister to Moon',
    sidebar: '#fffaf3',
    nav: '#faf4ed',
    navHeader: '#f2e9e1',
    content: '#faf4ed',
    editor: '#fffaf3',
    border: '#dfdad9',
    borderHover: '#cecacd',
    borderSubtle: '#f4ede8',
    textPrimary: '#575279',
    textSecondary: '#6e6a86',
    textMuted: '#797593',
    textFaint: '#9893a5',
    modalBg: '#fffaf3',
    surfaceCard: 'rgba(0,0,0,0.025)',
    surfaceHover: 'rgba(0,0,0,0.05)',
    ok: '#286983',
    warn: '#ea9d34',
    err: '#b4637a',
    tokens: { str: '#286983', num: '#d7827e', key: '#907aa9', boo: '#56949f' },
    glass: false,
    accent: '#907aa9',
  },
  // Atelier — premium theme. Soft blush-pink surfaces (NOT another dark
  // theme) with deep wine-ink text and a rose accent. A botanical SVG
  // wallpaper is painted ON TOP of the chrome via `body.fx-atelier::after`
  // with `mix-blend-mode: multiply` (see app.css), so the rose-wine line
  // art tints the whole window visibly — boudoir / Parisian-salon vibe.
  // Surfaces are FULLY OPAQUE here because visibility comes from the
  // overlay blend, not surface transmission. The walking critters in
  // CatsParade.svelte stay above the overlay (z-index 50 > overlay 5)
  // so they're not tinted; their warm tones harmonize with the rose accent.
  'atelier': {
    id: 'atelier',
    name: 'Atelier',
    description: 'Blush + botanical wallpaper — pixel critters wander the footer',
    sidebar: '#fbe8eb',
    nav: '#f9e0e5',
    navHeader: '#f3d4dc',
    content: '#fdf2f5',
    editor: '#fbe8eb',
    border: '#e6c4cd',
    borderHover: '#d9aab6',
    borderSubtle: '#f0d8de',
    textPrimary: '#4a1f2c',
    textSecondary: '#6e3a48',
    textMuted: '#9c6b78',
    textFaint: '#c49aa6',
    modalBg: '#fbe8eb',
    surfaceCard: 'rgba(74,31,44,0.03)',
    surfaceHover: 'rgba(74,31,44,0.06)',
    ok: '#5a8a6b',
    warn: '#c87a3f',
    err: '#b8395a',
    tokens: { str: '#5a8a6b', num: '#b8395a', key: '#7a4b8a', boo: '#c87a3f' },
    glass: false,
    accent: '#b8395a',
    effectClass: 'fx-atelier',
    premium: true,
  },
  // Petal — premium theme. Deep plum-violet surfaces with cherry-pink
  // accent. Animation: cherry blossom petals (PetalFall.svelte) drift
  // diagonally across the viewport while spinning — falling motion + spin
  // is mechanically different from Hearth's rising particles or Aurora
  // Drift's gradient swirl. Japanese garden at twilight aesthetic.
  'petal': {
    id: 'petal',
    name: 'Petal',
    description: 'Plum twilight with falling cherry petals',
    sidebar: '#221730',
    nav: '#1a0f28',
    navHeader: '#2c1d3d',
    content: '#14091e',
    editor: '#1a1126',
    border: '#3d2a4d',
    borderHover: '#523a66',
    borderSubtle: '#20152e',
    textPrimary: '#f4dde6',
    textSecondary: '#d8b8c8',
    textMuted: '#9a7d96',
    textFaint: '#5a4660',
    modalBg: '#221730',
    surfaceCard: 'rgba(244,221,230,0.04)',
    surfaceHover: 'rgba(244,221,230,0.07)',
    ok: '#98d4a8',
    warn: '#e8a875',
    err: '#e87592',
    tokens: { str: '#98d4a8', num: '#e8a875', key: '#b8a8e0', boo: '#e87592' },
    glass: false,
    accent: '#f4a5b8',
    effectClass: 'fx-petal',
    premium: true,
  },

  // Celeste — premium theme. Deep cosmic violet-black with silver
  // accent. Animation: 70 twinkling pinpoint stars (Starfield.svelte) at
  // independent pulse rates, plus 2 rare shooting stars streaking across
  // the viewport. Stationary pulse + rare burst — mechanically different
  // from every other theme's continuous motion. Night observatory feel.
  'celeste': {
    id: 'celeste',
    name: 'Celeste',
    description: 'Cosmic violet-black with twinkling stars',
    sidebar: '#0d0a1a',
    nav: '#08061a',
    navHeader: '#13102a',
    content: '#0a0814',
    editor: '#0d0a1a',
    border: '#25204a',
    borderHover: '#3a3565',
    borderSubtle: '#131028',
    textPrimary: '#e0d8f0',
    textSecondary: '#b8b0d0',
    textMuted: '#7a7595',
    textFaint: '#45405d',
    modalBg: '#0d0a1a',
    surfaceCard: 'rgba(224,216,240,0.04)',
    surfaceHover: 'rgba(224,216,240,0.07)',
    ok: '#a8d8c0',
    warn: '#d8b88c',
    err: '#d88c9c',
    tokens: { str: '#a8d8c0', num: '#d8b88c', key: '#a8b8e8', boo: '#d8a0e0' },
    glass: false,
    accent: '#a8b8e8',
    effectClass: 'fx-celeste',
    premium: true,
  },

  // Hearth — premium theme. Deep warm charcoal with amber accent, and
  // an animated swarm of rising embers (rendered by Embers.svelte, gated
  // to the active appearance.theme). A subtle bottom-radial firelight
  // glow on body.fx-hearth (see app.css) anchors the embers as if they
  // were drifting up from a hidden fire. Different KIND of animation
  // from Aurora Drift (particle motion vs gradient drift) so the two
  // premium themes feel distinct.
  'hearth': {
    id: 'hearth',
    name: 'Hearth',
    description: 'Warm charcoal with rising embers — animated firelight',
    sidebar: '#1c1812',
    nav: '#18140f',
    navHeader: '#25201a',
    content: '#14100c',
    editor: '#1a1612',
    border: '#3a3024',
    borderHover: '#4d4030',
    borderSubtle: '#20180f',
    textPrimary: '#f5d8b8',
    textSecondary: '#d9b888',
    textMuted: '#a08850',
    textFaint: '#5a4830',
    modalBg: '#1c1812',
    surfaceCard: 'rgba(245,216,184,0.04)',
    surfaceHover: 'rgba(245,216,184,0.07)',
    ok: '#b8c878',
    warn: '#ff8c42',
    err: '#d97462',
    tokens: { str: '#b8c878', num: '#ff8c42', key: '#88c4d4', boo: '#c4906a' },
    glass: false,
    accent: '#ff8c42',
    effectClass: 'fx-hearth',
    premium: true,
  },
  'carbon-grain': {
    id: 'carbon-grain',
    name: 'Carbon Grain',
    description: 'Warm-neutral dark with subtle film-grain texture',
    sidebar: '#14141a',
    nav: '#14141a',
    navHeader: '#1d1d24',
    content: '#0c0c0e',
    editor: '#14141a',
    border: '#2a2a32',
    borderHover: '#3a3a44',
    borderSubtle: '#1c1c22',
    textPrimary: '#d8d6cf',
    textSecondary: '#b8b6af',
    textMuted: '#8a887f',
    textFaint: '#5a5852',
    modalBg: '#14141a',
    surfaceCard: '#16161c',
    surfaceHover: '#1f1f26',
    ok: '#84a17d',
    warn: '#d4a373',
    err: '#c3736e',
    tokens: { str: '#b6c294', num: '#d4a373', key: '#a8b7d0', boo: '#d4a373' },
    glass: false,
    effectClass: 'fx-grain',
    premium: true,
  },
  'crt-phosphor': {
    id: 'crt-phosphor',
    name: 'CRT Phosphor',
    description: 'Retro green terminal with horizontal scanlines',
    sidebar: '#060a06',
    nav: '#060a06',
    navHeader: '#0a100a',
    content: '#060a06',
    editor: '#050805',
    border: '#1a2a1a',
    borderHover: '#2a4a2a',
    borderSubtle: '#102010',
    textPrimary: '#66ee66',
    textSecondary: '#4ecc4e',
    textMuted: '#3aa83a',
    textFaint: '#237823',
    modalBg: '#0a100a',
    surfaceCard: '#0a140a',
    surfaceHover: '#102010',
    ok: '#00ff80',
    warn: '#ffe066',
    err: '#ff7766',
    tokens: { str: '#b8ee88', num: '#ffe066', key: '#a8eecc', boo: '#ffe066' },
    glass: false,
    effectClass: 'fx-crt',
    premium: true,
  },
  'aurora-drift': {
    id: 'aurora-drift',
    name: 'Aurora Drift',
    description: 'Animated aurora gradient drifting behind the surfaces',
    sidebar: 'rgba(14,17,30,0.78)',
    nav: 'rgba(14,17,30,0.78)',
    navHeader: 'rgba(20,23,38,0.85)',
    content: 'rgba(14,17,30,0.72)',
    editor: 'rgba(20,23,38,0.82)',
    border: 'rgba(255,255,255,0.12)',
    borderHover: 'rgba(255,255,255,0.20)',
    borderSubtle: 'rgba(255,255,255,0.06)',
    textPrimary: '#e8e8f4',
    textSecondary: '#cfd0e4',
    textMuted: '#a0a4c0',
    textFaint: '#7078a0',
    modalBg: '#161928',
    surfaceCard: 'rgba(28,32,52,0.88)',
    surfaceHover: 'rgba(42,48,72,0.92)',
    ok: '#1dc880',
    warn: '#f5a623',
    err: '#f04444',
    tokens: { str: '#9ccfd8', num: '#f6c177', key: '#8cb4e0', boo: '#eb6f92' },
    glass: false,
    effectClass: 'fx-aurora',
    premium: true,
  },
};

export function applyTheme(themeId: string, accentColor?: string) {
  const theme = themes[themeId];
  if (!theme) return;

  const root = document.documentElement;
  root.style.setProperty('--s', theme.sidebar);
  root.style.setProperty('--n', theme.nav);
  root.style.setProperty('--n2', theme.navHeader);
  root.style.setProperty('--c', theme.content);
  root.style.setProperty('--e', theme.editor);
  root.style.setProperty('--b1', theme.border);
  root.style.setProperty('--b2', theme.borderHover);
  root.style.setProperty('--b-subtle', theme.borderSubtle);
  root.style.setProperty('--t1', theme.textPrimary);
  root.style.setProperty('--t2', theme.textSecondary);
  root.style.setProperty('--t3', theme.textMuted);
  root.style.setProperty('--t4', theme.textFaint);
  root.style.setProperty('--modal-bg', theme.modalBg);
  root.style.setProperty('--surface-card', theme.surfaceCard);
  root.style.setProperty('--surface-hover', theme.surfaceHover);

  // Locked themes force their designer-chosen accent and ignore the
  // user's picker. Free themes fall back to the user-supplied accent
  // (passed in from the appearance store) or leave the previous value
  // in place.
  if (theme.lockAccent && theme.accent) {
    root.style.setProperty('--acc', theme.accent);
  } else if (accentColor) {
    root.style.setProperty('--acc', accentColor);
  }

  // Status colors are now per-theme. The legacy `light-mode` body class is
  // intentionally not toggled here — the only theme that set it was removed,
  // and any `.light-mode` rules left in app.css are dormant. Re-introduce
  // the toggle if a light theme is added back later.
  root.style.setProperty('--ok', theme.ok);
  root.style.setProperty('--warn', theme.warn);
  root.style.setProperty('--err', theme.err);

  // Semantic state aliases (Phase 3). Defaults below match the prior
  // behavior we landed manually (saved/synced follow accent; live system
  // status keeps green via --ok). Themes can override either via
  // brandOverrides equivalents in a later phase — for now they piggyback
  // on --ok and --acc, which the theme already controls.
  root.style.setProperty('--state-live', theme.ok);
  root.style.setProperty('--state-saved', 'var(--acc)');
  root.style.setProperty('--state-info', 'var(--acc)');

  // Brand badge config — reactive store consumed by <BrandBadge>.
  brandConfig.set({
    overrides: theme.brandOverrides ?? {},
    display: theme.brandDisplay ?? 'text',
  });

  // Syntax token colors — read by `.str / .num / .key / .boo` in app.css
  // (the homemade JSON + document highlighters). Per-theme means each
  // theme owns its own JSON-viewer token palette.
  root.style.setProperty('--syntax-str', theme.tokens.str);
  root.style.setProperty('--syntax-num', theme.tokens.num);
  root.style.setProperty('--syntax-key', theme.tokens.key);
  root.style.setProperty('--syntax-boo', theme.tokens.boo);

  // Visual-effect class (e.g. `fx-aurora`). Strip any prior fx-* class
  // before applying this theme's so swaps clean up correctly. Effect CSS
  // lives in `app.css` keyed off the body class — pure CSS, no JS needed.
  for (const cls of Array.from(document.body.classList)) {
    if (cls.startsWith('fx-')) document.body.classList.remove(cls);
  }
  if (theme.effectClass) document.body.classList.add(theme.effectClass);

  // Glass-specific: add backdrop-filter class and set vibrancy
  if (theme.glass) {
    document.body.classList.add('glass-mode');
    setVibrancy('sidebar');
  } else {
    document.body.classList.remove('glass-mode');
    setVibrancy('none');
  }
}

export function getThemes() { return Object.values(themes); }
export function getTheme(id: string) { return themes[id]; }

export async function setVibrancy(material: string) {
  try {
    await invoke('set_vibrancy', { material });
  } catch (e) {
    console.warn('Vibrancy not supported:', e);
  }
}

// xterm.js terminal themes matched to each app theme
export const TERMINAL_THEMES: Record<string, Record<string, string>> = {
  'dark-glass': {
    background: '#0d0d18',
    foreground: '#e8e8f4',
    cursor: '#6366f1',
    cursorAccent: '#0d0d18',
    selectionBackground: 'rgba(99,102,241,0.3)',
    black: '#484858', red: '#ff7b72', green: '#3fb950', yellow: '#d29922',
    blue: '#58a6ff', magenta: '#d2a8ff', cyan: '#56d4dd', white: '#e6edf3',
    brightBlack: '#6e7681', brightRed: '#ffa198', brightGreen: '#56d364', brightYellow: '#e3b341',
    brightBlue: '#79c0ff', brightMagenta: '#d2a8ff', brightCyan: '#76e4f7', brightWhite: '#ffffff',
  },
  'dark-solid': {
    background: '#12121f',
    foreground: '#e4e4f0',
    cursor: '#6366f1',
    cursorAccent: '#12121f',
    selectionBackground: 'rgba(99,102,241,0.3)',
    black: '#484858', red: '#ff7b72', green: '#3fb950', yellow: '#d29922',
    blue: '#58a6ff', magenta: '#d2a8ff', cyan: '#56d4dd', white: '#e6edf3',
    brightBlack: '#6e7681', brightRed: '#ffa198', brightGreen: '#56d364', brightYellow: '#e3b341',
    brightBlue: '#79c0ff', brightMagenta: '#d2a8ff', brightCyan: '#76e4f7', brightWhite: '#ffffff',
  },
  'midnight': {
    background: '#000000',
    foreground: '#e8e8e8',
    cursor: '#6366f1',
    cursorAccent: '#000000',
    selectionBackground: 'rgba(99,102,241,0.25)',
    black: '#3a3a3a', red: '#ff7b72', green: '#3fb950', yellow: '#d29922',
    blue: '#58a6ff', magenta: '#d2a8ff', cyan: '#56d4dd', white: '#e8e8e8',
    brightBlack: '#666666', brightRed: '#ffa198', brightGreen: '#56d364', brightYellow: '#e3b341',
    brightBlue: '#79c0ff', brightMagenta: '#d2a8ff', brightCyan: '#76e4f7', brightWhite: '#ffffff',
  },
  // Rose Pine Moon terminal palette per the official VSCode theme
  // `terminal.ansi*` keys. Note: cyan maps to Rose (#ea9a97), not Foam —
  // that's the Rose Pine convention (warm-cyan).
  'rose-pine-moon': {
    background: '#232136',
    foreground: '#e0def4',
    cursor: '#e0def4',
    cursorAccent: '#232136',
    selectionBackground: 'rgba(129,124,156,0.3)',
    black: '#393552', red: '#eb6f92', green: '#3e8fb0', yellow: '#f6c177',
    blue: '#9ccfd8', magenta: '#c4a7e7', cyan: '#ea9a97', white: '#e0def4',
    brightBlack: '#6e6a86', brightRed: '#eb6f92', brightGreen: '#3e8fb0', brightYellow: '#f6c177',
    brightBlue: '#9ccfd8', brightMagenta: '#c4a7e7', brightCyan: '#ea9a97', brightWhite: '#e0def4',
  },
  // Rose Pine Dawn — per the official xterm spec. Same green=pine,
  // cyan=rose warm-cyan convention as Moon (intentional Rose Pine quirk).
  // `black` is overlay (the soft cream) so dim ANSI black reads as a low-
  // contrast neutral on the cream base instead of disappearing.
  'rose-pine-dawn': {
    background: '#faf4ed',
    foreground: '#575279',
    cursor: '#575279',
    cursorAccent: '#faf4ed',
    selectionBackground: 'rgba(110,106,134,0.3)',
    black: '#f2e9e1', red: '#b4637a', green: '#286983', yellow: '#ea9d34',
    blue: '#56949f', magenta: '#907aa9', cyan: '#d7827e', white: '#575279',
    brightBlack: '#9893a5', brightRed: '#b4637a', brightGreen: '#286983', brightYellow: '#ea9d34',
    brightBlue: '#56949f', brightMagenta: '#907aa9', brightCyan: '#d7827e', brightWhite: '#575279',
  },
  'atelier': {
    background: '#fdf2f5',
    foreground: '#4a1f2c',
    cursor: '#b8395a',
    cursorAccent: '#fdf2f5',
    selectionBackground: 'rgba(184,57,90,0.18)',
    black: '#e6c4cd', red: '#b8395a', green: '#5a8a6b', yellow: '#c87a3f',
    blue: '#5a7a9c', magenta: '#7a4b8a', cyan: '#5a9c9c', white: '#4a1f2c',
    brightBlack: '#c49aa6', brightRed: '#a32d4d', brightGreen: '#4a7a5b', brightYellow: '#b56934',
    brightBlue: '#4a6a8c', brightMagenta: '#6a3b7a', brightCyan: '#4a8a8a', brightWhite: '#2a0f17',
  },
  'petal': {
    background: '#14091e',
    foreground: '#f4dde6',
    cursor: '#f4a5b8',
    cursorAccent: '#14091e',
    selectionBackground: 'rgba(244,165,184,0.22)',
    black: '#3d2a4d', red: '#e87592', green: '#98d4a8', yellow: '#e8a875',
    blue: '#b8a8e0', magenta: '#d68ad0', cyan: '#a0c4d4', white: '#f4dde6',
    brightBlack: '#5a4660', brightRed: '#f0a0b5', brightGreen: '#b0e0bc', brightYellow: '#f0c094',
    brightBlue: '#cbc0e8', brightMagenta: '#e0a4dc', brightCyan: '#b8d0dc', brightWhite: '#ffffff',
  },
  'celeste': {
    background: '#0a0814',
    foreground: '#e0d8f0',
    cursor: '#a8b8e8',
    cursorAccent: '#0a0814',
    selectionBackground: 'rgba(168,184,232,0.22)',
    black: '#25204a', red: '#d88c9c', green: '#a8d8c0', yellow: '#d8b88c',
    blue: '#a8b8e8', magenta: '#d8a0e0', cyan: '#a0c4d8', white: '#e0d8f0',
    brightBlack: '#45405d', brightRed: '#e0a4b0', brightGreen: '#bce0cc', brightYellow: '#e4c8a4',
    brightBlue: '#bccaf0', brightMagenta: '#e4b4ec', brightCyan: '#b4d0e0', brightWhite: '#ffffff',
  },
  'hearth': {
    background: '#14100c',
    foreground: '#f5d8b8',
    cursor: '#ff8c42',
    cursorAccent: '#14100c',
    selectionBackground: 'rgba(255,140,66,0.22)',
    black: '#3a3024', red: '#d97462', green: '#b8c878', yellow: '#ff8c42',
    blue: '#88c4d4', magenta: '#c4906a', cyan: '#a0c4c4', white: '#f5d8b8',
    brightBlack: '#5a4830', brightRed: '#e89380', brightGreen: '#c8d489', brightYellow: '#ffaa6a',
    brightBlue: '#a4d4e0', brightMagenta: '#d6a585', brightCyan: '#b8d4d4', brightWhite: '#ffffff',
  },
  'carbon-grain': {
    background: '#0c0c0e',
    foreground: '#d8d6cf',
    cursor: '#d8d6cf',
    cursorAccent: '#0c0c0e',
    selectionBackground: 'rgba(168,183,208,0.22)',
    black: '#2a2a32', red: '#c3736e', green: '#84a17d', yellow: '#d4a373',
    blue: '#a8b7d0', magenta: '#b39bbf', cyan: '#8fb3b1', white: '#b8b6af',
    brightBlack: '#5a5852', brightRed: '#d68a85', brightGreen: '#a4bb9c', brightYellow: '#e6bd91',
    brightBlue: '#bdcae0', brightMagenta: '#c8b4d0', brightCyan: '#a9c8c6', brightWhite: '#e8e6df',
  },
  'crt-phosphor': {
    background: '#050805',
    foreground: '#66ee66',
    cursor: '#88ff88',
    cursorAccent: '#050805',
    selectionBackground: 'rgba(102,238,102,0.22)',
    black: '#1a2a1a', red: '#ff7766', green: '#66ee66', yellow: '#ffe066',
    blue: '#a8eecc', magenta: '#88ddaa', cyan: '#c0ffc0', white: '#b8ee88',
    brightBlack: '#3aa83a', brightRed: '#ff9988', brightGreen: '#88ff88', brightYellow: '#ffefa8',
    brightBlue: '#c0ffe0', brightMagenta: '#a8eecc', brightCyan: '#e0ffe0', brightWhite: '#ffffff',
  },
  'aurora-drift': {
    background: '#0e111e',
    foreground: '#e8e8f4',
    cursor: '#9aa6ff',
    cursorAccent: '#0e111e',
    selectionBackground: 'rgba(124,92,248,0.25)',
    black: '#3b3b55', red: '#ff7b9c', green: '#3fdda6', yellow: '#f6c177',
    blue: '#7aa2f7', magenta: '#c4a7e7', cyan: '#7fdcd0', white: '#cfd0e4',
    brightBlack: '#5a5a78', brightRed: '#ffa3bd', brightGreen: '#76e4be', brightYellow: '#ffd589',
    brightBlue: '#9cb8ff', brightMagenta: '#d9c5f0', brightCyan: '#a4ebe2', brightWhite: '#ffffff',
  },
};

/** Get xterm theme for a given app theme, with accent color as cursor.
 *  For glass themes, the background is swapped to a translucent rgba
 *  so the terminal canvas blends with the app's vibrancy chrome
 *  instead of painting an opaque rectangle over it. Solid themes
 *  keep their opaque hex — the terminal stays fully readable on a
 *  solid backdrop and is unaffected by this change. */
export function getTerminalTheme(themeId: string, accentColor?: string): Record<string, string> {
  const termTheme = TERMINAL_THEMES[themeId] || TERMINAL_THEMES['dark-glass'];
  const appTheme = themes[themeId];
  let background = termTheme.background;
  if (appTheme?.glass) {
    // Convert the opaque hex (e.g. #0d0d18) to rgba(13,13,24,0.55).
    // 0.55 keeps terminal text crisp while letting the vibrancy
    // bleed through enough that the panel reads as glass.
    const hex = termTheme.background.replace('#', '');
    if (hex.length === 6) {
      const r = parseInt(hex.slice(0, 2), 16);
      const g = parseInt(hex.slice(2, 4), 16);
      const b = parseInt(hex.slice(4, 6), 16);
      background = `rgba(${r},${g},${b},0.55)`;
    }
  }
  const merged: Record<string, string> = { ...termTheme, background };
  if (accentColor) merged.cursor = accentColor;
  return merged;
}

// Method colors for HTTP methods
export const METHOD_COLORS: Record<string, { color: string; bg: string }> = {
  GET:    { color: '#60a5fa', bg: '#162640' },
  POST:   { color: '#34d399', bg: '#0d2818' },
  PUT:    { color: '#fbbf24', bg: '#1c1808' },
  PATCH:  { color: '#c4b5fd', bg: '#1e162e' },
  DELETE: { color: '#f87171', bg: '#2a1010' },
};

export interface AppearanceConfig {
  theme: string;
  accentColor: string;
  /**
   * CSS `font-family` value applied to xterm.js terminals (agent + SSH panels).
   * Empty string means "use the built-in default stack" — see
   * `DEFAULT_TERMINAL_FONT_FAMILY` in `$lib/shared/primitives/terminal-utils`.
   */
  terminalFontFamily?: string;
}

// SQL dialect registry — TypeScript mirror of `src-tauri/src/modes/sql/dialects.rs`.
//
// Single source of truth for per-driver metadata (display names, default
// ports, abbreviations, parser profiles, capability flags) consumed by the
// SQL UI. Keep the entries here in lockstep with the Rust descriptor list.

export type SqlDialectKey = 'postgresql' | 'mysql' | 'sqlite';

export interface SqlDialectDescriptor {
  key: SqlDialectKey;
  displayName: string;
  abbreviation: string;
  defaultPort: number;
  usesHostPort: boolean;
  usesCredentials: boolean;
  /** Matches the CodeMirror lang-sql dialect name. */
  parserProfile: string | null;
  /** Quote character used for delimited identifiers in DDL emitted from
   * the UI (e.g. DROP DATABASE). MySQL uses backticks; the rest use
   * double quotes per ANSI SQL. */
  identifierQuote: string;
}

export const SQL_DIALECTS: readonly SqlDialectDescriptor[] = [
  { key: 'postgresql', displayName: 'PostgreSQL', abbreviation: 'PG', defaultPort: 5432, usesHostPort: true,  usesCredentials: true,  parserProfile: 'PostgreSQL', identifierQuote: '"' },
  { key: 'mysql',      displayName: 'MySQL',      abbreviation: 'MY', defaultPort: 3306, usesHostPort: true,  usesCredentials: true,  parserProfile: 'MySQL',      identifierQuote: '`' },
  { key: 'sqlite',     displayName: 'SQLite',     abbreviation: 'SL', defaultPort: 0,    usesHostPort: false, usesCredentials: false, parserProfile: 'SQLite',     identifierQuote: '"' },
] as const;

export function descriptorFor(key: string): SqlDialectDescriptor | undefined {
  return SQL_DIALECTS.find((d) => d.key === key);
}

export function defaultPortFor(key: string): number {
  return descriptorFor(key)?.defaultPort ?? 0;
}

/** Returns the parser profile for the given driver key, falling back to
 * `'PostgreSQL'` for unknown / not-yet-registered drivers — same default
 * the editor used before the registry existed. */
export function parserProfileFor(key: string): string {
  return descriptorFor(key)?.parserProfile ?? 'PostgreSQL';
}

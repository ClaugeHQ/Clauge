// SSH profile shape — mirrors the Rust `SshProfile` struct exactly.
// Tauri v2 returns Rust `Option<T>` as `T | null`; timestamps come back as
// ISO-8601 strings from the Rust serializer.

export type SshAuthType = 'key' | 'password' | 'agent';

// IMPORTANT: Rust SshProfile uses `#[serde(rename_all = "camelCase")]`.
// All multi-word field names below MUST be camelCase to match the JSON wire
// format — using snake_case here makes the field undefined at runtime.
export interface SshProfile {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  authType: SshAuthType;
  keyPath: string | null;
  accentColor: string | null;
  lastUsedAt: string | null;
  createdAt: string;
}

export interface SshCreateProfileArgs {
  name: string;
  host: string;
  port: number;
  username: string;
  authType: SshAuthType;
  keyPath?: string | null;
  accentColor?: string | null;
  // Secret material — never persisted in the DB, sent straight to Keychain.
  secret?: string | null;
  passphrase?: string | null;
}

export interface SshUpdateProfileArgs {
  id: string;
  name?: string;
  host?: string;
  port?: number;
  username?: string;
  authType?: SshAuthType;
  keyPath?: string | null;
  accentColor?: string | null;
  // If provided, replace the existing Keychain secret. If undefined, keep.
  secret?: string | null;
  passphrase?: string | null;
}

// Channel payload from Rust — same shape used by Agent terminal.
export interface SshTerminalPayload {
  data?: string; // base64-encoded chunk
  exit?: boolean;
}

// One host parsed from ~/.ssh/config. `alreadyExists` means a profile
// with the same name (== ssh_config alias) is already in the DB and
// should be excluded from import (UI shows it greyed out).
export interface SshConfigHost {
  alias: string;
  hostname: string;
  user: string | null;
  port: number;
  identityFile: string | null;
  alreadyExists: boolean;
}

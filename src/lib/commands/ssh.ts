import { invoke } from '@tauri-apps/api/core';
import type { SshProfile, SshCreateProfileArgs, SshUpdateProfileArgs } from '$lib/types/ssh';

// ── Profile CRUD ─────────────────────────────────────────────────────────────

export const sshListProfiles = () => invoke<SshProfile[]>('ssh_list_profiles');

export const sshCreateProfile = (args: SshCreateProfileArgs) =>
  invoke<SshProfile>('ssh_create_profile', {
    name: args.name,
    host: args.host,
    port: args.port,
    username: args.username,
    authType: args.authType,
    keyPath: args.keyPath ?? null,
    accentColor: args.accentColor ?? null,
    secret: args.secret ?? null,
    passphrase: args.passphrase ?? null,
  });

export const sshUpdateProfile = (args: SshUpdateProfileArgs) =>
  invoke<SshProfile>('ssh_update_profile', {
    id: args.id,
    name: args.name,
    host: args.host,
    port: args.port,
    username: args.username,
    authType: args.authType,
    keyPath: args.keyPath ?? null,
    accentColor: args.accentColor ?? null,
    secret: args.secret ?? null,
    passphrase: args.passphrase ?? null,
  });

export const sshDeleteProfile = (id: string) => invoke<void>('ssh_delete_profile', { id });

export const sshTouchProfile = (id: string) => invoke<void>('ssh_touch_profile', { id });

// ── Terminal ─────────────────────────────────────────────────────────────────

export const sshSpawnTerminal = (profileId: string, channel: any) =>
  invoke<string>('ssh_spawn_terminal', { profileId, onOutput: channel });

// xterm's `onData` hands us a JS string. The Rust side expects base64-encoded
// bytes (same protocol as the Agent terminal). Mirror agentWriteToTerminal's
// encoding pattern: encode the JS string as UTF-8 bytes, then base64.
function encodeUtf8ToBase64(input: string): string {
  // TextEncoder gives correct UTF-8 bytes; Latin1 string + btoa is the standard
  // Tauri pattern for base64-from-binary used elsewhere in the codebase.
  const bytes = new TextEncoder().encode(input);
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

export const sshWriteToTerminal = (terminalId: string, data: string) =>
  invoke<void>('ssh_write_to_terminal', { terminalId, data: encodeUtf8ToBase64(data) });

export const sshResizeTerminal = (terminalId: string, cols: number, rows: number) =>
  invoke<void>('ssh_resize_terminal', { terminalId, cols, rows });

export const sshKillTerminal = (terminalId: string) =>
  invoke<void>('ssh_kill_terminal', { terminalId });

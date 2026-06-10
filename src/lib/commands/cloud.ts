import { invoke } from '@tauri-apps/api/core';
import type { CloudUser, CloudProviderLink, Provider } from '$lib/stores/cloud';

export interface CloudEntitlements {
  plan: string;
  credits?: {
    remaining: number;
    allowance: number;
    resets_at: string | null;
  };
  subscription?: {
    status: string;
    cancel_at_period_end: boolean;
    is_lifetime?: boolean;
    current_period_end?: string | null;
    current_period_start?: string | null;
    interval?: 'monthly' | 'yearly' | 'lifetime' | null;
    price_usd?: number | null;
  };
}

export interface CloudStatus {
  connected: boolean;
  activeProvider: Provider | null;
  user: CloudUser | null;
  providers: CloudProviderLink[];
  plan: string;
  lastSynced: Record<string, string>;
  entitlements?: CloudEntitlements;
}

export const cloudGetStatus = () =>
  invoke<CloudStatus>('cloud_get_status');

export interface MissingCredentials {
  ssh: string[];
  sql: string[];
  nosql: string[];
  explorer: string[];
}

export const cloudProbeMissingCredentials = () =>
  invoke<MissingCredentials>('cloud_probe_missing_credentials');

export const cloudGithubLoginUrl = () =>
  invoke<string>('cloud_github_login_url');

export const cloudGoogleLoginUrl = () =>
  invoke<string>('cloud_google_login_url');

export const cloudExchangeCode = (provider: Provider, code: string) =>
  invoke<CloudStatus>('cloud_exchange_code', { provider, code });

export const cloudLinkProvider = (provider: Provider, code: string) =>
  invoke<CloudStatus>('cloud_link_provider', { provider, code });

export const cloudUnlinkProvider = (provider: Provider) =>
  invoke<CloudStatus>('cloud_unlink_provider', { provider });

export const cloudUpdateProfile = (fields: {
  displayName?: string;
  firstName?: string;
  lastName?: string;
}) => invoke<CloudStatus>('cloud_update_profile', {
  displayName: fields.displayName,
  firstName: fields.firstName,
  lastName: fields.lastName,
});

export const cloudCheckRemoteExists = () =>
  invoke<boolean>('cloud_check_remote_exists');

export const cloudSyncPushNow = () =>
  invoke<string[]>('cloud_sync_push_now');

export const cloudSyncRestore = () =>
  invoke<string[]>('cloud_sync_restore');

export const cloudGetConflicts = () =>
  invoke<string[]>('cloud_get_conflicts');

export const cloudResolveKeepLocal = () =>
  invoke<void>('cloud_resolve_keep_local');

export const cloudResolveUseRemote = () =>
  invoke<void>('cloud_resolve_use_remote');

export const cloudPullIfRemoteNewer = () =>
  invoke<string[]>('cloud_pull_if_remote_newer');

export const cloudLocalHasData = () =>
  invoke<boolean>('cloud_local_has_data');

export const cloudLogout = () =>
  invoke<void>('cloud_logout');

export const cloudWipeRemote = () =>
  invoke<void>('cloud_wipe_remote');

export const cloudDeleteAccount = (confirmationSlug: string) =>
  invoke<void>('cloud_delete_account', { confirmationSlug });

export interface SnapshotInfo {
  fileName: string;
  kind: string;
  reason: string;
  createdAt: string;
  sizeBytes: number;
}

export const cloudListSnapshots = () =>
  invoke<SnapshotInfo[]>('cloud_list_snapshots');

export const cloudRestoreSnapshot = (fileName: string) =>
  invoke<void>('cloud_restore_snapshot', { fileName });

import type { ProState } from '$lib/stores/cloud';

/** Read the current in-memory ProState from Rust. Called once at boot before
 *  the `cloud:pro-state` event subscription is set up — gives the frontend
 *  the latest known state immediately without waiting for a state-change
 *  event that may not fire on the boot path. */
export const proStateCurrent = () => invoke<ProState>('pro_state_current');

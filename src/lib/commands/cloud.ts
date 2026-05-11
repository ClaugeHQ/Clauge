import { invoke } from '@tauri-apps/api/core';
import type { CloudUser, CloudProviderLink, Provider } from '$lib/stores/cloud';

export interface CloudStatus {
  connected: boolean;
  activeProvider: Provider | null;
  user: CloudUser | null;
  providers: CloudProviderLink[];
  plan: string;
  lastSynced: Record<string, string>;
}

export const cloudGetStatus = () =>
  invoke<CloudStatus>('cloud_get_status');

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

export const cloudLocalHasData = () =>
  invoke<boolean>('cloud_local_has_data');

export const cloudLogout = () =>
  invoke<void>('cloud_logout');

export const cloudWipeRemote = () =>
  invoke<void>('cloud_wipe_remote');

export const cloudDeleteAccount = (confirmationSlug: string) =>
  invoke<void>('cloud_delete_account', { confirmationSlug });

import { invoke } from '@tauri-apps/api/core';

export interface CompanionStatus {
  running: boolean;
  port: number | null;
}

export interface PairCodeInfo {
  hosts: string[];
  port: number;
  code: string;
}

export interface CompanionDevice {
  id: string;
  name: string;
  platform: string;
  createdAt: string;
  lastSeenAt: string | null;
  revoked: boolean;
}

export const companionStatus = () =>
  invoke<CompanionStatus>('companion_status');

export const companionStart = () =>
  invoke<CompanionStatus>('companion_start');

export const companionStop = () =>
  invoke<CompanionStatus>('companion_stop');

export const companionNewPairCode = () =>
  invoke<PairCodeInfo>('companion_new_pair_code');

export const companionListDevices = () =>
  invoke<CompanionDevice[]>('companion_list_devices');

export const companionRevokeDevice = (deviceId: string) =>
  invoke<void>('companion_revoke_device', { deviceId });

export const companionApprovePair = (requestId: string) =>
  invoke<void>('companion_approve_pair', { requestId });

export const companionDenyPair = (requestId: string) =>
  invoke<void>('companion_deny_pair', { requestId });

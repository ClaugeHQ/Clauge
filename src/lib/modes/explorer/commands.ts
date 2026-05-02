import { invoke } from '@tauri-apps/api/core';
import type { ExplorerConnection, DirEntry, Stat } from './types';

// ─── Connection CRUD ──────────────────────────────────────────────────

export const listConnections = () =>
  invoke<ExplorerConnection[]>('explorer_list_connections');

export const getConnection = (id: string) =>
  invoke<ExplorerConnection | null>('explorer_get_connection', { id });

export const createConnection = (connection: ExplorerConnection) =>
  invoke<ExplorerConnection>('explorer_create_connection', { connection });

export const updateConnection = (connection: ExplorerConnection) =>
  invoke<void>('explorer_update_connection', { connection });

export const deleteConnection = (id: string) =>
  invoke<void>('explorer_delete_connection', { id });

// ─── Secrets ──────────────────────────────────────────────────────────

export const setSecret = (connectionId: string, secretName: string, value: string) =>
  invoke<void>('explorer_set_secret', { connectionId, secretName, value });

export const getSecret = (connectionId: string, secretName: string) =>
  invoke<string | null>('explorer_get_secret', { connectionId, secretName });

export const deleteSecrets = (connectionId: string) =>
  invoke<void>('explorer_delete_secrets', { connectionId });

// ─── Session lifecycle ────────────────────────────────────────────────

export const openSession = (connectionId: string, tabKey: string) =>
  invoke<void>('explorer_open_session', { connectionId, tabKey });

export const closeSession = (tabKey: string) =>
  invoke<void>('explorer_close_session', { tabKey });

// ─── File system operations ──────────────────────────────────────────

export const fsList = (tabKey: string, path: string) =>
  invoke<DirEntry[]>('explorer_fs_list', { tabKey, path });

export const fsStat = (tabKey: string, path: string) =>
  invoke<Stat>('explorer_fs_stat', { tabKey, path });

/** Returns base64-encoded bytes. */
export const fsRead = (tabKey: string, path: string, maxBytes?: number) =>
  invoke<string>('explorer_fs_read', { tabKey, path, maxBytes });

/** `contentB64` is base64-encoded payload. */
export const fsWrite = (tabKey: string, path: string, contentB64: string) =>
  invoke<void>('explorer_fs_write', { tabKey, path, contentB64 });

export const fsDelete = (tabKey: string, paths: string[]) =>
  invoke<void>('explorer_fs_delete', { tabKey, paths });

export const fsMkdir = (tabKey: string, path: string) =>
  invoke<void>('explorer_fs_mkdir', { tabKey, path });

export const fsRename = (tabKey: string, from: string, to: string) =>
  invoke<void>('explorer_fs_rename', { tabKey, from, to });

export const fsSearch = (tabKey: string, prefix: string, glob: string) =>
  invoke<DirEntry[]>('explorer_fs_search', { tabKey, prefix, glob });

/** Server-side default starting directory ("remote home"). SFTP returns
 *  realpath("."), FTP returns PWD, S3/Azure return null. */
export const fsHomeDir = (tabKey: string) =>
  invoke<string | null>('explorer_fs_home_dir', { tabKey });

export const fsGetUrl = (tabKey: string, path: string, ttlSecs?: number) =>
  invoke<string | null>('explorer_fs_get_url', { tabKey, path, ttlSecs });

// ─── Transfers (upload / download / cancel) ──────────────────────────

export const uploadFile = (
  transferId: string,
  tabKey: string,
  localPath: string,
  remotePath: string,
) => invoke<void>('explorer_upload_file', { transferId, tabKey, localPath, remotePath });

export const downloadFile = (
  transferId: string,
  tabKey: string,
  remotePath: string,
  localPath: string,
) => invoke<void>('explorer_download_file', { transferId, tabKey, remotePath, localPath });

export const cancelTransfer = (transferId: string) =>
  invoke<boolean>('explorer_cancel_transfer', { transferId });

/** Explorer mode shared TypeScript types — mirrors src-tauri Rust models. */

export type ExplorerKind = 'sftp' | 'ftp' | 's3' | 'azure_blob';

export interface ExplorerConnection {
  id: string;
  name: string;
  kind: ExplorerKind;
  accentColor: string | null;
  lastUsedAt: string | null;
  createdAt: string;

  // SFTP — preferred path: link to an existing ssh_profiles row.
  sshProfileId: string | null;
  sftpWorkingDir: string | null;

  // SFTP-direct + FTP shared.
  host: string | null;
  port: number | null;
  username: string | null;
  authType: string | null;
  keyPath: string | null;

  // FTP-specific.
  ftpPassive: number;
  ftpTls: 'none' | 'explicit' | 'implicit' | null;

  // S3-specific.
  s3Preset: string | null;
  s3Endpoint: string | null;
  s3Region: string | null;
  s3Bucket: string | null;
  s3PathStyle: number;

  // Azure Blob-specific.
  azureAccount: string | null;
  azureContainer: string | null;
  azureAuthKind: 'shared_key' | 'sas' | 'connection_string' | null;
}

export interface DirEntry {
  name: string;
  /** Full POSIX path with leading slash. */
  path: string;
  kind: 'file' | 'dir' | 'symlink' | 'other';
  size: number | null;
  /** RFC3339 with millisecond precision. */
  modified: string | null;
  /** chmod-style "drwxr-xr-x" for SFTP/FTP; null for S3/Azure. */
  permissions: string | null;
  symlinkTarget: string | null;
}

export interface Stat {
  kind: string;
  size: number | null;
  modified: string | null;
  permissions: string | null;
  mime: string | null;
  isBinary: boolean | null;
}

export interface Transfer {
  id: string;
  direction: 'upload' | 'download';
  /** File name for display — derived backend-side from local/remote path. */
  name: string;
  localPath: string;
  remotePath: string;
  bytesTotal: number | null;
  bytesDone: number;
  state: 'running' | 'completed' | 'failed' | 'cancelled';
  error: string | null;
  /** RFC3339 timestamp; set when added on the frontend. */
  startedAt: string;
  /** Set when state transitions to a terminal value. */
  completedAt: string | null;
}

/** Base structure for the four connection-modal forms. Each modal gathers
 *  the kind-specific fields and the shared fields (name, accentColor). */
export type ExplorerConnectionDraft = Partial<ExplorerConnection> & {
  kind: ExplorerKind;
  name: string;
};

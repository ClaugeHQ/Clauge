import { invoke } from '@tauri-apps/api/core';
import type {
  AgentSession,
  AgentContext,
  DiscoveredSession,
  ContextUsage,
  TokenUsage,
  GitFileChange,
  UsageAnalytics,
  ClaudePlugin,
  MarketplacePlugin,
} from './types';

// Session CRUD
export const agentListSessions = () => invoke<AgentSession[]>('agent_list_sessions');
export const agentCreateSession = (params: {
  title: string;
  purpose: string;
  projectPath: string;
  skipPermissions?: boolean;
  customPrompt?: string;
  gitName?: string;
  gitEmail?: string;
  /** 'claude' | 'codex' | 'opencode'. Omit for Claude default. */
  provider?: string;
  /** Absolute path to the CLI binary (when the user picked one in the
   *  Advanced section). Omit / empty string = use $PATH lookup. */
  binaryPath?: string;
}) => invoke<AgentSession>('agent_create_session', params);
export const agentUpdateSession = (params: {
  id: string;
  title?: string;
  skipPermissions?: boolean;
  gitName?: string;
  gitEmail?: string;
  contextPrompt?: string;
  /** Pass an empty string to CLEAR the per-session binary override
   *  (restore $PATH lookup). Omit entirely to leave it untouched. */
  binaryPath?: string;
}) => invoke<void>('agent_update_session', params);
export const agentDeleteSession = (id: string) => invoke<void>('agent_delete_session', { id });
export const agentUpdateSessionId = (id: string, claudeSessionId: string) => invoke<void>('agent_update_session_id', { id, claudeSessionId });
export const agentUpdateLastUsed = (id: string) => invoke<void>('agent_update_last_used', { id });
export const agentUpdateWorktree = (id: string, worktreePath: string | null, worktreeBranch: string | null) => invoke<void>('agent_update_worktree', { id, worktreePath, worktreeBranch });

// Context CRUD
export const agentListContexts = () => invoke<AgentContext[]>('agent_list_contexts');
export const agentSaveContext = (params: { id?: string; name: string; content: string }) => invoke<AgentContext>('agent_save_context', params);
export const agentDeleteContext = (id: string) => invoke<void>('agent_delete_context', { id });
export const agentGetSessionContexts = (sessionId: string) => invoke<AgentContext[]>('agent_get_session_contexts', { sessionId });
export const agentAttachContext = (sessionId: string, contextId: string) => invoke<void>('agent_attach_context', { sessionId, contextId });
export const agentDetachContext = (sessionId: string, contextId: string) => invoke<void>('agent_detach_context', { sessionId, contextId });
export const agentInjectContexts = (projectPath: string, contextIds: string[], provider?: string) =>
  invoke<void>('agent_inject_contexts', { projectPath, contextIds, provider });
export const agentRemoveInjectedContexts = (projectPath: string) => invoke<void>('agent_remove_injected_contexts', { projectPath });
/** Write the session's purpose prompt into the provider's project-level
 *  context file (e.g. GEMINI.md) within a Clauge-managed marker block.
 *  Currently only takes effect for Gemini — every other provider has a
 *  real system-prompt flag and uses it directly at spawn. Safe to call
 *  for any provider; non-Gemini calls are no-ops on the Rust side. */
export const agentInjectPurpose = (projectPath: string, provider: string, purposePrompt: string) =>
  invoke<void>('agent_inject_purpose', { projectPath, provider, purposePrompt });

// Terminal
export const agentSpawnTerminal = (params: {
  sessionId?: string;
  /** Canonical session row id. Stamped as the terminal's session_ref so
   *  the mobile companion can match this live terminal to its row for
   *  every provider (codex/opencode produce no resume id). */
  rowId?: string;
  projectPath: string;
  contextPrompt?: string;
  skipPermissions?: boolean;
  gitName?: string;
  gitEmail?: string;
  /** Which CLI to spawn — 'claude' | 'codex' | 'opencode'. Defaults to Claude. */
  provider?: string;
  /** Absolute binary path override for this session. Omit / empty
   *  string = use the standard $PATH lookup. */
  binaryPath?: string;
  onOutput: any;
}) => invoke<string>('agent_spawn_terminal', params);
export const agentSpawnShell = (projectPath: string, onOutput: any) => invoke<string>('agent_spawn_shell', { projectPath, onOutput });
export const agentWriteToTerminal = (terminalId: string, data: string) => invoke<void>('agent_write_to_terminal', { terminalId, data });
export const agentResizeTerminal = (terminalId: string, cols: number, rows: number) => invoke<void>('agent_resize_terminal', { terminalId, cols, rows });
export const agentKillTerminal = (terminalId: string) => invoke<void>('agent_kill_terminal', { terminalId });

// Local file explorer
export interface FsEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
  modified: number;
}
export interface FileContent {
  content: string | null;
  isBinary: boolean;
  size: number;
  tooLarge: boolean;
}
export interface FsChange {
  paths: string[];
}
export const agentFsListDir = (path: string) => invoke<FsEntry[]>('agent_fs_list_dir', { path });
export const agentFsReadFile = (path: string) => invoke<FileContent>('agent_fs_read_file', { path });
export const agentFsWriteFile = (path: string, content: string) => invoke<void>('agent_fs_write_file', { path, content });
export const agentFsRename = (from: string, to: string) => invoke<void>('agent_fs_rename', { from, to });
export const agentFsDelete = (path: string) => invoke<void>('agent_fs_delete', { path });
export const agentFsCreate = (path: string, isDir: boolean) => invoke<void>('agent_fs_create', { path, isDir });
export const agentFsReveal = (path: string) => invoke<void>('agent_fs_reveal', { path });
export const agentFsWatchStart = (path: string, onEvent: any) => invoke<void>('agent_fs_watch_start', { path, onEvent });
export const agentFsWatchStop = () => invoke<void>('agent_fs_watch_stop');
export const agentFileReference = (provider: string, relPath: string) => invoke<string>('agent_file_reference', { provider, relPath });

// Worktree
export const agentIsGitRepo = (path: string) => invoke<boolean>('agent_is_git_repo', { path });
export const agentCreateWorktree = (projectPath: string, branchName: string) => invoke<string>('agent_create_worktree', { projectPath, branchName });
export const agentRemoveWorktree = (projectPath: string, worktreePath: string) => invoke<void>('agent_remove_worktree', { projectPath, worktreePath });
export const agentWorktreeIsDirty = (worktreePath: string) => invoke<boolean>('agent_worktree_is_dirty', { worktreePath });

// Git — all use projectPath (camelCase for Tauri v2 auto-conversion to project_path)
export const agentGitStatus = (projectPath: string) => invoke<GitFileChange[]>('agent_git_status', { projectPath });
export const agentGitBranch = (projectPath: string) => invoke<string>('agent_git_branch', { projectPath });
export const agentGitAheadBehind = (projectPath: string) => invoke<[number, number]>('agent_git_ahead_behind', { projectPath });
export const agentGitCommit = (projectPath: string, message: string) => invoke<string>('agent_git_commit', { projectPath, message });
export const agentGitPush = (projectPath: string) => invoke<string>('agent_git_push', { projectPath });
export const agentGitPull = (projectPath: string) => invoke<string>('agent_git_pull', { projectPath });
export const agentGitDiffFile = (projectPath: string, filePath: string) => invoke<string>('agent_git_diff_file', { projectPath, filePath });
export const agentGitStageFile = (projectPath: string, filePath: string) => invoke<void>('agent_git_stage_file', { projectPath, filePath });
export const agentGitUnstageFile = (projectPath: string, filePath: string) => invoke<void>('agent_git_unstage_file', { projectPath, filePath });
export const agentGitLog = (projectPath: string, limit?: number) => invoke<any[]>('agent_git_log', { projectPath, limit });
export const agentGitStash = (projectPath: string) => invoke<string>('agent_git_stash', { projectPath });
export const agentGitStashPop = (projectPath: string) => invoke<string>('agent_git_stash_pop', { projectPath });
export const agentGitListBranches = (projectPath: string) => invoke<any[]>('agent_git_list_branches', { projectPath });
export const agentGitSwitchBranch = (projectPath: string, branchName: string) => invoke<void>('agent_git_switch_branch', { projectPath, branchName });

// Plugins — `provider` selects which CLI's plugin universe to query
// ('claude' | 'codex'). OpenCode returns an empty list since it uses npm.
// Omit for the legacy Claude-only path.
export const agentGetPlugins = (provider?: string) => invoke<ClaudePlugin[]>('agent_get_plugins', { provider });
export const agentTogglePlugin = (pluginKey: string, enabled: boolean, provider?: string) =>
  invoke<void>('agent_toggle_plugin', { provider, pluginKey, enabled });
export const agentGetMarketplacePlugins = (provider?: string) =>
  invoke<MarketplacePlugin[]>('agent_get_marketplace_plugins', { provider });
export const agentInstallPlugin = (name: string, marketplace: string, provider?: string) =>
  invoke<void>('agent_install_plugin', { provider, name, marketplace });
export const agentUninstallPlugin = (name: string, marketplace: string, provider?: string) =>
  invoke<void>('agent_uninstall_plugin', { provider, name, marketplace });

// Check whether a given provider's CLI binary is installed on PATH.
// Used post-spawn to decide whether to show an install guide on
// failure; no longer drives any UI gating in the New Session picker.
export const agentCheckCliInstalled = (provider: string) =>
  invoke<boolean>('agent_check_cli_installed', { provider });

/** Probe a custom binary path by running `<path> --version` with a
 *  3-second timeout. Returns the stdout banner on success, or rejects
 *  with the stderr / error reason. Used by the Advanced > Custom
 *  Binary Path picker in NewSessionModal / EditSessionModal — devs
 *  can still save a path that fails this probe (it's a hint, not a
 *  gate). */
export const agentValidateBinary = (path: string) =>
  invoke<string>('agent_validate_binary', { path });

// Usage
export const agentGetUsageAnalytics = (days?: number, provider?: string) =>
  invoke<UsageAnalytics>('agent_get_usage_analytics', { days, provider });
export const agentFetchUsageLimits = (sessionKey: string) => invoke<any>('agent_fetch_usage_limits', { sessionKey });
export const agentFetchCodexUsageLimits = (accessToken: string) => invoke<any>('agent_fetch_codex_usage_limits', { accessToken });
export const agentDiscoverSessions = (projectPath: string, provider?: string) =>
  invoke<DiscoveredSession[]>('agent_discover_sessions', { projectPath, provider });
export const agentResolveResumeId = (projectPath: string, provider?: string) =>
  invoke<string | null>('agent_resolve_resume_id', { projectPath, provider });
export const agentGetSessionTokens = (projectPath: string, sessionId?: string) => invoke<TokenUsage>('agent_get_session_tokens', { projectPath, sessionId });
export const agentGetSessionContextUsage = (projectPath: string, sessionId: string, provider?: string) =>
  invoke<ContextUsage>('agent_get_session_context_usage', { projectPath, sessionId, provider });

// System
export const agentUpdateTrayTitle = (title: string) => invoke<void>('agent_update_tray_title', { title });
export const agentGetClaudePlan = () => invoke<string>('agent_get_claude_plan');
export const agentCheckClaudeInstalled = () => invoke<boolean>('agent_check_claude_installed');

export interface AgentSession {
  id: string;
  title: string;
  purpose: string;
  projectPath: string;
  projectName: string;
  claudeSessionId: string | null;
  contextPrompt: string;
  worktreePath: string | null;
  worktreeBranch: string | null;
  skipPermissions: number;
  gitName: string | null;
  gitEmail: string | null;
  createdAt: string;
  lastUsedAt: string;
  /** 'manual' = user-spawned terminal session (Agent panel).
   *  'card'   = drawer-spawned hidden session for a workspace card. */
  origin: string;
  /** Backref to the card that owns this hidden session. null otherwise. */
  cardId: string | null;
  /** 'claude' | 'codex' | 'gemini' | 'opencode'. */
  provider: string;
  /** Per-session override of the CLI binary location. null/empty =
   *  fall back to the standard $PATH lookup. Used when the user has
   *  the agent installed somewhere `find_binary` can't reach. */
  binaryPath: string | null;
}

/** Provider ids that map to a `CliRunner` in `runner_for`. */
export type AgentProvider = 'claude' | 'codex' | 'gemini' | 'opencode';
export const AGENT_PROVIDERS: { id: AgentProvider; label: string }[] = [
  { id: 'claude', label: 'Claude' },
  { id: 'codex', label: 'Codex' },
  // Gemini CLI was replaced by Antigravity CLI (`agy`) on 2026-06-18.
  // Internal id stays so existing sessions and coworkers keep working.
  { id: 'gemini', label: 'Antigravity' },
  { id: 'opencode', label: 'OpenCode' },
];

export interface AgentContext {
  id: string;
  name: string;
  content: string;
  createdAt: string;
  updatedAt: string;
}

export interface TokenUsage {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalTokens: number;
}

export interface DiscoveredSession {
  sessionId: string;
  modifiedAt: string;
  preview: string | null;
}

export interface ContextUsage {
  inputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalContextTokens: number;
  contextWindow: number;
  fillPercent: number;
  model: string;
  compacted: boolean;
}

export interface GitFileChange {
  path: string;
  status: string;
}

export interface UsageAnalytics {
  totalCost: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheReadTokens: number;
  totalCacheWriteTokens: number;
  totalSessions: number;
  totalApiCalls: number;
  cacheHitPercent: number;
  daily: DailyUsage[];
  byModel: ModelUsage[];
  byProject: ProjectUsage[];
  topSessions: SessionCost[];
  tools: ToolCount[];
  shellCommands: ToolCount[];
}

export interface DailyUsage { date: string; cost: number; calls: number; inputTokens: number; outputTokens: number; }
export interface ModelUsage { model: string; cost: number; calls: number; inputTokens: number; outputTokens: number; cacheHitPercent: number; }
export interface ProjectUsage { project: string; cost: number; sessions: number; calls: number; }
export interface SessionCost { sessionId: string; project: string; cost: number; calls: number; model: string; }
export interface ToolCount { name: string; count: number; }

export interface ClaudePlugin {
  name: string;
  marketplace: string;
  enabled: boolean;
  version: string | null;
  installPath: string | null;
}

export interface MarketplacePlugin {
  name: string;
  description: string;
  marketplace: string;
  category: string | null;
  installed: boolean;
  installs: number | null;
}

import { writable, get } from 'svelte/store';
import type { AgentSession, AgentContext, ContextUsage, GitFileChange } from './types';
import { agentListSessions, agentListContexts, agentGitStatus, agentGitBranch, agentGitAheadBehind, agentGetSessionContextUsage, agentFetchUsageLimits, agentFetchCodexUsageLimits, agentUpdateTrayTitle, agentGetClaudePlan } from './commands';

// Sessions
export const agentSessions = writable<AgentSession[]>([]);
export const activeAgentSession = writable<AgentSession | null>(null);

// (The provider install-state pre-warm was removed: the New Session
// modal no longer disables provider tiles. The spawn-time check still
// triggers the per-provider install guide if/when a real spawn fails.)

// Terminal tracking (frontend-only state)
export const agentTerminalMap = writable<Map<string, any>>(new Map());
export const agentShellMap = writable<Map<string, any>>(new Map());
export const agentTerminalIds = writable<Map<string, string>>(new Map());
export const agentShellIds = writable<Map<string, string>>(new Map());
export const agentShellOpen = writable<boolean>(false);

// Context usage per session
export const agentContextUsage = writable<Map<string, ContextUsage>>(new Map());

// Git state for active session
export const agentGitBranchName = writable<string>('');
export const agentGitFiles = writable<GitFileChange[]>([]);
export const agentGitAhead = writable<number>(0);
export const agentGitBehind = writable<number>(0);

// Contexts
export const agentContexts = writable<AgentContext[]>([]);

// Session activity tracking
export const agentSessionActivity = writable<Map<string, 'running' | 'done'>>(new Map());

// Per-session "awaiting input" tracking. Holds the sessionIds currently
// waiting on the user (a prompt was detected and not yet answered). This is
// the inverse of agentSessionActivity and drives the AgentNav badge plus the
// dock-bounce/chime alert. Cleared authoritatively by the backend
// `agent-attention-cleared` event when input is sent from any source.
export const agentSessionAwaiting = writable<Set<string>>(new Set());

// Notification preferences (loaded from settings)
export const agentSoundEnabled = writable<boolean>(true);
export const agentDockBounceEnabled = writable<boolean>(true);

// Usage limits (raw payload from whichever provider is selected for the
// footer chip — Claude session-key API or Codex/ChatGPT wham/usage). The
// shape differs per provider; StatusBar.svelte detects which by inspecting
// the payload.
export const agentUsageLimits = writable<any>(null);
export const agentSessionKey = writable<string>('');
export const agentCodexToken = writable<string>('');
/** Which provider's usage to show in the Agent footer. Mirrors the
 *  `agent_footer_usage_provider` setting; hydrated on app boot. */
export type AgentFooterProvider = 'claude' | 'codex' | 'gemini' | 'opencode';
export const agentFooterProvider = writable<AgentFooterProvider>('claude');
export type AgentUsageAuthState = 'unconfigured' | 'checking' | 'valid' | 'invalid';
export const agentUsageAuthStatus = writable<{
  state: AgentUsageAuthState;
  message: string;
}>({ state: 'unconfigured', message: '' });

// Claude subscription plan
export const agentClaudePlan = writable<string>('');

export async function loadAgentClaudePlan() {
  try {
    const plan = await agentGetClaudePlan();
    agentClaudePlan.set(plan);
  } catch { /* ignore */ }
}

export async function loadAgentUsageLimits() {
  const provider = get(agentFooterProvider);
  if (provider === 'codex') {
    return loadAgentUsageLimitsCodex();
  }
  return loadAgentUsageLimitsClaude();
}

async function loadAgentUsageLimitsClaude() {
  const key = get(agentSessionKey);
  if (!key) {
    agentUsageLimits.set(null);
    agentUsageAuthStatus.set({ state: 'unconfigured', message: '' });
    return;
  }
  agentUsageAuthStatus.set({ state: 'checking', message: '' });
  try {
    const limits = await agentFetchUsageLimits(key);
    agentUsageLimits.set(limits);
    agentUsageAuthStatus.set({ state: 'valid', message: 'Session key verified' });
    // Update tray title with usage stats
    // Claude API returns { five_hour: { utilization }, seven_day: { utilization } }
    // Also handle alternate shape: { standard: { percentUsed }, extended: { percentUsed } }
    try {
      const sessionPct = limits?.five_hour?.utilization ?? limits?.standard?.percentUsed;
      const weeklyPct = limits?.seven_day?.utilization ?? limits?.extended?.percentUsed;
      const parts: string[] = [];
      if (sessionPct != null) {
        parts.push(`S:${Math.round(sessionPct)}%`);
      }
      if (weeklyPct != null) {
        parts.push(`W:${Math.round(weeklyPct)}%`);
      }
      if (parts.length > 0) {
        await agentUpdateTrayTitle(parts.join(' '));
      }
    } catch { /* tray update best-effort */ }
  } catch (e: any) {
    agentUsageLimits.set(null);
    agentUsageAuthStatus.set({
      state: 'invalid',
      message: typeof e === 'string' ? e : e?.message || 'Claude session key is expired or invalid',
    });
  }
}

/** Map a Codex `limit_window_seconds` value to a one-letter tray-title
 *  prefix. Mirrors the StatusBar's full-word labels (Session/Daily/Weekly)
 *  but compressed for the menu-bar string length budget.
 *  S=Session(≤5h)  D=Daily(≤1d)  W=Weekly(≤7d)  M=Monthly(else) */
function codexTrayPrefix(seconds: number | null | undefined): string {
  if (seconds == null) return 'L';
  if (seconds <= 18000) return 'S';
  if (seconds <= 86400) return 'D';
  if (seconds <= 604800) return 'W';
  return 'M';
}

async function loadAgentUsageLimitsCodex() {
  const token = get(agentCodexToken);
  if (!token) {
    agentUsageLimits.set(null);
    agentUsageAuthStatus.set({ state: 'unconfigured', message: '' });
    return;
  }
  agentUsageAuthStatus.set({ state: 'checking', message: '' });
  try {
    const limits = await agentFetchCodexUsageLimits(token);
    agentUsageLimits.set(limits);
    agentUsageAuthStatus.set({ state: 'valid', message: 'Codex token verified' });
    try {
      // wham/usage shape: rate_limit.{primary_window, secondary_window}.{used_percent, limit_window_seconds}
      const primary = limits?.rate_limit?.primary_window;
      const secondary = limits?.rate_limit?.secondary_window;
      const parts: string[] = [];
      if (primary?.used_percent != null) {
        parts.push(`${codexTrayPrefix(primary.limit_window_seconds)}:${Math.round(primary.used_percent)}%`);
      }
      if (secondary?.used_percent != null) {
        parts.push(`${codexTrayPrefix(secondary.limit_window_seconds)}:${Math.round(secondary.used_percent)}%`);
      }
      if (parts.length > 0) await agentUpdateTrayTitle(parts.join(' '));
    } catch { /* tray update best-effort */ }
  } catch (e: any) {
    agentUsageLimits.set(null);
    agentUsageAuthStatus.set({
      state: 'invalid',
      message: typeof e === 'string' ? e : e?.message || 'Codex access token is expired or invalid',
    });
  }
}

export async function loadAgentSessions() {
  try {
    const sessions = await agentListSessions();
    agentSessions.set(sessions);
  } catch (e) {
    console.error('Failed to load agent sessions:', e);
  }
}

export async function loadAgentContexts() {
  try {
    const contexts = await agentListContexts();
    agentContexts.set(contexts);
  } catch (e) {
    console.error('Failed to load agent contexts:', e);
  }
}

export async function refreshAgentGitStatus() {
  const session = get(activeAgentSession);
  if (!session) return;
  const projectPath = session.worktreePath || session.projectPath;
  try {
    const [branch, files, [ahead, behind]] = await Promise.all([
      agentGitBranch(projectPath),
      agentGitStatus(projectPath),
      agentGitAheadBehind(projectPath),
    ]);
    agentGitBranchName.set(branch);
    agentGitFiles.set(files);
    agentGitAhead.set(ahead);
    agentGitBehind.set(behind);
  } catch { /* ignore — not a git repo */ }
}

export async function refreshAgentContextUsage(
  sessionId: string,
  projectPath: string,
  claudeSessionId: string,
  provider?: string,
) {
  try {
    const usage = await agentGetSessionContextUsage(projectPath, claudeSessionId, provider);
    agentContextUsage.update(m => { m.set(sessionId, usage); return new Map(m); });
  } catch { /* ignore */ }
}

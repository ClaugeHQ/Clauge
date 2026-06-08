// Canonical metadata for the four supported agent CLI providers. Used by
// the install-help modal, the provider picker chips, and any place that
// needs a user-facing name or per-OS install command. Single source of
// truth — modals and pickers stay consistent.

export type AgentProvider = 'claude' | 'codex' | 'gemini' | 'opencode';

export const AGENT_PROVIDERS: AgentProvider[] = ['claude', 'codex', 'gemini', 'opencode'];

export interface InstallCommand {
  label: string;
  cmd: string;
}

export interface ProviderInstallInfo {
  /** User-facing display name. */
  name: string;
  /** Docs URL the modal's "Setup docs" button opens. */
  docsUrl: string;
  /** Per-OS install command list. First is the recommended path. */
  mac: InstallCommand[];
  windows: InstallCommand[];
  linux: InstallCommand[];
}

export const PROVIDER_INSTALL_INFO: Record<AgentProvider, ProviderInstallInfo> = {
  claude: {
    name: 'Claude Code',
    docsUrl: 'https://code.claude.com/docs/en/setup',
    mac: [
      { label: 'Install script', cmd: 'curl -fsSL https://claude.ai/install.sh | bash' },
      { label: 'Homebrew', cmd: 'brew install --cask claude-code' },
    ],
    windows: [
      { label: 'PowerShell', cmd: 'irm https://claude.ai/install.ps1 | iex' },
      { label: 'WinGet', cmd: 'winget install Anthropic.ClaudeCode' },
    ],
    linux: [
      { label: 'Install script', cmd: 'curl -fsSL https://claude.ai/install.sh | bash' },
    ],
  },
  codex: {
    name: 'Codex CLI',
    docsUrl: 'https://developers.openai.com/codex/quickstart',
    mac: [
      { label: 'Homebrew', cmd: 'brew install --cask codex' },
      { label: 'npm', cmd: 'npm install -g @openai/codex' },
    ],
    windows: [
      { label: 'npm', cmd: 'npm install -g @openai/codex' },
    ],
    linux: [
      { label: 'npm', cmd: 'npm install -g @openai/codex' },
    ],
  },
  gemini: {
    // Google replaced gemini-cli with the `agy` Antigravity CLI on
    // 2026-06-18. Internal provider id stays "gemini" so existing
    // coworkers + sessions keep working.
    name: 'Antigravity CLI',
    docsUrl: 'https://antigravity.google/docs/cli-getting-started',
    mac: [
      { label: 'Install script', cmd: 'curl -fsSL https://antigravity.google/cli/install.sh | bash' },
      { label: 'Homebrew', cmd: 'brew install antigravity' },
    ],
    windows: [
      { label: 'PowerShell', cmd: 'irm https://antigravity.google/cli/install.ps1 | iex' },
    ],
    linux: [
      { label: 'Install script', cmd: 'curl -fsSL https://antigravity.google/cli/install.sh | bash' },
    ],
  },
  opencode: {
    name: 'OpenCode',
    docsUrl: 'https://opencode.ai/docs/cli/',
    mac: [
      { label: 'Install script', cmd: 'curl -fsSL https://opencode.ai/install | bash' },
      { label: 'Homebrew', cmd: 'brew install anomalyco/tap/opencode' },
      { label: 'npm', cmd: 'npm install -g opencode-ai' },
    ],
    windows: [
      { label: 'Scoop', cmd: 'scoop install opencode' },
      { label: 'Chocolatey', cmd: 'choco install opencode' },
      { label: 'npm', cmd: 'npm install -g opencode-ai' },
    ],
    linux: [
      { label: 'Install script', cmd: 'curl -fsSL https://opencode.ai/install | bash' },
      { label: 'Arch Linux', cmd: 'sudo pacman -S opencode' },
      { label: 'npm', cmd: 'npm install -g opencode-ai' },
    ],
  },
};

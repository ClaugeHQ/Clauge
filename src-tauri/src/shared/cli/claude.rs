// Claude CLI implementation of [`CliRunner`].
//
// The literals embedded here used to be scattered across
// `modes/agent/{terminal,plugins,usage}.rs`. Centralising them means a future
// Codex / Gemini / Aider implementation is one new file alongside this one.

use std::path::{Path, PathBuf};

use super::runner::{CliRunner, SpawnOpts};
use crate::shared::platform::shell::default_user_shell;

pub struct ClaudeRunner;

/// The Claude binary name on `$PATH`.
const BINARY: &str = "claude";

/// Sub-directory under `$HOME` that holds Claude's state.
const HOME_SUBDIR: &str = ".claude";

/// Sub-directory under `<home>` that holds installed plugins.
const PLUGINS_SUBDIR: &str = "plugins";

/// Sub-directory under `<home>` that holds per-project session logs.
const SESSIONS_SUBDIR: &str = "projects";

/// Session log file extension (without the dot).
const SESSION_EXT: &str = "jsonl";

impl ClaudeRunner {
    fn dot_claude(&self) -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(HOME_SUBDIR))
    }
}

impl CliRunner for ClaudeRunner {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn binary_name(&self) -> &'static str {
        BINARY
    }

    fn resolve_binary_path(&self) -> String {
        crate::shared::platform::path::find_binary(BINARY)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| BINARY.to_string())
    }

    fn build_spawn_command(&self, opts: &SpawnOpts) -> String {
        // Per-session override (Custom binary path in the modal's Advanced
        // section) takes precedence over the bare binary name; the shell
        // would otherwise resolve "claude" via $PATH only.
        let head = opts.binary_path_override.as_deref()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(crate::shared::cli::runner::shell_quote_path)
            .unwrap_or_else(|| BINARY.to_string());
        let mut cmd = head;
        if let Some(ref sid) = opts.resume_session_id {
            cmd.push_str(&format!(" --resume \"{}\"", sid));
        }
        if opts.skip_permissions {
            cmd.push_str(" --dangerously-skip-permissions");
        }
        // Hook-driven attention: register Clauge's notify hooks via an
        // additive settings source. The user's own settings.json (auth /
        // MCP) is unaffected; this file carries only a `hooks` block.
        if let Some(path) = opts.claude_settings_path.as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            cmd.push_str(&format!(
                " --settings {}",
                crate::shared::cli::runner::shell_quote_path(path)
            ));
        }
        if let Some(ref prompt) = opts.system_prompt {
            if !prompt.is_empty() {
                // Single quotes prevent ALL shell interpretation (< > $ ` etc.).
                // Escape any single quotes in the prompt: ' -> '\''
                let escaped = prompt.replace('\'', "'\\''");
                cmd.push_str(&format!(" --append-system-prompt '{}'", escaped));
            }
        }
        cmd
    }

    fn home_dir(&self) -> Option<PathBuf> {
        self.dot_claude()
    }

    fn plugins_dir(&self) -> Option<PathBuf> {
        self.dot_claude().map(|p| p.join(PLUGINS_SUBDIR))
    }

    fn settings_file(&self) -> Option<PathBuf> {
        self.dot_claude().map(|p| p.join("settings.json"))
    }

    fn installed_plugins_file(&self) -> Option<PathBuf> {
        self.plugins_dir().map(|p| p.join("installed_plugins.json"))
    }

    fn plugin_marketplaces_dir(&self) -> Option<PathBuf> {
        self.plugins_dir().map(|p| p.join("marketplaces"))
    }

    fn plugin_install_counts_file(&self) -> Option<PathBuf> {
        self.plugins_dir().map(|p| p.join("install-counts-cache.json"))
    }

    fn run_plugin_subcommand(&self, args: &[&str]) -> Result<(bool, String), String> {
        // Build "claude plugins <args...>" as a single shell string and run it
        // through the user's login + interactive shell so PATH entries added by
        // nvm / fnm / asdf / Homebrew are visible. Running the bare BINARY name
        // via Command::new bypasses those rc-file additions and silently fails
        // when claude is installed via a version manager.
        let mut parts: Vec<&str> = vec![BINARY, "plugins"];
        parts.extend_from_slice(args);
        let cmd = parts.join(" ");

        let (shell_path, shell_kind) = default_user_shell();
        let shell_args = shell_kind.exec_command_argv(&cmd);

        let output = std::process::Command::new(&shell_path)
            .args(&shell_args)
            .output()
            .map_err(|e| format!("Failed to run plugin subcommand: {}", e))?;

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // Surface stdout in the error when the command fails — some CLI tools
        // write their error message to stdout rather than stderr.
        if !output.status.success() {
            let msg = if stderr.is_empty() { stdout } else { stderr };
            return Ok((false, msg));
        }
        Ok((true, String::new()))
    }

    fn sessions_root(&self) -> Option<PathBuf> {
        self.dot_claude().map(|p| p.join(SESSIONS_SUBDIR))
    }

    fn session_dir_for_project(&self, project_path: &str) -> Option<PathBuf> {
        // Claude CLI flattens ANY character that isn't [A-Za-z0-9-] to a
        // single `-` (no run-collapsing — each special char becomes one
        // dash, so "/.foo" produces "--foo"). Earlier versions of this
        // encoder only handled "/" and ".", which silently failed for
        // paths containing spaces, underscores, or other punctuation;
        // the Custom-purpose picker would return empty for those paths.
        // Verified against a live `~/.claude/projects/` against 30 cwds.
        let encoded: String = project_path
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
            .collect();
        self.sessions_root().map(|r| r.join(encoded))
    }

    fn session_file_extension(&self) -> &'static str {
        SESSION_EXT
    }

    fn extract_resume_id_from_output(&self, buffer: &str) -> Option<String> {
        // Mirror of the frontend regex: /claude --resume ([a-f0-9-]+)/
        // Walk the buffer manually so we don't pull in the `regex` crate just
        // for a single hex-uuid extraction.
        let needle = "claude --resume ";
        let start = buffer.find(needle)? + needle.len();
        let rest = &buffer[start..];
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
            .collect();
        if id.is_empty() {
            None
        } else {
            Some(id)
        }
    }

    fn usage_api_orgs_url(&self) -> Option<String> {
        Some("https://claude.ai/api/organizations".to_string())
    }

    fn usage_api_url_for(&self, org_id: &str) -> Option<String> {
        Some(format!(
            "https://claude.ai/api/organizations/{}/usage",
            org_id
        ))
    }

    fn is_session_file(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some(SESSION_EXT)
    }
}

/// Process-wide stateless instance.
pub static CLAUDE: ClaudeRunner = ClaudeRunner;

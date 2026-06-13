// Codex CLI implementation of [`CliRunner`].
//
// Differences from Claude worth flagging:
//   - Resume is a subcommand (`codex resume <id>`), not a flag.
//   - Skip-permissions flag is `--dangerously-bypass-approvals-and-sandbox`.
//   - System-prompt injection uses `-c instructions="<text>"` (TOML config
//     override) — Codex has no `--append-system-prompt` analogue.
//   - Plugin subcommand is singular (`codex plugin ...`), one level deeper
//     than Claude's (`claude plugins ...`).
//   - Sessions are stored by DATE under `~/.codex/sessions/YYYY/MM/DD/`,
//     not by project. `session_dir_for_project` therefore returns None
//     (and any per-project session discovery is skipped for Codex). The
//     UUID lives in the filename: `rollout-<ts>-<UUID>.jsonl`, and also
//     inside the first JSONL line's `payload.id`.

use std::path::{Path, PathBuf};

use super::runner::{CliRunner, SpawnOpts};
use crate::shared::platform::shell::default_user_shell;

pub struct CodexRunner;

const BINARY: &str = "codex";
const HOME_SUBDIR: &str = ".codex";
const PLUGINS_SUBDIR: &str = "plugins";
const SESSIONS_SUBDIR: &str = "sessions";
const SESSION_EXT: &str = "jsonl";

impl CodexRunner {
    /// Resolve Codex's home directory. Honors `$CODEX_HOME` when the
    /// user has relocated it (power-users who keep state outside their
    /// real home; Codex itself supports this var). Same pattern works
    /// on macOS / Linux / Windows — `std::env::var` is OS-agnostic,
    /// and `dirs::home_dir` resolves to `%USERPROFILE%` on Windows.
    pub(crate) fn dot_codex(&self) -> Option<PathBuf> {
        if let Ok(raw) = std::env::var("CODEX_HOME") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }
        dirs::home_dir().map(|h| h.join(HOME_SUBDIR))
    }
}

impl CliRunner for CodexRunner {
    fn id(&self) -> &'static str {
        "codex"
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
        let head = opts.binary_path_override.as_deref()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(crate::shared::cli::runner::shell_quote_path)
            .unwrap_or_else(|| BINARY.to_string());
        let mut cmd = match opts.resume_session_id.as_deref() {
            // Codex session ids are UUIDs (`<hex>-<hex>-<hex>-<hex>-<hex>`
            // with 8-4-4-4-12 layout). Anything else (e.g. an OpenCode
            // `ses_…` id that leaked through from a stale session row)
            // would either be rejected or silently treated as a thread
            // name. Be defensive: spawn fresh on a malformed id.
            Some(sid) if looks_like_uuid(sid) => {
                format!("{head} resume \"{}\"", sid)
            }
            _ => head,
        };
        if opts.skip_permissions {
            cmd.push_str(" --dangerously-bypass-approvals-and-sandbox");
        }
        // Hook-driven attention: Codex runs the notify program on task
        // lifecycle events. `-c notify=["bash","<path>"]` — a TOML array
        // literal. Escape the path for the TOML double-quoted strings, then
        // wrap the whole value in single quotes for the shell (so its
        // brackets / quotes aren't interpreted), using the standard '\''
        // dance for any embedded single quote.
        if let Some(path) = opts.notify_script_path.as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            let toml_escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
            let value = format!("notify=[\"bash\",\"{}\"]", toml_escaped);
            let shell_escaped = value.replace('\'', "'\\''");
            cmd.push_str(&format!(" -c '{}'", shell_escaped));
        }
        if let Some(ref prompt) = opts.system_prompt {
            if !prompt.is_empty() {
                // `-c instructions=<TOML literal>`. We want the TOML value to be
                // a quoted string. Wrap the value in single quotes for the shell
                // (so $ and backticks aren't expanded), and put a TOML double-
                // quoted string inside. Embedded double quotes / backslashes in
                // the prompt are escaped for TOML; embedded single quotes are
                // escaped for shell using the standard '\'' dance.
                let toml_escaped = prompt.replace('\\', "\\\\").replace('"', "\\\"");
                let shell_escaped = toml_escaped.replace('\'', "'\\''");
                cmd.push_str(&format!(" -c instructions='\"{}\"'", shell_escaped));
            }
        }
        cmd
    }

    fn home_dir(&self) -> Option<PathBuf> {
        self.dot_codex()
    }

    fn plugins_dir(&self) -> Option<PathBuf> {
        self.dot_codex().map(|p| p.join(PLUGINS_SUBDIR))
    }

    fn settings_file(&self) -> Option<PathBuf> {
        // Codex's user config lives in TOML, not JSON.
        self.dot_codex().map(|p| p.join("config.toml"))
    }

    fn installed_plugins_file(&self) -> Option<PathBuf> {
        // Mirrors Claude's convention; verify once Codex's plugin tooling
        // settles on an exact filename.
        self.plugins_dir().map(|p| p.join("installed_plugins.json"))
    }

    fn plugin_marketplaces_dir(&self) -> Option<PathBuf> {
        self.plugins_dir().map(|p| p.join("marketplaces"))
    }

    fn plugin_install_counts_file(&self) -> Option<PathBuf> {
        self.plugins_dir().map(|p| p.join("install-counts-cache.json"))
    }

    fn run_plugin_subcommand(&self, args: &[&str]) -> Result<(bool, String), String> {
        // Note: Codex uses `codex plugin <...>` (singular). Run via the user's
        // login + interactive shell so version-manager PATHs are honored.
        let mut parts: Vec<&str> = vec![BINARY, "plugin"];
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
        if !output.status.success() {
            let msg = if stderr.is_empty() { stdout } else { stderr };
            return Ok((false, msg));
        }
        Ok((true, String::new()))
    }

    fn sessions_root(&self) -> Option<PathBuf> {
        self.dot_codex().map(|p| p.join(SESSIONS_SUBDIR))
    }

    fn session_dir_for_project(&self, _project_path: &str) -> Option<PathBuf> {
        // Codex stores sessions by date, not by project. Per-project
        // discovery would require querying the SQLite logs DB; deferred
        // for v1. Returning None tells discovery callers to no-op.
        None
    }

    fn session_file_extension(&self) -> &'static str {
        SESSION_EXT
    }

    fn extract_resume_id_from_output(&self, _buffer: &str) -> Option<String> {
        // Codex's TUI banner doesn't print a deterministic "resume with"
        // line we can scrape. Capture works by polling the sessions
        // directory for a newly-created `rollout-<ts>-<UUID>.jsonl` file
        // after spawn — implemented at the caller, not here.
        None
    }

    fn usage_api_orgs_url(&self) -> Option<String> {
        // Codex usage data lives behind the OpenAI rate-limit endpoint,
        // which is keyed by the user's access token rather than an
        // organization id. The token-based path is wired separately as
        // `agent_fetch_codex_usage_limits`; the trait's orgs/url pair
        // doesn't fit, so we return None and skip the generic flow.
        None
    }

    fn usage_api_url_for(&self, _org_id: &str) -> Option<String> {
        None
    }

    fn is_session_file(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some(SESSION_EXT)
    }
}

pub static CODEX: CodexRunner = CodexRunner;

/// Quick shape-check for Codex's UUID session ids. Eight-four-four-
/// four-twelve, hex + dash, length 36. Avoids dragging in the `uuid`
/// crate just to validate at the spawn boundary.
fn looks_like_uuid(s: &str) -> bool {
    if s.len() != 36 { return false; }
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        let expect_dash = matches!(i, 8 | 13 | 18 | 23);
        if expect_dash {
            if *b != b'-' { return false; }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

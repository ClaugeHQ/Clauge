// Some trait methods are part of the CliRunner *shape* (so a future
// Codex / Gemini / Aider implementation has somewhere obvious to plug in)
// but are not yet called by today's Agent code. Silence the dead-code lint
// on the trait declaration so adding the abstraction doesn't grow the
// project's warning count.
#![allow(dead_code)]

// CliRunner trait — abstracts the contract that Agent mode (and future modes
// targeting other coding-assistant CLIs) relies on.
//
// Each CLI binary has its own conventions for:
//   - binary name and how to discover it on PATH
//   - command-line flags for resuming a session, skipping permissions, and
//     injecting a system prompt
//   - on-disk layout of session logs (root directory, per-project encoding,
//     file extension, line format)
//   - on-disk layout of installed plugins / extensions
//   - usage analytics HTTP endpoints
//   - PTY-output markers that surface session ids for resume
//
// Today only the Claude implementation exists; the trait is shaped so adding
// a new CLI (Codex, Gemini, Aider, ...) is one new file implementing this
// trait, with no further changes to Agent-mode code.

use std::path::{Path, PathBuf};

/// Options consumed by [`CliRunner::build_spawn_command`].
#[derive(Debug, Clone, Default)]
pub struct SpawnOpts {
    /// Existing session id to resume, if any.
    pub resume_session_id: Option<String>,
    /// System prompt to append when starting / resuming.
    pub system_prompt: Option<String>,
    /// Whether to pass the CLI's "skip permission prompts" flag.
    pub skip_permissions: bool,
    /// Per-session absolute path to the CLI binary, overriding the
    /// default $PATH lookup. `None` / empty = use the bare binary name
    /// (default behaviour). When set, the implementation is expected
    /// to shell-quote it appropriately before splicing into the spawn
    /// command — see [`shell_quote_path`] for the canonical helper.
    pub binary_path_override: Option<String>,
    /// Hook-driven attention (Phase 1): absolute path to the Clauge-owned
    /// Claude `--settings` file. When set, the Claude runner appends
    /// `--settings <path>` so the agent registers our notify hooks. An
    /// additive settings source — the user's own settings.json (auth/MCP)
    /// is untouched. `None` = hooks disabled / unavailable.
    pub claude_settings_path: Option<String>,
    /// Hook-driven attention (Phase 1): absolute path to the Clauge-owned
    /// `notify.sh`. When set, the Codex runner appends `-c
    /// notify=["bash","<path>"]`. `None` = hooks disabled / unavailable.
    pub notify_script_path: Option<String>,
}

/// Shell-quote a binary path for safe inclusion in a spawn command
/// string. POSIX shells get single-quotes (with `'\''` escapes); cmd /
/// PowerShell on Windows get double-quotes with `"` escaping. Paths
/// without special characters pass through unquoted for readability.
pub fn shell_quote_path(path: &str) -> String {
    // Pass through cleanly when the path is "boring" — no whitespace,
    // no quote chars, no shell metacharacters. Keeps logs readable on
    // the 95% case where the user picked /usr/local/bin/<cli>.
    let needs_quoting = path.chars().any(|c| {
        c.is_whitespace() || matches!(c, '\'' | '"' | '\\' | '$' | '`' | '&' | '|' | ';' | '(' | ')' | '<' | '>' | '*' | '?' | '[' | ']' | '{' | '}')
    });
    if !needs_quoting {
        return path.to_string();
    }
    if cfg!(windows) {
        format!("\"{}\"", path.replace('"', "\\\""))
    } else {
        // POSIX: '...' is literal; only ' itself needs escaping via '\''
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

pub trait CliRunner: Send + Sync {
    // ---- Identity ------------------------------------------------------

    /// Stable identifier for this CLI ("claude", "codex", "gemini", ...).
    fn id(&self) -> &'static str;

    /// The binary name as found on `$PATH`.
    fn binary_name(&self) -> &'static str;

    /// Resolve an absolute path to the CLI binary.
    ///
    /// Implementations may shell out (e.g. `which <bin>` under the user's
    /// login + interactive shell so PATH manipulations from `~/.zshrc` are
    /// honored) or fall back to the bare binary name.
    fn resolve_binary_path(&self) -> String;

    // ---- Spawn ---------------------------------------------------------

    /// Build the shell command string used to start (or resume) a session.
    /// The returned string is meant to be passed to `<user-shell> -l -i -c`.
    fn build_spawn_command(&self, opts: &SpawnOpts) -> String;

    // ---- Home / plugins ------------------------------------------------

    /// CLI home directory, e.g. `~/.claude`.
    fn home_dir(&self) -> Option<PathBuf>;

    /// Plugins directory, e.g. `~/.claude/plugins`.
    fn plugins_dir(&self) -> Option<PathBuf>;

    /// User settings file (the one storing `enabledPlugins`, etc.).
    fn settings_file(&self) -> Option<PathBuf>;

    /// `installed_plugins.json` location.
    fn installed_plugins_file(&self) -> Option<PathBuf>;

    /// Directory containing one sub-folder per registered marketplace.
    fn plugin_marketplaces_dir(&self) -> Option<PathBuf>;

    /// Cached install-counts file (used to rank marketplace listings).
    fn plugin_install_counts_file(&self) -> Option<PathBuf>;

    /// Run the CLI's plugin sub-command (`<bin> plugins <args>`). Returns
    /// (status_success, stderr_string) on successful invocation; an `Err`
    /// only on process-spawn failure.
    fn run_plugin_subcommand(&self, args: &[&str]) -> Result<(bool, String), String>;

    // ---- Sessions ------------------------------------------------------

    /// Root of per-project session logs, e.g. `~/.claude/projects`.
    fn sessions_root(&self) -> Option<PathBuf>;

    /// Per-project session directory. Encodes `project_path` the same way
    /// the CLI does (Claude flattens slashes and dots to `-`).
    fn session_dir_for_project(&self, project_path: &str) -> Option<PathBuf>;

    /// File extension used for session logs (without the leading dot).
    fn session_file_extension(&self) -> &'static str;

    // ---- Output parsing ------------------------------------------------

    /// Extract a session id from a chunk of PTY output, if the CLI emitted
    /// a "resume this session with: ..." marker. The frontend currently
    /// owns this regex; this hook lets us move it Rust-side later.
    fn extract_resume_id_from_output(&self, buffer: &str) -> Option<String>;

    // ---- Usage analytics ----------------------------------------------

    /// HTTP endpoint listing the user's organizations, if the CLI vendor
    /// exposes one.
    fn usage_api_orgs_url(&self) -> Option<String>;

    /// Per-organization usage endpoint, if any.
    fn usage_api_url_for(&self, org_id: &str) -> Option<String>;

    // ---- Convenience ---------------------------------------------------

    /// True if `path` looks like one of this CLI's session log files
    /// (used when iterating directories that may contain mixed contents).
    fn is_session_file(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e == self.session_file_extension())
            .unwrap_or(false)
    }
}

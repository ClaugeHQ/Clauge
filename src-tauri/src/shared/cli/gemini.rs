// Gemini CLI implementation of [`CliRunner`].
//
// Differences from Claude / Codex worth flagging:
//   - Trust gate: Gemini refuses non-interactive runs in an "untrusted"
//     workspace. `--skip-trust` opts that out for the headless / oneshot
//     code path. Interactive PTY spawns lean on the same flag rather
//     than letting the user re-answer the trust prompt every reopen.
//   - Resume is index-based (`--resume <N>` or `--resume latest`), not
//     UUID-based. Sessions DO have UUIDs internally, but the CLI's
//     resume flag can't take them. We store the UUID for display +
//     dedupe in the new-session discovery list, and resume with
//     `--resume latest` whenever we know a matching session exists.
//   - System-prompt injection has no `--append-system-prompt` analogue.
//     The closest interactive equivalent is `--prompt-interactive
//     <text>` — runs the prompt and stays in the TUI. Oneshot
//     (`-p <text>`) doesn't accept a separate system prompt, so the
//     persona is prepended into the message itself (same approach as
//     OpenCode).
//   - Sessions live at `~/.gemini/tmp/<slug>/chats/session-*.jsonl`.
//     The project→slug mapping is in `~/.gemini/projects.json`
//     ({ projects: { "<abs_path>": "<slug>" } }); per-project discovery
//     reads that file rather than re-deriving the slug.
//   - Plugins/extensions are managed via `gemini extensions <cmd>` — a
//     subcommand surface, not a marketplace directory tree. The plugin
//     manager UI is hidden for this provider (run_plugin_subcommand
//     returns Err with a hint so it's discoverable in logs).
//   - No first-party rate-limit / usage analytics endpoint we can scrape
//     unauthenticated. Footer-usage chip is omitted for Gemini.

use std::path::{Path, PathBuf};

use super::runner::{CliRunner, SpawnOpts};
use crate::shared::platform::shell::default_user_shell;

pub struct GeminiRunner;

// `agy` is the Antigravity CLI binary that replaces gemini-cli for free,
// Pro, and Ultra tiers as of 2026-06-18. The internal provider id stays
// "gemini" so existing user data (coworkers + sessions with
// provider="gemini") keeps working.
//
// On-disk layout shift vs the old gemini-cli (verified on a real install):
//   gemini-cli:   ~/.gemini/projects.json   {"projects": {path: slug}}
//                 ~/.gemini/tmp/<slug>/chats/session-*.jsonl
//   antigravity:  ~/.gemini/antigravity-cli/cache/projects.json
//                                                    {path: uuid}   (no wrapper)
//                 ~/.gemini/antigravity-cli/conversations/<uuid>.db (SQLite)
//
// GEMINI.md (context file used by `agent_inject_purpose`) still lives at
// ~/.gemini/GEMINI.md — Antigravity reads it from the user-home location,
// not from antigravity-cli/.
const BINARY: &str = "agy";
const HOME_SUBDIR: &str = ".gemini";
const CLI_SUBDIR: &str = "antigravity-cli";
const SESSIONS_SUBDIR: &str = "conversations";
const SESSION_EXT: &str = "db";

impl GeminiRunner {
    fn dot_gemini(&self) -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(HOME_SUBDIR))
    }

    fn agy_home(&self) -> Option<PathBuf> {
        self.dot_gemini().map(|p| p.join(CLI_SUBDIR))
    }

    /// Read the per-project UUID Antigravity assigns to `project_path`
    /// from `~/.gemini/antigravity-cli/cache/projects.json`. Returns
    /// None when the file is missing or the path hasn't been registered
    /// (no sessions ever started). The new format is a flat map of
    /// `{ "<abs path>": "<uuid>" }` — the old `{"projects": {...}}`
    /// wrapper is gone.
    pub(crate) fn slug_for_project(&self, project_path: &str) -> Option<String> {
        let path = self.agy_home()?.join("cache").join("projects.json");
        let text = std::fs::read_to_string(&path).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
        parsed.get(project_path)?.as_str().map(|s| s.to_string())
    }
}

impl CliRunner for GeminiRunner {
    fn id(&self) -> &'static str {
        "gemini"
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
        // Antigravity (agy) replaced the old gemini-cli flags:
        //   --skip-trust  → removed (no trust gate concept)
        //   --yolo        → --dangerously-skip-permissions
        //   --resume <N>  → --continue (most recent) | --conversation <id>
        let mut cmd = head.clone();
        // Resume: agy accepts `--conversation <uuid>` to target a
        // specific conversation, or `--continue` to grab the most
        // recent. Conversation UUIDs are the .db filenames under
        // ~/.gemini/antigravity-cli/conversations/, so when discovery
        // hands us a well-shaped UUID we use it directly. Anything
        // else (truthy but not a UUID) falls back to `--continue`.
        if let Some(sid) = opts.resume_session_id.as_deref() {
            if looks_like_uuid(sid) {
                cmd.push_str(&format!(" --conversation {sid}"));
            } else {
                cmd.push_str(" --continue");
            }
        }
        if opts.skip_permissions {
            cmd.push_str(" --dangerously-skip-permissions");
        }
        // No first-class system-prompt flag exists, and the previous
        // workaround (`--prompt-interactive '<text>'`) had a serious
        // side effect: Gemini treats `--prompt-interactive` as the
        // user's first message, so the persona prompt was running
        // immediately on every spawn / resume instead of waiting for
        // the user's first turn. Now the purpose prompt is injected
        // into `GEMINI.md` pre-spawn (see
        // `agent_inject_purpose` in modes/agent/commands.rs). Gemini
        // reads that file at startup as ambient context and the TUI
        // opens idle, matching Claude/Codex behaviour. We consume the
        // field so the spawn opts are uniform across runners.
        let _ = opts.system_prompt;
        cmd
    }

    fn home_dir(&self) -> Option<PathBuf> {
        self.dot_gemini()
    }

    fn plugins_dir(&self) -> Option<PathBuf> {
        // Gemini calls them extensions; user-installed ones live under
        // `~/.gemini/extensions/`. We surface that path for the plugin
        // listing UI even though installation is via `gemini extensions
        // install <name>` rather than a marketplace directory write.
        self.dot_gemini().map(|p| p.join("extensions"))
    }

    fn settings_file(&self) -> Option<PathBuf> {
        // Antigravity CLI keeps its own settings.json under antigravity-cli/.
        self.agy_home().map(|p| p.join("settings.json"))
    }

    fn installed_plugins_file(&self) -> Option<PathBuf> {
        // Gemini doesn't write an `installed_plugins.json` index — the
        // list is derived from the extensions directory contents. Hide
        // the plugin tab for this provider.
        None
    }

    fn plugin_marketplaces_dir(&self) -> Option<PathBuf> {
        None
    }

    fn plugin_install_counts_file(&self) -> Option<PathBuf> {
        None
    }

    fn run_plugin_subcommand(&self, args: &[&str]) -> Result<(bool, String), String> {
        // Gemini's extensions surface: `gemini extensions <args>`.
        let mut parts: Vec<&str> = vec![BINARY, "extensions"];
        parts.extend_from_slice(args);
        let cmd = parts.join(" ");

        let (shell_path, shell_kind) = default_user_shell();
        let shell_args = shell_kind.exec_command_argv(&cmd);

        let output = std::process::Command::new(&shell_path)
            .args(&shell_args)
            .output()
            .map_err(|e| format!("Failed to run extensions subcommand: {}", e))?;

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !output.status.success() {
            let msg = if stderr.is_empty() { stdout } else { stderr };
            return Ok((false, msg));
        }
        Ok((true, String::new()))
    }

    fn sessions_root(&self) -> Option<PathBuf> {
        // ~/.gemini/antigravity-cli/conversations — flat `.db` files
        // keyed by their own UUIDs (not per-project subdirs anymore).
        self.agy_home().map(|p| p.join(SESSIONS_SUBDIR))
    }

    fn session_dir_for_project(&self, _project_path: &str) -> Option<PathBuf> {
        // Antigravity flattened conversation storage: all `.db` files
        // sit directly under `conversations/`. The project UUID held in
        // `cache/projects.json` doesn't match a conversation filename,
        // and the project<->conversation mapping lives inside the
        // SQLite databases themselves. Returning the flat dir means
        // resume-discovery lists every conversation rather than only
        // ones for `_project_path`. Acceptable until we add a SQLite
        // read step to filter on disk.
        self.sessions_root()
    }

    fn session_file_extension(&self) -> &'static str {
        SESSION_EXT
    }

    fn extract_resume_id_from_output(&self, buffer: &str) -> Option<String> {
        // On exit, `agy` prints a banner like
        //   `agy --conversation=<uuid>` or `agy -c`
        // telling the user how to resume. Capture the UUID form so we
        // can persist it into the session row and pass it back via
        // --conversation on the next spawn. The "-c" hint isn't useful
        // (no specific id) so we ignore it; discovery handles fallback.
        for marker in ["--conversation=", "--conversation "] {
            if let Some(idx) = buffer.find(marker) {
                let rest = &buffer[idx + marker.len()..];
                let candidate: String = rest
                    .chars()
                    .take(36)
                    .collect();
                if candidate.len() == 36 && looks_like_uuid(&candidate) {
                    return Some(candidate);
                }
            }
        }
        None
    }

    fn usage_api_orgs_url(&self) -> Option<String> {
        None
    }

    fn usage_api_url_for(&self, _org_id: &str) -> Option<String> {
        None
    }

    fn is_session_file(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some(SESSION_EXT)
    }
}

pub static GEMINI: GeminiRunner = GeminiRunner;

/// Quick shape-check for Gemini's UUID session ids (8-4-4-4-12 hex with
/// dashes, total length 36). Lifted from `codex.rs` rather than shared
/// to keep the runner files independent — see also the architecture
/// gotcha about cross-runner imports rotting on rename.
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

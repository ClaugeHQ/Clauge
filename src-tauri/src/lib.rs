use base64::Engine;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::Mutex;
use tauri::ipc::Channel;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIconBuilder, TrayIconId};
use tauri::{Manager, State};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionProfile {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub project_path: String,
    pub project_name: String,
    pub claude_session_id: Option<String>,
    pub context_prompt: String,
    pub created_at: String,
    pub last_used_at: String,
    #[serde(default)]
    pub worktree_path: Option<String>,
    #[serde(default)]
    pub worktree_branch: Option<String>,
    #[serde(default)]
    pub skip_permissions: bool,
    #[serde(default)]
    pub claude_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStore {
    pub profiles: Vec<SessionProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredSession {
    pub session_id: String,
    pub modified_at: String,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_tokens: u64,
}



#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputPayload {
    pub terminal_id: String,
    pub data: String, // base64 encoded
}

// ---------------------------------------------------------------------------
// Terminal state
// ---------------------------------------------------------------------------

struct TerminalEntry {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    #[allow(dead_code)]
    child: Box<dyn portable_pty::Child + Send>,
}

pub struct TerminalState {
    terminals: Arc<Mutex<HashMap<String, TerminalEntry>>>,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self {
            terminals: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// ---------------------------------------------------------------------------
// Context prompt helper
// ---------------------------------------------------------------------------

fn get_context_prompt(purpose: &str) -> String {
    match purpose {
        "Brainstorming" => r#"# Session: Brainstorming

You are in a brainstorming session. Your role:

- Explore multiple approaches before settling on one
- Ask clarifying questions to understand the full picture
- Think out loud — share tradeoffs, risks, and alternatives
- Do NOT write implementation code unless explicitly asked
- Focus on architecture, design decisions, and high-level strategy
- Challenge assumptions — push back if something seems off
- Summarize options with pros/cons when presenting choices"#.to_string(),

        "Development" => r#"# Session: Development

You are in a development session. Your role:

- Write clean, working code — prioritize correctness over cleverness
- Follow existing patterns and conventions in the codebase
- Make small, focused changes — one thing at a time
- Run tests and verify changes work before moving on
- Keep commits logical and atomic
- If requirements are unclear, ask before guessing
- Prefer editing existing files over creating new ones"#.to_string(),

        "Code Review" => r#"# Session: Code Review

You are in a code review session. Your role:

- Review recent changes with a critical eye
- Check for: bugs, security issues, performance problems, edge cases
- Reference specific files and line numbers
- Suggest concrete improvements, not vague advice
- Flag anything that could break in production
- Check error handling — are failures handled gracefully?
- Look for missing tests or untested paths
- Be direct — don't sugarcoat issues"#.to_string(),

        "Debugging" => r#"# Session: Debugging

You are in a debugging session. Your role:

- Reproduce the issue first — confirm the symptoms
- Form a hypothesis, then verify it with evidence (logs, output, traces)
- Do NOT guess fixes — trace the root cause methodically
- Check recent changes that might have introduced the bug
- Use binary search (git bisect, selective commenting) to isolate
- Once found, explain the root cause before proposing a fix
- After fixing, verify the original issue is resolved
- Check for related bugs that might have the same root cause"#.to_string(),

        "PR Review" => r#"# Session: PR Review

You are in a PR review session. Your role:

- Start by asking which branch or PR to review
- Run `git diff main...<branch>` to see all changes
- Review every changed file systematically
- Check for: bugs, security issues, logic errors, edge cases
- Verify error handling and test coverage for new code
- Comment on code style only if it hurts readability
- Flag breaking changes or missing migrations
- Summarize: what the PR does, what's good, what needs fixing
- Give a clear verdict: approve, request changes, or needs discussion"#.to_string(),

        _ => String::new(),
    }
}


// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn get_storage_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let new_dir = home.join(".clauge");
        let old_dir = home.join(".ctx-mgr");
        // Migrate from old name
        if old_dir.exists() && !new_dir.exists() {
            let _ = std::fs::rename(&old_dir, &new_dir);
        }
        let _ = std::fs::create_dir_all(&new_dir);
        new_dir
    } else {
        PathBuf::from(".")
    }
}

fn get_storage_path() -> PathBuf {
    get_storage_dir().join("sessions.json")
}

fn settings_path() -> PathBuf {
    get_storage_dir().join("settings.json")
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSettings {
    #[serde(default)]
    pub claude_path: Option<String>,
    #[serde(default)]
    pub default_skip_permissions: Option<bool>,
}

fn load_settings() -> GlobalSettings {
    let path = settings_path();
    if !path.exists() {
        return GlobalSettings::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => GlobalSettings::default(),
    }
}

fn save_settings_to_disk(s: &GlobalSettings) -> Result<(), String> {
    let path = settings_path();
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

fn load_profiles() -> Vec<SessionProfile> {
    let path = get_storage_path();
    if !path.exists() {
        return Vec::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let store: SessionStore = serde_json::from_str(&contents).unwrap_or(SessionStore {
                profiles: Vec::new(),
            });
            store.profiles
        }
        Err(_) => Vec::new(),
    }
}

fn save_profiles(profiles: &[SessionProfile]) -> Result<(), String> {
    let path = get_storage_path();
    let store = SessionStore {
        profiles: profiles.to_vec(),
    };
    let json = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Claude session discovery helpers
// ---------------------------------------------------------------------------

fn encode_project_path(project_path: &str) -> String {
    // Claude encodes project paths by replacing '/' and '.' with '-'
    // e.g. /Users/me/.clauge-worktrees -> -Users-me--clauge-worktrees
    project_path.replace('/', "-").replace('.', "-")
}


fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn project_name_from_path(project_path: &str) -> String {
    std::path::Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_profiles() -> Result<Vec<SessionProfile>, String> {
    Ok(load_profiles())
}

#[tauri::command]
fn get_settings() -> Result<GlobalSettings, String> {
    Ok(load_settings())
}

#[tauri::command]
fn save_settings(settings: GlobalSettings) -> Result<(), String> {
    let normalized = GlobalSettings {
        claude_path: settings.claude_path.and_then(|p| {
            let t = p.trim();
            if t.is_empty() { None } else { Some(t.to_string()) }
        }),
        default_skip_permissions: settings.default_skip_permissions,
    };
    save_settings_to_disk(&normalized)
}

#[tauri::command]
fn create_profile(title: String, purpose: String, project_path: String, skip_permissions: Option<bool>, custom_prompt: Option<String>, claude_path: Option<String>) -> Result<SessionProfile, String> {
    let mut profiles = load_profiles();
    let now = now_iso8601();

    // Don't auto-discover session ID on creation — new profiles start fresh.
    // The frontend will discover and link the session ID after claude starts.
    let profile = SessionProfile {
        id: Uuid::new_v4().to_string(),
        title,
        purpose: purpose.clone(),
        project_path: project_path.clone(),
        project_name: project_name_from_path(&project_path),
        claude_session_id: None,
        context_prompt: custom_prompt.unwrap_or_else(|| get_context_prompt(&purpose)),
        created_at: now.clone(),
        last_used_at: now,
        worktree_path: None,
        worktree_branch: None,
        skip_permissions: skip_permissions.unwrap_or(false),
        claude_path: claude_path.and_then(|p| {
            let t = p.trim();
            if t.is_empty() { None } else { Some(t.to_string()) }
        }),
    };

    profiles.push(profile.clone());
    save_profiles(&profiles)?;
    Ok(profile)
}

#[tauri::command]
fn delete_profile(id: String) -> Result<(), String> {
    let mut profiles = load_profiles();
    profiles.retain(|p| p.id != id);
    save_profiles(&profiles)?;
    Ok(())
}

#[tauri::command]
fn rename_profile(id: String, new_title: String) -> Result<(), String> {
    let mut profiles = load_profiles();
    if let Some(profile) = profiles.iter_mut().find(|p| p.id == id) {
        profile.title = new_title;
    } else {
        return Err("Profile not found".to_string());
    }
    save_profiles(&profiles)?;
    Ok(())
}

#[tauri::command]
fn update_last_used(id: String) -> Result<(), String> {
    let mut profiles = load_profiles();
    if let Some(profile) = profiles.iter_mut().find(|p| p.id == id) {
        profile.last_used_at = now_iso8601();
    } else {
        return Err("Profile not found".to_string());
    }
    save_profiles(&profiles)?;
    Ok(())
}

/// Get all profiles — no auto-discovery of session IDs
/// Session IDs are only set when explicitly captured after a session starts
#[tauri::command]
fn refresh_session_ids() -> Result<Vec<SessionProfile>, String> {
    Ok(load_profiles())
}

/// Update the claude session ID for a specific profile
#[tauri::command]
fn update_session_id(id: String, claude_session_id: String) -> Result<(), String> {
    let mut profiles = load_profiles();
    if let Some(profile) = profiles.iter_mut().find(|p| p.id == id) {
        profile.claude_session_id = Some(claude_session_id);
    } else {
        return Err("Profile not found".to_string());
    }
    save_profiles(&profiles)?;
    Ok(())
}

/// Check if a path is inside a git repo
#[tauri::command]
fn is_git_repo(path: String) -> Result<bool, String> {
    let output = std::process::Command::new("git")
        .args(["-C", &path, "rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(output.status.success())
}

/// Sanitize a branch name to comply with git naming rules.
/// Removes dangerous characters, prevents flag injection (leading `-`),
/// and enforces git's naming constraints.
fn sanitize_branch_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '/' || *c == '-' || *c == '_' || *c == '.')
        .collect();

    // Collapse consecutive dots/dashes, remove trailing dots/locks
    let sanitized = sanitized
        .replace("..", ".")
        .replace(".lock", "")
        .trim_matches(|c: char| c == '.' || c == '/' || c == '-')
        .to_string();

    if sanitized.is_empty() {
        return "clauge/unnamed".to_string();
    }

    // Prevent flag injection: if any path segment starts with `-`, prefix it
    sanitized
        .split('/')
        .map(|seg| {
            if seg.starts_with('-') {
                format!("x{}", seg)
            } else {
                seg.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Create a git worktree for session isolation
#[tauri::command]
fn create_worktree(project_path: String, branch_name: String) -> Result<String, String> {
    let branch_name = sanitize_branch_name(&branch_name);
    let worktree_dir = PathBuf::from(&project_path)
        .join(".clauge-worktrees")
        .join(&branch_name);
    let worktree_path = worktree_dir.to_string_lossy().to_string();

    // If worktree directory already exists and is valid, just reuse it
    if worktree_dir.exists() {
        return Ok(worktree_path);
    }

    let _ = std::fs::create_dir_all(worktree_dir.parent().unwrap_or(&worktree_dir));

    // Prune stale worktrees first
    let _ = std::process::Command::new("git")
        .args(["-C", &project_path, "worktree", "prune"])
        .output();

    // Try creating with new branch
    let output = std::process::Command::new("git")
        .args(["-C", &project_path, "worktree", "add", "-b", &branch_name, &worktree_path])
        .output()
        .map_err(|e| format!("git worktree add failed: {}", e))?;

    if !output.status.success() {
        // Branch exists — reuse it
        let output2 = std::process::Command::new("git")
            .args(["-C", &project_path, "worktree", "add", &worktree_path, &branch_name])
            .output()
            .map_err(|e| format!("git worktree add failed: {}", e))?;

        if !output2.status.success() {
            return Err(format!("git worktree add failed: {}", String::from_utf8_lossy(&output2.stderr)));
        }
    }

    // Add .clauge-worktrees to .gitignore if not already there
    let gitignore = PathBuf::from(&project_path).join(".gitignore");
    if let Ok(contents) = std::fs::read_to_string(&gitignore) {
        if !contents.contains(".clauge-worktrees") {
            let _ = std::fs::write(&gitignore, format!("{}\n.clauge-worktrees/\n", contents.trim_end()));
        }
    } else {
        let _ = std::fs::write(&gitignore, ".clauge-worktrees/\n");
    }

    Ok(worktree_path)
}

/// Remove a git worktree (keeps the branch — user may have commits there)
#[tauri::command]
fn remove_worktree(project_path: String, worktree_path: String, _branch_name: Option<String>) -> Result<(), String> {
    let _ = std::process::Command::new("git")
        .args(["-C", &project_path, "worktree", "remove", "--force", &worktree_path])
        .output();

    let _ = std::process::Command::new("git")
        .args(["-C", &project_path, "worktree", "prune"])
        .output();

    Ok(())
}

/// Update worktree info for a profile
#[tauri::command]
fn update_profile_worktree(id: String, worktree_path: Option<String>, worktree_branch: Option<String>) -> Result<(), String> {
    let mut profiles = load_profiles();
    if let Some(profile) = profiles.iter_mut().find(|p| p.id == id) {
        profile.worktree_path = worktree_path;
        profile.worktree_branch = worktree_branch;
    } else {
        return Err("Profile not found".to_string());
    }
    save_profiles(&profiles)?;
    Ok(())
}

/// Count active sessions for a project path (profiles that have been used)
#[tauri::command]
fn count_project_sessions(project_path: String) -> Result<u32, String> {
    let profiles = load_profiles();
    let count = profiles.iter()
        .filter(|p| p.project_path == project_path)
        .count() as u32;
    Ok(count)
}

#[tauri::command]
fn discover_sessions(project_path: String) -> Result<Vec<DiscoveredSession>, String> {
    let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
    let encoded = encode_project_path(&project_path);
    let projects_dir = home.join(".claude").join("projects").join(&encoded);

    if !projects_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    let entries = std::fs::read_dir(&projects_dir).map_err(|e| e.to_string())?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let session_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let modified_at = path
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    let datetime: chrono::DateTime<chrono::Utc> = t.into();
                    datetime.to_rfc3339()
                })
                .unwrap_or_default();

            // Extract first user message as preview
            let preview = std::fs::read_to_string(&path).ok().and_then(|content| {
                for line in content.lines().take(20) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                        if val.get("type").and_then(|t| t.as_str()) == Some("human") {
                            if let Some(msg) = val.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_str()) {
                                let trimmed = msg.chars().take(80).collect::<String>();
                                return Some(trimmed);
                            }
                        }
                    }
                }
                None
            });

            sessions.push(DiscoveredSession {
                session_id,
                modified_at,
                preview,
            });
        }
    }

    sessions.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(sessions)
}

#[tauri::command]
fn get_session_tokens(
    project_path: String,
    session_id: Option<String>,
) -> Result<TokenUsage, String> {
    let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
    let encoded = encode_project_path(&project_path);
    let projects_dir = home.join(".claude").join("projects").join(&encoded);

    if !projects_dir.exists() {
        return Err("Project directory not found".to_string());
    }

    let file_path = if let Some(sid) = session_id {
        projects_dir.join(format!("{}.jsonl", sid))
    } else {
        // Find most recent .jsonl file
        let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
        if let Ok(entries) = std::fs::read_dir(&projects_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    if let Ok(meta) = path.metadata() {
                        if let Ok(modified) = meta.modified() {
                            if best.as_ref().map_or(true, |(_, t)| modified > *t) {
                                best = Some((path, modified));
                            }
                        }
                    }
                }
            }
        }
        best.map(|(p, _)| p)
            .ok_or("No session files found")?
    };

    if !file_path.exists() {
        return Err("Session file not found".to_string());
    }

    let contents = std::fs::read_to_string(&file_path).map_err(|e| e.to_string())?;

    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut cache_read_tokens: u64 = 0;
    let mut cache_creation_tokens: u64 = 0;

    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            // Check both direct usage and message.usage patterns
            let usage = val.get("usage").or_else(|| {
                val.get("message").and_then(|m| m.get("usage"))
            });
            if let Some(u) = usage {
                input_tokens += u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                output_tokens += u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                cache_read_tokens += u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                cache_creation_tokens += u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            }
        }
    }

    let total_tokens = input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens;

    Ok(TokenUsage {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        total_tokens,
    })
}


/// Fetch usage limits via reqwest with native-tls (uses macOS SecureTransport to bypass Cloudflare)
#[tauri::command]
fn fetch_usage_limits(session_key: String) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .use_native_tls()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
    let cookie = format!("sessionKey={}", session_key);

    // Step 1: Get org ID
    let orgs_resp = client
        .get("https://claude.ai/api/organizations")
        .header("Cookie", &cookie)
        .header("User-Agent", ua)
        .send()
        .map_err(|e| format!("orgs request failed: {}", e))?;

    let orgs: Vec<serde_json::Value> = orgs_resp
        .json()
        .map_err(|e| format!("orgs parse failed: {}", e))?;

    let org_id = orgs
        .first()
        .and_then(|o| o.get("uuid"))
        .and_then(|v| v.as_str())
        .ok_or("No organization found")?
        .to_string();

    // Step 2: Get usage
    let usage_resp = client
        .get(&format!("https://claude.ai/api/organizations/{}/usage", org_id))
        .header("Cookie", &cookie)
        .header("User-Agent", ua)
        .send()
        .map_err(|e| format!("usage request failed: {}", e))?;

    let usage: serde_json::Value = usage_resp
        .json()
        .map_err(|e| format!("usage parse failed: {}", e))?;

    Ok(usage)
}

/// Save session key to local storage
#[tauri::command]
fn save_session_key(key: String) -> Result<(), String> {
    let path = get_storage_dir().join("session_key");
    std::fs::write(&path, &key).map_err(|e| e.to_string())
}

/// Load session key from local storage
#[tauri::command]
fn load_session_key() -> Result<Option<String>, String> {
    let path = get_storage_dir().join("session_key");
    if path.exists() {
        let key = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let key = key.trim().to_string();
        if key.is_empty() { Ok(None) } else { Ok(Some(key)) }
    } else {
        Ok(None)
    }
}

/// Get app version from Cargo.toml
#[tauri::command]
fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get Claude subscription plan from keychain
#[tauri::command]
fn get_claude_plan() -> Result<String, String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .map_err(|e| format!("Keychain error: {}", e))?;
    if !output.status.success() { return Ok(String::new()); }
    let json_str = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let parsed: serde_json::Value = serde_json::from_str(json_str.trim()).map_err(|e| e.to_string())?;
    Ok(parsed.get("claudeAiOauth")
        .and_then(|o| o.get("subscriptionType").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string())
}

/// Update the tray title text (shown in menu bar)
#[tauri::command]
fn update_tray_title(app_handle: tauri::AppHandle, title: String) -> Result<(), String> {
    if let Some(tray) = app_handle.tray_by_id(&TrayIconId::new("main-tray")) {
        tray.set_title(Some(&title)).map_err(|e| format!("Tray title error: {}", e))?;
    }
    Ok(())
}


/// Strip ANSI/OSC escape sequences that shell-integration plugins
/// (iTerm2, VS Code terminal, etc.) inject into interactive-shell output.
fn strip_terminal_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b']' => {
                    // OSC: ESC ] ... (BEL | ESC \)
                    i += 2;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 { i += 1; break; }
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' { i += 2; break; }
                        i += 1;
                    }
                    continue;
                }
                b'[' => {
                    // CSI: ESC [ ... final-byte in 0x40..=0x7E
                    i += 2;
                    while i < bytes.len() {
                        let c = bytes[i];
                        i += 1;
                        if (0x40..=0x7e).contains(&c) { break; }
                    }
                    continue;
                }
                _ => { i += 2; continue; } // skip ESC + next byte
            }
        }
        // Skip non-printable ASCII control chars (except newline/tab)
        if b < 0x20 && b != b'\n' && b != b'\t' { i += 1; continue; }
        // Preserve full UTF-8 characters (input is &str, so guaranteed valid)
        if b < 0x80 {
            out.push(b as char);
            i += 1;
        } else {
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Ask the user's login shell for both `which claude` and `$PATH`.
/// Returns `(claude_path, path_env)`. Either may be `None` if resolution fails.
fn resolve_shell_env() -> (Option<String>, Option<String>) {
    let user_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    if let Ok(output) = std::process::Command::new(&user_shell)
        .args(["-l", "-i", "-c", "printf '%s\\n%s\\n' \"$(which claude)\" \"$PATH\""])
        .output()
    {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout);
            let stdout = strip_terminal_escapes(&raw);
            let mut lines = stdout.lines();
            let claude = lines.next().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
            let path = lines.next().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
            return (claude, path);
        }
    }
    (None, None)
}

/// Build a PATH env value suitable for spawning claude.
/// Prepends the parent directory of `claude_bin` (if absolute) and
/// appends the user's login-shell PATH plus sensible defaults.
fn build_spawn_path(claude_bin: &str, shell_path: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(parent) = std::path::Path::new(claude_bin).parent() {
        let p = parent.to_string_lossy();
        if !p.is_empty() { parts.push(p.to_string()); }
    }
    if let Some(sp) = shell_path {
        for p in sp.split(':').filter(|s| !s.is_empty()) {
            if !parts.iter().any(|x| x == p) { parts.push(p.to_string()); }
        }
    }
    for p in ["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        if !parts.iter().any(|x| x == p) { parts.push(p.to_string()); }
    }
    parts.join(":")
}

/// Spawn a terminal using Tauri's Channel API for streaming PTY output to the frontend.
/// The `on_output` channel sends TerminalOutputPayload messages directly to JS callback.
#[tauri::command]
fn spawn_terminal(
    state: State<'_, TerminalState>,
    session_id: Option<String>,
    project_path: String,
    context_prompt: Option<String>,
    skip_permissions: Option<bool>,
    claude_path: Option<String>,
    on_output: Channel<TerminalOutputPayload>,
) -> Result<String, String> {
    let terminal_id = Uuid::new_v4().to_string();

    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to open PTY: {}", e))?;

    let (detected_claude, shell_path) = resolve_shell_env();
    let global_claude = load_settings()
        .claude_path
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let claude_bin = claude_path
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or(global_claude)
        .or(detected_claude)
        .unwrap_or_else(|| "claude".to_string());
    let path_env = build_spawn_path(&claude_bin, shell_path.as_deref());
    eprintln!("[Clauge] Resolved claude binary: {}", claude_bin);
    eprintln!("[Clauge] CWD: {}", project_path);
    eprintln!("[Clauge] PATH: {}", path_env);

    let mut cmd = CommandBuilder::new(&claude_bin);

    if let Some(ref sid) = session_id {
        cmd.arg("--resume");
        cmd.arg(sid);
    }
    if skip_permissions.unwrap_or(false) {
        cmd.arg("--dangerously-skip-permissions");
    }
    if let Some(ref prompt) = context_prompt {
        if !prompt.is_empty() {
            cmd.arg("--append-system-prompt");
            cmd.arg(prompt);
        }
    }

    cmd.cwd(&project_path);

    if let Some(home) = dirs::home_dir() {
        cmd.env("HOME", home.to_string_lossy().to_string());
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("PATH", &path_env);
    if let Ok(shell) = std::env::var("SHELL") {
        cmd.env("SHELL", shell);
    }
    if let Ok(lang) = std::env::var("LANG") {
        cmd.env("LANG", lang);
    }

    let child = pty_pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| {
            eprintln!("[Clauge] Spawn FAILED: {}", e);
            format!("Failed to spawn claude: {}", e)
        })?;

    eprintln!("[Clauge] Spawn SUCCESS, terminal_id: {}", terminal_id);

    let writer = pty_pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to get PTY writer: {}", e))?;

    let reader = pty_pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("Failed to clone PTY reader: {}", e))?;

    let tid_clone = terminal_id.clone();
    std::thread::spawn(move || {
        eprintln!("[Clauge] Reader thread started for {}", tid_clone);
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        let mut total_bytes = 0usize;
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    eprintln!("[Clauge] Reader EOF (total {} bytes)", total_bytes);
                    break;
                }
                Ok(n) => {
                    total_bytes += n;
                    if total_bytes == n {
                        eprintln!("[Clauge] First read: {} bytes", n);
                    }
                    let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    let payload = TerminalOutputPayload {
                        terminal_id: tid_clone.clone(),
                        data,
                    };
                    if let Err(e) = on_output.send(payload) {
                        eprintln!("[Clauge] Channel send error: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("[Clauge] Reader error: {}", e);
                    break;
                }
            }
        }
    });

    let entry = TerminalEntry {
        master: pty_pair.master,
        writer,
        child,
    };

    state
        .terminals
        .lock()
        .insert(terminal_id.clone(), entry);

    Ok(terminal_id)
}

/// Spawn a plain shell terminal (no Claude) in the given project directory
#[tauri::command]
fn spawn_shell(
    state: State<'_, TerminalState>,
    project_path: String,
    on_output: Channel<TerminalOutputPayload>,
) -> Result<String, String> {
    let terminal_id = Uuid::new_v4().to_string();
    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Failed to open PTY: {}", e))?;

    let user_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut cmd = CommandBuilder::new(&user_shell);
    cmd.arg("-l");
    cmd.arg("-i");
    cmd.cwd(&project_path);
    if let Some(home) = dirs::home_dir() {
        cmd.env("HOME", home.to_string_lossy().to_string());
    }
    cmd.env("TERM", "xterm-256color");

    let child = pty_pair.slave.spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn shell: {}", e))?;

    let writer = pty_pair.master.take_writer()
        .map_err(|e| format!("Failed to get PTY writer: {}", e))?;
    let reader = pty_pair.master.try_clone_reader()
        .map_err(|e| format!("Failed to clone PTY reader: {}", e))?;

    let tid_clone = terminal_id.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    if on_output.send(TerminalOutputPayload { terminal_id: tid_clone.clone(), data }).is_err() { break; }
                }
                Err(_) => break,
            }
        }
    });

    state.terminals.lock()
        .insert(terminal_id.clone(), TerminalEntry { master: pty_pair.master, writer, child });

    Ok(terminal_id)
}

#[tauri::command]
fn write_to_terminal(
    state: State<'_, TerminalState>,
    terminal_id: String,
    data: String,
) -> Result<(), String> {
    let mut terminals = state
        .terminals
        .lock();

    let entry = terminals
        .get_mut(&terminal_id)
        .ok_or("Terminal not found")?;

    entry
        .writer
        .write_all(data.as_bytes())
        .map_err(|e| format!("Write error: {}", e))?;

    entry
        .writer
        .flush()
        .map_err(|e| format!("Flush error: {}", e))?;

    Ok(())
}

#[tauri::command]
fn resize_terminal(
    state: State<'_, TerminalState>,
    terminal_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let terminals = state
        .terminals
        .lock();

    let entry = terminals.get(&terminal_id).ok_or("Terminal not found")?;

    entry
        .master
        .resize(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Resize error: {}", e))?;

    Ok(())
}

#[tauri::command]
fn kill_terminal(
    state: State<'_, TerminalState>,
    terminal_id: String,
) -> Result<(), String> {
    let mut terminals = state.terminals.lock();
    if let Some(mut entry) = terminals.remove(&terminal_id) {
        let _ = entry.child.kill();
        eprintln!("[Clauge] Killed terminal {}", terminal_id);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(TerminalState::default())
        .invoke_handler(tauri::generate_handler![
            get_profiles,
            get_settings,
            save_settings,
            create_profile,
            delete_profile,
            rename_profile,
            update_last_used,
            refresh_session_ids,
            update_session_id,
            is_git_repo,
            create_worktree,
            remove_worktree,
            update_profile_worktree,
            count_project_sessions,
            discover_sessions,
            get_session_tokens,
            fetch_usage_limits,
            get_app_version,
            get_claude_plan,
            update_tray_title,
            save_session_key,
            load_session_key,
            spawn_terminal,
            spawn_shell,
            write_to_terminal,
            resize_terminal,
            kill_terminal
        ])
        .setup(|app| {
            let setup_start = std::time::Instant::now();
            eprintln!("[TIMING] setup start");

            let window = app.get_webview_window("main").unwrap();

            #[cfg(target_os = "macos")]
            {
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
                apply_vibrancy(&window, NSVisualEffectMaterial::Sidebar, None, None)
                    .expect("Failed to apply vibrancy");
            }
            eprintln!("[TIMING] vibrancy applied: {:?}", setup_start.elapsed());

            // ---- App menu bar ----
            let app_menu = Submenu::with_items(app, "Clauge", true, &[
                &PredefinedMenuItem::about(app, Some("About Clauge"), None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::show_all(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, None)?,
            ])?;
            let edit_menu = Submenu::with_items(app, "Edit", true, &[
                &PredefinedMenuItem::undo(app, None)?,
                &PredefinedMenuItem::redo(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::cut(app, None)?,
                &PredefinedMenuItem::copy(app, None)?,
                &PredefinedMenuItem::paste(app, None)?,
                &PredefinedMenuItem::select_all(app, None)?,
            ])?;
            let window_menu = Submenu::with_items(app, "Window", true, &[
                &PredefinedMenuItem::minimize(app, None)?,
                &PredefinedMenuItem::maximize(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::close_window(app, None)?,
            ])?;
            let menu_bar = Menu::with_items(app, &[&app_menu, &edit_menu, &window_menu])?;
            app.set_menu(menu_bar)?;

            // ---- System tray ----
            let show_item = MenuItem::with_id(app, "show", "Back to App", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[&show_item, &separator, &quit])?;

            // Use custom tray icon — template mode so macOS adapts to light/dark menu bar
            let icon_png = include_bytes!("../icons/tray-dark.png");
            let img = image::load_from_memory(icon_png).expect("Failed to load tray icon");
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let tray_icon = tauri::image::Image::new_owned(rgba.into_raw(), w, h);
            TrayIconBuilder::with_id("main-tray")
                .icon(tray_icon)
                .icon_as_template(true)
                .menu(&menu)
                .title("Clauge")
                .tooltip("Clauge — Claude Session Manager")
                .on_menu_event(move |app: &tauri::AppHandle, event: tauri::menu::MenuEvent| {
                    let id = event.id().as_ref();
                    if id == "quit" {
                        app.exit(0);
                    } else if id == "show" {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            eprintln!("[TIMING] tray built: {:?}", setup_start.elapsed());

            // Enable autostart on first run
            use tauri_plugin_autostart::ManagerExt;
            let _ = app.autolaunch().enable();

            eprintln!("[TIMING] setup complete: {:?}", setup_start.elapsed());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide instead of quit — user can quit from tray
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            match event {
                tauri::RunEvent::Reopen { .. } => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                tauri::RunEvent::ExitRequested { .. } => {
                    if let Some(state) = app.try_state::<TerminalState>() {
                        let mut terminals = state.terminals.lock();
                        for (id, mut entry) in terminals.drain() {
                            let _ = entry.child.kill();
                            eprintln!("[Clauge] Cleaned up terminal {} on exit", id);
                        }
                    }
                }
                _ => {}
            }
        });
}

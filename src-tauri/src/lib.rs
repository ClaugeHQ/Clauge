use base64::Engine;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
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

/// Write a CLAUDE.md file in the session's working directory
/// Claude Code reads this automatically on every turn
fn inject_claude_md(working_dir: &str, purpose: &str, title: &str) -> Result<(), String> {
    let prompt = get_context_prompt(purpose);
    if prompt.is_empty() { return Ok(()); }

    let claude_dir = PathBuf::from(working_dir).join(".claude");
    let _ = std::fs::create_dir_all(&claude_dir);
    let md_path = claude_dir.join("CLAUDE.md");

    // Don't overwrite if it already exists (user may have customized it)
    if md_path.exists() {
        let existing = std::fs::read_to_string(&md_path).unwrap_or_default();
        if existing.contains("# Session:") {
            // Our file — safe to update
            std::fs::write(&md_path, &prompt).map_err(|e| e.to_string())?;
        }
        // User's custom file — leave it alone
    } else {
        std::fs::write(&md_path, &prompt).map_err(|e| e.to_string())?;
    }

    Ok(())
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
    // Claude encodes project paths by replacing '/' with '-'
    // e.g. /Users/macbook/Personal/pulse -> -Users-macbook-Personal-pulse
    project_path.replace('/', "-")
}

fn discover_claude_session_id(project_path: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    let encoded = encode_project_path(project_path);
    let projects_dir = home.join(".claude").join("projects").join(&encoded);

    if !projects_dir.exists() {
        return None;
    }

    let mut best: Option<(String, std::time::SystemTime)> = None;

    if let Ok(entries) = std::fs::read_dir(&projects_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Ok(meta) = path.metadata() {
                    if let Ok(modified) = meta.modified() {
                        let session_id = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some((_, best_time)) = &best {
                            if modified > *best_time {
                                best = Some((session_id, modified));
                            }
                        } else {
                            best = Some((session_id, modified));
                        }
                    }
                }
            }
        }
    }

    best.map(|(id, _)| id)
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
fn create_profile(title: String, purpose: String, project_path: String) -> Result<SessionProfile, String> {
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
        context_prompt: get_context_prompt(&purpose),
        created_at: now.clone(),
        last_used_at: now,
        worktree_path: None,
        worktree_branch: None,
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

/// Refresh claudeSessionId for all profiles that have null session IDs
#[tauri::command]
fn refresh_session_ids() -> Result<Vec<SessionProfile>, String> {
    let mut profiles = load_profiles();
    let mut changed = false;
    for profile in profiles.iter_mut() {
        if profile.claude_session_id.is_none() {
            if let Some(sid) = discover_claude_session_id(&profile.project_path) {
                profile.claude_session_id = Some(sid);
                changed = true;
            }
        }
    }
    if changed {
        save_profiles(&profiles)?;
    }
    Ok(profiles)
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

/// Create a git worktree for session isolation
#[tauri::command]
fn create_worktree(project_path: String, branch_name: String) -> Result<String, String> {
    // Worktree goes inside a hidden directory in the project
    let worktree_dir = PathBuf::from(&project_path)
        .join(".clauge-worktrees")
        .join(&branch_name);
    let worktree_path = worktree_dir.to_string_lossy().to_string();

    // Create parent dir
    let _ = std::fs::create_dir_all(worktree_dir.parent().unwrap_or(&worktree_dir));

    // Create worktree with new branch
    let output = std::process::Command::new("git")
        .args(["-C", &project_path, "worktree", "add", "-b", &branch_name, &worktree_path])
        .output()
        .map_err(|e| format!("git worktree add failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Branch might already exist — try without -b
        if stderr.contains("already exists") {
            let output2 = std::process::Command::new("git")
                .args(["-C", &project_path, "worktree", "add", &worktree_path, &branch_name])
                .output()
                .map_err(|e| format!("git worktree add (existing branch) failed: {}", e))?;
            if !output2.status.success() {
                return Err(format!("git worktree add failed: {}", String::from_utf8_lossy(&output2.stderr)));
            }
        } else {
            return Err(format!("git worktree add failed: {}", stderr));
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

/// Remove a git worktree
#[tauri::command]
fn remove_worktree(project_path: String, worktree_path: String) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(["-C", &project_path, "worktree", "remove", "--force", &worktree_path])
        .output()
        .map_err(|e| format!("git worktree remove failed: {}", e))?;

    if !output.status.success() {
        // If worktree dir is already gone, just prune
        let _ = std::process::Command::new("git")
            .args(["-C", &project_path, "worktree", "prune"])
            .output();
    }
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

            sessions.push(DiscoveredSession {
                session_id,
                modified_at,
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

fn get_session_key_from_keychain() -> Result<String, String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .map_err(|e| format!("Keychain access failed: {}", e))?;

    if !output.status.success() {
        return Err("No Claude Code credentials in keychain".to_string());
    }

    let json_str = String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8: {}", e))?;

    let parsed: serde_json::Value = serde_json::from_str(json_str.trim())
        .map_err(|e| format!("JSON parse failed: {}", e))?;

    parsed
        .get("claudeAiOauth")
        .and_then(|o| o.get("accessToken").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .ok_or("No access token found in credentials".to_string())
}

/// Fetch usage limits via precompiled Swift binary (NSURLSession bypasses Cloudflare, ~1.5s)
#[tauri::command]
fn fetch_usage_limits(session_key: String) -> Result<serde_json::Value, String> {
    // Find the precompiled binary — check next to the app binary first, then scripts/
    let binary_candidates = vec![
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("fetch_usage"))),
        Some(PathBuf::from("scripts/fetch_usage")),
        Some(PathBuf::from("../scripts/fetch_usage")),
        Some(PathBuf::from("src-tauri/scripts/fetch_usage")),
    ];

    let binary = binary_candidates
        .into_iter()
        .flatten()
        .find(|p| p.exists())
        .ok_or("fetch_usage binary not found")?;

    let output = std::process::Command::new(&binary)
        .arg(&session_key)
        .output()
        .map_err(|e| format!("fetch_usage exec failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Err("No response from API".to_string());
    }

    serde_json::from_str(&stdout)
        .map_err(|e| format!("Parse error: {} (response: {})", e, &stdout[..stdout.len().min(200)]))
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

/// Update the tray title text (shown in menu bar)
#[tauri::command]
fn update_tray_title(app_handle: tauri::AppHandle, title: String) -> Result<(), String> {
    if let Some(tray) = app_handle.tray_by_id(&TrayIconId::new("main-tray")) {
        tray.set_title(Some(&title)).map_err(|e| format!("Tray title error: {}", e))?;
    }
    Ok(())
}


/// Spawn a terminal using Tauri's Channel API for streaming PTY output to the frontend.
/// The `on_output` channel sends TerminalOutputPayload messages directly to JS callback.
#[tauri::command]
fn spawn_terminal(
    state: State<'_, TerminalState>,
    session_id: Option<String>,
    project_path: String,
    context_prompt: Option<String>,
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

    let mut claude_cmd = String::from("claude");
    if let Some(ref sid) = session_id {
        claude_cmd.push_str(&format!(" --resume {}", sid));
    }

    eprintln!("[Clauge] Spawning: /bin/zsh -l -c '{}'", claude_cmd);
    eprintln!("[Clauge] CWD: {}", project_path);

    let mut cmd = CommandBuilder::new("/bin/zsh");
    cmd.arg("-l");
    cmd.arg("-c");
    cmd.arg(&claude_cmd);
    cmd.cwd(&project_path);

    if let Some(home) = dirs::home_dir() {
        cmd.env("HOME", home.to_string_lossy().to_string());
    }
    cmd.env("TERM", "xterm-256color");

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

    // Inject CLAUDE.md if this is a new session (no session_id = first run)
    if session_id.is_none() {
        if let Some(ref prompt) = context_prompt {
            if !prompt.is_empty() {
                // prompt format: "purpose|title" — extract and inject CLAUDE.md
                let parts: Vec<&str> = prompt.splitn(2, '|').collect();
                if parts.len() == 2 {
                    let _ = inject_claude_md(&project_path, parts[0], parts[1]);
                }
            }
        }
    }

    let entry = TerminalEntry {
        master: pty_pair.master,
        writer,
        child,
    };

    state
        .terminals
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .insert(terminal_id.clone(), entry);

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
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

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
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

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

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(TerminalState::default())
        .invoke_handler(tauri::generate_handler![
            get_profiles,
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
            update_tray_title,
            save_session_key,
            load_session_key,
            spawn_terminal,
            write_to_terminal,
            resize_terminal
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
            eprintln!("[TIMING] setup complete: {:?}", setup_start.elapsed());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

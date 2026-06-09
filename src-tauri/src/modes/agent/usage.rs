use crate::modes::agent::models::*;
use crate::shared::cli::{registry::runner_for, runner::CliRunner};
use std::fs;
use std::path::PathBuf;

// Usage analytics today reads Claude's per-project JSONL files. The
// equivalent surfaces on Codex (`~/.codex/state_5.sqlite`) and OpenCode
// (`~/.local/share/opencode/opencode.db`) need their own parsers — for
// now, all callers hard-code Claude. Each function calls this helper
// instead of referencing the static `CLAUDE` to make swapping in
// per-session dispatch a one-line change later.
fn claude_cli() -> &'static dyn CliRunner {
    runner_for("claude")
}

#[tauri::command]
pub async fn agent_get_usage_analytics(
    days: Option<u32>,
    provider: Option<String>,
) -> Result<UsageAnalytics, String> {
    let prov = provider.unwrap_or_else(|| "claude".to_string());
    match prov.as_str() {
        "codex" => tauri::async_runtime::spawn_blocking(move || codex_usage_analytics(days))
            .await
            .map_err(|e| format!("Thread error: {}", e))?,
        "gemini" => tauri::async_runtime::spawn_blocking(move || gemini_usage_analytics(days))
            .await
            .map_err(|e| format!("Thread error: {}", e))?,
        "opencode" => opencode_usage_analytics(days).await,
        _ => tauri::async_runtime::spawn_blocking(move || agent_get_usage_analytics_sync(days))
            .await
            .map_err(|e| format!("Thread error: {}", e))?,
    }
}

pub fn agent_get_usage_analytics_sync(days: Option<u32>) -> Result<UsageAnalytics, String> {
    let cli: &dyn CliRunner = claude_cli();
    let projects_dir = cli.sessions_root().ok_or("Cannot determine home directory")?;

    if !projects_dir.exists() {
        return Ok(UsageAnalytics {
            total_cost: 0.0, total_input_tokens: 0, total_output_tokens: 0,
            total_cache_read_tokens: 0, total_cache_write_tokens: 0,
            total_sessions: 0, total_api_calls: 0, cache_hit_percent: 0.0,
            daily: vec![], by_model: vec![], by_project: vec![],
            top_sessions: vec![], tools: vec![], shell_commands: vec![],
        });
    }

    let days_limit = days.unwrap_or(30);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days_limit as i64);

    // Pricing per million tokens (approximate Claude pricing)
    let price_for_model = |model: &str| -> (f64, f64, f64, f64) {
        // (input, output, cache_read, cache_write) per million tokens
        let m = model.to_lowercase();
        if m.contains("opus") { (15.0, 75.0, 1.5, 18.75) }
        else if m.contains("haiku") { (0.80, 4.0, 0.08, 1.0) }
        else { (3.0, 15.0, 0.3, 3.75) } // sonnet default
    };

    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut total_cache_read: u64 = 0;
    let mut total_cache_write: u64 = 0;
    let mut total_cost: f64 = 0.0;
    let mut total_calls: u32 = 0;
    let mut total_sessions: u32 = 0;

    let mut daily_map: std::collections::HashMap<String, (f64, u32, u64, u64)> = std::collections::HashMap::new();
    let mut model_map: std::collections::HashMap<String, (f64, u32, u64, u64, u64, u64)> = std::collections::HashMap::new();
    let mut project_map: std::collections::HashMap<String, (f64, u32, u32)> = std::collections::HashMap::new();
    let mut session_costs: Vec<SessionCost> = Vec::new();
    let mut tool_map: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut shell_map: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    // Iterate all project directories
    for project_entry in std::fs::read_dir(&projects_dir).map_err(|e| e.to_string())?.flatten() {
        let project_name = project_entry.file_name().to_string_lossy().to_string();
        let project_dir = project_entry.path();
        if !project_dir.is_dir() { continue; }

        let mut project_cost: f64 = 0.0;
        let mut project_sessions: u32 = 0;
        let mut project_calls: u32 = 0;

        // Iterate session files
        for session_entry in std::fs::read_dir(&project_dir).map_err(|e| e.to_string())?.flatten() {
            let path = session_entry.path();
            if !cli.is_session_file(&path) { continue; }

            // Check modification time
            if let Ok(metadata) = path.metadata() {
                if let Ok(modified) = metadata.modified() {
                    let modified_time: chrono::DateTime<chrono::Utc> = modified.into();
                    if modified_time < cutoff { continue; }
                }
            }

            let session_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let mut session_cost: f64 = 0.0;
            let mut session_calls: u32 = 0;
            let mut session_model = String::new();
            total_sessions += 1;
            project_sessions += 1;

            for line in content.lines() {
                if line.trim().is_empty() { continue; }
                let val: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Extract model and usage from assistant messages
                let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if msg_type != "assistant" { continue; }

                let message = match val.get("message") {
                    Some(m) => m,
                    None => continue,
                };

                let model = message.get("model").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                if session_model.is_empty() { session_model = model.clone(); }

                let usage = match message.get("usage") {
                    Some(u) => u,
                    None => continue,
                };

                let input = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let output = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let cache_read = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let cache_write = usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

                let (pi, po, pcr, pcw) = price_for_model(&model);
                let call_cost = (input as f64 * pi + output as f64 * po + cache_read as f64 * pcr + cache_write as f64 * pcw) / 1_000_000.0;

                total_input += input;
                total_output += output;
                total_cache_read += cache_read;
                total_cache_write += cache_write;
                total_cost += call_cost;
                total_calls += 1;
                session_cost += call_cost;
                session_calls += 1;
                project_cost += call_cost;
                project_calls += 1;

                // Daily
                let date_str = val.get("timestamp").and_then(|v| v.as_str())
                    .map(|t| t[..10].to_string())
                    .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
                let daily = daily_map.entry(date_str).or_insert((0.0, 0, 0, 0));
                daily.0 += call_cost;
                daily.1 += 1;
                daily.2 += input;
                daily.3 += output;

                // Model
                let short_model = if model.contains("opus") { "Opus".to_string() }
                    else if model.contains("haiku") { "Haiku".to_string() }
                    else if model.contains("sonnet") { "Sonnet".to_string() }
                    else { model.clone() };
                let me = model_map.entry(short_model).or_insert((0.0, 0, 0, 0, 0, 0));
                me.0 += call_cost;
                me.1 += 1;
                me.2 += input;
                me.3 += output;
                me.4 += cache_read;
                me.5 += cache_write;

                // Tools
                if let Some(content_arr) = message.get("content").and_then(|v| v.as_array()) {
                    for block in content_arr {
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            let tool_name = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                            *tool_map.entry(tool_name.clone()).or_insert(0) += 1;

                            // Extract shell commands from Bash tool
                            if tool_name == "Bash" || tool_name == "bash" {
                                if let Some(input_obj) = block.get("input") {
                                    if let Some(cmd) = input_obj.get("command").and_then(|v| v.as_str()) {
                                        let shell_cmd = cmd.split_whitespace().next().unwrap_or("").to_string();
                                        if !shell_cmd.is_empty() {
                                            *shell_map.entry(shell_cmd).or_insert(0) += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if session_calls > 0 {
                session_costs.push(SessionCost {
                    session_id,
                    project: project_name.clone(),
                    cost: session_cost,
                    calls: session_calls,
                    model: session_model,
                });
            }
        }

        if project_sessions > 0 {
            project_map.insert(project_name, (project_cost, project_sessions, project_calls));
        }
    }

    // Sort and format results
    let mut daily: Vec<DailyUsage> = daily_map.into_iter().map(|(date, (cost, calls, input, output))| {
        DailyUsage { date, cost, calls, input_tokens: input, output_tokens: output }
    }).collect();
    daily.sort_by(|a, b| a.date.cmp(&b.date));

    let mut by_model: Vec<ModelUsage> = model_map.into_iter().map(|(model, (cost, calls, input, output, cr, cw))| {
        let total_input_for_model = input + cr + cw;
        let cache_pct = if total_input_for_model > 0 { (cr as f64 / total_input_for_model as f64) * 100.0 } else { 0.0 };
        ModelUsage { model, cost, calls, input_tokens: input, output_tokens: output, cache_hit_percent: cache_pct }
    }).collect();
    by_model.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));

    let mut by_project: Vec<ProjectUsage> = project_map.into_iter().map(|(project, (cost, sessions, calls))| {
        ProjectUsage { project, cost, sessions, calls }
    }).collect();
    by_project.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));

    session_costs.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let top_sessions = session_costs.into_iter().take(5).collect();

    let mut tools: Vec<ToolCount> = tool_map.into_iter().map(|(name, count)| ToolCount { name, count }).collect();
    tools.sort_by(|a, b| b.count.cmp(&a.count));

    let mut shell_commands: Vec<ToolCount> = shell_map.into_iter().map(|(name, count)| ToolCount { name, count }).collect();
    shell_commands.sort_by(|a, b| b.count.cmp(&a.count));
    shell_commands.truncate(15);

    let total_all_input = total_input + total_cache_read + total_cache_write;
    let cache_hit_percent = if total_all_input > 0 { (total_cache_read as f64 / total_all_input as f64) * 100.0 } else { 0.0 };

    Ok(UsageAnalytics {
        total_cost,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_cache_read_tokens: total_cache_read,
        total_cache_write_tokens: total_cache_write,
        total_sessions,
        total_api_calls: total_calls,
        cache_hit_percent,
        daily,
        by_model,
        by_project,
        top_sessions,
        tools,
        shell_commands,
    })
}

/// Fetch usage limits via reqwest with native-tls (uses macOS SecureTransport to bypass Cloudflare)
#[tauri::command]
pub async fn agent_fetch_usage_limits(session_key: String) -> Result<serde_json::Value, String> {
    let cli: &dyn CliRunner = claude_cli();
    let orgs_url = cli.usage_api_orgs_url().ok_or("CLI does not expose a usage API")?;

    let client = reqwest::Client::builder()
        .use_native_tls()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
    let cookie = format!("sessionKey={}", session_key);

    // Step 1: Get org ID
    let orgs_resp = client
        .get(&orgs_url)
        .header("Cookie", &cookie)
        .header("User-Agent", ua)
        .send()
        .await
        .map_err(|e| format!("orgs request failed: {}", e))?;

    if !orgs_resp.status().is_success() {
        return Err(usage_auth_error("organization", orgs_resp.status()));
    }

    let orgs: Vec<serde_json::Value> = orgs_resp
        .json()
        .await
        .map_err(|e| format!("orgs parse failed: {}", e))?;

    let org_id = orgs
        .first()
        .and_then(|o: &serde_json::Value| o.get("uuid"))
        .and_then(|v: &serde_json::Value| v.as_str())
        .ok_or("No organization found")?
        .to_string();

    // Step 2: Get usage
    let usage_url = cli
        .usage_api_url_for(&org_id)
        .ok_or("CLI does not expose a per-org usage URL")?;
    let usage_resp = client
        .get(&usage_url)
        .header("Cookie", &cookie)
        .header("User-Agent", ua)
        .send()
        .await
        .map_err(|e| format!("usage request failed: {}", e))?;

    if !usage_resp.status().is_success() {
        return Err(usage_auth_error("usage", usage_resp.status()));
    }

    let usage: serde_json::Value = usage_resp
        .json()
        .await
        .map_err(|e| format!("usage parse failed: {}", e))?;

    Ok(usage)
}

fn usage_auth_error(stage: &str, status: reqwest::StatusCode) -> String {
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return "Claude session key is expired or invalid. Reconfigure usage tracking in Settings > Agent.".to_string();
    }

    format!("Claude {} request failed with HTTP {}", stage, status)
}

/// Fetch ChatGPT/Codex live rate-limit usage for the footer chip.
///
/// Hits `https://chatgpt.com/backend-api/wham/usage` with the user's
/// Codex CLI access token (`agent_codex_access_token`). Response shape:
/// ```json
/// { "rate_limit": { "primary_window": { "used_percent": 100,
///                                       "limit_window_seconds": 604800 },
///                   "secondary_window": { ... } | null },
///   "plan_type": "go", ... }
/// ```
/// Frontend uses `rate_limit.primary_window.used_percent` (and the
/// optional secondary window) for the StatusBar chips. The full payload
/// is returned so the Settings UI can surface plan/credits later.
#[tauri::command]
pub async fn agent_fetch_codex_usage_limits(access_token: String) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .use_native_tls()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";

    let resp = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Accept", "*/*")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("User-Agent", ua)
        .send()
        .await
        .map_err(|e| format!("Codex usage request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(codex_auth_error(resp.status()));
    }

    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Codex usage parse failed: {}", e))
}

fn codex_auth_error(status: reqwest::StatusCode) -> String {
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return "Codex access token is expired or invalid. Reconfigure usage tracking in Settings > Agent.".to_string();
    }
    format!("Codex usage request failed with HTTP {}", status)
}

#[tauri::command]
pub fn agent_discover_sessions(
    project_path: String,
    provider: Option<String>,
) -> Result<Vec<DiscoveredSession>, String> {
    match provider.as_deref().unwrap_or("claude") {
        "codex" => discover_codex_sessions(&project_path),
        "gemini" => discover_gemini_sessions(&project_path),
        "opencode" => discover_opencode_sessions(&project_path),
        _ => discover_claude_sessions(&project_path),
    }
}

/// Look up the most recently-touched session id for `(provider,
/// project_path)`. Used by the spawn path to "rehydrate" a Clauge
/// session row whose stored session id was never captured. Critical
/// for crash/update recovery: if the app died before the PTY-output
/// regex matched, the UUID is otherwise unrecoverable and clicking
/// the existing session row would silently start a fresh Claude
/// session instead of resuming. Returns `None` when no matching
/// session exists on disk.
#[tauri::command]
pub fn agent_resolve_resume_id(
    project_path: String,
    provider: Option<String>,
) -> Result<Option<String>, String> {
    let p = provider.as_deref().unwrap_or("claude");
    let sessions = match p {
        "claude" => discover_claude_sessions(&project_path)?,
        "codex" => discover_codex_sessions(&project_path)?,
        "gemini" => discover_gemini_sessions(&project_path)?,
        "opencode" => discover_opencode_sessions(&project_path)?,
        _ => return Ok(None),
    };
    // discover_*_sessions sort descending by modified_at, so the first
    // entry is the newest. Empty list → None (no session created yet).
    Ok(sessions.into_iter().next().map(|s| s.session_id))
}

/// Canonicalize a path for comparison. Falls back to the literal path
/// when the path doesn't exist on disk — `realpath` errors on missing
/// paths, but we still want a reasonable comparison value (e.g. a
/// stored project path whose folder has been moved). macOS APFS is
/// case-insensitive at the FS level, so case-insensitive compare
/// covers the rare Linux user who creates a case-mismatch dir.
fn canon_for_compare(path: &str) -> String {
    let resolved = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string());
    // Strip the macOS /private/ prefix (added by canonicalize for /var
    // and /tmp) so both sides compare equally regardless of which
    // form the caller used.
    let stripped = resolved.strip_prefix("/private").unwrap_or(&resolved);
    stripped.to_lowercase()
}

/// Peek the FIRST `cwd` value out of a JSONL session file. Claude
/// records its launch CWD on the early lines; later lines can record
/// additional cwds when the session uses Bash to `cd` into subdirs —
/// we only want the launch cwd because that's what determines which
/// `~/.claude/projects/<dir>/` the file lives in.
fn peek_session_cwd(path: &std::path::Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(f);
    for (i, line) in reader.lines().enumerate() {
        if i > 50 { break; }
        let line = line.ok()?;
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(cwd) = val.get("cwd").and_then(|v| v.as_str()) {
                return Some(cwd.to_string());
            }
        }
    }
    None
}

fn read_one_session_file(path: &std::path::Path) -> Option<DiscoveredSession> {
    let session_id = path.file_stem().and_then(|s| s.to_str())?.to_string();
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
    let preview = fs::read_to_string(path).ok().and_then(|content| {
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

    Some(DiscoveredSession { session_id, modified_at, preview })
}

fn discover_claude_sessions(project_path: &str) -> Result<Vec<DiscoveredSession>, String> {
    let cli: &dyn CliRunner = claude_cli();
    let projects_root = cli.sessions_root().ok_or("Cannot determine home directory")?;
    let mut sessions: Vec<DiscoveredSession> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // L1 fast path: try the predictably-encoded directory first.
    let primary = cli.session_dir_for_project(project_path);
    if let Some(dir) = primary.as_ref() {
        if dir.exists() {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !cli.is_session_file(&path) { continue; }
                    if let Some(s) = read_one_session_file(&path) {
                        if seen_ids.insert(s.session_id.clone()) {
                            sessions.push(s);
                        }
                    }
                }
            }
        }
    }

    // L2 fallback: enumerate every project dir under ~/.claude/projects
    // and match by the launch-cwd recorded in each session file. Covers
    // (a) paths whose encoder rule we don't have exact knowledge of,
    // (b) macOS /tmp ↔ /private/tmp symlinks, (c) case-quirk dirs on
    // case-sensitive filesystems, (d) any future Claude CLI change to
    // its encoder. Idempotent against the fast path via `seen_ids`.
    if projects_root.exists() {
        let target = canon_for_compare(project_path);
        if let Ok(top) = fs::read_dir(&projects_root) {
            for entry in top.flatten() {
                let dir = entry.path();
                if !dir.is_dir() { continue; }
                // Skip the dir we already scanned in the fast path.
                if let Some(p) = primary.as_ref() {
                    if &dir == p { continue; }
                }
                // Peek the first .jsonl file's cwd. If it matches our
                // canonicalized target, every session file in this dir
                // belongs to the same project.
                let first_file = fs::read_dir(&dir).ok().and_then(|mut it| {
                    it.find_map(|r| {
                        let p = r.ok()?.path();
                        if cli.is_session_file(&p) { Some(p) } else { None }
                    })
                });
                let Some(first_file) = first_file else { continue };
                let Some(cwd) = peek_session_cwd(&first_file) else { continue };
                if canon_for_compare(&cwd) != target { continue; }

                if let Ok(entries) = fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !cli.is_session_file(&path) { continue; }
                        if let Some(s) = read_one_session_file(&path) {
                            if seen_ids.insert(s.session_id.clone()) {
                                sessions.push(s);
                            }
                        }
                    }
                }
            }
        }
    }

    sessions.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(sessions)
}

/// Codex sessions live at
/// `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-<ts>-<UUID>.jsonl`,
/// keyed by date rather than per-project. Each file's first line is a
/// `session_meta` event whose payload carries `id` (the resume UUID),
/// `cwd` (the project path that session was bound to), and a
/// timestamp. We walk the date tree and filter by `cwd == project_path`.
/// The preview is the first user-typed message (`type: "response_item"`
/// → `payload.role: "user"` → first string content field) — Codex
/// distinguishes between system meta events and the user's first turn.
fn discover_codex_sessions(project_path: &str) -> Result<Vec<DiscoveredSession>, String> {
    let root = dirs::home_dir()
        .map(|h| h.join(".codex").join("sessions"))
        .ok_or("Cannot determine home directory")?;
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    walk_codex_sessions(&root, project_path, &mut sessions);
    sessions.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(sessions)
}

fn walk_codex_sessions(
    dir: &std::path::Path,
    project_path: &str,
    out: &mut Vec<DiscoveredSession>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            walk_codex_sessions(&path, project_path, out);
            continue;
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "jsonl")
            .unwrap_or(false)
        {
            continue;
        }
        if let Some(found) = parse_codex_session(&path, project_path) {
            out.push(found);
        }
    }
}

fn parse_codex_session(
    path: &std::path::Path,
    project_path: &str,
) -> Option<DiscoveredSession> {
    let content = fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let first = lines.next()?;
    let meta: serde_json::Value = serde_json::from_str(first).ok()?;
    if meta.get("type").and_then(|t| t.as_str()) != Some("session_meta") {
        return None;
    }
    let payload = meta.get("payload")?;
    let cwd = payload.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
    if cwd != project_path {
        return None;
    }
    let session_id = payload.get("id").and_then(|v| v.as_str())?.to_string();
    let modified_at = path
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let datetime: chrono::DateTime<chrono::Utc> = t.into();
            datetime.to_rfc3339()
        })
        .unwrap_or_default();

    // Best-effort preview: scan a few subsequent lines for the first
    // user turn. Codex stores conversation events as `response_item`
    // entries with a `role`. Fall back to None on anything unexpected.
    let mut preview: Option<String> = None;
    for line in lines.take(40) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let payload = val.get("payload");
            let role = payload
                .and_then(|p| p.get("role"))
                .and_then(|r| r.as_str());
            if role != Some("user") {
                continue;
            }
            // Content can be a string, or an array of {type, text} blocks.
            if let Some(text) = payload.and_then(|p| p.get("content")).and_then(|c| c.as_str()) {
                preview = Some(text.chars().take(80).collect());
                break;
            }
            if let Some(arr) = payload.and_then(|p| p.get("content")).and_then(|c| c.as_array()) {
                for item in arr {
                    if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                        preview = Some(t.chars().take(80).collect());
                        break;
                    }
                }
                if preview.is_some() {
                    break;
                }
            }
        }
    }

    Some(DiscoveredSession {
        session_id,
        modified_at,
        preview,
    })
}

/// OpenCode data directory — honors `$XDG_DATA_HOME` per OpenCode's
/// own discovery, falling back to `~/.local/share/opencode` (macOS,
/// Linux, and any Windows install that respects the convention).
/// Returns `None` only when neither env nor home dir is resolvable.
fn opencode_data_dir() -> Option<std::path::PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return Some(std::path::PathBuf::from(trimmed).join("opencode"));
        }
    }
    dirs::home_dir().map(|h| h.join(".local").join("share").join("opencode"))
}

/// Every `opencode*.db` file under the data dir. OpenCode ships
/// per-channel databases (e.g. `opencode.db`, `opencode-nightly.db`,
/// `opencode-canary.db`) — analytics should sum across them rather
/// than pick one. Cross-OS: `fs::read_dir` over a `PathBuf`, no
/// shell-glob.
fn opencode_db_paths() -> Vec<std::path::PathBuf> {
    let dir = match opencode_data_dir() {
        Some(d) => d,
        None => return Vec::new(),
    };
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("opencode") { continue; }
        // Accept only the primary DB file — sidecars like `.db-shm`
        // and `.db-wal` mustn't be passed to sqlx.
        if path.extension().and_then(|e| e.to_str()) != Some("db") { continue; }
        out.push(path);
    }
    out
}

/// Build read-only `SqliteConnectOptions` from a `PathBuf` without
/// formatting a URI string. The URI-string form (`sqlite://<path>`)
/// breaks on Windows because the drive-letter colon (`C:\…`) tripa
/// URI scheme parsing in sqlx's `from_str`. `filename(path)` bypasses
/// the URI lexer entirely — same effect, portable.
fn opencode_connect_opts(db_path: &std::path::Path) -> sqlx::sqlite::SqliteConnectOptions {
    sqlx::sqlite::SqliteConnectOptions::new()
        .filename(db_path)
        .read_only(true)
        .immutable(false)
}

/// OpenCode keeps every session in one or more SQLite databases
/// under its data dir (`opencode*.db`). The `session` table carries
/// `id`, `directory` (cwd it was started in), `title` (truncated
/// first prompt — perfect for preview), and `time_updated` (epoch
/// millis). Filter by `directory = project_path` exact match, then
/// merge results across all channel DBs.
fn discover_opencode_sessions(project_path: &str) -> Result<Vec<DiscoveredSession>, String> {
    let dbs = opencode_db_paths();
    if dbs.is_empty() {
        return Ok(Vec::new());
    }

    // Read-only async connect avoids contention with a running
    // opencode server (WAL mode is opencode's default).
    let project_owned = project_path.to_string();
    let runtime = tokio::runtime::Handle::try_current().ok();
    let mut all = Vec::new();
    for db_path in dbs {
        let rows_res = match &runtime {
            Some(handle) => handle.block_on(query_opencode_sessions(&db_path, &project_owned)),
            None => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("opencode discover runtime: {}", e))?;
                rt.block_on(query_opencode_sessions(&db_path, &project_owned))
            }
        };
        if let Ok(rows) = rows_res {
            all.extend(rows);
        }
    }
    all.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(all)
}

async fn query_opencode_sessions(
    db_path: &std::path::Path,
    project_path: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    let opts = opencode_connect_opts(db_path);
    let pool = sqlx::SqlitePool::connect_with(opts)
        .await
        .map_err(|e| format!("opencode db open: {}", e))?;
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT id, COALESCE(title, '') as title, time_updated \
         FROM session WHERE directory = ? \
         ORDER BY time_updated DESC LIMIT 50",
    )
    .bind(project_path)
    .fetch_all(&pool)
    .await
    .map_err(|e| format!("opencode db query: {}", e))?;
    pool.close().await;

    Ok(rows
        .into_iter()
        .map(|(id, title, ts_ms)| {
            let modified_at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();
            let preview = if title.trim().is_empty() {
                None
            } else {
                Some(title.chars().take(80).collect())
            };
            DiscoveredSession {
                session_id: id,
                modified_at,
                preview,
            }
        })
        .collect())
}

/// Antigravity (agy) stores conversations flat at
/// `~/.gemini/antigravity-cli/conversations/<uuid>.db` (SQLite). The
/// filename IS the conversation UUID, so resume discovery doesn't have
/// to open the database — we just enumerate `.db` files and use their
/// stems as the resumable id (`agy --conversation <uuid>` accepts it
/// directly). Per-project filtering would require reading each db's
/// `project_path` row; for now we list every conversation and let the
/// picker UI sort by recency.
fn discover_gemini_sessions(_project_path: &str) -> Result<Vec<DiscoveredSession>, String> {
    let cli: &dyn CliRunner = runner_for("gemini");
    let conversations_dir = match cli.session_dir_for_project(_project_path) {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };
    if !conversations_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    let entries = fs::read_dir(&conversations_dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !cli.is_session_file(&path) {
            continue;
        }
        let session_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Skip anything whose filename isn't a UUID — agy's
        // --conversation flag would reject it anyway.
        if !is_uuid_filename(&session_id) {
            continue;
        }
        let modified_at = path
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let datetime: chrono::DateTime<chrono::Utc> = t.into();
                datetime.to_rfc3339()
            })
            .unwrap_or_default();

        // Preview would require opening the SQLite database. Surface
        // the bare conversation id for now; a follow-up can wire
        // sqlx/rusqlite to pull the first user message.
        sessions.push(DiscoveredSession {
            session_id,
            modified_at,
            preview: None,
        });
    }
    sessions.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(sessions)
}

fn is_uuid_filename(s: &str) -> bool {
    if s.len() != 36 { return false; }
    for (i, b) in s.as_bytes().iter().enumerate() {
        let expect_dash = matches!(i, 8 | 13 | 18 | 23);
        if expect_dash {
            if *b != b'-' { return false; }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

#[tauri::command]
pub fn agent_get_session_tokens(
    project_path: String,
    session_id: Option<String>,
) -> Result<TokenUsage, String> {
    let cli: &dyn CliRunner = claude_cli();
    let projects_dir = cli
        .session_dir_for_project(&project_path)
        .ok_or("Cannot determine home directory")?;

    if !projects_dir.exists() {
        return Err("Project directory not found".to_string());
    }

    let file_path = if let Some(sid) = session_id {
        projects_dir.join(format!("{}.{}", sid, cli.session_file_extension()))
    } else {
        // Find most recent session file
        let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
        if let Ok(entries) = fs::read_dir(&projects_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if cli.is_session_file(&path) {
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

    let contents = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;

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

/// Get the context window usage for a session. Provider-aware:
///   • Claude — last assistant entry from the per-project .jsonl
///   • Codex  — `threads.tokens_used` from `~/.codex/state_5.sqlite`
///   • OpenCode — latest assistant `message.data.tokens.total` from
///     `~/.local/share/opencode/opencode.db`
/// Returns fill percentage relative to the model's known context window.
#[tauri::command]
pub fn agent_get_session_context_usage(
    project_path: String,
    session_id: String,
    provider: Option<String>,
) -> Result<ContextUsage, String> {
    match provider.as_deref().unwrap_or("claude") {
        "codex" => codex_context_usage(&session_id),
        "gemini" => gemini_context_usage(&session_id),
        "opencode" => opencode_context_usage(&session_id),
        _ => claude_context_usage(&project_path, &session_id),
    }
}

/// Map a model name to its context window in tokens. Used by the
/// per-provider context-usage helpers below — both Codex and OpenCode
/// store the model id but not the window size. We pattern-match on
/// well-known prefixes; unknown models fall back to 200k (a
/// conservative bound that under-states %used rather than overstating
/// it). Refine the table as new models land.
fn model_context_window(model: &str) -> u64 {
    let m = model.to_ascii_lowercase();
    // Claude (used for opencode runs that target anthropic provider)
    if m.contains("opus") { return 1_000_000; }
    if m.contains("sonnet") || m.contains("haiku") { return 200_000; }
    // OpenAI / Codex
    if m.contains("gpt-5.5") { return 384_000; }
    if m.contains("gpt-5") { return 256_000; }
    if m.contains("gpt-4o") || m.contains("gpt-4-turbo") { return 128_000; }
    if m.starts_with("o1") || m.starts_with("o3") { return 200_000; }
    // Gemini family. Gemini 1.5 Pro / 2.x / 3.x all ship with a 1M
    // token context window today; 1.5-pro-experimental briefly offered
    // 2M but isn't a default selectable model. Keep the cap at 1M for
    // the user-facing % calculation — overestimating "% used" is worse
    // than underestimating headroom.
    if m.starts_with("gemini-") || m.contains("gemini") { return 1_000_000; }
    200_000
}

fn claude_context_usage(project_path: &str, session_id: &str) -> Result<ContextUsage, String> {
    let cli: &dyn CliRunner = claude_cli();
    let file_path = cli
        .session_dir_for_project(project_path)
        .ok_or("Cannot determine home directory")?
        .join(format!("{}.{}", session_id, cli.session_file_extension()));

    if !file_path.exists() {
        return Err("Session file not found".to_string());
    }

    // Read from the end for efficiency — find last two assistant entries
    let contents = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = contents.lines().collect();

    let mut last_usage: Option<(u64, u64, u64, String)> = None; // (input, cache_read, cache_create, model)
    let mut prev_total: Option<u64> = None;
    let mut found_last = false;

    // Iterate from the end to find the last two assistant entries
    for line in lines.iter().rev() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let is_assistant = val.get("type").and_then(|t| t.as_str()) == Some("assistant");
            if !is_assistant {
                continue;
            }

            let usage = val.get("message").and_then(|m| m.get("usage"));
            if let Some(u) = usage {
                let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let cache_read = u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let cache_create = u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let total = input + cache_read + cache_create;

                let model = val.get("message")
                    .and_then(|m| m.get("model"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                if !found_last {
                    last_usage = Some((input, cache_read, cache_create, model));
                    found_last = true;
                } else {
                    prev_total = Some(total);
                    break;
                }
            }
        }
    }

    let (input_tokens, cache_read_tokens, cache_creation_tokens, model) =
        last_usage.unwrap_or((0, 0, 0, "unknown".to_string()));

    let total_context_tokens = input_tokens + cache_read_tokens + cache_creation_tokens;
    let context_window: u64 = model_context_window(&model);
    let fill_percent = if context_window > 0 {
        (total_context_tokens as f64 / context_window as f64) * 100.0
    } else {
        0.0
    };

    // Detect compaction: previous total was >50% higher than current
    let compacted = if let Some(prev) = prev_total {
        prev > 0 && total_context_tokens < prev / 2
    } else {
        false
    };

    Ok(ContextUsage {
        input_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        total_context_tokens,
        context_window,
        fill_percent,
        model,
        compacted,
    })
}

/// Codex context usage — walk the session's rollout JSONL for the
/// last `token_count` event and read its `last_token_usage` block.
/// Replaces the old `~/.codex/state_5.sqlite` reader, which only had
/// a single rolling counter (no input/output/cached split). The
/// rollout file is the same data source Codeburn uses, with richer
/// per-turn detail.
///
/// Cross-OS: path joins go through `dirs::home_dir()` +
/// `PathBuf::join`, so Windows `%USERPROFILE%\.codex\sessions\…`
/// resolves the same way as `$HOME/.codex/sessions/…` elsewhere.
fn codex_context_usage(session_id: &str) -> Result<ContextUsage, String> {
    let path = match codex_find_session_file(session_id) {
        Some(p) => p,
        None => return Err("Codex session file not found".to_string()),
    };
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;

    let mut model = String::from("unknown");
    let mut context_window: u64 = 0;
    let mut last_total: u64 = 0;
    let mut last_input: u64 = 0;
    let mut last_cached: u64 = 0;

    for line in content.lines() {
        if line.trim().is_empty() { continue; }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue; };
        let payload = match val.get("payload") { Some(p) => p, None => continue };
        // Latest model selection wins — turn_context fires whenever
        // the user (or the agent) changes models mid-session.
        if val.get("type").and_then(|t| t.as_str()) == Some("turn_context") {
            if let Some(m) = payload.get("model").and_then(|v| v.as_str()) {
                model = m.to_string();
            }
            continue;
        }
        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") { continue; }
        let info = match payload.get("info") {
            Some(v) if !v.is_null() => v,
            _ => continue, // early events report info=null
        };
        // Prefer per-turn block; fall back to cumulative if Codex
        // didn't emit a per-turn delta this turn.
        let usage = info
            .get("last_token_usage")
            .or_else(|| info.get("total_token_usage"));
        let Some(u) = usage else { continue };
        let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cached = u.get("cached_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let total = u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        last_input = input;
        last_cached = cached;
        last_total = total;
        if let Some(w) = info.get("model_context_window").and_then(|v| v.as_u64()) {
            context_window = w;
        }
    }

    if context_window == 0 {
        context_window = model_context_window(&model);
    }
    let fill_percent = if context_window > 0 {
        (last_total as f64 / context_window as f64) * 100.0
    } else { 0.0 };

    Ok(ContextUsage {
        // Cached is reported inside input by OpenAI — subtract before
        // surfacing as the "true new input" line so the breakdown is
        // honest (matches the Claude semantics callers expect).
        input_tokens: last_input.saturating_sub(last_cached),
        cache_read_tokens: last_cached,
        cache_creation_tokens: 0,
        total_context_tokens: last_total,
        context_window,
        fill_percent,
        model,
        compacted: false,
    })
}

/// Walk `<codex_home>/sessions/<Y>/<M>/<D>/rollout-*.jsonl` looking
/// for the session whose `session_meta.payload.id` matches.
/// Returns the first match — codex never reuses session ids so the
/// first hit is the only hit.
///
/// Cross-OS: relies on `fs::read_dir` + `PathBuf` only; no shell
/// globbing. Honors `$CODEX_HOME` via the runner's `dot_codex`.
fn codex_find_session_file(session_id: &str) -> Option<std::path::PathBuf> {
    let runner = crate::shared::cli::registry::runner_for("codex");
    let root = runner.sessions_root()?;
    if !root.exists() { return None; }

    fn walk(dir: &std::path::Path, session_id: &str) -> Option<std::path::PathBuf> {
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let ft = entry.file_type().ok()?;
            if ft.is_dir() {
                if let Some(found) = walk(&path, session_id) {
                    return Some(found);
                }
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") { continue; }
            // Filename includes the UUID (`rollout-<ts>-<uuid>.jsonl`).
            // Cheap pre-check before opening the file.
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !fname.contains(session_id) {
                // Fallback: confirm by header (filename UUID may be a
                // different generation token; the canonical id is in
                // the session_meta payload).
                if let Ok(first) = read_capped_first_line(&path, 1_048_576) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&first) {
                        if val.get("payload").and_then(|p| p.get("id"))
                            .and_then(|v| v.as_str()) == Some(session_id)
                        {
                            return Some(path);
                        }
                    }
                }
                continue;
            }
            return Some(path);
        }
        None
    }

    walk(&root, session_id)
}

/// Read the first line of a file with a hard byte cap so a corrupt
/// rollout (no newline, huge embedded system prompt) can't pull the
/// whole file into memory. Codex 0.128+ embeds the full system prompt
/// in `session_meta`, which legitimately runs 20–27 KB; the 1 MB cap
/// leaves comfortable headroom.
fn read_capped_first_line(
    path: &std::path::Path,
    cap_bytes: usize,
) -> std::io::Result<String> {
    use std::io::{BufRead, BufReader, Read};
    let f = std::fs::File::open(path)?;
    let mut reader = BufReader::new(f).take(cap_bytes as u64);
    // Try the cheap path first.
    let mut line = String::new();
    let mut handle = reader.by_ref();
    let mut buf_reader = BufReader::new(&mut handle);
    let _ = buf_reader.read_line(&mut line)?;
    if line.is_empty() {
        // No newline within the cap — read whatever was in the
        // window and let JSON parsing fail loudly on truncated text.
        let mut buf = Vec::with_capacity(cap_bytes);
        reader.read_to_end(&mut buf)?;
        return Ok(String::from_utf8_lossy(&buf).to_string());
    }
    Ok(line)
}

/// OpenCode context usage — aggregate the latest assistant message's
/// `tokens` block from whichever `opencode*.db` channel holds the
/// session. `message.data` is a JSON blob containing `tokens.{total,
/// input,output,cache:{read,write}}` and `modelID`/`providerID`. The
/// latest assistant message's `tokens.total` is what the agent itself
/// considers the running context size for that session.
fn opencode_context_usage(session_id: &str) -> Result<ContextUsage, String> {
    let dbs = opencode_db_paths();
    if dbs.is_empty() {
        return Err("OpenCode DB not found".to_string());
    }

    let handle = tokio::runtime::Handle::try_current().ok();
    for db_path in &dbs {
        let result = match &handle {
            Some(h) => h.block_on(query_opencode_message(db_path, session_id)),
            None => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("opencode context runtime: {}", e))?;
                rt.block_on(query_opencode_message(db_path, session_id))
            }
        };
        if let Ok(ctx) = result {
            return Ok(ctx);
        }
    }
    Err("No assistant messages found in any OpenCode DB".to_string())
}

async fn query_opencode_message(
    db_path: &std::path::Path,
    session_id: &str,
) -> Result<ContextUsage, String> {
    let opts = opencode_connect_opts(db_path);
    let pool = sqlx::SqlitePool::connect_with(opts)
        .await
        .map_err(|e| format!("opencode db open: {}", e))?;
    // Latest assistant message wins — that's what the agent itself
    // measures context against. `data` is a JSON text column; we
    // pull it raw and parse client-side so mixed int/float fields
    // inside the JSON deserialize without sqlite-affinity surprises.
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT data FROM message \
         WHERE session_id = ? \
         AND json_extract(data, '$.role') = 'assistant' \
         ORDER BY time_created DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| format!("opencode message query: {}", e))?;
    pool.close().await;

    let blob = row.ok_or_else(|| "No assistant messages yet".to_string())?.0;
    let parsed: serde_json::Value = serde_json::from_str(&blob)
        .map_err(|e| format!("opencode message JSON: {}", e))?;

    let tokens = parsed.get("tokens");
    let input = tokens
        .and_then(|t| t.get("input"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = tokens
        .and_then(|t| t.get("output"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_read = tokens
        .and_then(|t| t.get("cache"))
        .and_then(|c| c.get("read"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_write = tokens
        .and_then(|t| t.get("cache"))
        .and_then(|c| c.get("write"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    // Prefer the explicit `total` if present; otherwise sum the parts.
    let total_context_tokens = tokens
        .and_then(|t| t.get("total"))
        .and_then(|v| v.as_u64())
        .unwrap_or(input + output + cache_read + cache_write);

    let model = parsed
        .get("modelID")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| "unknown".to_string());
    let context_window = model_context_window(&model);
    let fill_percent = if context_window > 0 {
        (total_context_tokens as f64 / context_window as f64) * 100.0
    } else {
        0.0
    };
    Ok(ContextUsage {
        input_tokens: input,
        cache_read_tokens: cache_read,
        cache_creation_tokens: cache_write,
        total_context_tokens,
        context_window,
        fill_percent,
        model,
        compacted: false,
    })
}

// ─── Dashboard analytics (Codex + OpenCode) ─────────────────────────
//
// Both providers expose their session/turn data in SQLite, which means
// daily / by-model / by-project rollups are simple GROUP BY queries —
// no JSONL parsing, no full-table scans the planner can't optimise.
// We do read-only pool connections so an actively-running codex /
// opencode process can't be locked out.
//
// Costs:
//   • Codex stores a single `tokens_used` rolling counter per thread.
//     We can't split input/output/cache, so those fields stay zero;
//     totals reflect the rolling counter. Cost requires OpenAI rate
//     cards keyed by model — wired in `codex_price_for_model` below
//     with public list prices (kept conservative; update as needed).
//   • OpenCode embeds a full `tokens.{input,output,cache.{read,write}}`
//     block in every assistant message PLUS an explicit `cost` field
//     the agent computed at message time. We sum those directly.

const ANALYTICS_DEFAULT_DAYS: u32 = 30;
const ANALYTICS_TOP_SESSIONS: i64 = 10;

/// Public list pricing for OpenAI's coding models (USD per 1M tokens).
/// Returned as (input, output, cached_read). OpenAI counts cached
/// tokens INSIDE `input_tokens` on the wire — we subtract them out
/// before applying the input rate and price cached separately at the
/// cache-read tier (same approach codeburn uses). Refresh against
/// `https://openai.com/api/pricing` as new models drop.
fn codex_price_for_model(model: &str) -> (f64, f64, f64) {
    let m = model.to_ascii_lowercase();
    if m.contains("gpt-5.5") { (1.25, 10.0, 0.125) }
    else if m.contains("gpt-5") { (1.25, 10.0, 0.125) }
    else if m.contains("gpt-4o") { (2.5, 10.0, 1.25) }
    else if m.starts_with("o1") { (15.0, 60.0, 7.5) }
    else if m.starts_with("o3") { (10.0, 40.0, 2.5) }
    else { (2.5, 10.0, 1.25) }
}

/// Codex analytics — walk every `rollout-*.jsonl` under
/// `<codex_home>/sessions/<Y>/<M>/<D>/`. Replaces the old
/// `state_5.sqlite` reader (which only exposed a rolling counter,
/// not the input/output/cached split). The on-disk events carry:
///   • `event_msg.payload.token_count` with both `last_token_usage`
///     (per-turn) and `total_token_usage` (cumulative).
///   • `response_item.function_call` for tool tracking. The shell
///     tool surfaces as `exec_command` with `arguments` JSON
///     embedding the `cmd` field.
///   • `turn_context.payload.model` for the current session model.
///
/// Cross-OS: discovery is `fs::read_dir` against `dirs::home_dir()`
/// (or `$CODEX_HOME`); no shell-glob. JSONL parsing is byte-stream
/// only, so Windows CRLF is fine.
fn codex_usage_analytics(days: Option<u32>) -> Result<UsageAnalytics, String> {
    let runner = crate::shared::cli::registry::runner_for("codex");
    let sessions_root = match runner.sessions_root() {
        Some(r) if r.exists() => r,
        _ => return Ok(empty_analytics()),
    };
    let days_limit = days.unwrap_or(ANALYTICS_DEFAULT_DAYS) as i64;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days_limit);

    // First, collect every rollout-*.jsonl file's path. We filter by
    // file mtime against the cutoff so old sessions don't bloat the
    // walk. Year/month/day folder structure is regular, but we don't
    // rely on parsing the segments — just descend.
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() { collect(&path, out); continue; }
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") { continue; }
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !fname.starts_with("rollout-") { continue; }
            out.push(path);
        }
    }
    collect(&sessions_root, &mut files);

    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut total_cache_read: u64 = 0;
    let mut total_cost: f64 = 0.0;
    let mut total_calls: u32 = 0;
    let mut by_day: std::collections::BTreeMap<String, DailyUsage> = Default::default();
    let mut by_model: std::collections::HashMap<String, ModelUsage> = Default::default();
    let mut by_project: std::collections::HashMap<String, ProjectUsage> = Default::default();
    let mut per_session: std::collections::HashMap<String, SessionCost> = Default::default();
    let mut project_sessions: std::collections::HashMap<String, std::collections::HashSet<String>> = Default::default();
    let mut tool_counts: std::collections::HashMap<String, u32> = Default::default();
    let mut shell_counts: std::collections::HashMap<String, u32> = Default::default();

    for path in files {
        // mtime filter — cheap rejection without parsing the file.
        if let Ok(meta) = path.metadata() {
            if let Ok(modified) = meta.modified() {
                let dt: chrono::DateTime<chrono::Utc> = modified.into();
                if dt < cutoff { continue; }
            }
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut lines = content.lines();

        // First line = session_meta. Validate originator before
        // committing to parse — guards against stray jsonl droppings.
        let first = match lines.next() {
            Some(l) => l,
            None => continue,
        };
        let meta: serde_json::Value = match serde_json::from_str(first) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if meta.get("type").and_then(|t| t.as_str()) != Some("session_meta") { continue; }
        let payload = match meta.get("payload") { Some(p) => p, None => continue };
        let originator = payload
            .get("originator")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !originator.to_ascii_lowercase().starts_with("codex") { continue; }

        let session_id = payload
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if session_id.is_empty() { continue; }
        let project = payload
            .get("cwd")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let mut model = String::from("unknown");
        let mut prev_total_input: Option<u64> = None;
        let mut prev_total_output: Option<u64> = None;
        let mut prev_total_cached: Option<u64> = None;
        let mut session_cost: f64 = 0.0;
        let mut session_calls: u32 = 0;

        for line in lines {
            if line.trim().is_empty() { continue; }
            let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let t = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let payload = match val.get("payload") { Some(p) => p, None => continue };

            // Track model — latest turn_context wins.
            if t == "turn_context" {
                if let Some(m) = payload.get("model").and_then(|v| v.as_str()) {
                    model = m.to_string();
                }
                continue;
            }

            // Tool tracking — response_item.function_call events.
            if t == "response_item"
                && payload.get("type").and_then(|v| v.as_str()) == Some("function_call")
            {
                let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !name.is_empty() {
                    *tool_counts.entry(name.clone()).or_insert(0) += 1;
                    // Codex's shell tool is `exec_command`; `arguments`
                    // is a JSON-encoded string with a `cmd` field.
                    if name == "exec_command" || name == "shell" {
                        if let Some(args_str) = payload.get("arguments").and_then(|v| v.as_str()) {
                            if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str) {
                                if let Some(cmd) = args.get("cmd").and_then(|v| v.as_str()) {
                                    if let Some(head) = cmd.split_whitespace().next() {
                                        *shell_counts.entry(head.to_string()).or_insert(0) += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                continue;
            }

            // Token accounting — event_msg.token_count.
            if t != "event_msg" { continue; }
            if payload.get("type").and_then(|v| v.as_str()) != Some("token_count") { continue; }
            let info = match payload.get("info") {
                Some(v) if !v.is_null() => v,
                _ => continue,
            };

            // Prefer `last_token_usage` (per-turn delta supplied by
            // Codex itself). When missing, derive a delta from the
            // cumulative counter — initialize `prev_*` to None so a
            // session whose first event reports total=0 isn't dropped
            // as a "duplicate" of an uninitialized state.
            let (input_raw, cached, output_raw) =
                if let Some(last) = info.get("last_token_usage").filter(|v| !v.is_null()) {
                    let i = last.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let c = last.get("cached_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let o = last.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
                          + last.get("reasoning_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    (i, c, o)
                } else if let Some(total) = info.get("total_token_usage").filter(|v| !v.is_null()) {
                    let ti = total.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let tc = total.get("cached_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let to = total.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
                           + total.get("reasoning_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let di = ti.saturating_sub(prev_total_input.unwrap_or(0));
                    let dc = tc.saturating_sub(prev_total_cached.unwrap_or(0));
                    let do_ = to.saturating_sub(prev_total_output.unwrap_or(0));
                    // Update prev counters on every event, including
                    // ones where we used last_token_usage above — they
                    // must stay in lockstep with reality or a mixed
                    // session would double-count when we hit a turn
                    // that lacks last_token_usage.
                    prev_total_input = Some(ti);
                    prev_total_cached = Some(tc);
                    prev_total_output = Some(to);
                    (di, dc, do_)
                } else {
                    continue;
                };

            // Subtract cached from input — OpenAI semantics.
            let input = input_raw.saturating_sub(cached);
            let output = output_raw;

            // Always advance prev_* once per token_count event.
            if let Some(total) = info.get("total_token_usage").filter(|v| !v.is_null()) {
                if prev_total_input.is_none() {
                    prev_total_input = total.get("input_tokens").and_then(|v| v.as_u64());
                    prev_total_cached = total.get("cached_input_tokens").and_then(|v| v.as_u64());
                    prev_total_output = total.get("output_tokens").and_then(|v| v.as_u64()).map(|o|
                        o + total.get("reasoning_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
                    );
                }
            }

            let (in_p, out_p, cache_p) = codex_price_for_model(&model);
            let cost = (input as f64 / 1_000_000.0) * in_p
                     + (output as f64 / 1_000_000.0) * out_p
                     + (cached as f64 / 1_000_000.0) * cache_p;

            let timestamp = val.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
            let day = timestamp.get(..10).unwrap_or("unknown").to_string();

            total_input += input;
            total_output += output;
            total_cache_read += cached;
            total_cost += cost;
            total_calls += 1;
            session_calls += 1;
            session_cost += cost;

            let d = by_day.entry(day.clone()).or_insert_with(|| DailyUsage {
                date: day.clone(), cost: 0.0, calls: 0, input_tokens: 0, output_tokens: 0,
            });
            d.cost += cost; d.calls += 1; d.input_tokens += input; d.output_tokens += output;

            let m = by_model.entry(model.clone()).or_insert_with(|| ModelUsage {
                model: model.clone(), cost: 0.0, calls: 0,
                input_tokens: 0, output_tokens: 0, cache_hit_percent: 0.0,
            });
            m.cost += cost; m.calls += 1;
            m.input_tokens += input; m.output_tokens += output;

            let p = by_project.entry(project.clone()).or_insert_with(|| ProjectUsage {
                project: project.clone(), cost: 0.0, sessions: 0, calls: 0,
            });
            p.cost += cost; p.calls += 1;
        }

        if session_calls > 0 {
            project_sessions
                .entry(project.clone())
                .or_default()
                .insert(session_id.clone());
            per_session.entry(session_id.clone()).or_insert(SessionCost {
                session_id, project: project.clone(),
                cost: session_cost, calls: session_calls, model,
            });
        }
    }

    for (proj, ids) in &project_sessions {
        if let Some(p) = by_project.get_mut(proj) {
            p.sessions = ids.len() as u32;
        }
    }
    let cache_hit_percent = if total_input + total_cache_read > 0 {
        (total_cache_read as f64 / (total_input + total_cache_read) as f64) * 100.0
    } else { 0.0 };

    let mut daily: Vec<DailyUsage> = by_day.into_values().collect();
    daily.sort_by(|a, b| a.date.cmp(&b.date));
    let mut by_model_vec: Vec<ModelUsage> = by_model.into_values().collect();
    by_model_vec.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_project_vec: Vec<ProjectUsage> = by_project.into_values().collect();
    by_project_vec.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let mut top: Vec<SessionCost> = per_session.into_values().collect();
    top.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let total_sessions = top.len() as u32;
    top.truncate(ANALYTICS_TOP_SESSIONS as usize);

    let mut tools: Vec<ToolCount> = tool_counts
        .into_iter()
        .map(|(name, count)| ToolCount { name, count })
        .collect();
    tools.sort_by(|a, b| b.count.cmp(&a.count));
    let mut shell_commands: Vec<ToolCount> = shell_counts
        .into_iter()
        .map(|(name, count)| ToolCount { name, count })
        .collect();
    shell_commands.sort_by(|a, b| b.count.cmp(&a.count));

    Ok(UsageAnalytics {
        total_cost,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_cache_read_tokens: total_cache_read,
        total_cache_write_tokens: 0,
        total_sessions,
        total_api_calls: total_calls,
        cache_hit_percent,
        daily,
        by_model: by_model_vec,
        by_project: by_project_vec,
        top_sessions: top,
        tools,
        shell_commands,
    })
}

async fn opencode_usage_analytics(days: Option<u32>) -> Result<UsageAnalytics, String> {
    let dbs = opencode_db_paths();
    if dbs.is_empty() {
        return Ok(empty_analytics());
    }

    let days_limit = days.unwrap_or(ANALYTICS_DEFAULT_DAYS) as i64;
    // OpenCode stores time as epoch millis.
    let cutoff_ms = (chrono::Utc::now() - chrono::Duration::days(days_limit)).timestamp_millis();

    // Accumulate rows from every channel DB. Per-DB failures (missing
    // table from a schema mismatch, locked WAL, etc.) get logged and
    // skipped rather than failing the whole report — partial data
    // beats no data when the user has multiple channels installed.
    let mut rows: Vec<(
        String,           // session_id
        i64,              // time_created (ms)
        Option<String>,   // session.directory
        Option<String>,   // model id
        Option<f64>,      // cost
        Option<i64>,      // input tokens
        Option<i64>,      // output tokens
        Option<i64>,      // cache_read tokens
        Option<i64>,      // cache_write tokens
    )> = Vec::new();

    for db_path in &dbs {
        let opts = opencode_connect_opts(db_path);
        let pool = match sqlx::SqlitePool::connect_with(opts).await {
            Ok(p) => p,
            Err(_) => continue,
        };
        let batch: Result<Vec<(
            String, i64, Option<String>, Option<String>,
            Option<f64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>,
        )>, _> = sqlx::query_as(
        // SQLite's json_extract returns the raw JSON type, so `{"cost":0}`
        // comes back as INTEGER, not REAL. sqlx then refuses to decode
        // INTEGER → f64 and the whole call fails (frontend showed "no
        // data found" instead of the real rows). CAST forces the right
        // affinity for every numeric column so mixed int/float JSON
        // values deserialize cleanly.
        "SELECT m.session_id, \
                m.time_created, \
                s.directory, \
                CAST(json_extract(m.data, '$.modelID') AS TEXT), \
                CAST(json_extract(m.data, '$.cost') AS REAL), \
                CAST(json_extract(m.data, '$.tokens.input') AS INTEGER), \
                CAST(json_extract(m.data, '$.tokens.output') AS INTEGER), \
                CAST(json_extract(m.data, '$.tokens.cache.read') AS INTEGER), \
                CAST(json_extract(m.data, '$.tokens.cache.write') AS INTEGER) \
         FROM message m JOIN session s ON s.id = m.session_id \
         WHERE json_extract(m.data, '$.role') = 'assistant' \
           AND m.time_created >= ?",
        )
        .bind(cutoff_ms)
        .fetch_all(&pool)
        .await;
        pool.close().await;
        if let Ok(batch_rows) = batch {
            rows.extend(batch_rows);
        }
    }

    let mut total_cost: f64 = 0.0;
    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut total_cache_read: u64 = 0;
    let mut total_cache_write: u64 = 0;
    let mut by_day: std::collections::BTreeMap<String, DailyUsage> = Default::default();
    let mut by_model: std::collections::HashMap<String, ModelUsage> = Default::default();
    let mut by_project: std::collections::HashMap<String, ProjectUsage> = Default::default();
    let mut per_session: std::collections::HashMap<String, SessionCost> = Default::default();
    let mut project_sessions: std::collections::HashMap<String, std::collections::HashSet<String>> = Default::default();

    for (session_id, ts_ms, directory, model_opt, cost_opt, input_opt, output_opt, cache_r_opt, cache_w_opt) in rows {
        let model = model_opt.unwrap_or_else(|| "unknown".to_string());
        let project = directory.unwrap_or_else(|| "unknown".to_string());
        let cost = cost_opt.unwrap_or(0.0);
        let input = input_opt.unwrap_or(0).max(0) as u64;
        let output = output_opt.unwrap_or(0).max(0) as u64;
        let cache_r = cache_r_opt.unwrap_or(0).max(0) as u64;
        let cache_w = cache_w_opt.unwrap_or(0).max(0) as u64;

        total_cost += cost;
        total_input += input;
        total_output += output;
        total_cache_read += cache_r;
        total_cache_write += cache_w;

        let day = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let d = by_day.entry(day.clone()).or_insert_with(|| DailyUsage {
            date: day.clone(), cost: 0.0, calls: 0, input_tokens: 0, output_tokens: 0,
        });
        d.cost += cost; d.calls += 1; d.input_tokens += input; d.output_tokens += output;

        let m = by_model.entry(model.clone()).or_insert_with(|| ModelUsage {
            model: model.clone(), cost: 0.0, calls: 0,
            input_tokens: 0, output_tokens: 0, cache_hit_percent: 0.0,
        });
        m.cost += cost; m.calls += 1;
        m.input_tokens += input; m.output_tokens += output;

        let p = by_project.entry(project.clone()).or_insert_with(|| ProjectUsage {
            project: project.clone(), cost: 0.0, sessions: 0, calls: 0,
        });
        p.cost += cost; p.calls += 1;
        project_sessions
            .entry(project.clone())
            .or_default()
            .insert(session_id.clone());

        let sc = per_session.entry(session_id.clone()).or_insert_with(|| SessionCost {
            session_id: session_id.clone(),
            project: project.clone(),
            cost: 0.0, calls: 0,
            model: model.clone(),
        });
        sc.cost += cost; sc.calls += 1;
    }

    // Fill in distinct-session counts on by_project.
    for (proj, ids) in &project_sessions {
        if let Some(p) = by_project.get_mut(proj) { p.sessions = ids.len() as u32; }
    }

    // Tool + shell tracking — second-pass query against `part` so the
    // dashboard's Tools panel and Shell panel surface OpenCode usage.
    // We sum across every channel DB. Schema: `part.data` is JSON with
    // `type` and `tool` fields; shell calls live under `tool == 'bash'`
    // with the command line in `state.input.command`. Tool name
    // normalization for MCP servers (codeburn's `<server>_<tool>` →
    // `mcp__<server>__<tool>`) needs a server-name list which OpenCode
    // doesn't expose on disk; deferred — names land verbatim for now.
    let mut tool_counts: std::collections::HashMap<String, u32> = Default::default();
    let mut shell_counts: std::collections::HashMap<String, u32> = Default::default();
    for db_path in &dbs {
        let opts = opencode_connect_opts(db_path);
        let pool = match sqlx::SqlitePool::connect_with(opts).await {
            Ok(p) => p,
            Err(_) => continue,
        };
        let parts: Result<Vec<(Option<String>, Option<String>)>, _> = sqlx::query_as(
            "SELECT CAST(json_extract(p.data, '$.tool') AS TEXT), \
                    CAST(json_extract(p.data, '$.state.input.command') AS TEXT) \
             FROM part p \
             WHERE json_extract(p.data, '$.type') = 'tool' \
               AND p.time_created >= ?",
        )
        .bind(cutoff_ms)
        .fetch_all(&pool)
        .await;
        pool.close().await;
        if let Ok(rows) = parts {
            for (tool_opt, cmd_opt) in rows {
                let Some(tool) = tool_opt else { continue };
                if tool.is_empty() { continue; }
                *tool_counts.entry(tool.clone()).or_insert(0) += 1;
                if tool == "bash" {
                    if let Some(cmd) = cmd_opt {
                        if let Some(head) = cmd.split_whitespace().next() {
                            *shell_counts.entry(head.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }
    let mut tools: Vec<ToolCount> = tool_counts
        .into_iter()
        .map(|(name, count)| ToolCount { name, count })
        .collect();
    tools.sort_by(|a, b| b.count.cmp(&a.count));
    let mut shell_commands: Vec<ToolCount> = shell_counts
        .into_iter()
        .map(|(name, count)| ToolCount { name, count })
        .collect();
    shell_commands.sort_by(|a, b| b.count.cmp(&a.count));

    // Cache-hit % across all turns.
    let cache_hit_percent = if total_input + total_cache_read > 0 {
        (total_cache_read as f64 / (total_input + total_cache_read) as f64) * 100.0
    } else { 0.0 };

    let mut daily: Vec<DailyUsage> = by_day.into_values().collect();
    daily.sort_by(|a, b| a.date.cmp(&b.date));
    let mut by_model_vec: Vec<ModelUsage> = by_model.into_values().collect();
    // OpenCode per-model cache hit: aggregate from per-model accumulated
    // input + cache_read by re-running once. Simpler: leave at 0 for v1
    // (the top-level cache_hit_percent is the headline metric).
    by_model_vec.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_project_vec: Vec<ProjectUsage> = by_project.into_values().collect();
    by_project_vec.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let mut top: Vec<SessionCost> = per_session.into_values().collect();
    top.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let total_sessions = top.len() as u32;
    let total_api_calls: u32 = daily.iter().map(|d| d.calls).sum();
    top.truncate(ANALYTICS_TOP_SESSIONS as usize);

    Ok(UsageAnalytics {
        total_cost,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_cache_read_tokens: total_cache_read,
        total_cache_write_tokens: total_cache_write,
        total_sessions,
        total_api_calls,
        cache_hit_percent,
        daily,
        by_model: by_model_vec,
        by_project: by_project_vec,
        top_sessions: top,
        tools,
        shell_commands,
    })
}

fn empty_analytics() -> UsageAnalytics {
    UsageAnalytics {
        total_cost: 0.0, total_input_tokens: 0, total_output_tokens: 0,
        total_cache_read_tokens: 0, total_cache_write_tokens: 0,
        total_sessions: 0, total_api_calls: 0, cache_hit_percent: 0.0,
        daily: vec![], by_model: vec![], by_project: vec![],
        top_sessions: vec![], tools: vec![], shell_commands: vec![],
    }
}

// ─── Gemini usage analytics + per-session context usage ────────────
//
// Gemini's per-project session log is JSONL at
// `~/.gemini/tmp/<slug>/chats/session-*.jsonl`. The project→slug map
// in `~/.gemini/projects.json` lets us recover the absolute path for
// the by-project breakdown. Each `type:"gemini"` event embeds:
//
//     "tokens": { "input": …, "output": …, "cached": …,
//                 "thoughts": …, "tool": …, "total": … },
//     "model":  "gemini-3-flash-preview" (etc.)
//
// Pricing semantics we follow (matches the codeburn Gemini provider):
//   - `cached` is a SUBSET of `input` already (Google reports
//     prompt_token_count inclusive of cache). Subtract cached from
//     input before applying the input rate so cached tokens are
//     charged only at the cache-read rate.
//   - `thoughts` are billed at the OUTPUT rate.
//   - `tool` tokens aren't a Gemini billable line item; ignore for
//     cost but keep them in the raw input total for completeness.
//
// Format note: Gemini CLI versions ≤0.40 wrote a single big JSON doc
// per session; ≥0.41 writes JSONL. We sniff the first non-whitespace
// character to decide. JSON path collapses to a single events array;
// JSONL path streams line-by-line.

/// Public list pricing for Gemini coding models (USD per 1M tokens).
/// Returned as (input, output, cached_read). Sources: ai.google.dev
/// /pricing as of 2026-05. Refresh as new models drop. Unknown models
/// fall back to flash-tier rates — under-states for pro/ultra usage,
/// which we accept as "best-effort estimate".
fn gemini_price_for_model(model: &str) -> (f64, f64, f64) {
    let m = model.to_ascii_lowercase();
    // Gemini 3 family (Dec 2025 / Mar 2026). Cached read = 10% of input (Google docs).
    if m.contains("gemini-3.1-pro") { (2.00, 12.00, 0.20) }
    else if m.contains("gemini-3.1-flash-lite") { (0.25, 1.50, 0.025) }
    else if m.contains("gemini-3-flash") { (0.50, 3.00, 0.05) }
    else if m.contains("gemini-3") { (0.50, 3.00, 0.05) }
    // Gemini 2.5 family. 2.5 Pro output corrected to $10 (was $5 — stale).
    else if m.contains("gemini-2.5-pro") { (1.25, 10.00, 0.20) }
    else if m.contains("gemini-2.5-flash-lite") { (0.10, 0.40, 0.01) }
    else if m.contains("gemini-2.5-flash") { (0.30, 2.50, 0.03) }
    else { (0.30, 2.50, 0.03) }
}

/// Iterate a Gemini session file regardless of JSON vs JSONL shape.
/// Calls `on_header` once with the session header (sessionId,
/// startTime, ...) and `on_event` for every subsequent event. The
/// caller decides whether to filter on event type.
fn walk_gemini_session<F1, F2>(
    path: &std::path::Path,
    mut on_header: F1,
    mut on_event: F2,
) -> Result<(), String>
where
    F1: FnMut(&serde_json::Value),
    F2: FnMut(&serde_json::Value),
{
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let trimmed = content.trim_start();
    if trimmed.is_empty() {
        return Ok(());
    }
    // Sniff: `[` → JSON array of events; `{` followed by another `{`
    // on a new line → JSONL. The single-document case wraps everything
    // under a top-level object (sessionId at root, then a `messages`
    // or similar key — varies by CLI version).
    if trimmed.starts_with('[') {
        // Bare array of events — older Gemini-style export. First
        // entry typically carries the session header.
        let arr: Vec<serde_json::Value> = serde_json::from_str(trimmed)
            .map_err(|e| format!("gemini session JSON parse: {}", e))?;
        let mut iter = arr.into_iter();
        if let Some(first) = iter.next() { on_header(&first); }
        for ev in iter { on_event(&ev); }
    } else {
        // JSONL: one JSON object per line. First non-empty line is
        // the header; subsequent lines are events.
        let mut header_seen = false;
        for line in trimmed.lines() {
            if line.trim().is_empty() { continue; }
            let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue; };
            if !header_seen {
                on_header(&val);
                header_seen = true;
            } else {
                on_event(&val);
            }
        }
    }
    Ok(())
}

/// Inverse of `GeminiRunner::slug_for_project`: given a slug, look up
/// the real project path from `~/.gemini/projects.json` so the
/// analytics by-project breakdown shows a meaningful label instead of
/// a 1-word slug. Returns `None` when the slug isn't in the map (rare
/// — only happens if the user deleted projects.json but kept tmp).
fn gemini_project_path_for_slug(slug: &str) -> Option<String> {
    let path = dirs::home_dir()?.join(".gemini").join("projects.json");
    let text = fs::read_to_string(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
    let map = parsed.get("projects")?.as_object()?;
    for (project_path, mapped_slug) in map {
        if mapped_slug.as_str() == Some(slug) {
            return Some(project_path.clone());
        }
    }
    None
}

fn gemini_usage_analytics(days: Option<u32>) -> Result<UsageAnalytics, String> {
    let tmp_root = match dirs::home_dir().map(|h| h.join(".gemini").join("tmp")) {
        Some(p) if p.exists() => p,
        _ => return Ok(empty_analytics()),
    };

    let days_limit = days.unwrap_or(ANALYTICS_DEFAULT_DAYS) as i64;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days_limit);

    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut total_cache_read: u64 = 0;
    let mut total_cost: f64 = 0.0;
    let mut total_calls: u32 = 0;
    let mut by_day: std::collections::BTreeMap<String, DailyUsage> = Default::default();
    let mut by_model: std::collections::HashMap<String, ModelUsage> = Default::default();
    let mut by_project: std::collections::HashMap<String, ProjectUsage> = Default::default();
    let mut per_session: std::collections::HashMap<String, SessionCost> = Default::default();
    let mut project_sessions: std::collections::HashMap<String, std::collections::HashSet<String>> = Default::default();
    let mut tool_counts: std::collections::HashMap<String, u32> = Default::default();
    let mut shell_counts: std::collections::HashMap<String, u32> = Default::default();

    // tmp/<slug>/chats/session-*.jsonl — walk one slug at a time so we
    // can attach each session to its absolute project path via
    // projects.json. Sessions without a matching slug fall back to the
    // slug name (better than "unknown").
    for slug_entry in fs::read_dir(&tmp_root).map_err(|e| e.to_string())?.flatten() {
        let slug_dir = slug_entry.path();
        if !slug_dir.is_dir() { continue; }
        let slug = slug_dir.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let chats_dir = slug_dir.join("chats");
        if !chats_dir.exists() { continue; }
        let project_path = gemini_project_path_for_slug(&slug).unwrap_or_else(|| slug.clone());

        for entry in fs::read_dir(&chats_dir).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "jsonl" && ext != "json" { continue; }

            // Skip files older than the cutoff using mtime — Gemini's
            // events have per-event timestamps but the file mtime is
            // cheaper and ~always matches the last-touched event.
            if let Ok(meta) = path.metadata() {
                if let Ok(modified) = meta.modified() {
                    let dt: chrono::DateTime<chrono::Utc> = modified.into();
                    if dt < cutoff { continue; }
                }
            }

            let mut session_id = String::new();
            let mut session_calls: u32 = 0;
            let mut session_cost: f64 = 0.0;
            let mut session_model = String::from("unknown");
            let _ = walk_gemini_session(
                &path,
                |hdr| {
                    if let Some(s) = hdr.get("sessionId").and_then(|v| v.as_str()) {
                        session_id = s.to_string();
                    }
                },
                |ev| {
                    if ev.get("type").and_then(|t| t.as_str()) != Some("gemini") {
                        // Tool calls live under `toolCalls` on gemini
                        // events, but also (less commonly) as their
                        // own event type — count both shapes when seen.
                        if let Some(arr) = ev.get("toolCalls").and_then(|v| v.as_array()) {
                            for tc in arr {
                                if let Some(name) = tc.get("name").and_then(|v| v.as_str()) {
                                    *tool_counts.entry(name.to_string()).or_insert(0) += 1;
                                }
                            }
                        }
                        return;
                    }
                    let tokens = match ev.get("tokens") { Some(t) => t, None => return };
                    // Cached is INSIDE input per Google's accounting —
                    // subtract before applying the input rate so cached
                    // tokens aren't double-charged.
                    let input_raw = tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cached = tokens.get("cached").and_then(|v| v.as_u64()).unwrap_or(0);
                    let output = tokens.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
                    let thoughts = tokens.get("thoughts").and_then(|v| v.as_u64()).unwrap_or(0);
                    let input = input_raw.saturating_sub(cached);

                    let model = ev.get("model").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    let (in_p, out_p, cache_p) = gemini_price_for_model(&model);
                    // Thoughts billed at output rate.
                    let cost = (input as f64 / 1_000_000.0) * in_p
                             + ((output + thoughts) as f64 / 1_000_000.0) * out_p
                             + (cached as f64 / 1_000_000.0) * cache_p;

                    let timestamp = ev.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
                    let day = if !timestamp.is_empty() {
                        timestamp.get(..10).unwrap_or("unknown").to_string()
                    } else {
                        "unknown".to_string()
                    };

                    total_input += input;
                    total_output += output + thoughts;
                    total_cache_read += cached;
                    total_cost += cost;
                    total_calls += 1;
                    session_calls += 1;
                    session_cost += cost;
                    session_model = model.clone();

                    let d = by_day.entry(day.clone()).or_insert_with(|| DailyUsage {
                        date: day.clone(), cost: 0.0, calls: 0, input_tokens: 0, output_tokens: 0,
                    });
                    d.cost += cost; d.calls += 1;
                    d.input_tokens += input; d.output_tokens += output + thoughts;

                    let m = by_model.entry(model.clone()).or_insert_with(|| ModelUsage {
                        model: model.clone(), cost: 0.0, calls: 0,
                        input_tokens: 0, output_tokens: 0, cache_hit_percent: 0.0,
                    });
                    m.cost += cost; m.calls += 1;
                    m.input_tokens += input; m.output_tokens += output + thoughts;

                    let p = by_project.entry(project_path.clone()).or_insert_with(|| ProjectUsage {
                        project: project_path.clone(), cost: 0.0, sessions: 0, calls: 0,
                    });
                    p.cost += cost; p.calls += 1;

                    if let Some(arr) = ev.get("toolCalls").and_then(|v| v.as_array()) {
                        for tc in arr {
                            if let Some(name) = tc.get("name").and_then(|v| v.as_str()) {
                                *tool_counts.entry(name.to_string()).or_insert(0) += 1;
                                // Shell commands surface inside the
                                // `run_shell_command` tool's `command`
                                // argument. Capture the head word so
                                // the dashboard's shell breakdown is
                                // populated for Gemini too.
                                if name == "run_shell_command" {
                                    if let Some(cmd) = tc.get("args")
                                        .and_then(|a| a.get("command"))
                                        .and_then(|v| v.as_str())
                                    {
                                        if let Some(head) = cmd.split_whitespace().next() {
                                            *shell_counts.entry(head.to_string()).or_insert(0) += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            );

            if !session_id.is_empty() && session_calls > 0 {
                project_sessions
                    .entry(project_path.clone())
                    .or_default()
                    .insert(session_id.clone());
                per_session.entry(session_id.clone()).or_insert(SessionCost {
                    session_id, project: project_path.clone(),
                    cost: session_cost, calls: session_calls, model: session_model,
                });
            }
        }
    }

    for (proj, ids) in &project_sessions {
        if let Some(p) = by_project.get_mut(proj) {
            p.sessions = ids.len() as u32;
        }
    }
    let cache_hit_percent = if total_input + total_cache_read > 0 {
        (total_cache_read as f64 / (total_input + total_cache_read) as f64) * 100.0
    } else { 0.0 };

    let mut daily: Vec<DailyUsage> = by_day.into_values().collect();
    daily.sort_by(|a, b| a.date.cmp(&b.date));
    let mut by_model_vec: Vec<ModelUsage> = by_model.into_values().collect();
    by_model_vec.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_project_vec: Vec<ProjectUsage> = by_project.into_values().collect();
    by_project_vec.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let mut top: Vec<SessionCost> = per_session.into_values().collect();
    top.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let total_sessions = top.len() as u32;
    top.truncate(ANALYTICS_TOP_SESSIONS as usize);

    let mut tools: Vec<ToolCount> = tool_counts.into_iter().map(|(name, count)| ToolCount { name, count }).collect();
    tools.sort_by(|a, b| b.count.cmp(&a.count));
    let mut shell_commands: Vec<ToolCount> = shell_counts.into_iter().map(|(name, count)| ToolCount { name, count }).collect();
    shell_commands.sort_by(|a, b| b.count.cmp(&a.count));

    Ok(UsageAnalytics {
        total_cost,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_cache_read_tokens: total_cache_read,
        total_cache_write_tokens: 0,
        total_sessions,
        total_api_calls: total_calls,
        cache_hit_percent,
        daily,
        by_model: by_model_vec,
        by_project: by_project_vec,
        top_sessions: top,
        tools,
        shell_commands,
    })
}

/// Walk `~/.gemini/tmp/*/chats/*.jsonl` for a session file whose
/// header `sessionId` matches `session_id`, then return the cumulative
/// context-fill from the LAST `gemini` event's `tokens.total`. The
/// total field is already the running input-window size Google's
/// backend reports back, so no summation needed.
fn gemini_context_usage(session_id: &str) -> Result<ContextUsage, String> {
    let tmp_root = dirs::home_dir()
        .map(|h| h.join(".gemini").join("tmp"))
        .ok_or("Cannot determine home directory")?;
    if !tmp_root.exists() {
        return Err("Gemini tmp directory not found".into());
    }

    let mut found: Option<std::path::PathBuf> = None;
    'outer: for slug_entry in fs::read_dir(&tmp_root).map_err(|e| e.to_string())?.flatten() {
        let chats_dir = slug_entry.path().join("chats");
        if !chats_dir.exists() { continue; }
        for f in fs::read_dir(&chats_dir).map_err(|e| e.to_string())?.flatten() {
            let path = f.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "jsonl" && ext != "json" { continue; }
            // Cheap match: header is in the first kilobyte — peek the
            // start of the file rather than reading it whole.
            if let Ok(mut buf) = fs::read(&path) {
                buf.truncate(buf.len().min(2048));
                if let Ok(text) = std::str::from_utf8(&buf) {
                    if text.contains(session_id) {
                        found = Some(path);
                        break 'outer;
                    }
                }
            }
        }
    }
    let path = found.ok_or("Session file not found")?;

    let mut last_total: u64 = 0;
    let mut last_input: u64 = 0;
    let mut last_cached: u64 = 0;
    let mut model = String::from("unknown");
    let _ = walk_gemini_session(
        &path,
        |_hdr| {},
        |ev| {
            if ev.get("type").and_then(|t| t.as_str()) != Some("gemini") { return; }
            if let Some(tokens) = ev.get("tokens") {
                last_total = tokens.get("total").and_then(|v| v.as_u64()).unwrap_or(last_total);
                last_input = tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(last_input);
                last_cached = tokens.get("cached").and_then(|v| v.as_u64()).unwrap_or(last_cached);
            }
            if let Some(m) = ev.get("model").and_then(|v| v.as_str()) {
                model = m.to_string();
            }
        },
    );

    let context_window = model_context_window(&model);
    let fill_percent = if context_window > 0 {
        (last_total as f64 / context_window as f64) * 100.0
    } else { 0.0 };

    Ok(ContextUsage {
        input_tokens: last_input.saturating_sub(last_cached),
        cache_read_tokens: last_cached,
        cache_creation_tokens: 0,
        total_context_tokens: last_total,
        context_window,
        fill_percent,
        model,
        compacted: false,
    })
}

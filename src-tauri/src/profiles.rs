use crate::models::SessionProfile;
use crate::storage::{load_profiles, save_profiles, now_iso8601, project_name_from_path};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Context prompt helper
// ---------------------------------------------------------------------------

pub fn get_context_prompt(purpose: &str) -> String {
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
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_profiles() -> Result<Vec<SessionProfile>, String> {
    Ok(load_profiles())
}

#[tauri::command]
pub fn create_profile(title: String, purpose: String, project_path: String, skip_permissions: Option<bool>, custom_prompt: Option<String>, git_name: Option<String>, git_email: Option<String>, contexts: Option<Vec<String>>) -> Result<SessionProfile, String> {
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
        git_name,
        git_email,
        contexts: contexts.unwrap_or_default(),
    };

    profiles.push(profile.clone());
    save_profiles(&profiles)?;
    Ok(profile)
}

#[tauri::command]
pub fn delete_profile(id: String) -> Result<(), String> {
    let mut profiles = load_profiles();
    profiles.retain(|p| p.id != id);
    save_profiles(&profiles)?;
    Ok(())
}

#[tauri::command]
pub fn rename_profile(id: String, new_title: String) -> Result<(), String> {
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
pub fn update_profile(
    id: String,
    title: Option<String>,
    skip_permissions: Option<bool>,
    git_name: Option<String>,
    git_email: Option<String>,
    context_prompt: Option<String>,
) -> Result<(), String> {
    let mut profiles = load_profiles();
    if let Some(profile) = profiles.iter_mut().find(|p| p.id == id) {
        if let Some(t) = title { profile.title = t; }
        if let Some(sp) = skip_permissions { profile.skip_permissions = sp; }
        if let Some(ref name) = git_name { profile.git_name = Some(name.clone()); }
        if let Some(ref email) = git_email { profile.git_email = Some(email.clone()); }
        if let Some(ref prompt) = context_prompt { profile.context_prompt = prompt.clone(); }
    } else {
        return Err("Profile not found".to_string());
    }
    save_profiles(&profiles)?;
    Ok(())
}

#[tauri::command]
pub fn update_last_used(id: String) -> Result<(), String> {
    let mut profiles = load_profiles();
    if let Some(profile) = profiles.iter_mut().find(|p| p.id == id) {
        profile.last_used_at = now_iso8601();
    } else {
        return Err("Profile not found".to_string());
    }
    save_profiles(&profiles)?;
    Ok(())
}

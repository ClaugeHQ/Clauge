use crate::commands::agent_models::{TerminalEntry, TerminalOutputPayload, TerminalState};
use base64::Engine;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use tauri::ipc::Channel;
use tauri::State;
use uuid::Uuid;

fn resolve_claude_path() -> String {
    let user_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    if let Ok(output) = std::process::Command::new(&user_shell)
        .args(["-l", "-i", "-c", "which claude"])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() { return path; }
        }
    }
    "claude".to_string()
}

#[tauri::command]
pub fn agent_spawn_terminal(
    state: State<'_, TerminalState>,
    session_id: Option<String>,
    project_path: String,
    context_prompt: Option<String>,
    skip_permissions: Option<bool>,
    git_name: Option<String>,
    git_email: Option<String>,
    on_output: Channel<TerminalOutputPayload>,
) -> Result<String, String> {
    let terminal_id = Uuid::new_v4().to_string();
    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Failed to open PTY: {}", e))?;

    let mut claude_cmd = String::from("claude");
    if let Some(ref sid) = session_id {
        claude_cmd.push_str(&format!(" --resume \"{}\"", sid));
    }
    if skip_permissions.unwrap_or(false) {
        claude_cmd.push_str(" --dangerously-skip-permissions");
    }
    if let Some(ref prompt) = context_prompt {
        if !prompt.is_empty() {
            // Use single quotes to prevent ALL shell interpretation (< > $ ` etc.)
            // Escape single quotes within the prompt: ' -> '\''
            let escaped = prompt.replace('\'', "'\\''");
            claude_cmd.push_str(&format!(" --append-system-prompt '{}'", escaped));
        }
    }

    let user_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut cmd = CommandBuilder::new(&user_shell);
    cmd.arg("-l"); cmd.arg("-c"); cmd.arg(&claude_cmd);
    cmd.cwd(&project_path);
    if let Some(home) = dirs::home_dir() { cmd.env("HOME", home.to_string_lossy().to_string()); }
    cmd.env("TERM", "xterm-256color");
    if let Some(ref name) = git_name { cmd.env("GIT_AUTHOR_NAME", name); cmd.env("GIT_COMMITTER_NAME", name); }
    if let Some(ref email) = git_email { cmd.env("GIT_AUTHOR_EMAIL", email); cmd.env("GIT_COMMITTER_EMAIL", email); }

    let child = pty_pair.slave.spawn_command(cmd).map_err(|e| format!("Failed to spawn claude: {}", e))?;
    let writer = pty_pair.master.take_writer().map_err(|e| format!("Failed to get PTY writer: {}", e))?;
    let reader = pty_pair.master.try_clone_reader().map_err(|e| format!("Failed to clone PTY reader: {}", e))?;

    let tid_clone = terminal_id.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    if on_output.send(TerminalOutputPayload { terminal_id: tid_clone.clone(), data, exit: None }).is_err() { break; }
                }
                Err(_) => break,
            }
        }
        // PTY closed — signal the frontend so it can clean up without waiting for a stray write.
        let _ = on_output.send(TerminalOutputPayload { terminal_id: tid_clone.clone(), data: String::new(), exit: Some(true) });
    });

    state.terminals.lock().insert(terminal_id.clone(), TerminalEntry { master: pty_pair.master, writer, child });
    Ok(terminal_id)
}

#[tauri::command]
pub fn agent_spawn_shell(
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
    cmd.arg("-l"); cmd.arg("-i"); cmd.cwd(&project_path);
    if let Some(home) = dirs::home_dir() { cmd.env("HOME", home.to_string_lossy().to_string()); }
    cmd.env("TERM", "xterm-256color");

    let child = pty_pair.slave.spawn_command(cmd).map_err(|e| format!("Failed to spawn shell: {}", e))?;
    let writer = pty_pair.master.take_writer().map_err(|e| format!("Failed to get PTY writer: {}", e))?;
    let reader = pty_pair.master.try_clone_reader().map_err(|e| format!("Failed to clone PTY reader: {}", e))?;

    let tid_clone = terminal_id.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    if on_output.send(TerminalOutputPayload { terminal_id: tid_clone.clone(), data, exit: None }).is_err() { break; }
                }
                Err(_) => break,
            }
        }
        let _ = on_output.send(TerminalOutputPayload { terminal_id: tid_clone.clone(), data: String::new(), exit: Some(true) });
    });

    state.terminals.lock().insert(terminal_id.clone(), TerminalEntry { master: pty_pair.master, writer, child });
    Ok(terminal_id)
}

#[tauri::command]
pub fn agent_write_to_terminal(state: State<'_, TerminalState>, terminal_id: String, data: String) -> Result<(), String> {
    let mut terminals = state.terminals.lock();
    let entry = terminals.get_mut(&terminal_id).ok_or("Terminal not found")?;
    entry.writer.write_all(data.as_bytes()).map_err(|e| format!("Write error: {}", e))?;
    entry.writer.flush().map_err(|e| format!("Flush error: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn agent_resize_terminal(state: State<'_, TerminalState>, terminal_id: String, cols: u32, rows: u32) -> Result<(), String> {
    let terminals = state.terminals.lock();
    let entry = terminals.get(&terminal_id).ok_or("Terminal not found")?;
    entry.master.resize(PtySize { rows: rows as u16, cols: cols as u16, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Resize error: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn agent_kill_terminal(state: State<'_, TerminalState>, terminal_id: String) -> Result<(), String> {
    let mut terminals = state.terminals.lock();
    if let Some(mut entry) = terminals.remove(&terminal_id) { let _ = entry.child.kill(); }
    Ok(())
}

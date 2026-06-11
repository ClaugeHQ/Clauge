use crate::companion::fanout;
use crate::modes::agent::models::{TerminalEntry, TerminalOutputPayload, TerminalState};
use crate::shared::repos::settings as settings_repo;
use crate::shared::cli::{registry::runner_for, runner::{CliRunner, SpawnOpts}};
use crate::shared::platform::shell::default_user_shell;
use base64::Engine;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use sqlx::SqlitePool;
use std::io::{Read, Write};
use tauri::ipc::Channel;
use tauri::State;
use uuid::Uuid;

#[cfg(target_os = "windows")]
fn apply_windows_env(cmd: &mut CommandBuilder) {
    if let Some(home) = dirs::home_dir() {
        cmd.env("USERPROFILE", home.to_string_lossy().to_string());
    }
    if let Ok(v) = std::env::var("APPDATA") {
        cmd.env("APPDATA", v);
    }
    if let Ok(v) = std::env::var("LOCALAPPDATA") {
        cmd.env("LOCALAPPDATA", v);
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_windows_env(_cmd: &mut CommandBuilder) {}

#[tauri::command]
pub async fn agent_spawn_terminal(
    state: State<'_, TerminalState>,
    pool: State<'_, SqlitePool>,
    session_id: Option<String>,
    project_path: String,
    context_prompt: Option<String>,
    skip_permissions: Option<bool>,
    git_name: Option<String>,
    git_email: Option<String>,
    provider: Option<String>,
    // Per-session override of the CLI binary path. Forwarded into
    // `SpawnOpts::binary_path_override` so the provider's
    // `build_spawn_command` substitutes it (shell-quoted) in place of
    // the bare binary name. `None`/empty = default $PATH lookup.
    binary_path: Option<String>,
    // Legacy frontend-supplied fallback. The backend now reads the
    // persisted workspace MCP token directly from settings so token
    // injection can't be skipped by stale frontend state.
    workspace_mcp_token: Option<String>,
    on_output: Channel<TerminalOutputPayload>,
) -> Result<String, String> {
    // The desktop only knows the claude resume id at this point, so
    // that doubles as the entry's session_ref (see TerminalEntry).
    spawn_agent_terminal_impl(
        &state,
        pool.inner(),
        session_id.clone(),
        session_id,
        project_path,
        context_prompt,
        skip_permissions,
        git_name,
        git_email,
        provider,
        binary_path,
        workspace_mcp_token,
        Some(on_output),
    )
    .await
}

/// Shared spawn path for the Tauri command above and the companion
/// server (POST /v1/sessions/agent). `on_output: None` means no one is
/// streaming yet — the reader thread still drains the PTY so the child
/// never blocks on a full pipe; the companion fan-out (D3) taps output
/// separately.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn spawn_agent_terminal_impl(
    state: &TerminalState,
    pool: &SqlitePool,
    session_ref: Option<String>,
    resume_session_id: Option<String>,
    project_path: String,
    context_prompt: Option<String>,
    skip_permissions: Option<bool>,
    git_name: Option<String>,
    git_email: Option<String>,
    provider: Option<String>,
    binary_path: Option<String>,
    workspace_mcp_token: Option<String>,
    on_output: Option<Channel<TerminalOutputPayload>>,
) -> Result<String, String> {
    crate::telemetry::bump("agent.spawn");
    let terminal_id = Uuid::new_v4().to_string();
    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Failed to open PTY: {}", e))?;

    // Provider is passed in from the frontend (which reads it off the
    // session row). Unknown / missing → Claude via runner_for's default.
    let provider = provider.unwrap_or_else(|| "claude".to_string());
    let cli: &dyn CliRunner = runner_for(&provider);
    let spawn_cmd = cli.build_spawn_command(&SpawnOpts {
        resume_session_id,
        system_prompt: context_prompt,
        skip_permissions: skip_permissions.unwrap_or(false),
        binary_path_override: binary_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    });

    let (shell_path, shell_kind) = default_user_shell();
    let mut cmd = CommandBuilder::new(&shell_path);
    // For bash/zsh: -l (login) sources ~/.zprofile but tools like nvm/fnm/asdf
    // configure node on PATH inside ~/.zshrc which only loads with -i. PowerShell
    // and cmd.exe don't have these concepts; ShellKind handles that.
    for arg in shell_kind.exec_command_argv(&spawn_cmd) {
        cmd.arg(&arg);
    }
    cmd.cwd(&project_path);
    if let Some(home) = dirs::home_dir() { cmd.env("HOME", home.to_string_lossy().to_string()); }
    apply_windows_env(&mut cmd);
    cmd.env("TERM", "xterm-256color");
    if let Some(ref name) = git_name { cmd.env("GIT_AUTHOR_NAME", name); cmd.env("GIT_COMMITTER_NAME", name); }
    if let Some(ref email) = git_email { cmd.env("GIT_AUTHOR_EMAIL", email); cmd.env("GIT_COMMITTER_EMAIL", email); }

    // Codex registers the workspace MCP with `--bearer-token-env-var
    // CLAUGE_WORKSPACE_TOKEN` (see modes/workspace/commands.rs
    // ::register_codex). Inject the persisted token into the env
    // exactly when we're spawning codex, so codex can authenticate
    // without the token ever touching ~/.codex/config.toml.
    if provider == "codex" {
        let persisted_token = match settings_repo::get_by_key(pool, "workspace_mcp_token").await {
            Ok(Some(s)) => Some(s.value),
            Ok(None) => None,
            Err(e) => {
                log::warn!(target: "agent::terminal", "failed to read workspace MCP token for codex spawn: {e}");
                None
            }
        };
        let token = persisted_token
            .as_deref()
            .filter(|t| !t.is_empty())
            .or_else(|| workspace_mcp_token.as_deref().filter(|t| !t.is_empty()));
        log::info!(
            target: "agent::terminal",
            "codex spawn workspace MCP token present: {}",
            token.is_some()
        );
        if let Some(token) = token {
            cmd.env(
                crate::modes::workspace::commands::CODEX_BEARER_ENV,
                token,
            );
        }
    } else {
        log::debug!(target: "agent::terminal", "agent spawn provider={provider}; codex MCP token injection skipped");
    }

    let child = pty_pair.slave.spawn_command(cmd).map_err(|e| format!("Failed to spawn {}: {}", cli.id(), e))?;
    let writer = pty_pair.master.take_writer().map_err(|e| format!("Failed to get PTY writer: {}", e))?;
    let reader = pty_pair.master.try_clone_reader().map_err(|e| format!("Failed to clone PTY reader: {}", e))?;

    // Register with the companion fan-out before the reader starts so
    // the very first bytes land in the mirror scrollback.
    fanout::register(&terminal_id, fanout::TermKind::Agent);

    let tid_clone = terminal_id.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    fanout::publish(&tid_clone, &buf[..n]);
                    if let Some(ch) = &on_output {
                        let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                        if ch.send(TerminalOutputPayload { terminal_id: tid_clone.clone(), data, exit: None }).is_err() { break; }
                    }
                }
                Err(_) => break,
            }
        }
        // PTY closed — signal mirrors and the frontend so both clean up
        // without waiting for a stray write.
        fanout::publish_exit(&tid_clone);
        if let Some(ch) = &on_output {
            let _ = ch.send(TerminalOutputPayload { terminal_id: tid_clone.clone(), data: String::new(), exit: Some(true) });
        }
    });

    state.terminals.lock().insert(terminal_id.clone(), TerminalEntry { master: pty_pair.master, writer, child, session_ref });
    Ok(terminal_id)
}

fn spawn_shell_pty(
    cwd: &str,
    state: &TerminalState,
    on_output: Channel<TerminalOutputPayload>,
) -> Result<String, String> {
    let terminal_id = Uuid::new_v4().to_string();
    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Failed to open PTY: {}", e))?;

    let (shell_path, shell_kind) = default_user_shell();
    let mut cmd = CommandBuilder::new(&shell_path);
    for arg in shell_kind.interactive_login_args() {
        cmd.arg(arg);
    }
    cmd.cwd(cwd);
    if let Some(home) = dirs::home_dir() { cmd.env("HOME", home.to_string_lossy().to_string()); }
    apply_windows_env(&mut cmd);
    cmd.env("TERM", "xterm-256color");

    let child = pty_pair.slave.spawn_command(cmd).map_err(|e| format!("Failed to spawn shell: {}", e))?;
    let writer = pty_pair.master.take_writer().map_err(|e| format!("Failed to get PTY writer: {}", e))?;
    let reader = pty_pair.master.try_clone_reader().map_err(|e| format!("Failed to clone PTY reader: {}", e))?;

    // Shell PTYs aren't listed by the companion API, but they share
    // agent_resize_terminal — registering keeps the size-indirection
    // path (set_client_size → effective_size) uniform for every entry
    // in TerminalState.
    fanout::register(&terminal_id, fanout::TermKind::Agent);

    let tid_clone = terminal_id.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    fanout::publish(&tid_clone, &buf[..n]);
                    let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    if on_output.send(TerminalOutputPayload { terminal_id: tid_clone.clone(), data, exit: None }).is_err() { break; }
                }
                Err(_) => break,
            }
        }
        fanout::publish_exit(&tid_clone);
        let _ = on_output.send(TerminalOutputPayload { terminal_id: tid_clone.clone(), data: String::new(), exit: Some(true) });
    });

    state.terminals.lock().insert(terminal_id.clone(), TerminalEntry { master: pty_pair.master, writer, child, session_ref: None });
    Ok(terminal_id)
}

#[tauri::command]
pub fn agent_spawn_shell(
    state: State<'_, TerminalState>,
    project_path: String,
    on_output: Channel<TerminalOutputPayload>,
) -> Result<String, String> {
    spawn_shell_pty(&project_path, &*state, on_output)
}

#[tauri::command]
pub fn canvas_shell_terminal_spawn(
    state: State<'_, TerminalState>,
    workspace_id: String,
    cwd: String,
    on_output: Channel<TerminalOutputPayload>,
) -> Result<String, String> {
    let _ = workspace_id; // reserved for future logging / multi-window scope
    spawn_shell_pty(&cwd, &*state, on_output)
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
    // The desktop is one mirror client among potentially many: record
    // its viewport and drive the PTY at the effective (min) size, which
    // is exactly the desktop size while no phone is attached. None =
    // effective size unchanged → nothing to apply.
    if let Some((c, r)) =
        fanout::set_client_size(&terminal_id, fanout::DESKTOP_CLIENT, cols as u16, rows as u16)
    {
        entry.master.resize(PtySize { rows: r, cols: c, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("Resize error: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
pub fn agent_kill_terminal(state: State<'_, TerminalState>, terminal_id: String) -> Result<(), String> {
    let mut terminals = state.terminals.lock();
    if let Some(mut entry) = terminals.remove(&terminal_id) { let _ = entry.child.kill(); }
    Ok(())
}

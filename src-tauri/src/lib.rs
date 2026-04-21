pub mod models;
pub use models::*;

pub mod storage;
use storage::*;

pub mod profiles;
pub mod git;
pub mod worktree;
pub mod terminal;
pub mod plugins;
pub mod usage;
pub mod sessions;
pub mod contexts;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIconBuilder, TrayIconId};
use tauri::Manager;

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

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
            profiles::get_profiles,
            profiles::create_profile,
            profiles::delete_profile,
            profiles::rename_profile,
            profiles::update_last_used,
            sessions::refresh_session_ids,
            sessions::update_session_id,
            worktree::is_git_repo,
            git::get_git_status,
            git::get_git_branch,
            git::get_git_ahead_behind,
            git::git_commit,
            git::git_push,
            git::git_pull,
            git::git_diff_file,
            git::git_stage_file,
            git::git_unstage_file,
            git::git_log,
            git::git_stash,
            git::git_stash_pop,
            git::git_list_branches,
            git::git_switch_branch,
            worktree::create_worktree,
            worktree::remove_worktree,
            worktree::update_profile_worktree,
            sessions::count_project_sessions,
            sessions::discover_sessions,
            sessions::get_session_tokens,
            usage::fetch_usage_limits,
            usage::get_usage_analytics,
            get_app_version,
            get_claude_plan,
            update_tray_title,
            save_session_key,
            load_session_key,
            terminal::spawn_terminal,
            terminal::spawn_shell,
            terminal::write_to_terminal,
            terminal::resize_terminal,
            terminal::kill_terminal,
            plugins::get_claude_plugins,
            plugins::toggle_claude_plugin,
            plugins::get_marketplace_plugins,
            plugins::install_plugin,
            plugins::uninstall_plugin,
            contexts::get_context_snippets,
            contexts::save_context_snippet,
            contexts::delete_context_snippet,
            contexts::inject_session_context,
            contexts::remove_injected_context,
            sessions::update_session_contexts
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

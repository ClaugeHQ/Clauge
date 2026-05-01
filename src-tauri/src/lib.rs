mod appearance;
mod commands;
mod db;
mod github;
mod modes;
mod shared;

use std::str::FromStr;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    // Single-instance enforcement on Linux + Windows. macOS .app bundles
    // are single-instance natively, so the plugin is gated to other OSes.
    // Must be registered FIRST — the plugin intercepts startup, brings the
    // running window to focus, and forwards deep-link URIs from the new
    // attempt (e.g. clauge:// OAuth callbacks) to the existing process.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_http::init())
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations("sqlite:clauge.db", db::migrations::get_migrations())
                .build(),
        )
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Per-OS window chrome (rounded corners on macOS/Win11, dock icon
            // on macOS, no-op on Linux).
            appearance::window_chrome::apply(app);

            // Initialize sqlx connection pool for Rust commands
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            std::fs::create_dir_all(&app_data_dir).ok();

            let db_path = app_data_dir.join("clauge.db");
            let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

            let pool = tauri::async_runtime::block_on(async {
                let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&db_url)
                    .expect("invalid db url")
                    .pragma("foreign_keys", "ON")
                    .create_if_missing(true);
                sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(5)
                    .connect_with(opts)
                    .await
                    .expect("failed to connect to database")
            });

            // Run schema migrations directly via sqlx
            tauri::async_runtime::block_on(async {
                for migration in db::migrations::get_migrations() {
                    for statement in migration.sql.split(';') {
                        let stmt = statement.trim();
                        if !stmt.is_empty() {
                            if let Err(e) = sqlx::query(stmt).execute(&pool).await {
                                // Ignore "already exists" / "duplicate column" errors from re-running migrations
                                let err_str = e.to_string();
                                if !err_str.contains("already exists") && !err_str.contains("duplicate column") {
                                    eprintln!("Migration v{} statement failed: {}", migration.version, e);
                                }
                            }
                        }
                    }
                }
            });

            // Migrate existing Clauge data (one-time)
            // Old Clauge stored: ~/.clauge/sessions.json, ~/.clauge/contexts/*.md, ~/.clauge/session_key
            // Sessions contain a `contexts: Vec<String>` field with attached context names
            tauri::async_runtime::block_on(async {
                if let Some(home) = dirs::home_dir() {
                    let clauge_dir = home.join(".clauge");
                    let sessions_json = clauge_dir.join("sessions.json");
                    let migrated_key = "clauge_migration_done";
                    let already_migrated: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
                        .bind(migrated_key).fetch_optional(&pool).await.ok().flatten();
                    if sessions_json.exists() && already_migrated.is_none() {
                        // Step 1: Import context snippets from ~/.clauge/contexts/*.md
                        // Do this FIRST so we have context IDs available for session-context linking
                        let contexts_dir = clauge_dir.join("contexts");
                        let mut context_name_to_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                        if contexts_dir.exists() {
                            if let Ok(entries) = std::fs::read_dir(&contexts_dir) {
                                for entry in entries.flatten() {
                                    let path = entry.path();
                                    if path.extension().and_then(|e| e.to_str()) == Some("md") {
                                        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                                        let content = std::fs::read_to_string(&path).unwrap_or_default();
                                        if name.is_empty() || content.is_empty() { continue; }
                                        let ctx_id = uuid::Uuid::new_v4().to_string();
                                        let now = chrono::Utc::now().to_rfc3339();
                                        if sqlx::query("INSERT OR IGNORE INTO agent_contexts (id, name, content, created_at, updated_at) VALUES (?,?,?,?,?)")
                                            .bind(&ctx_id).bind(&name).bind(&content).bind(&now).bind(&now)
                                            .execute(&pool).await.is_ok() {
                                            context_name_to_id.insert(name, ctx_id);
                                        }
                                    }
                                }
                            }
                        }

                        // Step 2: Import sessions and link their attached contexts
                        if let Ok(content) = std::fs::read_to_string(&sessions_json) {
                            if let Ok(store) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(profiles) = store.get("profiles").and_then(|v| v.as_array()) {
                                    for p in profiles {
                                        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                                        if id.is_empty() { continue; }
                                        let inserted = sqlx::query("INSERT OR IGNORE INTO agent_sessions (id, title, purpose, project_path, project_name, claude_session_id, context_prompt, worktree_path, worktree_branch, skip_permissions, git_name, git_email, created_at, last_used_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
                                            .bind(id)
                                            .bind(p.get("title").and_then(|v| v.as_str()).unwrap_or(""))
                                            .bind(p.get("purpose").and_then(|v| v.as_str()).unwrap_or("Custom"))
                                            .bind(p.get("projectPath").and_then(|v| v.as_str()).unwrap_or(""))
                                            .bind(p.get("projectName").and_then(|v| v.as_str()).unwrap_or(""))
                                            .bind(p.get("claudeSessionId").and_then(|v| v.as_str()))
                                            .bind(p.get("contextPrompt").and_then(|v| v.as_str()).unwrap_or(""))
                                            .bind(p.get("worktreePath").and_then(|v| v.as_str()))
                                            .bind(p.get("worktreeBranch").and_then(|v| v.as_str()))
                                            .bind(if p.get("skipPermissions").and_then(|v| v.as_bool()).unwrap_or(false) { 1 } else { 0 })
                                            .bind(p.get("gitName").and_then(|v| v.as_str()))
                                            .bind(p.get("gitEmail").and_then(|v| v.as_str()))
                                            .bind(p.get("createdAt").and_then(|v| v.as_str()).unwrap_or(""))
                                            .bind(p.get("lastUsedAt").and_then(|v| v.as_str()).unwrap_or(""))
                                            .execute(&pool).await;

                                        // Link attached contexts via junction table
                                        if inserted.is_ok() {
                                            if let Some(ctx_names) = p.get("contexts").and_then(|v| v.as_array()) {
                                                for ctx_name in ctx_names {
                                                    if let Some(name_str) = ctx_name.as_str() {
                                                        if let Some(ctx_id) = context_name_to_id.get(name_str) {
                                                            let _ = sqlx::query("INSERT OR IGNORE INTO agent_session_contexts (session_id, context_id) VALUES (?,?)")
                                                                .bind(id).bind(ctx_id)
                                                                .execute(&pool).await;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Step 3: Import session key
                        let key_path = clauge_dir.join("session_key");
                        if key_path.exists() {
                            if let Ok(key) = std::fs::read_to_string(&key_path) {
                                let key = key.trim();
                                if !key.is_empty() {
                                    let _ = sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES ('agent_session_key', ?)")
                                        .bind(key).execute(&pool).await;
                                }
                            }
                        }

                        // Step 4: Mark migration done
                        let _ = sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, 'true')")
                            .bind(migrated_key).execute(&pool).await;

                        // Step 5: Archive old files to ~/.clauge/backup/
                        let backup = clauge_dir.join("backup");
                        let _ = std::fs::create_dir_all(&backup);
                        let _ = std::fs::rename(&sessions_json, backup.join("sessions.json"));
                        if key_path.exists() {
                            let _ = std::fs::rename(&key_path, backup.join("session_key"));
                        }
                        if contexts_dir.exists() {
                            let _ = std::fs::rename(&contexts_dir, backup.join("contexts"));
                        }
                    }
                }
            });

            // Load saved vibrancy material before managing pool (which moves it)
            let saved_material = tauri::async_runtime::block_on(async {
                sqlx::query_as::<_, (String,)>(
                    "SELECT value FROM settings WHERE key = 'vibrancy_material'",
                )
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten()
                .map(|r| r.0)
                .unwrap_or_else(|| "sidebar".to_string())
            });

            app.manage(pool);
            app.manage(Arc::new(modes::sql::client::SqlConnectionManager::new()));
            app.manage(modes::nosql::client::create_nosql_state());
            app.manage(modes::agent::models::TerminalState::default());
            app.manage(modes::ssh::models::SshTerminalState::default());
            app.manage(shared::ai::types::PendingFrontendTools::default());
            app.manage(shared::updater::state::PendingUpdate::default());

            // Register every mode's AI tools into the shared dispatch registry.
            // Adding a new tool to a mode = one new function + one entry in
            // that mode's `register_tools()`; zero edits to the dispatch loop.
            modes::agent::ai_tools::register_tools();
            modes::rest::ai_tools::register_tools();
            modes::sql::ai_tools::register_tools();
            modes::nosql::ai_tools::register_tools();
            modes::ssh::ai_tools::register_tools();

            // Apply vibrancy on macOS — use Sidebar material (what native macOS apps use)
            if let Some(window) = app.get_webview_window("main") {
                let _ = appearance::vibrancy::apply_vibrancy(&window, &saved_material);
            }

            // System tray with menu
            {
                use tauri::tray::TrayIconBuilder;
                use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

                let show_item = MenuItem::with_id(app, "show", "Back to App", true, None::<&str>)?;
                let separator = PredefinedMenuItem::separator(app)?;
                let quit_item = MenuItem::with_id(app, "quit", "Quit Clauge", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

                // tray-dark.png is a black silhouette designed to be used as a
                // template image on macOS — the system inverts it automatically
                // for the menubar. Linux and Windows don't do template inversion,
                // so a black-on-dark-panel icon becomes invisible. Use the colored
                // tray-light.png there.
                #[cfg(target_os = "macos")]
                let icon_png: &[u8] = include_bytes!("../icons/tray-dark.png");
                #[cfg(not(target_os = "macos"))]
                let icon_png: &[u8] = include_bytes!("../icons/tray-light.png");

                let img = image::load_from_memory(icon_png).expect("Failed to load tray icon");
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let tray_icon = tauri::image::Image::new_owned(rgba.into_raw(), w, h);

                #[allow(unused_mut)]
                let mut tray_builder = TrayIconBuilder::with_id("main-tray")
                    .icon(tray_icon)
                    .menu(&menu)
                    .tooltip("Clauge");

                // Template mode is a macOS-only concept — the system uses the
                // alpha channel to render the icon in the right color for the
                // current menubar appearance. Skipping this on Linux/Windows
                // means the icon's natural colors are used.
                #[cfg(target_os = "macos")]
                {
                    tray_builder = tray_builder.icon_as_template(true);
                }

                tray_builder
                    .on_menu_event(move |app_handle: &tauri::AppHandle, event: tauri::menu::MenuEvent| {
                        let id = event.id().as_ref();
                        if id == "quit" {
                            app_handle.exit(0);
                        } else if id == "show" {
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            shared::platform::install_type::get_install_type,
            shared::platform::install_type::supports_self_update,
            shared::updater::commands::check_for_update_in_channel,
            shared::updater::commands::install_pending_update,
            modes::rest::collections::list_collections,
            modes::rest::collections::create_collection,
            modes::rest::collections::update_collection,
            modes::rest::collections::delete_collection,
            modes::rest::collections::reorder_collections,
            modes::rest::requests::list_requests,
            modes::rest::requests::get_request,
            modes::rest::requests::create_request,
            modes::rest::requests::update_request,
            modes::rest::requests::delete_request,
            modes::rest::requests::duplicate_request,
            modes::rest::requests::move_request,
            modes::rest::requests::update_request_headers,
            modes::rest::requests::update_request_params,
            modes::rest::environments::list_environments,
            modes::rest::environments::create_environment,
            modes::rest::environments::update_environment,
            modes::rest::environments::delete_environment,
            modes::rest::environments::set_default_environment,
            modes::rest::environments::list_env_variables,
            modes::rest::environments::set_env_variable,
            modes::rest::environments::update_env_variable,
            modes::rest::environments::delete_env_variable,
            modes::rest::environments::get_env_variables_for_resolution,
            modes::rest::http_executor::execute_request,
            modes::rest::http_executor::quick_execute,
            modes::rest::history::list_history,
            modes::rest::history::clear_history,
            modes::rest::history::delete_history_entry,
            commands::settings::get_setting,
            commands::settings::set_setting,
            commands::settings::get_all_settings,
            appearance::vibrancy::set_vibrancy,
            appearance::vibrancy::get_appearance,
            appearance::vibrancy::set_appearance,
            appearance::vibrancy::get_available_themes,
            github::oauth::github_connect,
            github::oauth::github_disconnect,
            github::oauth::github_get_status,
            github::oauth::github_get_oauth_url,
            github::oauth::github_connect_with_token,
            github::gist::gist_check_exists,
            github::gist::gist_sync_push,
            github::gist::gist_sync_pull,
            modes::rest::import_export::export_collection,
            modes::rest::import_export::export_all_collections,
            modes::rest::import_export::import_clauge,
            modes::rest::import_export::import_postman,
            modes::rest::import_export::import_curl,
            modes::rest::import_export::export_as_curl,
            modes::sql::client::sql_connect,
            modes::sql::client::sql_connect_database,
            modes::sql::client::sql_disconnect,
            modes::sql::client::sql_test_connection,
            modes::sql::client::sql_execute_query,
            modes::sql::client::sql_list_databases,
            modes::sql::client::sql_create_database,
            modes::sql::client::sql_list_schemas,
            modes::sql::client::sql_list_tables,
            modes::sql::client::sql_describe_table,
            modes::sql::client::sql_save_connection,
            modes::sql::client::sql_list_saved_connections,
            modes::sql::client::sql_delete_saved_connection,
            modes::sql::client::sql_update_saved_connection,
            modes::sql::client::sql_save_script,
            modes::sql::client::sql_list_scripts,
            modes::sql::client::sql_update_script,
            modes::sql::client::sql_delete_script,
            modes::nosql::client::nosql_connect,
            modes::nosql::client::nosql_disconnect,
            modes::nosql::client::nosql_test_connection,
            modes::nosql::client::nosql_list_databases,
            modes::nosql::client::nosql_list_collections,
            modes::nosql::client::nosql_find_documents,
            modes::nosql::client::nosql_insert_document,
            modes::nosql::client::nosql_update_document,
            modes::nosql::client::nosql_delete_document,
            modes::nosql::client::nosql_count_documents,
            modes::nosql::client::nosql_aggregate,
            modes::nosql::client::nosql_create_collection,
            modes::nosql::client::nosql_drop_database,
            modes::nosql::client::nosql_drop_collection,
            modes::nosql::client::nosql_rename_collection,
            modes::nosql::client::redis_execute,
            modes::nosql::client::redis_list_keys,
            modes::nosql::client::redis_get_key,
            modes::nosql::client::redis_set_key,
            modes::nosql::client::redis_delete_key,
            modes::nosql::client::redis_get_info,
            modes::nosql::client::nosql_save_connection,
            modes::nosql::client::nosql_list_saved_connections,
            modes::nosql::client::nosql_delete_saved_connection,
            modes::nosql::client::nosql_update_saved_connection,
            shared::ai::test_ai_key,
            shared::ai::get_ai_usage_stats,
            shared::ai::get_ai_provider_stats,
            shared::ai::reset_ai_usage,
            shared::ai::record_ai_usage,
            shared::ai::ai_chat,
            shared::ai::ai_resolve_pending_tool,
            // Agent mode
            modes::agent::commands::agent_list_sessions,
            modes::agent::commands::agent_create_session,
            modes::agent::commands::agent_update_session,
            modes::agent::commands::agent_delete_session,
            modes::agent::commands::agent_update_session_id,
            modes::agent::commands::agent_update_last_used,
            modes::agent::commands::agent_update_worktree,
            modes::agent::commands::agent_list_contexts,
            modes::agent::commands::agent_save_context,
            modes::agent::commands::agent_delete_context,
            modes::agent::commands::agent_get_session_contexts,
            modes::agent::commands::agent_attach_context,
            modes::agent::commands::agent_detach_context,
            modes::agent::commands::agent_inject_contexts,
            modes::agent::commands::agent_remove_injected_contexts,
            modes::agent::terminal::agent_spawn_terminal,
            modes::agent::terminal::agent_spawn_shell,
            modes::agent::terminal::agent_write_to_terminal,
            modes::agent::terminal::agent_resize_terminal,
            modes::agent::terminal::agent_kill_terminal,
            modes::agent::worktree::agent_is_git_repo,
            modes::agent::worktree::agent_create_worktree,
            modes::agent::worktree::agent_remove_worktree,
            modes::agent::git::agent_git_status,
            modes::agent::git::agent_git_branch,
            modes::agent::git::agent_git_ahead_behind,
            modes::agent::git::agent_git_commit,
            modes::agent::git::agent_git_push,
            modes::agent::git::agent_git_pull,
            modes::agent::git::agent_git_diff_file,
            modes::agent::git::agent_git_stage_file,
            modes::agent::git::agent_git_unstage_file,
            modes::agent::git::agent_git_log,
            modes::agent::git::agent_git_stash,
            modes::agent::git::agent_git_stash_pop,
            modes::agent::git::agent_git_list_branches,
            modes::agent::git::agent_git_switch_branch,
            modes::agent::plugins::agent_get_plugins,
            modes::agent::plugins::agent_toggle_plugin,
            modes::agent::plugins::agent_get_marketplace_plugins,
            modes::agent::plugins::agent_install_plugin,
            modes::agent::plugins::agent_uninstall_plugin,
            modes::agent::usage::agent_get_usage_analytics,
            modes::agent::usage::agent_fetch_usage_limits,
            modes::agent::usage::agent_discover_sessions,
            modes::agent::usage::agent_get_session_tokens,
            modes::agent::usage::agent_get_session_context_usage,
            modes::agent::commands::agent_update_tray_title,
            modes::agent::commands::agent_get_claude_plan,
            // SSH mode
            modes::ssh::profiles::ssh_list_profiles,
            modes::ssh::profiles::ssh_create_profile,
            modes::ssh::profiles::ssh_update_profile,
            modes::ssh::profiles::ssh_delete_profile,
            modes::ssh::profiles::ssh_touch_profile,
            modes::ssh::profiles::ssh_get_credential,
            modes::ssh::terminal::ssh_spawn_terminal,
            modes::ssh::terminal::ssh_write_to_terminal,
            modes::ssh::terminal::ssh_resize_terminal,
            modes::ssh::terminal::ssh_kill_terminal,
            modes::ssh::tunnel::ssh_tunnel_test,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let win = window.clone();
                tauri::async_runtime::spawn(async move {
                    // macOS: hiding a window mid-fullscreen leaves a blank fullscreen
                    // space. Exit fullscreen first, wait for the animation, then hide.
                    if matches!(win.is_fullscreen(), Ok(true)) {
                        let _ = win.set_fullscreen(false);
                        for _ in 0..30 {
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            if matches!(win.is_fullscreen(), Ok(false)) { break; }
                        }
                    }
                    let _ = win.hide();
                });
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {
            // macOS: clicking dock icon re-shows the hidden window. The
            // Reopen variant only exists in tauri's macOS feature set.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                if let Some(window) = _app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });
}

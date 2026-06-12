mod appearance;
mod cloud;
mod commands;
mod companion;
mod db;
mod modes;
mod shared;
mod telemetry;

use std::sync::Arc;
use tauri::Manager;

/// Best-effort: write any not-yet-pushed sync kinds to settings so the next
/// boot can re-queue them. Never blocks quit on the network.
async fn persist_pending_sync(app: &tauri::AppHandle) {
    let kinds = cloud::scheduler::pending_kinds();
    if kinds.is_empty() {
        return;
    }
    if let Some(pool) = app.try_state::<sqlx::SqlitePool>() {
        let joined = kinds.join(",");
        let _ = shared::repos::settings::upsert(pool.inner(), "cloud:pending_dirty", &joined).await;
    }
}

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
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            // Explicitly forward clauge:// URIs from the new launch to the
            // running deep-link plugin. The single-instance plugin's
            // deep-link cargo feature is supposed to do this automatically,
            // but the auto-forward can miss on Linux .deb installs (the
            // .desktop handoff produces a slightly different argv shape).
            // The frontend's centralized handler in +layout.svelte listens
            // for this event via onOpenUrl() and dispatches clauge:oauth-callback.
            use tauri::Emitter;
            let uris: Vec<String> = args
                .iter()
                .filter(|a| a.starts_with("clauge://"))
                .cloned()
                .collect();
            if !uris.is_empty() {
                let _ = app.emit("deep-link://new-url", &uris);
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
        // tauri-plugin-sql is registered without migrations — the Clauge
        // database lives in app_data_dir/clauge.db and its schema is
        // managed end-to-end by `db::migrator` (see src-tauri/migrations/).
        // The plugin remains available for any frontend SQL access against
        // user-configured databases (Postgres/MySQL/Mongo via the SQL/NoSQL
        // modes), but it does NOT touch the Clauge schema.
        .plugin(tauri_plugin_sql::Builder::default().build())
        .setup(|app| {
            // Rolling file logger — per-day folder, per-hour file, 30-day retention.
            // Replaces the debug-only tauri-plugin-log initialisation.
            if let Ok(log_dir) = app.path().app_log_dir() {
                let _ = shared::logger::init(&log_dir);
            }

            // Route whisper.cpp/GGML native logging into the `log` crate
            // (log_backend feature) instead of spamming stderr on every
            // transcription. Safe to call once; permanent for the process.
            whisper_rs::install_logging_hooks();

            // Apply advanced/diagnostic settings from `settings.json`
            // (lives next to the SQLite DB in app_config_dir). This is
            // the hidden knob for log verbosity + future feature flags.
            // The visible Settings UI deliberately doesn't expose it —
            // power users edit the file directly; see app_config.rs.
            let cfg = shared::app_config::load(app.handle());
            shared::app_config::apply(&cfg);

            // Per-OS window chrome (rounded corners on macOS/Win11, dock icon
            // on macOS, no-op on Linux).
            appearance::window_chrome::apply(app);

            // ── Database setup ───────────────────────────────────────
            // 1. Open the SQLite pool.
            // 2. Run schema migrations (with bootstrap for legacy installs).
            // 3. One-time import of pre-SQLite ~/.clauge/* data.
            //
            // All three steps are encapsulated under `db::*` so this
            // setup() block stays focused on plumbing.
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");

            let pool = tauri::async_runtime::block_on(async {
                db::pool::init(&app_data_dir).await
            }).expect("failed to open Clauge database");

            cloud::snapshots::init(&app_data_dir);

            tauri::async_runtime::block_on(async {
                db::migrator::run(&pool).await
            }).expect("failed to apply schema migrations");

            tauri::async_runtime::block_on(async {
                db::legacy_import::run_if_needed(&pool).await;
            });

            // Meetings left at status='recording' mean the app died
            // mid-recording. Finalize them now, before the detect poller
            // starts — no recording can be active at boot.
            match tauri::async_runtime::block_on(
                modes::workspace::meetings::repo::recover_interrupted(&pool),
            ) {
                Ok(n) if n > 0 => {
                    log::info!("[meetings] recovered {} interrupted meeting(s)", n)
                }
                Ok(_) => {}
                Err(e) => log::warn!("[meetings] recover_interrupted: {}", e),
            }

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
            app.manage(modes::ssh::models::PendingAuthPrompts::default());
            // Stash the AppHandle so the keyboard-interactive auth flow
            // (in modes/ssh/ssh_session.rs) can emit prompt events to the
            // frontend without threading AppHandle through every connect
            // entry point (terminal, tunnel, explorer all already exist).
            modes::ssh::ssh_session::set_app_handle(app.handle().clone());
            app.manage(modes::explorer::session::ExplorerSessions::default());
            app.manage(modes::explorer::transfers::Transfers::default());
            app.manage(shared::ai::types::PendingFrontendTools::default());
            app.manage(shared::updater::state::PendingUpdate::default());
            // Companion (mobile) server state. The server itself stays
            // OFF until companion_start — no autostart by design.
            app.manage(companion::CompanionState::default());

            // ── Cloud auth + sync scheduler ──────────────────────────
            // Holds provider tokens (loaded from OS keyring) and the per-kind
            // dirty set used by auto-sync. Spawned once, lives for app lifetime.
            app.manage(cloud::auth::AuthState::default());
            app.manage(cloud::scheduler::Scheduler::default());
            // Single in-memory authority for Pro state. Hydrated below.
            app.manage(cloud::pro_state::ProStateManager::default());

            // Load tokens from keyring (+ one-time migration from legacy
            // settings.github_token row). If a token resolves to a logged-in
            // session, enable the scheduler so subsequent mutations push.
            // Also hydrate the ProStateManager from on-disk snapshots so the
            // frontend's `is_pro` checks against the manager don't briefly
            // return free on cold boot before cloud_get_status resolves.
            {
                let pool_for_auth = app.state::<sqlx::SqlitePool>().inner().clone();
                let auth_state = app.state::<cloud::auth::AuthState>();
                let pro_state = app.state::<cloud::pro_state::ProStateManager>();
                if let Err(e) = tauri::async_runtime::block_on(async {
                    cloud::auth::load_from_keyring(&auth_state, &pool_for_auth).await
                }) {
                    log::warn!("[cloud] load_from_keyring: {}", e);
                }
                tauri::async_runtime::block_on(
                    pro_state.hydrate_from_snapshot(&pool_for_auth),
                );
                if auth_state.is_connected() {
                    app.state::<cloud::scheduler::Scheduler>().enable();
                    // Re-queue kinds that were dirty when the app last quit.
                    tauri::async_runtime::block_on(async {
                        if let Ok(Some(row)) = shared::repos::settings::get_by_key(
                            &pool_for_auth,
                            "cloud:pending_dirty",
                        )
                        .await
                        {
                            let slugs: Vec<String> =
                                row.value.split(',').map(|s| s.to_string()).collect();
                            cloud::scheduler::rebump_slugs(&slugs);
                            let _ = sqlx::query(
                                "DELETE FROM settings WHERE key = 'cloud:pending_dirty'",
                            )
                            .execute(&pool_for_auth)
                            .await;
                        }
                    });
                }
            }

            cloud::scheduler::spawn(app.handle().clone());

            // Telemetry scheduler — sleeps 30s after boot, then flushes
            // once per 24h. Fire-and-forget; failures back off to 1h.
            // Counters live in static AtomicU64s, so the hot path
            // (`telemetry::bump`) has no dependency on this spawn.
            telemetry::spawn_scheduler(app.handle().clone());

            // Register every mode's AI tools into the shared dispatch registry.
            // Adding a new tool to a mode = one new function + one entry in
            // that mode's `register_tools()`; zero edits to the dispatch loop.
            // Workspace + agent skip this — they have no in-app AI panel
            // (workspace owns the MCP server in `mcp/`; agent shells out
            // to the Claude CLI which manages its own tool calls).
            modes::rest::ai_tools::register_tools();
            modes::sql::ai_tools::register_tools();
            modes::nosql::ai_tools::register_tools();
            modes::ssh::ai_tools::register_tools();
            modes::explorer::ai_tools::register_tools();
            // Workspace MCP server state — single global handle.
            // Started/stopped via workspace_mcp_start/stop commands.
            app.manage(modes::workspace::commands::McpServerState::default());
            app.manage(modes::workspace::meetings::recorder::RecorderState::default());

            // Meeting call detection — polls mic-in-use + process names and
            // emits meetings:call-detected / meetings:call-ended for the
            // floating widget. The poller loads the persisted
            // `workspace_meeting_detect_enabled` setting itself (default on)
            // and stays silent while the recorder is busy.
            app.manage(modes::workspace::meetings::detect::DetectState::default());
            modes::workspace::meetings::detect::start_poller(app.handle().clone());

            // Auto-start the workspace MCP server in the background so
            // agents can connect without the user opening Settings.
            // Opt-out via the `workspace_mcp_enabled = "false"` setting.
            // Backgrounded so a slow port-fallback walk can't delay
            // the main window.
            {
                let app_handle = app.handle().clone();
                let pool_for_autostart = app.state::<sqlx::SqlitePool>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    modes::workspace::commands::maybe_autostart_mcp(app_handle, pool_for_autostart).await;
                });
            }

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
                            let ah = app_handle.clone();
                            tauri::async_runtime::spawn(async move {
                                persist_pending_sync(&ah).await;
                                ah.exit(0);
                            });
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
            shared::updater::commands::check_latest_version,
            shared::updater::commands::install_pending_update,
            modes::explorer::connections::explorer_list_connections,
            modes::explorer::connections::explorer_get_connection,
            modes::explorer::connections::explorer_create_connection,
            modes::explorer::connections::explorer_update_connection,
            modes::explorer::connections::explorer_delete_connection,
            modes::explorer::connections::explorer_set_secret,
            modes::explorer::connections::explorer_get_secret,
            modes::explorer::connections::explorer_delete_secrets,
            modes::explorer::session::explorer_open_session,
            modes::explorer::session::explorer_close_session,
            modes::explorer::session::explorer_fs_list,
            modes::explorer::session::explorer_fs_stat,
            modes::explorer::session::explorer_fs_read,
            modes::explorer::session::explorer_fs_write,
            modes::explorer::session::explorer_fs_delete,
            modes::explorer::session::explorer_fs_mkdir,
            modes::explorer::session::explorer_fs_rename,
            modes::explorer::session::explorer_fs_search,
            modes::explorer::session::explorer_fs_home_dir,
            modes::explorer::session::explorer_fs_get_url,
            modes::explorer::transfers::explorer_upload_file,
            modes::explorer::transfers::explorer_download_file,
            modes::explorer::transfers::explorer_cancel_transfer,
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
            modes::rest::history::count_history,
            modes::rest::history::rest_history_size_bytes,
            modes::rest::history::purge_history,
            commands::settings::get_setting,
            commands::settings::set_setting,
            commands::settings::get_all_settings,
            commands::logs::get_log_dir,
            commands::logs::open_log_folder,
            commands::logs::app_log,
            commands::logs::set_log_level,
            commands::logs::get_app_config_path,
            telemetry::telemetry_bump,
            appearance::vibrancy::set_vibrancy,
            appearance::vibrancy::get_appearance,
            appearance::vibrancy::set_appearance,
            appearance::vibrancy::get_available_themes,
            cloud::commands::cloud_get_status,
            cloud::commands::cloud_github_login_url,
            cloud::commands::cloud_google_login_url,
            cloud::commands::cloud_exchange_code,
            cloud::commands::cloud_link_provider,
            cloud::commands::cloud_unlink_provider,
            cloud::commands::cloud_update_profile,
            cloud::commands::cloud_check_remote_exists,
            cloud::commands::cloud_remote_state,
            cloud::commands::cloud_sync_push_now,
            cloud::commands::cloud_sync_restore,
            cloud::commands::cloud_get_conflicts,
            cloud::commands::cloud_resolve_keep_local,
            cloud::commands::cloud_resolve_use_remote,
            cloud::commands::cloud_resolve_kind,
            cloud::commands::cloud_merge_all,
            cloud::commands::cloud_force_push_all,
            cloud::commands::cloud_pull_if_remote_newer,
            cloud::commands::cloud_local_has_data,
            cloud::commands::cloud_logout,
            cloud::commands::cloud_wipe_remote,
            cloud::commands::cloud_delete_account,
            cloud::commands::cloud_get_pricing,
            cloud::commands::cloud_create_checkout,
            cloud::commands::cloud_open_portal,
            cloud::commands::cloud_ai_balance,
            cloud::commands::cloud_ai_usage,
            cloud::commands::cloud_get_active_token,
            cloud::commands::cloud_list_snapshots,
            cloud::commands::cloud_restore_snapshot,
            cloud::commands::cloud_history_list,
            cloud::commands::cloud_history_restore,
            cloud::pro_state::pro_state_current,
            cloud::credentials_probe::cloud_probe_missing_credentials,
            modes::rest::import_export::export_collection,
            modes::rest::import_export::export_all_collections,
            modes::rest::import_export::import_clauge,
            modes::rest::import_export::import_postman,
            modes::rest::import_export::import_curl,
            modes::rest::import_export::export_as_curl,
            modes::sql::client::sql_ensure_pool,
            modes::sql::client::sql_cancel_query,
            modes::sql::client::sql_disconnect_pool,
            modes::sql::client::sql_disconnect_connection,
            modes::sql::client::sql_test_connection,
            modes::sql::client::sql_execute_query,
            modes::sql::client::sql_execute_batch,
            modes::sql::client::sql_current_schema,
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
            modes::agent::commands::agent_inject_purpose,
            modes::agent::terminal::agent_spawn_terminal,
            modes::agent::terminal::agent_spawn_shell,
            modes::agent::terminal::agent_write_to_terminal,
            modes::agent::terminal::agent_resize_terminal,
            modes::agent::terminal::agent_kill_terminal,
            modes::agent::worktree::agent_is_git_repo,
            modes::agent::worktree::agent_create_worktree,
            modes::agent::worktree::agent_remove_worktree,
            modes::agent::worktree::agent_worktree_is_dirty,
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
            modes::agent::usage::agent_fetch_codex_usage_limits,
            modes::agent::usage::agent_discover_sessions,
            modes::agent::usage::agent_resolve_resume_id,
            modes::agent::usage::agent_get_session_tokens,
            modes::agent::usage::agent_get_session_context_usage,
            modes::agent::commands::agent_update_tray_title,
            modes::agent::commands::agent_get_claude_plan,
            modes::agent::commands::agent_check_claude_installed,
            modes::agent::commands::agent_check_cli_installed,
            modes::agent::commands::agent_check_clis_installed,
            modes::agent::commands::agent_validate_binary,
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
            modes::ssh::ssh_session::ssh_submit_auth_prompts,
            modes::ssh::config_import::ssh_read_config_hosts,
            modes::ssh::config_import::ssh_import_config_hosts,
            // Workspace mode
            modes::workspace::commands::workspace_list,
            modes::workspace::commands::workspace_get,
            modes::workspace::commands::workspace_create,
            modes::workspace::commands::workspace_update,
            modes::workspace::commands::workspace_delete,
            modes::workspace::commands::workspace_delete_preview,
            modes::workspace::commands::workspace_note_list,
            modes::workspace::commands::workspace_note_get,
            modes::workspace::commands::workspace_note_create,
            modes::workspace::commands::workspace_note_update,
            modes::workspace::commands::workspace_note_delete,
            modes::workspace::commands::workspace_note_export_to_file,
            modes::workspace::commands::workspace_board_list,
            modes::workspace::commands::workspace_board_get,
            modes::workspace::commands::workspace_board_create,
            modes::workspace::commands::workspace_board_rename,
            modes::workspace::commands::workspace_board_set_project,
            modes::workspace::commands::workspace_board_delete,
            modes::workspace::commands::workspace_column_list,
            modes::workspace::commands::workspace_card_list,
            modes::workspace::commands::workspace_card_create,
            modes::workspace::commands::workspace_card_update,
            modes::workspace::commands::workspace_card_move,
            modes::workspace::commands::workspace_card_clear_review,
            modes::workspace::commands::workspace_card_delete,
            modes::workspace::commands::workspace_board_dismissed_externals,
            modes::workspace::commands::workspace_card_add_comment,
            modes::workspace::commands::workspace_card_comment_list,
            modes::workspace::commands::workspace_card_comment_delete,
            modes::workspace::commands::workspace_card_push_to_repo,
            modes::workspace::commands::workspace_card_raise_pr,
            modes::workspace::commands::workspace_card_check_pr_state,
            modes::workspace::commands::workspace_card_get_claim,
            modes::workspace::commands::workspace_card_drawer_chat,
            modes::workspace::commands::workspace_card_release,
            modes::workspace::commands::workspace_card_start_work,
            modes::workspace::commands::workspace_coworker_list,
            modes::workspace::commands::workspace_coworker_get,
            modes::workspace::commands::workspace_coworker_create,
            modes::workspace::commands::workspace_coworker_update,
            modes::workspace::commands::workspace_coworker_delete,
            modes::workspace::commands::workspace_inbox_list,
            modes::workspace::commands::workspace_mcp_status,
            modes::workspace::commands::workspace_mcp_start,
            modes::workspace::commands::workspace_mcp_stop,
            modes::workspace::commands::workspace_mcp_register,
            modes::workspace::commands::workspace_mcp_unregister,
            modes::workspace::commands::workspace_mcp_new_token,
            modes::workspace::commands::workspace_scan_project_issues,
            modes::workspace::commands::workspace_scan_project_issues_by_url,
            modes::workspace::meetings::commands::workspace_meeting_list,
            modes::workspace::meetings::commands::workspace_meeting_get,
            modes::workspace::meetings::commands::workspace_meeting_update_title,
            modes::workspace::meetings::commands::workspace_meeting_update_notes,
            modes::workspace::meetings::commands::workspace_meeting_generate_notes,
            modes::workspace::meetings::commands::workspace_meeting_delete,
            modes::workspace::meetings::commands::workspace_meeting_models_list,
            modes::workspace::meetings::commands::workspace_meeting_model_download,
            modes::workspace::meetings::commands::workspace_meeting_model_delete,
            modes::workspace::meetings::commands::workspace_meeting_start,
            modes::workspace::meetings::commands::workspace_meeting_stop,
            modes::workspace::meetings::commands::workspace_meeting_recording_status,
            modes::workspace::meetings::commands::workspace_meeting_detect_set_enabled,
            modes::workspace::meetings::commands::workspace_meeting_detect_get_enabled,
            modes::workspace::meetings::commands::workspace_meeting_autostop_set_enabled,
            modes::workspace::meetings::commands::workspace_meeting_autostop_get_enabled,
            modes::workspace::meetings::commands::workspace_meeting_detect_dismiss,
            modes::workspace::meetings::commands::workspace_meeting_detect_status,
            modes::workspace::meetings::permissions::workspace_meeting_request_permissions,

            // Companion (mobile) server
            companion::server::companion_status,
            companion::server::companion_start,
            companion::server::companion_stop,
            companion::pairing::companion_new_pair_code,
            companion::pairing::companion_approve_pair,
            companion::pairing::companion_deny_pair,
            companion::devices::companion_list_devices,
            companion::devices::companion_revoke_device,
            companion::devices::companion_delete_device,
            companion::devices::companion_purge_revoked,
            companion::companion_report_opened,
            companion::companion_report_open_failed,

            // Canvas mode
            modes::canvas::commands::canvas_resolve_tiles,
            modes::canvas::commands::canvas_list_tiles,
            modes::canvas::commands::canvas_upsert_tiles_batch,
            modes::canvas::commands::canvas_minimize_tile,
            modes::canvas::commands::canvas_delete_tile,
            modes::canvas::commands::canvas_get_viewport,
            modes::canvas::commands::canvas_set_viewport,
            modes::canvas::commands::canvas_list_regions,
            modes::canvas::commands::canvas_upsert_region,
            modes::canvas::commands::canvas_delete_region,
            modes::agent::terminal::canvas_shell_terminal_spawn,
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

            // Quit (Cmd+Q / OS shutdown / exit(0)): persist any not-yet-pushed
            // sync kinds before letting the process die. The AtomicBool gates a
            // single deferral — the re-issued exit(0) passes straight through.
            if let tauri::RunEvent::ExitRequested { api, .. } = &_event {
                use std::sync::atomic::{AtomicBool, Ordering};
                static PERSISTED: AtomicBool = AtomicBool::new(false);
                if !PERSISTED.swap(true, Ordering::SeqCst) {
                    api.prevent_exit();
                    let ah = _app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        // Stop an active recording first so the last buffered
                        // segments flush to the DB instead of dying with the
                        // process. Bounded: a wedged stop must not block quit —
                        // the crash-recovery sweep finalizes the meeting at
                        // next boot if this can't finish in time.
                        let recorder = ah
                            .state::<modes::workspace::meetings::recorder::RecorderState>();
                        if recorder.status().recording {
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(8),
                                modes::workspace::meetings::recorder::stop_recording(ah.clone()),
                            )
                            .await
                            {
                                Ok(Ok(id)) => {
                                    log::info!("[meetings] recording {id} stopped on quit")
                                }
                                Ok(Err(e)) => {
                                    log::warn!("[meetings] stop on quit failed: {e}")
                                }
                                Err(_) => log::warn!(
                                    "[meetings] stop on quit timed out — crash recovery \
                                     finalizes the meeting at next boot"
                                ),
                            }
                        }
                        persist_pending_sync(&ah).await;
                        ah.exit(0);
                    });
                }
            }
        });
}

// Tauri commands exposed to the frontend. Thin wrappers around the `cloud::*`
// internals; bulk of the logic lives in `auth.rs`, `client.rs`, `sync.rs`.

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::cloud::auth::{self, AuthState};
use crate::cloud::client;
use crate::cloud::config::{settings_key_synced_at, SETTINGS_KEY_HAS_SYNCED};
use crate::cloud::domains::ALL_KINDS;
use crate::cloud::models::{CloudStatus, CloudUser};
use crate::cloud::scheduler::Scheduler;
use crate::cloud::sync;
use crate::shared::repos::settings;

// ─── Status / OAuth URL builders ────────────────────────────────────────────

#[tauri::command]
pub async fn cloud_get_status(
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
) -> Result<CloudStatus, String> {
    let snap = state.snapshot();
    if !state.is_connected() {
        return Ok(CloudStatus::default());
    }

    // Fetch fresh `me` from the server; if it fails we still return a partial
    // status so the UI can render the "we have a token but server is down" state.
    match client::me(pool.inner(), &state).await {
        Ok(me) => {
            let mut last_synced = std::collections::HashMap::new();
            for k in ALL_KINDS {
                if let Ok(Some(s)) = settings::get_by_key(pool.inner(), &settings_key_synced_at(k)).await {
                    last_synced.insert(k.to_string(), s.value);
                }
            }
            Ok(CloudStatus {
                connected: true,
                active_provider: snap.active_provider,
                user: Some(me.user),
                providers: me.providers,
                plan: me.plan,
                last_synced,
            })
        }
        Err(_) => Ok(CloudStatus {
            connected: true,
            active_provider: snap.active_provider,
            user: snap.user_id.map(|id| CloudUser {
                user_id: id,
                email: None,
                display_name: None,
                first_name: None,
                last_name: None,
                avatar_url: None,
                slug: String::new(),
            }),
            providers: Vec::new(),
            plan: "free".into(),
            last_synced: Default::default(),
        }),
    }
}

#[tauri::command]
pub fn cloud_github_login_url() -> String {
    auth::github_oauth_url()
}

#[tauri::command]
pub fn cloud_google_login_url() -> String {
    auth::google_oauth_url()
}

// ─── OAuth code exchange (deep-link → here) ─────────────────────────────────

#[tauri::command]
pub async fn cloud_exchange_code(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
    provider: String,
    code: String,
) -> Result<CloudStatus, String> {
    match provider.as_str() {
        "github" => {
            let resp = client::exchange_github(pool.inner(), &code).await?;
            let token = resp.token.clone().ok_or_else(|| "missing token".to_string())?;
            auth::store_github(&state, pool.inner(), &token, resp.user.user_id).await?;
            after_login(&app, pool.inner(), &state).await?;
            Ok(build_status(pool.inner(), &state, &resp).await)
        }
        "google" => {
            let resp = client::exchange_google(
                pool.inner(),
                &code,
                "https://clauge.in/auth/google-callback.html",
            )
            .await?;
            let id_token = resp.id_token.clone().ok_or_else(|| "missing id_token".to_string())?;
            auth::store_google(
                &state,
                pool.inner(),
                resp.token.as_deref(),
                resp.refresh.as_deref(),
                &id_token,
                resp.user.user_id,
            )
            .await?;
            after_login(&app, pool.inner(), &state).await?;
            Ok(build_status(pool.inner(), &state, &resp).await)
        }
        _ => Err(format!("unknown provider: {}", provider)),
    }
}

// ─── Linking ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn cloud_link_provider(
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
    provider: String,
    code: String,
) -> Result<CloudStatus, String> {
    let me = client::link(pool.inner(), &state, &provider, &code, None)
        .await
        .map_err(String::from)?;
    let snap = state.snapshot();
    Ok(CloudStatus {
        connected: true,
        active_provider: snap.active_provider,
        user: Some(me.user),
        providers: me.providers,
        plan: me.plan,
        last_synced: Default::default(),
    })
}

#[tauri::command]
pub async fn cloud_update_profile(
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
    display_name: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
) -> Result<CloudStatus, String> {
    let me = client::update_profile(pool.inner(), &state, display_name, first_name, last_name)
        .await
        .map_err(String::from)?;
    let snap = state.snapshot();
    Ok(CloudStatus {
        connected: true,
        active_provider: snap.active_provider,
        user: Some(me.user),
        providers: me.providers,
        plan: me.plan,
        last_synced: Default::default(),
    })
}

#[tauri::command]
pub async fn cloud_unlink_provider(
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
    provider: String,
) -> Result<CloudStatus, String> {
    let me = client::unlink(pool.inner(), &state, &provider)
        .await
        .map_err(String::from)?;
    let snap = state.snapshot();
    Ok(CloudStatus {
        connected: true,
        active_provider: snap.active_provider,
        user: Some(me.user),
        providers: me.providers,
        plan: me.plan,
        last_synced: Default::default(),
    })
}

// ─── Sync surface ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn cloud_check_remote_exists(
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
) -> Result<bool, String> {
    let rows = client::sync_state(pool.inner(), &state)
        .await
        .map_err(String::from)?;
    Ok(!rows.is_empty())
}

#[tauri::command]
pub async fn cloud_sync_push_now(
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
) -> Result<Vec<String>, String> {
    let kinds: Vec<&str> = ALL_KINDS.iter().copied().collect();
    sync::push_all(pool.inner(), &state, &kinds).await
}

#[tauri::command]
pub async fn cloud_sync_restore(
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
) -> Result<Vec<String>, String> {
    let pulled = sync::pull_all(pool.inner(), &state).await?;
    settings::upsert(pool.inner(), SETTINGS_KEY_HAS_SYNCED, "true")
        .await
        .map_err(|e| format!("mark synced: {}", e))?;
    Ok(pulled)
}

#[tauri::command]
pub async fn cloud_local_has_data(pool: State<'_, SqlitePool>) -> Result<bool, String> {
    sync::local_has_data(pool.inner()).await
}

// ─── Account management ───────────────────────────────────────────────────

#[tauri::command]
pub async fn cloud_logout(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
) -> Result<(), String> {
    if let Some(s) = app.try_state::<Scheduler>() {
        s.disable_and_clear();
    }
    auth::clear(&state, pool.inner()).await
}

#[tauri::command]
pub async fn cloud_wipe_remote(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
) -> Result<(), String> {
    client::sync_wipe(pool.inner(), &state)
        .await
        .map_err(String::from)?;
    cloud_logout(app, pool, state).await
}

#[tauri::command]
pub async fn cloud_delete_account(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    state: State<'_, AuthState>,
    confirmation_slug: String,
) -> Result<(), String> {
    client::delete_account(pool.inner(), &state, &confirmation_slug)
        .await
        .map_err(String::from)?;
    cloud_logout(app.clone(), pool, state).await?;
    let _ = app.emit_to("main", "cloud:account-deleted", ());
    Ok(())
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// After a successful login: enable the scheduler so subsequent mutations bump.
async fn after_login(
    app: &AppHandle,
    _pool: &SqlitePool,
    _state: &AuthState,
) -> Result<(), String> {
    if let Some(s) = app.try_state::<Scheduler>() {
        s.enable();
    }
    Ok(())
}

async fn build_status(
    pool: &SqlitePool,
    state: &AuthState,
    resp: &crate::cloud::models::AuthResponse,
) -> CloudStatus {
    let snap = state.snapshot();
    let mut last_synced = std::collections::HashMap::new();
    for k in ALL_KINDS {
        if let Ok(Some(s)) = settings::get_by_key(pool, &settings_key_synced_at(k)).await {
            last_synced.insert(k.to_string(), s.value);
        }
    }
    CloudStatus {
        connected: true,
        active_provider: snap.active_provider,
        user: Some(resp.user.clone()),
        providers: resp.providers.clone(),
        plan: resp.plan.clone(),
        last_synced,
    }
}


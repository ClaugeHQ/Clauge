// Provider tokens live in the OS keyring. We hold a snapshot in Tauri State
// for the hot path; writes always go through `AuthState::set_*` so the
// keyring and in-memory state stay in lockstep.

use parking_lot::Mutex;
use sqlx::SqlitePool;
use std::sync::Arc;

use crate::cloud::config::{
    KEYRING_SERVICE, KEY_GITHUB_ACCESS, KEY_GOOGLE_ACCESS, KEY_GOOGLE_ID, KEY_GOOGLE_REFRESH,
    SETTINGS_KEY_ACTIVE_PROVIDER, SETTINGS_KEY_USER_ID,
};
use crate::shared::platform::credential_store::{credential_store, CredentialStore};
use crate::shared::repos::settings;

const GITHUB_TOKEN_KEY_LEGACY: &str = "github_token";

#[derive(Default, Clone)]
pub struct AuthSnapshot {
    pub github_token: Option<String>,
    pub google_access: Option<String>,
    pub google_refresh: Option<String>,
    pub google_id_token: Option<String>,
    pub active_provider: Option<String>,
    pub user_id: Option<i64>,
}

#[derive(Default)]
pub struct AuthState(pub Arc<Mutex<AuthSnapshot>>);

impl AuthState {
    pub fn snapshot(&self) -> AuthSnapshot {
        self.0.lock().clone()
    }

    pub fn is_connected(&self) -> bool {
        let s = self.0.lock();
        s.active_provider.is_some()
            && (s.github_token.is_some() || s.google_id_token.is_some() || s.google_access.is_some())
    }

    pub fn active_token_and_provider(&self) -> Option<(String, String)> {
        let s = self.0.lock();
        match s.active_provider.as_deref() {
            Some("github") => s.github_token.clone().map(|t| (t, "github".to_string())),
            // For Google, we send the id_token as the bearer (server verifies offline via JWKS).
            Some("google") => s.google_id_token.clone().map(|t| (t, "google".to_string())),
            _ => None,
        }
    }
}

/// Load tokens from keyring into the in-memory snapshot at app boot, and
/// run the one-time migration from the legacy `settings.github_token` row.
pub async fn load_from_keyring(state: &AuthState, pool: &SqlitePool) -> Result<(), String> {
    // ─── One-time migration: legacy settings.github_token → keyring ────
    if let Ok(Some(row)) = settings::get_by_key(pool, GITHUB_TOKEN_KEY_LEGACY).await {
        let store = credential_store();
        if let Err(e) = store
            .store(
                &keyring_key(KEY_GITHUB_ACCESS),
                &row.value,
            )
            .await
        {
            log::warn!("[cloud:auth] legacy token migration to keyring failed: {}", e);
        } else {
            // Best-effort delete of the legacy row. If this fails we'll retry on next boot.
            let _ = sqlx::query("DELETE FROM settings WHERE key = ?")
                .bind(GITHUB_TOKEN_KEY_LEGACY)
                .execute(pool)
                .await;
            log::info!("[cloud:auth] migrated legacy github_token from settings to keyring");
        }
    }

    let store = credential_store();
    let github_token = store.get(&keyring_key(KEY_GITHUB_ACCESS)).await.ok().flatten();
    let google_access = store.get(&keyring_key(KEY_GOOGLE_ACCESS)).await.ok().flatten();
    let google_refresh = store.get(&keyring_key(KEY_GOOGLE_REFRESH)).await.ok().flatten();
    let google_id_token = store.get(&keyring_key(KEY_GOOGLE_ID)).await.ok().flatten();

    let active_provider = settings::get_by_key(pool, SETTINGS_KEY_ACTIVE_PROVIDER)
        .await
        .ok()
        .flatten()
        .map(|s| s.value);
    let user_id = settings::get_by_key(pool, SETTINGS_KEY_USER_ID)
        .await
        .ok()
        .flatten()
        .and_then(|s| s.value.parse::<i64>().ok());

    *state.0.lock() = AuthSnapshot {
        github_token,
        google_access,
        google_refresh,
        google_id_token,
        active_provider,
        user_id,
    };

    Ok(())
}

/// Persist GitHub tokens + active provider; updates keyring and in-memory state.
pub async fn store_github(
    state: &AuthState,
    pool: &SqlitePool,
    access_token: &str,
    user_id: i64,
) -> Result<(), String> {
    let store = credential_store();
    store
        .store(&keyring_key(KEY_GITHUB_ACCESS), access_token)
        .await
        .map_err(|e| format!("keyring store github: {}", e))?;

    settings::upsert(pool, SETTINGS_KEY_ACTIVE_PROVIDER, "github")
        .await
        .map_err(|e| format!("settings upsert active_provider: {}", e))?;
    settings::upsert(pool, SETTINGS_KEY_USER_ID, &user_id.to_string())
        .await
        .map_err(|e| format!("settings upsert user_id: {}", e))?;

    let mut s = state.0.lock();
    s.github_token = Some(access_token.to_string());
    s.active_provider = Some("github".to_string());
    s.user_id = Some(user_id);
    Ok(())
}

/// Persist Google tokens (access + refresh + id_token) + active provider.
pub async fn store_google(
    state: &AuthState,
    pool: &SqlitePool,
    access_token: Option<&str>,
    refresh_token: Option<&str>,
    id_token: &str,
    user_id: i64,
) -> Result<(), String> {
    let store = credential_store();
    if let Some(t) = access_token {
        store
            .store(&keyring_key(KEY_GOOGLE_ACCESS), t)
            .await
            .map_err(|e| format!("keyring store google access: {}", e))?;
    }
    if let Some(rt) = refresh_token {
        store
            .store(&keyring_key(KEY_GOOGLE_REFRESH), rt)
            .await
            .map_err(|e| format!("keyring store google refresh: {}", e))?;
    }
    store
        .store(&keyring_key(KEY_GOOGLE_ID), id_token)
        .await
        .map_err(|e| format!("keyring store google id_token: {}", e))?;

    settings::upsert(pool, SETTINGS_KEY_ACTIVE_PROVIDER, "google")
        .await
        .map_err(|e| format!("settings upsert active_provider: {}", e))?;
    settings::upsert(pool, SETTINGS_KEY_USER_ID, &user_id.to_string())
        .await
        .map_err(|e| format!("settings upsert user_id: {}", e))?;

    let mut s = state.0.lock();
    if let Some(t) = access_token {
        s.google_access = Some(t.to_string());
    }
    if let Some(rt) = refresh_token {
        s.google_refresh = Some(rt.to_string());
    }
    s.google_id_token = Some(id_token.to_string());
    s.active_provider = Some("google".to_string());
    s.user_id = Some(user_id);
    Ok(())
}

/// Clear all keyring entries + in-memory state. Local SQLite data is untouched.
pub async fn clear(state: &AuthState, pool: &SqlitePool) -> Result<(), String> {
    let store = credential_store();
    let _ = store.delete(&keyring_key(KEY_GITHUB_ACCESS)).await;
    let _ = store.delete(&keyring_key(KEY_GOOGLE_ACCESS)).await;
    let _ = store.delete(&keyring_key(KEY_GOOGLE_REFRESH)).await;
    let _ = store.delete(&keyring_key(KEY_GOOGLE_ID)).await;

    let _ = sqlx::query("DELETE FROM settings WHERE key LIKE 'cloud:%'")
        .execute(pool)
        .await;

    *state.0.lock() = AuthSnapshot::default();
    Ok(())
}

fn keyring_key(suffix: &str) -> String {
    format!("{}:{}", KEYRING_SERVICE, suffix)
}

// ─── OAuth URL builders ─────────────────────────────────────────────────────

const GITHUB_CLIENT_ID: &str = "Ov23liXcWby6XVM80TfG";
const GOOGLE_CLIENT_ID: &str =
    "361959797138-ahsfia59q9cf6h6njln6qt26jk763jp7.apps.googleusercontent.com";

pub fn github_oauth_url() -> String {
    // No scope — we only need public user info. Dropping the prior `scope=gist`
    // grant is a privacy win.
    format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}",
        GITHUB_CLIENT_ID,
        urlencoding::encode("https://clauge.in/auth/callback"),
    )
}

pub fn google_oauth_url() -> String {
    let redirect = urlencoding::encode("https://clauge.in/auth/google-callback.html");
    let scope = urlencoding::encode("openid email profile");
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?response_type=code\
         &client_id={}\
         &redirect_uri={}\
         &scope={}\
         &access_type=offline\
         &prompt=consent\
         &include_granted_scopes=true",
        GOOGLE_CLIENT_ID, redirect, scope,
    )
}

// Lightweight urlencoding without pulling another crate.
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }
}

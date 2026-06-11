// Push dispatch: turns fan-out triggers (terminal exit, "needs input")
// into FCM notifications via the Clauge Worker. The Worker's
// `/api/push/send` reuses the SAME Bearer provider-token auth as every
// other cloud endpoint (see cloud::client::post_json_auth), so push
// REQUIRES an active cloud sign-in — without a token we have nothing to
// authenticate to the Worker with, and we skip silently.
//
// Wiring (decoupled from fan-out): fanout emits `PushTrigger`s into an
// mpsc sink that this module installs on `start`. A drain task turns
// each trigger into a `notify_devices` call; a separate interval task
// sweeps the hubs for the attention heuristic. Both die when the
// companion server stops (watch channel), and the sink is cleared so
// late triggers are dropped.

use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::time::Duration;
use tauri::Manager;
use tokio::sync::watch;

use crate::cloud::auth::AuthState;
use crate::cloud::config::API_BASE_URL;
use crate::shared::http::build_app_http_client;

use super::fanout::{self, PushTrigger};

/// How often the attention sweep runs. Independent of ATTENTION_IDLE —
/// a shorter cadence just means the notification fires nearer the idle
/// threshold.
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);

/// Notification "kind" tags the mobile app deep-links on.
const KIND_EXIT: &str = "exit";
const KIND_ATTENTION: &str = "attention";

/// The Worker's `/api/push/send` accepts at most 10 tokens per call, so
/// we fan large device lists out in batches of this size.
const WORKER_TOKEN_BATCH: usize = 10;

/// Spawn the push tasks (trigger drain + attention sweep) and install
/// the fan-out sink. `shutdown` is the companion server's stop signal;
/// both tasks exit when it flips. Called from `companion_start`.
pub fn start(app: tauri::AppHandle, shutdown: watch::Receiver<bool>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PushTrigger>();
    fanout::set_push_sink(tx);

    // Trigger drain: one Worker call per trigger.
    let app_drain = app.clone();
    let mut sd_drain = shutdown.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                trigger = rx.recv() => match trigger {
                    Some(t) => handle_trigger(&app_drain, t).await,
                    None => break,
                },
                _ = sd_drain.changed() => break,
            }
        }
    });

    // Attention sweep: periodically flag idle prompts. The sweep itself
    // is sync + pure-ish (fanout::sweep_attention); any push it produces
    // flows back through the same sink → drain task above.
    let mut sd_sweep = shutdown;
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            tokio::select! {
                _ = ticker.tick() => fanout::sweep_attention(),
                _ = sd_sweep.changed() => break,
            }
        }
    });

    log::info!("[companion] push dispatch started");
}

/// Drop the fan-out sink so triggers stop being queued. The tasks
/// themselves wind down on the server's shutdown signal. Called from
/// `companion_stop`.
pub fn stop() {
    fanout::clear_push_sink();
    log::info!("[companion] push dispatch stopped");
}

async fn handle_trigger(app: &tauri::AppHandle, trigger: PushTrigger) {
    let (title, body, kind, terminal_id) = match trigger {
        PushTrigger::Exit { terminal_id, title } => {
            ("Session ended", title, KIND_EXIT, terminal_id)
        }
        PushTrigger::Attention { terminal_id, title } => {
            ("Needs your input", title, KIND_ATTENTION, terminal_id)
        }
    };
    let data = json!({ "kind": kind, "terminalId": terminal_id });
    notify_devices(app, title, &body, data).await;
}

/// Fetch every device with a stored FCM token and ask the Worker to
/// push `title`/`body`/`data` to them. No-ops (with a debug log) when
/// the user isn't signed in to cloud — push has no Worker auth then.
/// Per-token `stale:true` in the response clears that token so we stop
/// hitting a dead registration.
pub async fn notify_devices(app: &tauri::AppHandle, title: &str, body: &str, data: Value) {
    let pool = app.state::<SqlitePool>();
    let pool = pool.inner();

    let tokens = match super::devices::fcm_tokens(pool).await {
        Ok(t) => t,
        Err(e) => {
            log::warn!("[companion] push: load fcm tokens failed: {}", e);
            return;
        }
    };
    if tokens.is_empty() {
        return;
    }

    // Worker auth = the active cloud session's Bearer token + provider,
    // identical to cloud::client. No session → nothing to authenticate
    // with → skip (push is best-effort, not a hard dependency).
    let auth = app.state::<AuthState>();
    let Some((cloud_token, provider)) = auth.active_token_and_provider() else {
        log::debug!("[companion] push: not signed in to cloud — skipping {} device(s)", tokens.len());
        return;
    };

    let client = match build_app_http_client(pool).await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[companion] push: http client build failed: {}", e);
            return;
        }
    };

    // The Worker caps each call at 10 tokens, so send in chunks. Results
    // come back index-aligned with the chunk we sent; `stale:true` means
    // FCM rejected the registration (app uninstalled / token rotated) —
    // clear that device's token so we stop targeting it.
    for chunk in tokens.chunks(WORKER_TOKEN_BATCH) {
        let fcm_tokens: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
        let req_body = json!({
            "fcmTokens": fcm_tokens,
            "title": title,
            "body": body,
            "data": data,
        });
        let resp = client
            .post(format!("{}{}", API_BASE_URL, "/api/push/send"))
            .header("Authorization", format!("Bearer {}", cloud_token))
            .header("X-Provider", &provider)
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[companion] push: send failed: {}", e);
                continue;
            }
        };
        if !resp.status().is_success() {
            log::warn!("[companion] push: worker returned {}", resp.status().as_u16());
            continue;
        }
        let parsed: Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(results) = parsed.get("results").and_then(Value::as_array) {
            for (i, r) in results.iter().enumerate() {
                let stale = r.get("stale").and_then(Value::as_bool).unwrap_or(false);
                if !stale {
                    continue;
                }
                let Some((device_id, _)) = chunk.get(i) else {
                    continue;
                };
                if let Err(e) = super::devices::clear_fcm_token(pool, device_id).await {
                    log::warn!("[companion] push: clear stale token failed: {}", e);
                }
            }
        }
    }
}

// Pairing flow: Settings issues a one-time 6-digit code (rendered as a
// QR), the phone POSTs it to /pair, and the request parks on a oneshot
// until the user approves or denies the dialog — the same
// frontend-confirmation shape as ssh::models::PendingAuthPrompts. Only
// an approved request mints a device token, and only its SHA-256 hash
// is persisted.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json as JsonResponse, Response},
    Json,
};
use parking_lot::Mutex;
use rand::{Rng, RngExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::sync::oneshot;

use super::server::CompanionAppState;
use super::{auth, devices, EVT_PAIR_REQUEST};

/// A code is single-use AND short-lived: whichever runs out first wins.
const PAIR_CODE_TTL: Duration = Duration::from_secs(120);

/// How long /pair blocks waiting for the user to answer the desktop
/// approval dialog before giving up with 408.
const PAIR_APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Raw device-token entropy. Hex-encoded on the wire (64 chars).
const DEVICE_TOKEN_BYTES: usize = 32;

struct ActiveCode {
    code: String,
    expires_at: Instant,
}

#[derive(Default)]
pub struct PairingState {
    /// At most one live code — regenerating from Settings replaces it.
    code: Mutex<Option<ActiveCode>>,
    /// In-flight pair requests awaiting the desktop approval dialog,
    /// keyed by requestId. The approve/deny commands take the sender.
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

impl PairingState {
    pub fn issue_code(&self) -> String {
        self.issue_code_with_ttl(PAIR_CODE_TTL)
    }

    fn issue_code_with_ttl(&self, ttl: Duration) -> String {
        let code = format!("{:06}", rand::rng().random_range(0..1_000_000u32));
        *self.code.lock() = Some(ActiveCode {
            code: code.clone(),
            expires_at: Instant::now() + ttl,
        });
        code
    }

    /// One-shot validation: a matching, unexpired code is consumed; a
    /// wrong guess leaves the code in place (a typo on the phone must
    /// not force the user to regenerate); an expired code is cleared.
    pub fn consume_code(&self, candidate: &str) -> bool {
        let mut g = self.code.lock();
        match g.take() {
            Some(active) if Instant::now() >= active.expires_at => false,
            Some(active) if active.code == candidate => true,
            Some(active) => {
                *g = Some(active);
                false
            }
            None => false,
        }
    }

    fn register_pending(&self, request_id: String, tx: oneshot::Sender<bool>) {
        self.pending.lock().insert(request_id, tx);
    }

    fn remove_pending(&self, request_id: &str) {
        self.pending.lock().remove(request_id);
    }

    /// Answer a pending pair request. Err when the id is unknown —
    /// typically the 60s wait already timed out.
    pub fn resolve(&self, request_id: &str, approved: bool) -> Result<(), String> {
        match self.pending.lock().remove(request_id) {
            Some(tx) => {
                let _ = tx.send(approved);
                Ok(())
            }
            None => Err(format!("no pending pair request: {}", request_id)),
        }
    }
}

// ---------------------------------------------------------------------------
// POST /pair
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub code: String,
    pub device_name: String,
    pub platform: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResponse {
    pub device_token: String,
    pub device_id: String,
    pub server_name: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PairRequestEvent {
    request_id: String,
    device_name: String,
    platform: String,
}

fn pair_err(status: StatusCode, message: &str) -> Response {
    (status, JsonResponse(json!({ "error": message }))).into_response()
}

pub async fn handle_pair(
    State(state): State<Arc<CompanionAppState>>,
    Json(body): Json<PairRequest>,
) -> Response {
    if !state.pairing.consume_code(&body.code) {
        return pair_err(StatusCode::UNAUTHORIZED, "invalid or expired pairing code");
    }

    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel::<bool>();
    state.pairing.register_pending(request_id.clone(), tx);
    let _ = state.app.emit(
        EVT_PAIR_REQUEST,
        PairRequestEvent {
            request_id: request_id.clone(),
            device_name: body.device_name.clone(),
            platform: body.platform.clone(),
        },
    );
    log::info!(
        "[companion] pair request {} from '{}' ({})",
        request_id,
        body.device_name,
        body.platform
    );

    let approved = match tokio::time::timeout(PAIR_APPROVAL_TIMEOUT, rx).await {
        Ok(Ok(answer)) => answer,
        // Sender dropped without an answer (e.g. server stopping).
        Ok(Err(_)) => false,
        Err(_) => {
            state.pairing.remove_pending(&request_id);
            return pair_err(StatusCode::REQUEST_TIMEOUT, "pairing approval timed out");
        }
    };
    if !approved {
        log::info!("[companion] pair request {} denied", request_id);
        return pair_err(StatusCode::FORBIDDEN, "pairing denied");
    }

    let mut token_bytes = [0u8; DEVICE_TOKEN_BYTES];
    rand::rng().fill_bytes(&mut token_bytes);
    let device_token = hex::encode(token_bytes);
    let device_id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = devices::insert(
        &state.pool,
        &device_id,
        &body.device_name,
        &body.platform,
        &auth::hash_token(&device_token),
    )
    .await
    {
        log::error!("[companion] failed to persist device: {}", e);
        return pair_err(StatusCode::INTERNAL_SERVER_ERROR, "failed to persist device");
    }
    log::info!(
        "[companion] paired device {} '{}'",
        device_id,
        body.device_name
    );

    JsonResponse(PairResponse {
        device_token,
        device_id,
        server_name: tauri_plugin_os::hostname(),
    })
    .into_response()
}

// ---------------------------------------------------------------------------
// Pairing commands
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PairCodeInfo {
    pub hosts: Vec<String>,
    pub port: u16,
    pub code: String,
}

/// Issue a fresh code for the Settings QR. Requires the server to be
/// running — the QR encodes the bound port, which only exists then.
#[tauri::command]
pub async fn companion_new_pair_code(
    state: tauri::State<'_, super::CompanionState>,
) -> Result<PairCodeInfo, String> {
    let port = {
        let g = state.server.lock().await;
        g.as_ref()
            .map(|h| h.port)
            .ok_or_else(|| "companion server is not running".to_string())?
    };
    Ok(PairCodeInfo {
        hosts: local_hosts(),
        port,
        code: state.pairing.issue_code(),
    })
}

#[tauri::command]
pub async fn companion_approve_pair(
    state: tauri::State<'_, super::CompanionState>,
    request_id: String,
) -> Result<(), String> {
    state.pairing.resolve(&request_id, true)
}

#[tauri::command]
pub async fn companion_deny_pair(
    state: tauri::State<'_, super::CompanionState>,
    request_id: String,
) -> Result<(), String> {
    state.pairing.resolve(&request_id, false)
}

/// Candidate IPv4 addresses the phone can try, in interface order.
/// Loopback and link-local are useless to another device, so they are
/// filtered out; tailnet IPs (100.64/10) show up as plain interfaces.
fn local_hosts() -> Vec<String> {
    match local_ip_address::list_afinet_netifas() {
        Ok(ifas) => ifas
            .into_iter()
            .filter_map(|(_name, ip)| match ip {
                std::net::IpAddr::V4(v4)
                    if !v4.is_loopback() && !v4.is_link_local() && !v4.is_unspecified() =>
                {
                    Some(v4.to_string())
                }
                _ => None,
            })
            .collect(),
        Err(e) => {
            log::warn!("[companion] interface enumeration failed: {}", e);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_code_is_one_shot() {
        let state = PairingState::default();
        let code = state.issue_code();
        assert!(!state.consume_code("000000x"), "wrong guess must not match");
        assert!(state.consume_code(&code), "first use must succeed");
        assert!(!state.consume_code(&code), "second use must fail");
    }

    #[test]
    fn pair_code_expires() {
        let state = PairingState::default();
        let code = state.issue_code_with_ttl(Duration::ZERO);
        assert!(!state.consume_code(&code), "expired code must be rejected");
        // Expiry also clears the slot — a later identical guess stays dead.
        assert!(!state.consume_code(&code));
    }

    #[test]
    fn wrong_guess_preserves_code() {
        let state = PairingState::default();
        let code = state.issue_code();
        assert!(!state.consume_code("999999999"));
        assert!(state.consume_code(&code), "typo must not burn the code");
    }
}

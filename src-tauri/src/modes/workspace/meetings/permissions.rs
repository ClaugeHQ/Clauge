// One-time permission preflight for AI Meeting Notes. Triggered from
// Settings when the user first enables call detection, so the macOS
// TCC prompts appear at a calm moment instead of mid-meeting.

/// Briefly opens the mic and system-audio capture streams so macOS shows
/// its Microphone and System Audio Recording prompts. TCC only prompts
/// the very first time per permission — later calls are system no-ops.
/// A denied prompt (or any capture failure) is logged and swallowed;
/// the command never errors. Non-macOS platforms have no such prompts
/// and return immediately.
#[tauri::command]
pub async fn workspace_meeting_request_permissions() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = tauri::async_runtime::spawn_blocking(run_preflight).await {
            log::warn!("[meetings] permission preflight task failed: {e}");
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_preflight() {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use crate::shared::audio::{MicCapture, SystemAudioError, SystemCapture};

    const HOLD: Duration = Duration::from_millis(300);

    let (tx, _rx) = channel();
    match MicCapture::start(tx) {
        Ok(capture) => {
            std::thread::sleep(HOLD);
            capture.stop();
        }
        Err(e) => log::warn!("[meetings] permission preflight: mic capture failed: {e}"),
    }

    let (tx, _rx) = channel();
    match SystemCapture::start(tx) {
        Ok(capture) => {
            std::thread::sleep(HOLD);
            capture.stop();
        }
        Err(SystemAudioError::Unsupported(msg)) => {
            log::info!("[meetings] permission preflight: system audio unsupported, skipping: {msg}")
        }
        Err(e) => log::warn!("[meetings] permission preflight: system audio failed: {e}"),
    }
}

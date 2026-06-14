use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use super::{CaptureEvent, DEVICE_CHANGED_PREFIX};

/// How often the default-output watcher re-checks the OS default device.
const DEVICE_POLL: Duration = Duration::from_millis(1500);

/// Reads the OS default OUTPUT device's stable ID via cpal (independent of how
/// each backend actually captures), so the watcher is fully cross-platform.
/// Uses `id()` not the human-readable name — two devices can share a name, so
/// a name compare would miss an unplug→replug of a same-named device.
fn current_output_id() -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.id().ok())
        .map(|id| id.id().to_string())
}

/// Polls the OS default output device on a timer and, on a real change
/// (speakers→Bluetooth, default-sink switch, etc.), injects a
/// `DEVICE_CHANGED_PREFIX` error into the capture channel. The per-backend
/// streams bind to the default device once and don't follow it themselves —
/// without this a mid-call output switch silently strands capture on the old,
/// now-silent device. The recorder reacts by rebinding to the new device.
/// Fires once per watcher, then exits; the recorder's rebuild spawns a fresh
/// one with the new baseline.
struct DeviceWatch {
    stop: Arc<AtomicBool>,
}

impl DeviceWatch {
    fn spawn(tx: Sender<CaptureEvent>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            let mut baseline = current_output_id();
            loop {
                std::thread::sleep(DEVICE_POLL);
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                let now = current_output_id();
                // Fire only on a real switch between two KNOWN devices. A
                // transient `None` (device mid-swap) is ignored; `None -> Some`
                // just adopts the first resolvable device as the baseline
                // rather than counting as a change — otherwise an unresolved
                // default at startup would burn a needless rebind.
                match (&baseline, &now) {
                    (Some(prev), Some(curr)) if curr != prev => {
                        let _ = tx.send(CaptureEvent::Error(format!(
                            "{DEVICE_CHANGED_PREFIX}: default output is now {curr}"
                        )));
                        return;
                    }
                    (None, Some(_)) => baseline = now,
                    _ => {}
                }
            }
        });
        Self { stop }
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod stub;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug)]
pub enum SystemAudioError {
    /// System audio capture is not available on this platform/OS version.
    /// Callers should degrade to mic-only capture instead of failing.
    Unsupported(String),
    Failed(String),
}

impl std::fmt::Display for SystemAudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(msg) => write!(f, "system audio unsupported: {msg}"),
            Self::Failed(msg) => write!(f, "system audio capture failed: {msg}"),
        }
    }
}

impl std::error::Error for SystemAudioError {}

/// Captures system output audio (what plays through the speakers) and pushes
/// `CaptureEvent`s into `tx`, mirroring the `MicCapture` API shape.
pub struct SystemCapture {
    #[cfg(target_os = "macos")]
    inner: macos::MacSystemCapture,
    #[cfg(target_os = "windows")]
    inner: windows::WindowsSystemCapture,
    #[cfg(target_os = "linux")]
    inner: linux::LinuxSystemCapture,
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    inner: stub::StubCapture,
    watch: DeviceWatch,
}

impl SystemCapture {
    pub fn start(tx: Sender<CaptureEvent>) -> Result<Self, SystemAudioError> {
        // The backend captures the default output; the watcher uses a clone of
        // the same channel to flag a default-device change. Spawn it only after
        // the backend starts so a start failure doesn't leak a watcher thread.
        #[cfg(target_os = "macos")]
        let inner = macos::MacSystemCapture::start(tx.clone())?;
        #[cfg(target_os = "windows")]
        let inner = windows::WindowsSystemCapture::start(tx.clone())?;
        #[cfg(target_os = "linux")]
        let inner = linux::LinuxSystemCapture::start(tx.clone())?;
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let inner = stub::StubCapture::start(tx.clone())?;

        let watch = DeviceWatch::spawn(tx);
        Ok(Self { inner, watch })
    }

    pub fn stop(self) {
        self.watch.stop();
        self.inner.stop();
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use super::SystemCapture;
    use crate::shared::audio::CaptureEvent;

    /// Manual smoke test — triggers the macOS system-audio permission dialog
    /// on first run, so it must never run in CI. MUST be run while audio is
    /// playing (e.g. `while true; do afplay /System/Library/Sounds/Glass.aiff; done`),
    /// because a denied permission delivers frames of pure silence — this test
    /// asserts non-silence to distinguish the two:
    /// `cargo test --lib shared::audio::system -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn manual_system_tap_smoke() {
        let (tx, rx) = channel();
        let capture = SystemCapture::start(tx).expect("start system capture");
        std::thread::sleep(Duration::from_secs(3));
        capture.stop();

        let mut frames = 0usize;
        let mut samples = 0usize;
        let mut peak = 0.0f32;
        let mut format: Option<(u16, u32)> = None;
        let mut errors: Vec<String> = Vec::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                CaptureEvent::Frame(f) => {
                    frames += 1;
                    samples += f.samples.len();
                    peak = f.samples.iter().fold(peak, |p, s| p.max(s.abs()));
                    format.get_or_insert((f.channels, f.rate));
                }
                CaptureEvent::Error(e) => errors.push(e),
            }
        }
        println!(
            "system tap smoke: frames={frames} samples={samples} peak={peak} format={format:?} errors={errors:?}"
        );
        assert!(errors.is_empty(), "capture reported errors: {errors:?}");
        assert!(frames > 0, "no frames captured in 3s");
        assert!(
            peak > 1e-6,
            "tap delivered only silence — either nothing was playing during the \
             test, or System Audio Recording permission is denied for this \
             terminal (System Settings → Privacy & Security → Screen & System \
             Audio Recording)"
        );
    }
}

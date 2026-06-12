use std::sync::mpsc::Sender;

use super::CaptureEvent;

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
}

impl SystemCapture {
    pub fn start(tx: Sender<CaptureEvent>) -> Result<Self, SystemAudioError> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                inner: macos::MacSystemCapture::start(tx)?,
            })
        }
        #[cfg(target_os = "windows")]
        {
            Ok(Self {
                inner: windows::WindowsSystemCapture::start(tx)?,
            })
        }
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                inner: linux::LinuxSystemCapture::start(tx)?,
            })
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Ok(Self {
                inner: stub::StubCapture::start(tx)?,
            })
        }
    }

    pub fn stop(self) {
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

pub mod chunker;
pub mod mic;
pub mod resample;
mod stream;
pub mod system;

pub use chunker::Chunker;
pub use mic::MicCapture;
pub use resample::to_mono_16k;
pub use system::{SystemAudioError, SystemCapture};

pub struct AudioFrame {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub rate: u32,
}

/// Event emitted by an audio capture source. `Error` signals a fatal stream
/// failure (e.g. device disconnect); consumers must stop capture on receipt.
pub enum CaptureEvent {
    Frame(AudioFrame),
    Error(String),
}

/// Prefix of the `CaptureEvent::Error` raised when the OS default audio device
/// changes mid-capture. cpal's WASAPI backend emits this string natively; the
/// cross-platform `DeviceWatch` poller (system/mod.rs) emits it too. The
/// recorder's system drain matches on this prefix to rebind capture to the new
/// device instead of degrading to mic-only.
pub const DEVICE_CHANGED_PREFIX: &str = "Default audio device changed";

pub mod chunker;
pub mod mic;
pub mod resample;

pub use chunker::Chunker;
pub use mic::MicCapture;
pub use resample::to_mono_16k;

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

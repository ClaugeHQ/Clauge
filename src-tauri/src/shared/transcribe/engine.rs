//! whisper-rs inference wrapper.
//!
//! `Transcriber::load` is expensive (the whole ggml model is read and, on
//! macOS, uploaded to Metal/CoreML) — create ONE per recording and reuse
//! it across chunks; never load per-chunk.

use std::path::Path;
use std::time::Instant;

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadParams,
};

use crate::modes::workspace::models::TranscriptSegment;

pub struct Transcriber {
    ctx: WhisperContext,
    language: Option<String>,
    /// Path to the Silero VAD ggml model. When set, whisper.cpp's built-in
    /// VAD trims non-speech before decoding — fewer hallucinations + faster.
    vad_model_path: Option<String>,
}

impl Transcriber {
    pub fn load(
        model_path: &Path,
        language: Option<&str>,
        vad_model_path: Option<&str>,
    ) -> Result<Self, String> {
        let path_str = model_path
            .to_str()
            .ok_or_else(|| format!("Model path is not valid UTF-8: {:?}", model_path))?;

        let started = Instant::now();
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .map_err(|e| format!("Failed to load whisper model: {}", e))?;
        log::info!(
            "[transcribe] loaded whisper model {:?} in {:?}",
            model_path.file_name().unwrap_or_default(),
            started.elapsed()
        );

        Ok(Self {
            ctx,
            language: language
                .filter(|l| !l.is_empty() && *l != "auto")
                .map(str::to_owned),
            vad_model_path: vad_model_path
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
        })
    }

    /// `samples_16k_mono`: 16 kHz mono f32 PCM. `offset_ms` is added to
    /// whisper's chunk-relative timestamps so segments land on the
    /// recording's absolute timeline.
    pub fn transcribe(
        &mut self,
        samples_16k_mono: &[f32],
        offset_ms: u64,
        source: &str,
    ) -> Result<Vec<TranscriptSegment>, String> {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create whisper state: {}", e))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(self.language.as_deref());
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_timestamps(false);
        // Anti-hallucination. Meeting audio is full of pauses + room noise
        // where Whisper invents filler ("Thank you", "♪") or loops the last
        // phrase across the silent tail. These guards are the non-VAD defense:
        params.set_suppress_blank(true); //   drop blank/leading-space starts
        params.set_suppress_nst(true); //     drop non-speech tokens (♪, [sound])
        params.set_no_context(true); //       don't carry text across chunks → no repeat cascade
        params.set_temperature(0.0); //       deterministic first pass…
        params.set_temperature_inc(0.2); //   …with fallback ladder on low confidence
        params.set_logprob_thold(-1.0); //    low avg-logprob → failed decode → fallback
        params.set_entropy_thold(2.4); //     compression-ratio analog → kills repetition loops
        // (no_speech_thold is intentionally NOT set — it's a documented no-op
        // in whisper-rs 0.16. Silent chunks are dropped upstream by the RMS
        // gate in recorder.rs.)

        // Built-in Silero VAD: trims non-speech regions before decoding, so
        // whisper never sees the silent/noise spans that make it hallucinate
        // ("Thank you", looped phrases), and skips decoding silence entirely.
        // Only enabled when a VAD model is present (else plain decode).
        if let Some(vad) = self.vad_model_path.as_deref() {
            params.set_vad_model_path(Some(vad));
            let mut vad_params = WhisperVadParams::new();
            vad_params.set_threshold(0.5);
            vad_params.set_min_speech_duration(250);
            vad_params.set_min_silence_duration(100);
            vad_params.set_speech_pad(64);
            params.set_vad_params(vad_params);
            params.enable_vad(true);
        }

        state
            .full(params, samples_16k_mono)
            .map_err(|e| format!("Transcription failed: {}", e))?;

        let n = state.full_n_segments();
        let mut segments = Vec::with_capacity(n.max(0) as usize);
        for i in 0..n {
            let Some(seg) = state.get_segment(i) else { continue };
            let text = match seg.to_str_lossy() {
                Ok(t) => t.trim().to_string(),
                Err(_) => continue,
            };
            if text.is_empty() {
                continue;
            }
            segments.push(TranscriptSegment {
                start_ms: centiseconds_to_ms(seg.start_timestamp()) + offset_ms,
                end_ms: centiseconds_to_ms(seg.end_timestamp()) + offset_ms,
                source: source.to_string(),
                text,
            });
        }

        // NOTE: do NOT pin a per-recording language from the first chunk's
        // auto-detection. Whisper's per-chunk detect is low-confidence on short
        // audio (and meaningless on `.en` models, which always decode English),
        // so a garbage first detection (e.g. "sw" @ p=0.01) would be forced on
        // the whole call — wrong-language decoding then drops the quieter source
        // entirely. Leave `self.language` as set: an explicit user language, or
        // None for per-chunk auto-detect.
        Ok(segments)
    }
}

fn centiseconds_to_ms(cs: i64) -> u64 {
    (cs.max(0) as u64) * 10
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn centiseconds_map_to_milliseconds() {
        assert_eq!(centiseconds_to_ms(0), 0);
        assert_eq!(centiseconds_to_ms(150), 1500);
        assert_eq!(centiseconds_to_ms(-5), 0);
    }

    /// End-to-end smoke test: downloads ggml-tiny.bin (~75 MB) into the
    /// system temp dir, synthesizes speech with macOS `say`, and asserts
    /// the transcript contains "hello". Run with:
    /// `cargo test --release transcribe_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn transcribe_smoke_hello_world() {
        let model = std::env::temp_dir().join("clauge-test-models/ggml-tiny.bin");
        if !model.is_file() {
            std::fs::create_dir_all(model.parent().unwrap()).unwrap();
            let url = crate::shared::transcribe::models::download_url("tiny");
            let status = Command::new("/usr/bin/curl")
                .args(["-sSfL", "-o"])
                .arg(&model)
                .arg(&url)
                .status()
                .expect("curl");
            assert!(status.success(), "model download failed");
        }
        assert!(
            crate::shared::transcribe::models::validate_magic(&model),
            "downloaded model failed ggml magic validation"
        );

        let aiff = std::env::temp_dir().join("clauge-test-hello.aiff");
        let wav = std::env::temp_dir().join("clauge-test-hello-16k.wav");
        assert!(Command::new("/usr/bin/say")
            .arg("-o")
            .arg(&aiff)
            .arg("hello world this is a test")
            .status()
            .expect("say")
            .success());
        assert!(Command::new("/usr/bin/afconvert")
            .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
            .arg(&aiff)
            .arg(&wav)
            .status()
            .expect("afconvert")
            .success());

        let mut reader = hound::WavReader::open(&wav).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().channels, 1);
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let mut transcriber = Transcriber::load(&model, Some("en"), None).unwrap();
        let segments = transcriber.transcribe(&samples, 5_000, "mic").unwrap();
        let joined = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        println!("transcript: {joined:?}");
        println!("segments: {segments:?}");

        assert!(
            joined.to_lowercase().contains("hello"),
            "transcript missing 'hello': {joined:?}"
        );
        assert!(segments.iter().all(|s| s.start_ms >= 5_000));
        assert!(segments.iter().all(|s| s.source == "mic"));

        let _ = std::fs::remove_file(&aiff);
        let _ = std::fs::remove_file(&wav);
    }
}

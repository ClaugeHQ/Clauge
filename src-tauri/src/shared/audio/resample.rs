use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

pub const TARGET_RATE: u32 = 16000;

const RESAMPLER_CHUNK_SIZE: usize = 1024;
const RESAMPLER_SUB_CHUNKS: usize = 2;

/// Downmixes interleaved samples to mono and resamples to 16 kHz.
///
/// Must be called per accumulated chunk (~seconds of audio), never per ~10ms
/// frame: each call pads and flushes a fresh resampler, so per-frame calls
/// cause edge ringing and phase discontinuities at every call boundary.
pub fn to_mono_16k(samples: &[f32], channels: u16, rate: u32) -> Vec<f32> {
    debug_assert!(samples.len() % channels as usize == 0);
    let mono = downmix(samples, channels);
    if rate == TARGET_RATE || mono.is_empty() {
        return mono;
    }
    resample_to_16k(&mono, rate)
}

/// Channel-averages interleaved samples to mono at the native rate. Safe
/// per-frame, unlike `to_mono_16k` — no resampler state is involved.
pub fn downmix(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let ch = channels as usize;
    samples
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

fn resample_to_16k(mono: &[f32], rate: u32) -> Vec<f32> {
    let frames = mono.len();
    let mut resampler = Fft::<f32>::new(
        rate as usize,
        TARGET_RATE as usize,
        RESAMPLER_CHUNK_SIZE,
        RESAMPLER_SUB_CHUNKS,
        1,
        FixedSync::Both,
    )
    .unwrap_or_else(|e| panic!("failed to create resampler ({rate} Hz -> {TARGET_RATE} Hz): {e}"));
    let input = InterleavedSlice::new(mono, 1, frames).expect("input adapter");
    let capacity = resampler.process_all_needed_output_len(frames);
    let mut out_data = vec![0.0f32; capacity];
    let mut output = InterleavedSlice::new_mut(&mut out_data, 1, capacity).expect("output adapter");
    let (_, written) = resampler
        .process_all_into_buffer(&input, &mut output, frames, None)
        .unwrap_or_else(|e| panic!("resampling failed ({rate} Hz -> {TARGET_RATE} Hz): {e}"));
    out_data.truncate(written);
    out_data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / rate as f32).sin())
            .collect()
    }

    fn interleave_stereo(mono: &[f32]) -> Vec<f32> {
        mono.iter().flat_map(|&s| [s, s]).collect()
    }

    #[test]
    fn stereo_48k_one_second_gives_about_16k_samples() {
        let input = interleave_stereo(&sine(48000, 1.0));
        let out = to_mono_16k(&input, 2, 48000);
        let expected = 16000.0;
        let deviation = (out.len() as f32 - expected).abs() / expected;
        assert!(deviation < 0.01, "got {} samples", out.len());
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn mono_16k_passthrough_is_exact() {
        let input = sine(16000, 0.5);
        let out = to_mono_16k(&input, 1, 16000);
        assert_eq!(out, input);
    }

    #[test]
    fn mono_44k1_gives_about_16k_samples() {
        let input = sine(44100, 1.0);
        let out = to_mono_16k(&input, 1, 44100);
        let expected = 16000.0;
        let deviation = (out.len() as f32 - expected).abs() / expected;
        assert!(deviation < 0.01, "got {} samples", out.len());
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn tiny_input_48k_mono_still_produces_output() {
        let input = sine(48000, 0.01); // 480 samples, ~10ms
        let out = to_mono_16k(&input, 1, 48000);
        let expected = 160.0;
        let deviation = (out.len() as f32 - expected).abs() / expected;
        assert!(deviation <= 0.25, "got {} samples", out.len());
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn stereo_downmix_averages_channels() {
        let input = vec![1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        let out = to_mono_16k(&input, 2, 16000);
        assert_eq!(out, vec![0.5, 0.5, 0.0]);
    }
}

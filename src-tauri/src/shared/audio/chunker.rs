pub const SILENCE_RMS_THRESHOLD: f32 = 0.01;
pub const SILENCE_WINDOW_SECS: f32 = 0.3;

pub struct Chunker {
    buffer: Vec<f32>,
    target_len: usize,
    max_len: usize,
    silence_window_len: usize,
}

impl Chunker {
    pub fn new(sample_rate: u32, target_secs: f32, max_secs: f32) -> Self {
        debug_assert!(target_secs <= max_secs);
        Self {
            buffer: Vec::new(),
            target_len: (sample_rate as f32 * target_secs) as usize,
            max_len: (sample_rate as f32 * max_secs) as usize,
            silence_window_len: (sample_rate as f32 * SILENCE_WINDOW_SECS) as usize,
        }
    }

    /// Buffers samples and emits a chunk at a silence boundary past
    /// `target_secs`, or once the buffer reaches `max_secs`.
    ///
    /// An emitted chunk may exceed `max_secs` by up to one pushed frame, so
    /// callers must leave headroom below hard downstream limits (e.g. keep
    /// `max_secs` under whisper's 30s window).
    pub fn push(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        self.buffer.extend_from_slice(samples);
        if self.buffer.len() >= self.max_len {
            return Some(std::mem::take(&mut self.buffer));
        }
        if self.buffer.len() >= self.target_len && self.trailing_silence() {
            return Some(std::mem::take(&mut self.buffer));
        }
        None
    }

    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.buffer))
        }
    }

    fn trailing_silence(&self) -> bool {
        if self.buffer.len() < self.silence_window_len {
            return false;
        }
        let tail = &self.buffer[self.buffer.len() - self.silence_window_len..];
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        rms < SILENCE_RMS_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16000;

    fn tone(secs: f32) -> Vec<f32> {
        let n = (RATE as f32 * secs) as usize;
        (0..n)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / RATE as f32).sin() * 0.5)
            .collect()
    }

    fn silence(secs: f32) -> Vec<f32> {
        vec![0.0; (RATE as f32 * secs) as usize]
    }

    #[test]
    fn emits_at_silence_boundary_after_target() {
        let mut c = Chunker::new(RATE, 1.0, 3.0);
        assert!(c.push(&tone(1.0)).is_none());
        let chunk = c.push(&silence(0.4)).expect("chunk at silence boundary");
        assert_eq!(chunk.len(), (RATE as f32 * 1.4) as usize);
        assert!(c.flush().is_none());
    }

    #[test]
    fn continuous_tone_only_cuts_at_max() {
        let mut c = Chunker::new(RATE, 1.0, 2.0);
        let step = tone(0.1);
        let mut pushed = 0usize;
        let max_len = (RATE * 2) as usize;
        loop {
            pushed += step.len();
            match c.push(&step) {
                None => assert!(pushed < max_len, "expected emit at max_secs"),
                Some(chunk) => {
                    assert!(pushed >= max_len, "emitted before max_secs at {pushed}");
                    assert_eq!(chunk.len(), pushed);
                    break;
                }
            }
        }
    }

    #[test]
    fn flush_returns_remainder_and_empties() {
        let mut c = Chunker::new(RATE, 1.0, 2.0);
        assert!(c.push(&tone(0.5)).is_none());
        let rest = c.flush().expect("remainder");
        assert_eq!(rest.len(), (RATE / 2) as usize);
        assert!(c.flush().is_none());
    }

    #[test]
    fn odd_sized_pushes_lose_no_samples() {
        let mut c = Chunker::new(RATE, 1.0, 2.0);
        let data: Vec<f32> = tone(3.0)
            .into_iter()
            .chain(silence(0.5))
            .chain(tone(1.3))
            .collect();
        let mut pushed = 0usize;
        let mut emitted = 0usize;
        for piece in data.chunks(331) {
            pushed += piece.len();
            if let Some(chunk) = c.push(piece) {
                emitted += chunk.len();
            }
        }
        if let Some(rest) = c.flush() {
            emitted += rest.len();
        }
        assert_eq!(emitted, pushed);
    }
}

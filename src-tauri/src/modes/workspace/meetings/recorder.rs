//! Recording orchestrator: audio capture → chunking → whisper → DB.
//!
//! Mic and system audio run as separate pipelines (no mixing) so every
//! transcript segment carries source attribution: "mic" is the user,
//! "system" is the other participants. Both feed ONE transcriber thread
//! (one loaded model per recording) and ONE DB flush task (the single
//! writer `repo::append_segments` assumes).

use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use crate::modes::workspace::meetings::repo;
use crate::modes::workspace::meetings::widget;
use crate::modes::workspace::models::TranscriptSegment;
use crate::shared::audio::resample::downmix;
use crate::shared::audio::{
    to_mono_16k, AudioFrame, CaptureEvent, Chunker, MicCapture, SystemAudioError, SystemCapture,
    DEVICE_CHANGED_PREFIX,
};
use crate::shared::transcribe::engine::Transcriber;
use crate::shared::transcribe::models as whisper_models;

pub const DEFAULT_MODEL: &str = "base";
pub const DEFAULT_LANGUAGE: &str = "auto";

const SOURCE_MIC: &str = "mic";
const SOURCE_SYSTEM: &str = "system";

const CHUNK_TARGET_SECS: f32 = 20.0;
/// Chunker may overshoot max by one frame; 28s leaves headroom under
/// whisper's 30s window.
const CHUNK_MAX_SECS: f32 = 28.0;

/// Whole-chunk RMS below this is skipped before whisper: WASAPI loopback
/// gaps and muted mics produce long all-silence chunks that would only
/// hallucinate text and burn CPU.
const SILENT_CHUNK_RMS: f32 = 0.005;

/// A denied macOS System Audio Recording permission makes Core Audio
/// deliver all-zero frames instead of an error, so the system stream looks
/// healthy while every chunk gets RMS-skipped. If nothing but silence has
/// arrived for this long, warn the user once.
const SYSTEM_SILENCE_WARN_MS: u64 = 60_000;

/// Faster first line of defense for the same denied-silence symptom: a
/// Core Audio process tap created BEFORE the TCC grant landed delivers
/// zeros forever, and only a full teardown + recreate of the tap and
/// aggregate device recovers (Apple forum 825780). If the system source
/// has produced nothing but silent chunks for this long, restart
/// SystemCapture once — the recreated tap is post-grant and unmutes. The
/// 60s warning above stays as the second line, firing only if the stream
/// is still silent after the restart.
const SYSTEM_SILENCE_RESTART_MS: u64 = 15_000;

const FLUSH_MAX_BUFFERED: usize = 50;
const FLUSH_INTERVAL: Duration = Duration::from_secs(30);
/// Hard ceiling on unflushed segments while the DB keeps failing; beyond
/// this the oldest are dropped so a dead DB can't grow memory unbounded.
const FLUSH_BUFFER_CAP: usize = 5000;
/// Warn the user after this many consecutive flush failures (and every
/// multiple thereafter, not on every failed tick).
const FLUSH_FAILURE_WARN_EVERY: u32 = 3;
/// A long final chunk can take seconds of whisper CPU after stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
/// Bounded wait for the capture teardown itself (cpal stream drops can
/// wedge on a hung audio driver). Past this the threads are leaked so
/// stop can finish instead of hanging forever.
const CAPTURE_STOP_TIMEOUT: Duration = Duration::from_secs(10);
const SEGMENT_QUEUE_CAPACITY: usize = 256;

/// 8 pending 16k mono chunks ≈ 2.5 min of audio (~7.7 MB worst case). If
/// whisper runs sub-real-time the queue stays bounded; newer chunks are
/// dropped instead of growing the backlog without limit.
const CHUNK_QUEUE_CAPACITY: usize = 8;
/// Warn the user on the first dropped chunk, then every Nth after that.
const DROP_WARN_EVERY: u64 = 3;

/// How many times one recording will rebind system capture after a default
/// output-device change before giving up and degrading to mic-only. Bounds a
/// restart storm if device-change errors ever arrive in a tight loop, while
/// still surviving a handful of genuine mid-call output switches.
const MAX_DEVICE_RESTARTS: u32 = 4;

const EVT_STARTED: &str = "meetings:recording-started";
const EVT_STOPPED: &str = "meetings:recording-stopped";
const EVT_SEGMENT: &str = "meetings:transcript-segment";
const EVT_ERROR: &str = "meetings:recording-error";
const EVT_WARNING: &str = "meetings:recording-warning";

type ActiveSlot = Arc<Mutex<Option<ActiveRecording>>>;

#[derive(Default)]
pub struct RecorderState {
    active: ActiveSlot,
}

struct ActiveRecording {
    meeting_id: String,
    started_instant: Instant,
    started_at: String,
    source_app: Option<String>,
    system_audio: bool,
    /// `None` while a stop is in flight (handles taken by the stopper).
    mic: Option<MicCapture>,
    system: Option<SystemCapture>,
    done_rx: Option<tokio::sync::oneshot::Receiver<()>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    pub recording: bool,
    /// True while a stop is in flight: the capture handles are already
    /// taken but the pipeline is still finalizing (≤ `STOP_TIMEOUT`).
    pub stopping: bool,
    pub meeting_id: Option<String>,
    pub started_at: Option<String>,
    pub source_app: Option<String>,
    pub system_audio: bool,
    pub elapsed_secs: u64,
}

impl RecorderState {
    pub fn status(&self) -> RecorderStatus {
        match self.active.lock().as_ref() {
            Some(rec) => RecorderStatus {
                recording: true,
                stopping: rec.mic.is_none(),
                meeting_id: Some(rec.meeting_id.clone()),
                started_at: Some(rec.started_at.clone()),
                source_app: rec.source_app.clone(),
                system_audio: rec.system_audio,
                elapsed_secs: rec.started_instant.elapsed().as_secs(),
            },
            None => RecorderStatus {
                recording: false,
                stopping: false,
                meeting_id: None,
                started_at: None,
                source_app: None,
                system_audio: false,
                elapsed_secs: 0,
            },
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingStarted<'a> {
    meeting_id: &'a str,
    started_at: &'a str,
    source_app: Option<&'a str>,
    system_audio: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeetingEvent<'a> {
    meeting_id: &'a str,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingMessage<'a> {
    meeting_id: &'a str,
    message: &'a str,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SegmentEvent<'a> {
    meeting_id: &'a str,
    segment: &'a TranscriptSegment,
}

pub async fn start_recording(
    app: AppHandle,
    source_app: Option<String>,
    model: String,
    language: String,
) -> Result<String, String> {
    let active = app.state::<RecorderState>().active.clone();
    if active.lock().is_some() {
        return Err("already recording".to_string());
    }
    if !whisper_models::is_downloaded(&app, &model) {
        return Err("model_missing".to_string());
    }

    let model_path = whisper_models::model_path(&app, &model)?;
    let lang = language.clone();
    let transcriber =
        tauri::async_runtime::spawn_blocking(move || Transcriber::load(&model_path, Some(&lang)))
            .await
            .map_err(|e| format!("transcriber load task failed: {e}"))??;

    let pool = app.state::<SqlitePool>().inner().clone();
    let title = default_title(chrono::Local::now());
    let meeting = repo::insert_meeting(&pool, &title, source_app.as_deref(), &language)
        .await
        .map_err(|e| format!("failed to create meeting: {e}"))?;
    let meeting_id = meeting.id.clone();

    let (mic_tx, mic_rx) = channel();
    let mic = match MicCapture::start(mic_tx) {
        Ok(m) => m,
        Err(e) => {
            let _ = repo::delete_meeting(&pool, &meeting_id).await;
            return Err(format!("mic capture failed: {e}"));
        }
    };

    let (sys_tx, sys_rx) = channel();
    let mut mic_only_reason: Option<String> = None;
    let system = match SystemCapture::start(sys_tx) {
        Ok(s) => Some(s),
        Err(SystemAudioError::Unsupported(msg)) => {
            log::info!("[meetings] system audio unsupported, recording mic-only: {msg}");
            mic_only_reason = Some(msg);
            None
        }
        Err(SystemAudioError::Failed(msg)) => {
            log::warn!("[meetings] system audio failed to start, recording mic-only: {msg}");
            mic_only_reason = Some(msg);
            None
        }
    };
    let system_audio = system.is_some();

    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let recording = ActiveRecording {
        meeting_id: meeting_id.clone(),
        started_instant: Instant::now(),
        started_at: meeting.started_at.clone(),
        source_app: source_app.clone(),
        system_audio,
        mic: Some(mic),
        system,
        done_rx: Some(done_rx),
    };
    let rejected = {
        let mut slot = active.lock();
        if slot.is_some() {
            Some(recording)
        } else {
            *slot = Some(recording);
            None
        }
    };
    if let Some(recording) = rejected {
        let ActiveRecording { mic, system, .. } = recording;
        let _ = tauri::async_runtime::spawn_blocking(move || stop_captures(mic, system)).await;
        let _ = repo::delete_meeting(&pool, &meeting_id).await;
        return Err("already recording".to_string());
    }

    let (chunk_tx, chunk_rx) = sync_channel::<ChunkJob>(CHUNK_QUEUE_CAPACITY);
    let (seg_tx, seg_rx) = tokio::sync::mpsc::channel::<TranscriptSegment>(SEGMENT_QUEUE_CAPACITY);

    {
        let (app, active, id) = (app.clone(), active.clone(), meeting_id.clone());
        let tx = chunk_tx.clone();
        std::thread::Builder::new()
            .name("meeting-mic-drain".into())
            .spawn(move || run_mic_drain(app, active, id, mic_rx, tx))
            .map_err(|e| format!("failed to spawn mic drain thread: {e}"))?;
    }
    if system_audio {
        let (app, active, id) = (app.clone(), active.clone(), meeting_id.clone());
        std::thread::Builder::new()
            .name("meeting-system-drain".into())
            .spawn(move || run_system_drain(app, active, id, sys_rx, chunk_tx))
            .map_err(|e| format!("failed to spawn system drain thread: {e}"))?;
    } else {
        drop(chunk_tx);
    }

    {
        let (app, id) = (app.clone(), meeting_id.clone());
        std::thread::Builder::new()
            .name("meeting-transcribe".into())
            .spawn(move || {
                run_transcriber(transcriber, chunk_rx, |segment| {
                    let _ = app.emit(
                        EVT_SEGMENT,
                        SegmentEvent { meeting_id: &id, segment: &segment },
                    );
                    if seg_tx.blocking_send(segment).is_err() {
                        log::warn!("[meetings] segment flush channel closed early");
                    }
                });
            })
            .map_err(|e| format!("failed to spawn transcriber thread: {e}"))?;
    }

    tauri::async_runtime::spawn(run_flush(
        app.clone(),
        pool,
        meeting_id.clone(),
        seg_rx,
        active,
        done_tx,
    ));

    // Surface the mic-only degradation to the user (toast in the main
    // window via the warning listener), not just the log.
    if let Some(reason) = &mic_only_reason {
        let _ = app.emit(
            EVT_WARNING,
            RecordingMessage {
                meeting_id: &meeting_id,
                message: &format!(
                    "System audio unavailable — recording microphone only ({reason})"
                ),
            },
        );
    }
    let _ = app.emit(
        EVT_STARTED,
        RecordingStarted {
            meeting_id: &meeting_id,
            started_at: &meeting.started_at,
            source_app: source_app.as_deref(),
            system_audio,
        },
    );
    // The detect poller can emit call-ended and close the widget while this
    // start is still in flight, leaving a recording with no visible stop
    // affordance. Detected starts carry a source app, so reopen the pill for
    // them (`open_widget` is a no-op show if it already exists); manual
    // starts (source_app: None) never open it.
    if source_app.is_some() {
        widget::open_widget(&app);
    }
    Ok(meeting_id)
}

pub async fn stop_recording(app: AppHandle) -> Result<String, String> {
    let active = app.state::<RecorderState>().active.clone();
    let (meeting_id, mic, system, done_rx) = {
        let mut slot = active.lock();
        let rec = slot.as_mut().ok_or_else(|| "not recording".to_string())?;
        (
            rec.meeting_id.clone(),
            rec.mic.take(),
            rec.system.take(),
            rec.done_rx.take(),
        )
    };
    if mic.is_none() && system.is_none() && done_rx.is_none() {
        return Err("not recording".to_string());
    }

    if mic.is_some() || system.is_some() {
        let join = tauri::async_runtime::spawn_blocking(move || stop_captures(mic, system));
        match tokio::time::timeout(CAPTURE_STOP_TIMEOUT, join).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                // The slot still holds this recording (with the handles
                // already taken) — clear it before bailing or every later
                // stop/start sees a phantom "already recording".
                let mut slot = active.lock();
                if slot.as_ref().is_some_and(|r| r.meeting_id == meeting_id) {
                    *slot = None;
                }
                return Err(format!("capture stop task failed: {e}"));
            }
            Err(_) => {
                log::error!(
                    "[meetings] capture teardown timed out after {CAPTURE_STOP_TIMEOUT:?} — \
                     leaking capture threads and finalizing anyway"
                );
            }
        }
    }

    if let Some(done_rx) = done_rx {
        if tokio::time::timeout(STOP_TIMEOUT, done_rx).await.is_err() {
            log::warn!("[meetings] stop timed out waiting for pipeline finalization");
        }
        // Normally the flush task already cleared the slot before signaling
        // done; this also releases it after a timeout or a torn-down pipeline
        // so the recorder can't get stuck reporting "already recording".
        let mut slot = active.lock();
        if slot.as_ref().is_some_and(|r| r.meeting_id == meeting_id) {
            *slot = None;
        }
    }
    Ok(meeting_id)
}

fn stop_captures(mic: Option<MicCapture>, system: Option<SystemCapture>) {
    if let Some(m) = mic {
        m.stop();
    }
    if let Some(s) = system {
        s.stop();
    }
}

fn default_title(now: chrono::DateTime<chrono::Local>) -> String {
    format!("Meeting — {}", now.format("%Y-%m-%d %H:%M"))
}

// --- drain pipelines (capture frames → mono chunks → transcriber queue) ---

struct ChunkJob {
    source: &'static str,
    samples_16k: Vec<f32>,
    offset_ms: u64,
}

/// Maps emitted chunk boundaries to absolute recording time. The chunker
/// is lossless and in-order, so cumulative emitted samples at native rate
/// equal the next chunk's start offset. `rebase` carries the accumulated
/// milliseconds across a device-change restart with a different rate.
struct OffsetTracker {
    base_ms: u64,
    emitted_samples: u64,
    rate: u32,
}

impl OffsetTracker {
    fn new(rate: u32) -> Self {
        Self { base_ms: 0, emitted_samples: 0, rate }
    }

    fn chunk_start_ms(&self) -> u64 {
        self.base_ms + self.emitted_samples * 1000 / self.rate as u64
    }

    fn advance(&mut self, samples: usize) {
        self.emitted_samples += samples as u64;
    }

    fn rebase(&mut self, new_rate: u32) {
        self.rebase_to_ms(self.chunk_start_ms(), new_rate);
    }

    fn rebase_to_ms(&mut self, base_ms: u64, new_rate: u32) {
        self.base_ms = base_ms;
        self.emitted_samples = 0;
        self.rate = new_rate;
    }
}

struct DrainPipeline {
    source: &'static str,
    target_secs: f32,
    max_secs: f32,
    chunk_tx: SyncSender<ChunkJob>,
    chunker: Option<Chunker>,
    offset: Option<OffsetTracker>,
    /// Offset base applied when the first frame arrives, set by a
    /// `rebase_to_ms` that happens before any frame has been seen.
    pending_base_ms: u64,
    dropped_chunks: u64,
    dropped_ms: u64,
    on_backlog: Option<Box<dyn FnMut(&str) + Send>>,
    /// Fired once if the stream is still all-silence past
    /// `SYSTEM_SILENCE_WARN_MS`; disarmed by the first audible chunk.
    on_silence: Option<Box<dyn FnOnce() + Send>>,
    /// Armed by `with_silence_restart`; disarmed by the first audible chunk
    /// or by requesting one restart, so at most one fires per recording.
    silence_restart_armed: bool,
    /// A restart request waiting to be consumed by `take_silence_restart`.
    silence_restart_pending: bool,
}

impl DrainPipeline {
    fn new(source: &'static str, chunk_tx: SyncSender<ChunkJob>) -> Self {
        Self::with_chunking(source, chunk_tx, CHUNK_TARGET_SECS, CHUNK_MAX_SECS)
    }

    fn with_chunking(
        source: &'static str,
        chunk_tx: SyncSender<ChunkJob>,
        target_secs: f32,
        max_secs: f32,
    ) -> Self {
        Self {
            source,
            target_secs,
            max_secs,
            chunk_tx,
            chunker: None,
            offset: None,
            pending_base_ms: 0,
            dropped_chunks: 0,
            dropped_ms: 0,
            on_backlog: None,
            on_silence: None,
            silence_restart_armed: false,
            silence_restart_pending: false,
        }
    }

    fn with_backlog_warning(mut self, warn: impl FnMut(&str) + Send + 'static) -> Self {
        self.on_backlog = Some(Box::new(warn));
        self
    }

    fn with_silence_warning(mut self, warn: impl FnOnce() + Send + 'static) -> Self {
        self.on_silence = Some(Box::new(warn));
        self
    }

    fn with_silence_restart(mut self) -> Self {
        self.silence_restart_armed = true;
        self
    }

    /// True once when sustained initial silence has crossed
    /// `SYSTEM_SILENCE_RESTART_MS`; the drain loop reacts by recreating
    /// the capture.
    fn take_silence_restart(&mut self) -> bool {
        std::mem::take(&mut self.silence_restart_pending)
    }

    /// Wall-clock silence guard for a capture that delivers NO frames at all.
    /// The sample-based path in `emit` only advances while frames arrive, so a
    /// macOS tap that's dead silent because it predates the TCC grant (delivers
    /// zero callbacks, not zero-valued frames) would never trip the restart or
    /// the warning. `quiet_ms` is the wall time since the last frame. Returns
    /// true once when the capture should be recreated; fires the warning at
    /// `SYSTEM_SILENCE_WARN_MS`. Shares the same one-shot arming/warning state
    /// as the sample path, so the two can't double-fire. No-op for pipelines
    /// without silence handling (mic, and non-macOS system).
    fn note_no_frames(&mut self, quiet_ms: u64) -> bool {
        if self.silence_restart_armed && quiet_ms >= SYSTEM_SILENCE_RESTART_MS {
            self.silence_restart_armed = false;
            return true;
        }
        if quiet_ms >= SYSTEM_SILENCE_WARN_MS {
            if let Some(warn) = self.on_silence.take() {
                warn();
            }
        }
        false
    }

    /// Errs only when the transcriber side has gone away.
    fn handle_frame(&mut self, frame: &AudioFrame) -> Result<(), ()> {
        if frame.samples.is_empty() || frame.rate == 0 {
            return Ok(());
        }
        self.ensure_rate(frame.rate)?;
        // Per-frame: channel-average only, at native rate. Resampling is
        // strictly per-chunk (see `to_mono_16k` docs).
        let mono = downmix(&frame.samples, frame.channels);
        if let Some(chunk) = self.chunker.as_mut().expect("chunker initialized").push(&mono) {
            self.emit(chunk)?;
        }
        Ok(())
    }

    fn ensure_rate(&mut self, rate: u32) -> Result<(), ()> {
        match self.offset.as_ref().map(|o| o.rate) {
            Some(r) if r == rate => Ok(()),
            Some(_) => {
                self.flush()?;
                self.offset.as_mut().expect("offset initialized").rebase(rate);
                self.chunker = Some(Chunker::new(rate, self.target_secs, self.max_secs));
                Ok(())
            }
            None => {
                let mut offset = OffsetTracker::new(rate);
                offset.rebase_to_ms(self.pending_base_ms, rate);
                self.offset = Some(offset);
                self.chunker = Some(Chunker::new(rate, self.target_secs, self.max_secs));
                Ok(())
            }
        }
    }

    /// Re-anchors chunk offsets to the recording's wall-clock elapsed time
    /// after a capture restart, so the gap between the stream error and the
    /// new stream's first frame does not shift segments earlier.
    fn rebase_to_ms(&mut self, elapsed_ms: u64) {
        let _ = self.flush();
        match self.offset.as_mut() {
            Some(offset) => {
                let rate = offset.rate;
                offset.rebase_to_ms(elapsed_ms, rate);
            }
            None => self.pending_base_ms = elapsed_ms,
        }
    }

    fn flush(&mut self) -> Result<(), ()> {
        if let Some(chunk) = self.chunker.as_mut().and_then(Chunker::flush) {
            self.emit(chunk)?;
        }
        Ok(())
    }

    fn emit(&mut self, chunk: Vec<f32>) -> Result<(), ()> {
        let offset = self.offset.as_mut().expect("offset initialized");
        let offset_ms = offset.chunk_start_ms();
        let rate = offset.rate;
        offset.advance(chunk.len());
        let end_ms = offset.chunk_start_ms();
        if is_silent_chunk(&chunk) {
            if self.silence_restart_armed && end_ms >= SYSTEM_SILENCE_RESTART_MS {
                self.silence_restart_armed = false;
                self.silence_restart_pending = true;
            }
            if end_ms >= SYSTEM_SILENCE_WARN_MS {
                if let Some(warn) = self.on_silence.take() {
                    warn();
                }
            }
            return Ok(());
        }
        self.on_silence = None;
        self.silence_restart_armed = false;
        let samples_16k = to_mono_16k(&chunk, 1, rate);
        let duration_ms = samples_16k.len() as u64 * 1000 / 16_000;
        match self
            .chunk_tx
            .try_send(ChunkJob { source: self.source, samples_16k, offset_ms })
        {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.dropped_chunks += 1;
                self.dropped_ms += duration_ms;
                log::warn!(
                    "[meetings] transcription queue full, dropped {} chunk @ {offset_ms}ms ({} dropped so far)",
                    self.source,
                    self.dropped_chunks
                );
                if self.dropped_chunks % DROP_WARN_EVERY == 1 {
                    let message = format!(
                        "transcription falling behind — skipped {}s of audio",
                        self.dropped_ms / 1000
                    );
                    if let Some(warn) = self.on_backlog.as_mut() {
                        warn(&message);
                    }
                }
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => Err(()),
        }
    }
}

enum DrainExit {
    Closed,
    StreamError(String),
    /// The pipeline heard nothing but silence past
    /// `SYSTEM_SILENCE_RESTART_MS` and wants the capture recreated.
    SilenceRestart,
}

/// How often the drain wakes to run the wall-clock silence watchdog when no
/// frames are arriving. Short relative to the 15s restart / 60s warn windows.
const DRAIN_POLL: Duration = Duration::from_millis(500);

fn drain_until_exit(pipeline: &mut DrainPipeline, rx: &Receiver<CaptureEvent>) -> DrainExit {
    let mut last_frame = Instant::now();
    loop {
        match rx.recv_timeout(DRAIN_POLL) {
            Ok(CaptureEvent::Frame(frame)) => {
                last_frame = Instant::now();
                if pipeline.handle_frame(&frame).is_err() {
                    return DrainExit::Closed;
                }
                if pipeline.take_silence_restart() {
                    return DrainExit::SilenceRestart;
                }
            }
            Ok(CaptureEvent::Error(msg)) => return DrainExit::StreamError(msg),
            // No frame this tick. A dead-silent tap (denied/pre-grant) delivers
            // NO callbacks, so the sample-based silence path never runs — guard
            // on wall-clock time since the last frame instead.
            Err(RecvTimeoutError::Timeout) => {
                let quiet_ms = last_frame.elapsed().as_millis() as u64;
                if pipeline.note_no_frames(quiet_ms) {
                    return DrainExit::SilenceRestart;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return DrainExit::Closed,
        }
    }
}

/// A mic stream error is fatal for the whole recording: stop both
/// captures so every pipeline drains, flushes, and finalizes normally.
fn run_mic_drain(
    app: AppHandle,
    active: ActiveSlot,
    meeting_id: String,
    rx: Receiver<CaptureEvent>,
    chunk_tx: SyncSender<ChunkJob>,
) {
    let mut pipeline = DrainPipeline::new(SOURCE_MIC, chunk_tx)
        .with_backlog_warning(backlog_warner(app.clone(), meeting_id.clone()));
    if let DrainExit::StreamError(msg) = drain_until_exit(&mut pipeline, &rx) {
        log::error!("[meetings] mic stream failed, stopping recording: {msg}");
        let _ = app.emit(
            EVT_ERROR,
            RecordingMessage { meeting_id: &meeting_id, message: &msg },
        );
        widget::close_widget(&app);
        let (mic, system) = {
            let mut slot = active.lock();
            match slot.as_mut() {
                Some(rec) => (rec.mic.take(), rec.system.take()),
                None => (None, None),
            }
        };
        stop_captures(mic, system);
    }
    let _ = pipeline.flush();
}

/// A system stream error degrades the recording instead of killing it:
/// one restart attempt after a Windows default-device change, otherwise
/// drop to mic-only and keep going.
fn run_system_drain(
    app: AppHandle,
    active: ActiveSlot,
    meeting_id: String,
    mut rx: Receiver<CaptureEvent>,
    chunk_tx: SyncSender<ChunkJob>,
) {
    let pipeline = DrainPipeline::new(SOURCE_SYSTEM, chunk_tx)
        .with_backlog_warning(backlog_warner(app.clone(), meeting_id.clone()));
    // Silence handling is macOS-specific: a tap created before the TCC
    // grant delivers zeros forever. On Windows, loopback silence just
    // means nothing is playing — restarting there would be wrong.
    #[cfg(target_os = "macos")]
    let pipeline = {
        let app = app.clone();
        let meeting_id = meeting_id.clone();
        pipeline.with_silence_restart().with_silence_warning(move || {
            log::warn!("[meetings] system stream all-silent for 60s — likely denied permission");
            let _ = app.emit(
                EVT_WARNING,
                RecordingMessage {
                    meeting_id: &meeting_id,
                    message: "System audio seems silent — check System Settings → Privacy & Security → Screen & System Audio Recording and enable Clauge, then restart the recording.",
                },
            );
        })
    };
    let mut pipeline = pipeline;
    let mut device_restarts: u32 = 0;
    let mut silence_restart_done = false;
    loop {
        match drain_until_exit(&mut pipeline, &rx) {
            DrainExit::Closed => break,
            DrainExit::SilenceRestart => {
                if silence_restart_done {
                    continue;
                }
                silence_restart_done = true;
                log::warn!(
                    "[meetings] system stream all-silent for {SYSTEM_SILENCE_RESTART_MS}ms — \
                     restarting capture in case the tap predates the permission grant"
                );
                if let Some((new_rx, elapsed_ms)) = restart_system_capture(&active) {
                    log::info!("[meetings] system capture restarted after initial silence");
                    pipeline.rebase_to_ms(elapsed_ms);
                    rx = new_rx;
                } else {
                    // The old capture is already stopped (or a stop is in
                    // flight); the next drain sees the closed channel and
                    // exits normally.
                    log::warn!("[meetings] silent-stream capture restart failed");
                }
                continue;
            }
            DrainExit::StreamError(msg) => {
                if device_restarts < MAX_DEVICE_RESTARTS && msg.starts_with(DEVICE_CHANGED_PREFIX) {
                    device_restarts += 1;
                    if let Some((new_rx, elapsed_ms)) = restart_system_capture(&active) {
                        log::info!(
                            "[meetings] system capture rebound after output-device change (#{device_restarts})"
                        );
                        pipeline.rebase_to_ms(elapsed_ms);
                        rx = new_rx;
                        continue;
                    }
                }
                log::warn!("[meetings] system stream failed, continuing mic-only: {msg}");
                if let Some(rec) = active.lock().as_mut() {
                    rec.system_audio = false;
                }
                let _ = app.emit(
                    EVT_WARNING,
                    RecordingMessage { meeting_id: &meeting_id, message: &msg },
                );
                break;
            }
        }
    }
    let _ = pipeline.flush();
}

fn backlog_warner(app: AppHandle, meeting_id: String) -> impl FnMut(&str) + Send + 'static {
    move |message| {
        let _ = app.emit(
            EVT_WARNING,
            RecordingMessage { meeting_id: &meeting_id, message },
        );
    }
}

/// On success also returns the recording's wall-clock elapsed milliseconds,
/// for rebasing the drain pipeline's offsets onto the new stream.
fn restart_system_capture(active: &ActiveSlot) -> Option<(Receiver<CaptureEvent>, u64)> {
    let old = {
        let mut slot = active.lock();
        let rec = slot.as_mut()?;
        if rec.mic.is_none() {
            return None;
        }
        rec.system.take()
    };
    if let Some(old) = old {
        old.stop();
    }
    let (tx, rx) = channel();
    match SystemCapture::start(tx) {
        Ok(capture) => {
            let mut slot = active.lock();
            match slot.as_mut().filter(|rec| rec.mic.is_some()) {
                Some(rec) => {
                    rec.system = Some(capture);
                    rec.system_audio = true;
                    Some((rx, rec.started_instant.elapsed().as_millis() as u64))
                }
                None => {
                    drop(slot);
                    capture.stop();
                    None
                }
            }
        }
        Err(e) => {
            log::warn!("[meetings] system capture restart failed: {e}");
            None
        }
    }
}

// --- transcriber thread ---

fn run_transcriber<F: FnMut(TranscriptSegment)>(
    mut transcriber: Transcriber,
    rx: Receiver<ChunkJob>,
    mut sink: F,
) {
    while let Ok(job) = rx.recv() {
        match transcriber.transcribe(&job.samples_16k, job.offset_ms, job.source) {
            Ok(segments) => segments.into_iter().for_each(&mut sink),
            Err(e) => log::warn!(
                "[meetings] chunk transcription failed ({} @ {}ms): {e}",
                job.source,
                job.offset_ms
            ),
        }
    }
}

fn is_silent_chunk(samples: &[f32]) -> bool {
    chunk_rms(samples) < SILENT_CHUNK_RMS
}

fn chunk_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

// --- DB flush task ---

fn should_flush(buffered: usize) -> bool {
    buffered >= FLUSH_MAX_BUFFERED
}

async fn run_flush(
    app: AppHandle,
    pool: SqlitePool,
    meeting_id: String,
    mut rx: tokio::sync::mpsc::Receiver<TranscriptSegment>,
    active: ActiveSlot,
    done_tx: tokio::sync::oneshot::Sender<()>,
) {
    let mut buf: Vec<TranscriptSegment> = Vec::new();
    let mut failures: u32 = 0;
    let mut ticker =
        tokio::time::interval_at(tokio::time::Instant::now() + FLUSH_INTERVAL, FLUSH_INTERVAL);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                tracked_flush(&app, &pool, &meeting_id, &mut buf, &mut failures).await;
            }
            seg = rx.recv() => match seg {
                Some(seg) => {
                    buf.push(seg);
                    let dropped = cap_buffer(&mut buf);
                    if dropped > 0 {
                        log::error!(
                            "[meetings] segment buffer over {FLUSH_BUFFER_CAP} cap for \
                             {meeting_id}, dropped {dropped} oldest unsaved segment(s)"
                        );
                    }
                    if should_flush(buf.len()) {
                        tracked_flush(&app, &pool, &meeting_id, &mut buf, &mut failures).await;
                    }
                }
                None => break,
            },
        }
    }
    if !flush_buffer(&pool, &meeting_id, &mut buf).await {
        let _ = app.emit(
            EVT_ERROR,
            RecordingMessage {
                meeting_id: &meeting_id,
                message: "failed to save the final transcript segments — \
                          part of the transcript was lost",
            },
        );
    }
    if let Err(e) = repo::finish_meeting(&pool, &meeting_id).await {
        log::warn!("[meetings] failed to finalize meeting {meeting_id}: {e}");
    }
    let _ = app.emit(EVT_STOPPED, MeetingEvent { meeting_id: &meeting_id });
    // Only close the widget while it still belongs to THIS meeting: a
    // late finalize (e.g. after a stop timeout) must not tear down a
    // widget that meanwhile shows a different recording or a new prompt.
    let was_active = {
        let mut slot = active.lock();
        let ours = slot.as_ref().is_some_and(|rec| rec.meeting_id == meeting_id);
        if ours {
            *slot = None;
        }
        ours
    };
    if was_active {
        widget::close_widget(&app);
    }
    let _ = done_tx.send(());
}

async fn tracked_flush(
    app: &AppHandle,
    pool: &SqlitePool,
    meeting_id: &str,
    buf: &mut Vec<TranscriptSegment>,
    failures: &mut u32,
) {
    if flush_buffer(pool, meeting_id, buf).await {
        *failures = 0;
        return;
    }
    *failures += 1;
    if *failures % FLUSH_FAILURE_WARN_EVERY == 0 {
        let _ = app.emit(
            EVT_WARNING,
            RecordingMessage {
                meeting_id,
                message: "failed to save transcript segments — retrying",
            },
        );
    }
}

/// Failed appends keep the buffer so the next tick retries them.
/// Returns false when the append failed.
async fn flush_buffer(pool: &SqlitePool, meeting_id: &str, buf: &mut Vec<TranscriptSegment>) -> bool {
    if buf.is_empty() {
        return true;
    }
    match repo::append_segments(pool, meeting_id, buf).await {
        Ok(()) => {
            buf.clear();
            true
        }
        Err(e) => {
            log::warn!(
                "[meetings] failed to persist {} segments for {meeting_id}: {e}",
                buf.len()
            );
            false
        }
    }
}

/// Drops the oldest entries beyond `FLUSH_BUFFER_CAP`, returning the count.
fn cap_buffer<T>(buf: &mut Vec<T>) -> usize {
    let excess = buf.len().saturating_sub(FLUSH_BUFFER_CAP);
    if excess > 0 {
        buf.drain(..excess);
    }
    excess
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn offset_tracker_converts_samples_to_ms() {
        let mut t = OffsetTracker::new(48_000);
        assert_eq!(t.chunk_start_ms(), 0);
        t.advance(48_000);
        assert_eq!(t.chunk_start_ms(), 1_000);
        t.advance(24_000);
        assert_eq!(t.chunk_start_ms(), 1_500);
    }

    #[test]
    fn offset_tracker_rebase_carries_ms_across_rate_change() {
        let mut t = OffsetTracker::new(48_000);
        t.advance(72_000);
        t.rebase(16_000);
        assert_eq!(t.chunk_start_ms(), 1_500);
        t.advance(8_000);
        assert_eq!(t.chunk_start_ms(), 2_000);
    }

    #[test]
    fn offset_tracker_rebase_to_ms_anchors_to_wall_elapsed() {
        let mut t = OffsetTracker::new(48_000);
        t.advance(48_000);
        t.rebase_to_ms(5_000, 16_000);
        assert_eq!(t.chunk_start_ms(), 5_000, "base comes from elapsed, not emitted samples");
        t.advance(8_000);
        assert_eq!(t.chunk_start_ms(), 5_500);
    }

    #[test]
    fn pipeline_rebase_to_ms_covers_same_rate_restart() {
        let (tx, rx) = sync_channel(8);
        let mut p = DrainPipeline::with_chunking(SOURCE_SYSTEM, tx, 1.0, 2.0);
        let frame = |samples: Vec<f32>| AudioFrame { samples, channels: 1, rate: 16_000 };

        // Rebase before any frame: first chunk starts at the elapsed base.
        p.rebase_to_ms(10_000);
        for _ in 0..4 {
            p.handle_frame(&frame(tone(16_000, 0.5))).unwrap();
        }
        // Same-rate restart with a 2s capture gap (22s → 20s emitted).
        p.rebase_to_ms(20_000);
        for _ in 0..4 {
            p.handle_frame(&frame(tone(16_000, 0.5))).unwrap();
        }
        // Restart with a partial chunk pending: it flushes pre-rebase.
        for _ in 0..2 {
            p.handle_frame(&frame(tone(16_000, 0.5))).unwrap();
        }
        p.rebase_to_ms(30_000);
        for _ in 0..4 {
            p.handle_frame(&frame(tone(16_000, 0.5))).unwrap();
        }
        drop(p);

        let offsets: Vec<u64> = rx.into_iter().map(|j| j.offset_ms).collect();
        assert_eq!(offsets, vec![10_000, 20_000, 22_000, 30_000]);
    }

    #[test]
    fn silence_warning_fires_once_after_sustained_initial_silence() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let (tx, rx) = sync_channel(8);
        let fired = Arc::new(AtomicUsize::new(0));
        let f = Arc::clone(&fired);
        let mut p = DrainPipeline::with_chunking(SOURCE_SYSTEM, tx, 1.0, 2.0)
            .with_silence_warning(move || {
                f.fetch_add(1, Ordering::SeqCst);
            });
        let silent = || AudioFrame { samples: vec![0.0; 8_000], channels: 1, rate: 16_000 };

        for _ in 0..118 {
            p.handle_frame(&silent()).unwrap();
        }
        assert_eq!(fired.load(Ordering::SeqCst), 0, "no warning before 60s of silence");
        for _ in 0..8 {
            p.handle_frame(&silent()).unwrap();
        }
        assert_eq!(fired.load(Ordering::SeqCst), 1, "warning fired once past 60s");
        drop(rx);
    }

    #[test]
    fn silence_restart_requested_once_after_15s_of_initial_silence() {
        let (tx, rx) = sync_channel(8);
        let mut p =
            DrainPipeline::with_chunking(SOURCE_SYSTEM, tx, 1.0, 2.0).with_silence_restart();
        let silent = || AudioFrame { samples: vec![0.0; 8_000], channels: 1, rate: 16_000 };

        for _ in 0..28 {
            p.handle_frame(&silent()).unwrap();
            assert!(!p.take_silence_restart(), "no restart before 15s of silence");
        }
        for _ in 0..2 {
            p.handle_frame(&silent()).unwrap();
        }
        assert!(p.take_silence_restart(), "restart requested at 15s");
        for _ in 0..40 {
            p.handle_frame(&silent()).unwrap();
            assert!(!p.take_silence_restart(), "at most one restart per recording");
        }
        drop(rx);
    }

    #[test]
    fn silence_restart_disarmed_by_audible_chunk() {
        let (tx, rx) = sync_channel(8);
        let mut p =
            DrainPipeline::with_chunking(SOURCE_SYSTEM, tx, 1.0, 2.0).with_silence_restart();

        let audible = AudioFrame { samples: tone(16_000, 1.0), channels: 1, rate: 16_000 };
        p.handle_frame(&audible).unwrap();
        let silent = || AudioFrame { samples: vec![0.0; 8_000], channels: 1, rate: 16_000 };
        for _ in 0..60 {
            p.handle_frame(&silent()).unwrap();
            assert!(!p.take_silence_restart(), "real audio disarms the restart watchdog");
        }
        drop(rx);
    }

    #[test]
    fn silence_warning_disarmed_by_audible_chunk() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let (tx, rx) = sync_channel(8);
        let fired = Arc::new(AtomicUsize::new(0));
        let f = Arc::clone(&fired);
        let mut p = DrainPipeline::with_chunking(SOURCE_SYSTEM, tx, 1.0, 2.0)
            .with_silence_warning(move || {
                f.fetch_add(1, Ordering::SeqCst);
            });

        let audible = AudioFrame { samples: tone(16_000, 1.0), channels: 1, rate: 16_000 };
        p.handle_frame(&audible).unwrap();
        let silent = || AudioFrame { samples: vec![0.0; 8_000], channels: 1, rate: 16_000 };
        for _ in 0..130 {
            p.handle_frame(&silent()).unwrap();
        }
        assert_eq!(fired.load(Ordering::SeqCst), 0, "real audio disarms the warning");
        drop(rx);
    }

    #[test]
    fn pipeline_stamps_chunks_with_start_offsets() {
        let (tx, rx) = sync_channel(8);
        let mut p = DrainPipeline::with_chunking(SOURCE_MIC, tx, 1.0, 2.0);
        let frame = |samples: Vec<f32>| AudioFrame { samples, channels: 1, rate: 16_000 };

        for _ in 0..6 {
            p.handle_frame(&frame(tone(16_000, 0.5))).unwrap();
        }
        p.flush().unwrap();
        drop(p);

        let jobs: Vec<ChunkJob> = rx.into_iter().collect();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].offset_ms, 0);
        assert_eq!(jobs[0].samples_16k.len(), 32_000);
        assert_eq!(jobs[1].offset_ms, 2_000);
        assert_eq!(jobs[1].samples_16k.len(), 16_000);
        assert!(jobs.iter().all(|j| j.source == SOURCE_MIC));
    }

    #[test]
    fn pipeline_downmixes_per_frame_and_survives_rate_change() {
        let (tx, rx) = sync_channel(8);
        let mut p = DrainPipeline::with_chunking(SOURCE_SYSTEM, tx, 10.0, 20.0);

        let stereo: Vec<f32> = tone(16_000, 1.0).into_iter().flat_map(|s| [s, s]).collect();
        p.handle_frame(&AudioFrame { samples: stereo, channels: 2, rate: 16_000 })
            .unwrap();
        p.handle_frame(&AudioFrame { samples: tone(48_000, 1.0), channels: 1, rate: 48_000 })
            .unwrap();
        p.flush().unwrap();
        drop(p);

        let jobs: Vec<ChunkJob> = rx.into_iter().collect();
        assert_eq!(jobs.len(), 2, "rate change flushes the old-rate chunk");
        assert_eq!(jobs[0].offset_ms, 0);
        assert_eq!(jobs[0].samples_16k.len(), 16_000, "stereo folds to mono per frame");
        assert_eq!(jobs[1].offset_ms, 1_000, "offset carries across rebase");
        let resampled = jobs[1].samples_16k.len() as f32;
        assert!((resampled - 16_000.0).abs() / 16_000.0 < 0.01, "48k chunk resampled to 16k");
    }

    #[test]
    fn silent_chunk_predicate_filters_quiet_audio() {
        assert!(is_silent_chunk(&[]));
        assert!(is_silent_chunk(&vec![0.0f32; 16_000]));
        assert!(is_silent_chunk(&vec![0.001f32; 16_000]));
        assert!(!is_silent_chunk(&tone(16_000, 1.0)));
        assert!(chunk_rms(&vec![0.5f32; 100]) > 0.4);
    }

    #[test]
    fn pipeline_skips_silent_chunks_but_advances_offset() {
        let (tx, rx) = sync_channel(8);
        let mut p = DrainPipeline::with_chunking(SOURCE_MIC, tx, 1.0, 2.0);
        let frame = |samples: Vec<f32>| AudioFrame { samples, channels: 1, rate: 16_000 };

        for _ in 0..4 {
            p.handle_frame(&frame(vec![0.0f32; 8_000])).unwrap();
        }
        for _ in 0..4 {
            p.handle_frame(&frame(tone(16_000, 0.5))).unwrap();
        }
        p.flush().unwrap();
        drop(p);

        let jobs: Vec<ChunkJob> = rx.into_iter().collect();
        assert!(jobs.iter().all(|j| !is_silent_chunk(&j.samples_16k)), "silence never queued");
        assert_eq!(jobs[0].offset_ms, 2_000, "skipped silence still advances the offset");
    }

    /// Offset ground truth, computed independently of `OffsetTracker`:
    /// at 16 kHz mono the pipeline is byte-exact passthrough (no downmix,
    /// no resample), so every emitted chunk must be a contiguous slice of
    /// the input and its true start position can be recovered by content
    /// matching against unique noise. The stamped `offset_ms` must equal
    /// that position for every emit path: the RMS-skipped all-silence
    /// chunk (must still advance), the 28s max cut, the 20s silence-
    /// boundary cut, and the flush remainder at stop.
    #[test]
    fn pipeline_offsets_match_true_sample_positions() {
        const RATE: u32 = 16_000;
        // Odd frame size so no cut lands on a "nice" boundary.
        const FRAME: usize = 331;
        const TOTAL_FRAMES: usize = 3_625; // ~75s

        // Deterministic per-index noise: any 64-sample window is unique.
        fn noise(i: usize) -> f32 {
            (i as u32).wrapping_mul(2_654_435_761) as f32 / u32::MAX as f32 - 0.5
        }
        // 0–21s silence (one full skipped chunk), 21–50s speech (forces a
        // max cut mid-speech), 50–68s silence (silence-boundary cut),
        // 70–72s speech inside the flush remainder.
        let speech =
            |i: usize| (336_000..800_000).contains(&i) || (1_120_000..1_152_000).contains(&i);
        let input: Vec<f32> = (0..TOTAL_FRAMES * FRAME)
            .map(|i| if speech(i) { noise(i) } else { 0.0 })
            .collect();

        let (tx, rx) = sync_channel(64);
        // Production chunking: 20s target / 28s max.
        let mut p = DrainPipeline::new(SOURCE_MIC, tx);
        for frame in input.chunks(FRAME) {
            p.handle_frame(&AudioFrame { samples: frame.to_vec(), channels: 1, rate: RATE })
                .unwrap();
        }
        p.flush().unwrap();
        drop(p);

        let jobs: Vec<ChunkJob> = rx.into_iter().collect();
        assert_eq!(jobs.len(), 3, "max cut + silence cut + flush remainder");

        let mut last_end = 0usize;
        for (idx, job) in jobs.iter().enumerate() {
            let k = job
                .samples_16k
                .iter()
                .position(|s| s.abs() > 1e-6)
                .expect("audible chunk has a non-silent sample");
            let probe = &job.samples_16k[k..k + 64];
            let hit = (0..=input.len() - probe.len())
                .filter(|&q| input[q] == probe[0])
                .find(|&q| input[q..q + probe.len()] == *probe)
                .expect("probe window found in input");
            let true_start = hit - k;
            assert_eq!(
                &input[true_start..true_start + job.samples_16k.len()],
                &job.samples_16k[..],
                "chunk {idx} is a contiguous input slice — the chunker held nothing back"
            );
            assert_eq!(
                job.offset_ms,
                true_start as u64 * 1000 / RATE as u64,
                "chunk {idx} stamped offset == true position of its first sample"
            );
            if idx > 0 {
                assert_eq!(true_start, last_end, "chunk {idx} starts where chunk {} ended", idx - 1);
            }
            last_end = true_start + job.samples_16k.len();
        }

        assert!(
            jobs[0].offset_ms >= 20_000,
            "the leading RMS-skipped silence chunk still advanced the clock, got {}ms",
            jobs[0].offset_ms
        );
        assert!(
            jobs[0].samples_16k.len() >= (RATE as f32 * CHUNK_MAX_SECS) as usize,
            "chunk 0 was a max cut"
        );
        let target = (RATE as f32 * CHUNK_TARGET_SECS) as usize;
        let len1 = jobs[1].samples_16k.len();
        assert!(len1 >= target && len1 < target + FRAME, "chunk 1 cut at the silence boundary");
        assert_eq!(last_end, input.len(), "flush remainder reaches the final input sample");
    }

    #[test]
    fn pipeline_drops_chunks_when_queue_is_full_and_warns_once_per_n() {
        let warnings = Arc::new(Mutex::new(Vec::<String>::new()));
        let (tx, rx) = sync_channel(1);
        let sink = warnings.clone();
        let mut p = DrainPipeline::with_chunking(SOURCE_MIC, tx, 1.0, 2.0)
            .with_backlog_warning(move |msg| sink.lock().push(msg.to_string()));
        let frame = |samples: Vec<f32>| AudioFrame { samples, channels: 1, rate: 16_000 };

        // Three 2s chunks against an unread bound-1 queue: the first fills
        // the slot, the next two are dropped.
        for _ in 0..12 {
            p.handle_frame(&frame(tone(16_000, 0.5))).unwrap();
        }
        assert_eq!(p.dropped_chunks, 2);
        assert_eq!(p.dropped_ms, 4_000);
        drop(p);

        let jobs: Vec<ChunkJob> = rx.into_iter().collect();
        assert_eq!(jobs.len(), 1, "queue keeps only what fit");
        assert_eq!(jobs[0].offset_ms, 0);

        let warnings = warnings.lock();
        assert_eq!(warnings.len(), 1, "one warning per {DROP_WARN_EVERY} drops, not per drop");
        assert!(warnings[0].contains("skipped 2s of audio"), "got: {warnings:?}");
    }

    #[test]
    fn flush_batches_at_fifty_segments() {
        assert!(!should_flush(0));
        assert!(!should_flush(FLUSH_MAX_BUFFERED - 1));
        assert!(should_flush(FLUSH_MAX_BUFFERED));
        assert!(should_flush(FLUSH_MAX_BUFFERED + 1));
    }

    #[test]
    fn cap_buffer_drops_oldest_beyond_cap() {
        let mut buf: Vec<usize> = (0..FLUSH_BUFFER_CAP + 2).collect();
        assert_eq!(cap_buffer(&mut buf), 2);
        assert_eq!(buf.len(), FLUSH_BUFFER_CAP);
        assert_eq!(buf[0], 2, "oldest entries go first");

        let mut small = vec![1, 2, 3];
        assert_eq!(cap_buffer(&mut small), 0);
        assert_eq!(small, vec![1, 2, 3]);
    }

    #[test]
    fn default_title_uses_local_date_and_minute() {
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 6, 11, 9, 5, 0).unwrap();
        assert_eq!(default_title(now), "Meeting — 2026-06-11 09:05");
    }

    /// End-to-end pipeline smoke (no Tauri, no DB): synthetic speech frames
    /// flow drain → chunk → whisper and segments reach the sink. Reuses the
    /// tiny-model cache of `transcribe_smoke_hello_world`. Run with:
    /// `cargo test --release recorder_pipeline_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn recorder_pipeline_smoke() {
        use std::process::Command;

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

        let aiff = std::env::temp_dir().join("clauge-test-recorder.aiff");
        let wav = std::env::temp_dir().join("clauge-test-recorder-16k.wav");
        assert!(Command::new("/usr/bin/say")
            .arg("-o")
            .arg(&aiff)
            .arg("hello world this is a recording test")
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
        let samples: Vec<f32> = hound::WavReader::open(&wav)
            .unwrap()
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let (chunk_tx, chunk_rx) = sync_channel(64);
        let mut pipeline = DrainPipeline::new(SOURCE_MIC, chunk_tx);
        for frame in samples.chunks(1600) {
            pipeline
                .handle_frame(&AudioFrame { samples: frame.to_vec(), channels: 1, rate: 16_000 })
                .unwrap();
        }
        pipeline.flush().unwrap();
        drop(pipeline);

        let transcriber = Transcriber::load(&model, Some("en")).unwrap();
        let mut collected: Vec<TranscriptSegment> = Vec::new();
        run_transcriber(transcriber, chunk_rx, |seg| collected.push(seg));

        let joined = collected
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        println!("recorder pipeline transcript: {joined:?}");
        assert!(
            joined.to_lowercase().contains("hello"),
            "transcript missing 'hello': {joined:?}"
        );
        assert!(collected.iter().all(|s| s.source == SOURCE_MIC));

        let _ = std::fs::remove_file(&aiff);
        let _ = std::fs::remove_file(&wav);
    }
}

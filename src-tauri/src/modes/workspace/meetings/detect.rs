//! Meeting call detection: polls a mic-in-use probe + process-name
//! snapshot every 3s and emits `meetings:call-detected` / `meetings:call-ended`
//! for the floating widget (one detection per episode, dismissible).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager};

use crate::modes::workspace::meetings::recorder::RecorderState;
use crate::modes::workspace::meetings::widget;
use crate::shared::repos::settings as settings_repo;

pub const SETTING_KEY: &str = "workspace_meeting_detect_enabled";
pub const AUTOSTOP_SETTING_KEY: &str = "workspace_meeting_autostop_enabled";

const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Active episodes ride out brief mutes so the widget doesn't flap, but
/// Zoom/Teams/Meet keep the mic device open even while muted — OS-level
/// mic idle means the call actually ended. 8s is enough debounce to
/// survive device-handoff blips while still catching back-to-back calls.
const ACTIVE_RESET_SECS: u64 = 8;
const ACTIVE_RESET: Duration = Duration::from_secs(ACTIVE_RESET_SECS);
/// Dismissed episodes reset fast: dismissing call 1 then joining call 2
/// moments later must still detect call 2.
const DISMISSED_RESET_SECS: u64 = 5;
/// Auto-stop: a detected-call recording stops after this much CONSECUTIVE
/// "no other process is capturing the mic" — i.e. Zoom/Teams/Meet released
/// the input device because the call ended. Longer than the detection
/// windows on purpose: a false stop loses the tail of a real meeting.
const CALL_END_STOP_SECS: u64 = 20;

const EVT_DETECTED: &str = "meetings:call-detected";
const EVT_ENDED: &str = "meetings:call-ended";
const EVT_AUTOSTOPPED: &str = "meetings:recording-autostopped";
const EVT_CALL_SUPPRESSED: &str = "meetings:call-suppressed";

// --- Process labeling ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MeetingApp {
    Zoom,
    Teams,
    Webex,
    Discord,
    Slack,
    Browser,
}

/// `needle` (ASCII) must sit on word-ish boundaries inside `haystack`:
/// bare substring matching would label "searchpartyd" a browser ("arc")
/// and "teamspeak" as Teams.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let begin = start + pos;
        let end = begin + needle.len();
        let before_ok = begin == 0 || !bytes[begin - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = begin + 1;
    }
    false
}

/// Labels the most likely meeting app from a process-NAME snapshot.
/// Dedicated apps win over browsers regardless of order in the snapshot.
pub fn label_meeting_app(process_names: &[String]) -> Option<MeetingApp> {
    const DEDICATED: &[(MeetingApp, &[&str])] = &[
        (MeetingApp::Zoom, &["zoom.us", "zoom"]),
        (MeetingApp::Teams, &["msteams", "ms-teams", "teams"]),
        (MeetingApp::Webex, &["webex"]),
        (MeetingApp::Discord, &["discord"]),
        (MeetingApp::Slack, &["slack"]),
    ];
    const BROWSERS: &[&str] = &[
        "chrome", "msedge", "edge", "arc", "brave", "firefox", "safari", "vivaldi", "opera",
    ];

    let lower: Vec<String> = process_names.iter().map(|n| n.to_lowercase()).collect();

    for (app, patterns) in DEDICATED {
        for name in &lower {
            if patterns.iter().any(|p| contains_word(name, p)) {
                return Some(*app);
            }
        }
    }
    for name in &lower {
        if BROWSERS.iter().any(|b| contains_word(name, b)) {
            return Some(MeetingApp::Browser);
        }
    }
    None
}

// --- Episode state machine (pure, testable) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectEvent {
    CallDetected(MeetingApp),
    CallEnded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Active,
    Dismissed,
}

/// One "episode" spans mic-active → sustained mic silence (8s while
/// Active, 5s once Dismissed). The detected event fires once per episode;
/// dismissal (user or our own recording) keeps the episode alive but mute
/// until it resets to Idle.
pub struct EpisodeTracker {
    phase: Phase,
    idle_since: Option<Instant>,
    app: Option<MeetingApp>,
}

impl Default for EpisodeTracker {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            idle_since: None,
            app: None,
        }
    }
}

impl EpisodeTracker {
    pub fn tick(
        &mut self,
        now: Instant,
        recording: bool,
        mic: bool,
        app: Option<MeetingApp>,
    ) -> Option<DetectEvent> {
        if recording {
            // Our own recording counts as mic activity, so the episode must
            // not idle out underneath it. Treated like a dismissal: when the
            // recording ends mid-call the widget doesn't immediately re-offer
            // notes for the call it just recorded — a fresh Idle→Active
            // transition (sustained mic silence first) is required.
            if self.phase == Phase::Active {
                self.phase = Phase::Dismissed;
            }
            self.idle_since = None;
            return None;
        }
        if mic {
            self.idle_since = None;
            if self.phase == Phase::Idle {
                if let Some(app) = app {
                    self.phase = Phase::Active;
                    self.app = Some(app);
                    return Some(DetectEvent::CallDetected(app));
                }
            }
            return None;
        }
        if self.phase == Phase::Idle {
            self.idle_since = None;
            return None;
        }
        let reset_after = match self.phase {
            Phase::Dismissed => Duration::from_secs(DISMISSED_RESET_SECS),
            _ => ACTIVE_RESET,
        };
        let since = *self.idle_since.get_or_insert(now);
        if now.duration_since(since) >= reset_after {
            self.phase = Phase::Idle;
            self.idle_since = None;
            self.app = None;
            return Some(DetectEvent::CallEnded);
        }
        None
    }

    pub fn dismiss(&mut self) {
        if self.phase == Phase::Active {
            self.phase = Phase::Dismissed;
        }
    }

    /// Hard reset (detection toggled off). Returns true when an episode
    /// was live so the caller can emit `call-ended` for the widget.
    pub fn reset(&mut self) -> bool {
        let was_live = self.phase != Phase::Idle;
        self.phase = Phase::Idle;
        self.idle_since = None;
        self.app = None;
        was_live
    }

    /// (`active`, `app`) for widget re-sync: `active` only while the episode
    /// is live and undismissed, with the app it was detected as.
    pub fn snapshot(&self) -> (bool, Option<MeetingApp>) {
        match self.phase {
            Phase::Active => (true, self.app),
            Phase::Dismissed | Phase::Idle => (false, None),
        }
    }
}

// --- Auto-stop tracker (pure, testable) ---

/// Stops a recording once the call it belongs to ends. Arms when EITHER the
/// recording was call-detected at start (`source_app` set) OR — even for a
/// manually-started recording — another process is observed genuinely on the
/// mic during it (`Some(true)`), i.e. a call is in progress. A recording
/// where no other app ever touches the mic (a plain voice memo) stays manual
/// and never auto-stops. Fires at most once per recording: 20s of CONSECUTIVE
/// `Some(false)` from `other_process_uses_mic()`. `Some(true)` (call still
/// holds the mic) and `None` (probe unavailable/unknown) both reset the idle
/// run — unknown must never stop a recording.
#[derive(Default)]
pub struct AutoStopTracker {
    recording_id: Option<String>,
    armed: bool,
    fired: bool,
    idle_since: Option<Instant>,
}

impl AutoStopTracker {
    /// One observation per poll tick. `recording` is
    /// `Some((meeting_id, is_detected_call))` while the recorder is live,
    /// `None` otherwise. Returns true exactly when the recording should be
    /// stopped.
    pub fn tick(
        &mut self,
        now: Instant,
        recording: Option<(&str, bool)>,
        other_mic: Option<bool>,
    ) -> bool {
        let Some((meeting_id, detected)) = recording else {
            *self = Self::default();
            return false;
        };
        if self.recording_id.as_deref() != Some(meeting_id) {
            *self = Self {
                recording_id: Some(meeting_id.to_string()),
                armed: detected,
                fired: false,
                idle_since: None,
            };
        }
        // Arm dynamically: a manually-started recording becomes auto-stoppable
        // once another process is genuinely on the mic during it (a call in
        // progress). Covers "pressed Record, then joined the call" — which the
        // start-time `detected` flag alone misses.
        if !self.armed && other_mic == Some(true) {
            self.armed = true;
        }
        if !self.armed || self.fired {
            return false;
        }
        match other_mic {
            Some(false) => {
                let since = *self.idle_since.get_or_insert(now);
                if now.duration_since(since) >= Duration::from_secs(CALL_END_STOP_SECS) {
                    self.fired = true;
                    self.idle_since = None;
                    return true;
                }
                false
            }
            Some(true) | None => {
                self.idle_since = None;
                false
            }
        }
    }
}

// --- Suppressed-call notice tracker (pure, testable) ---

/// A NEW call needs this much CONSECUTIVE mic quiet first while recording a
/// detected call: the recorded call holds the mic from the start, so only
/// "original call over for a while, then someone on the mic again" is a new
/// call. Longer than CALL_END_STOP_SECS on purpose — when auto-stop is
/// enabled it wins and stops the recording instead.
const NOTICE_QUIET_SECS: u64 = 30;

/// Surfaces "a call started but detection is suppressed because a recording
/// is already in progress" (the widget otherwise stays silently away and
/// users think it broke). Watches `other_process_uses_mic()` while the
/// recorder is live and fires at most once per recording:
/// - Manual recordings (`detected = false`): the mic was observed free at
///   least once during the recording, then another process grabbed it — a
///   call started mid-recording. Already-captured at recording start never
///   fires (the user knowingly started recording during that call).
/// - Detected-call recordings (`detected = true`): requires ≥30s of
///   consecutive `Some(false)` (the recorded call ended) before a
///   `Some(true)` counts as a new call. Auto-stop (20s) normally stops the
///   recording before that, so this arm matters when auto-stop is disabled.
/// `None` (probe unavailable/unknown) resets the quiet run and never fires.
#[derive(Default)]
pub struct SuppressedCallTracker {
    recording_id: Option<String>,
    fired: bool,
    quiet_since: Option<Instant>,
    quiet_satisfied: bool,
}

impl SuppressedCallTracker {
    /// Whether the poller should keep running the `other_process_uses_mic`
    /// probe on this tracker's behalf — false once the one-shot fired for
    /// `meeting_id`, so the steady state after the notice costs nothing.
    pub fn wants_probe(&self, meeting_id: &str) -> bool {
        !self.fired || self.recording_id.as_deref() != Some(meeting_id)
    }

    /// One observation per poll tick; same inputs as `AutoStopTracker::tick`.
    /// Returns true exactly when the suppressed-call notice should fire.
    pub fn tick(
        &mut self,
        now: Instant,
        recording: Option<(&str, bool)>,
        other_mic: Option<bool>,
    ) -> bool {
        let Some((meeting_id, detected)) = recording else {
            *self = Self::default();
            return false;
        };
        if self.recording_id.as_deref() != Some(meeting_id) {
            *self = Self {
                recording_id: Some(meeting_id.to_string()),
                ..Self::default()
            };
        }
        if self.fired {
            return false;
        }
        let required_quiet = if detected {
            Duration::from_secs(NOTICE_QUIET_SECS)
        } else {
            Duration::ZERO
        };
        match other_mic {
            Some(false) => {
                let since = *self.quiet_since.get_or_insert(now);
                if now.duration_since(since) >= required_quiet {
                    self.quiet_satisfied = true;
                }
                false
            }
            Some(true) => {
                self.quiet_since = None;
                if self.quiet_satisfied {
                    self.fired = true;
                    return true;
                }
                false
            }
            None => {
                self.quiet_since = None;
                false
            }
        }
    }
}

// --- Managed state + commands surface ---

#[derive(Clone, Serialize)]
pub struct DetectStatus {
    pub enabled: bool,
    pub app: Option<MeetingApp>,
    pub active: bool,
}

pub struct DetectState {
    enabled: AtomicBool,
    autostop_enabled: AtomicBool,
    tracker: Mutex<EpisodeTracker>,
}

impl Default for DetectState {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            autostop_enabled: AtomicBool::new(true),
            tracker: Mutex::new(EpisodeTracker::default()),
        }
    }
}

impl DetectState {
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn autostop_enabled(&self) -> bool {
        self.autostop_enabled.load(Ordering::Relaxed)
    }

    pub fn set_autostop_enabled(&self, enabled: bool) {
        self.autostop_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn dismiss(&self) {
        log::info!("meeting detection: episode dismissed");
        self.tracker.lock().dismiss();
    }

    pub fn status(&self) -> DetectStatus {
        let (active, app) = self.tracker.lock().snapshot();
        DetectStatus {
            enabled: self.enabled(),
            app,
            active,
        }
    }
}

// --- Mic-in-use probe (per OS) ---

pub fn mic_in_use() -> bool {
    os::mic_in_use()
}

/// Does any process OTHER than Clauge currently capture the microphone?
/// `mic_in_use()` is useless while we record (our own cpal stream keeps the
/// device "running somewhere"), so call-end detection during a recording
/// needs the per-process view. `None` = unknown (probe unsupported or
/// failed) and must never be treated as "call ended".
pub fn other_process_uses_mic() -> Option<bool> {
    os::other_process_uses_mic()
}

#[cfg(target_os = "macos")]
mod os {
    use std::sync::OnceLock;

    use cidre::core_audio as ca;

    use crate::shared::platform::macos::macos_version;

    /// kAudioDevicePropertyDeviceIsRunningSomewhere on the default input
    /// device: 1 when ANY process has the mic running. Clauge's own
    /// recording also trips this, which is why the poller checks the
    /// recorder before probing.
    pub fn mic_in_use() -> bool {
        let Ok(device) = ca::System::default_input_device() else {
            return false;
        };
        let addr = ca::PropSelector::DEVICE_IS_RUNNING_SOMEWHERE.global_addr();
        matches!(device.prop::<u32>(&addr), Ok(1))
    }

    /// Core Audio process objects (kAudioHardwarePropertyProcessObjectList,
    /// macOS 14.4+, same generation as the tap API the system-audio capture
    /// relies on): a process with kAudioProcessPropertyIsRunningInput true
    /// is capturing input right now; kAudioProcessPropertyPID excludes
    /// Clauge itself.
    pub fn other_process_uses_mic() -> Option<bool> {
        static SUPPORTED: OnceLock<bool> = OnceLock::new();
        let supported = *SUPPORTED
            .get_or_init(|| matches!(macos_version(), Some((major, minor)) if (major, minor) >= (14, 4)));
        if !supported {
            return None;
        }
        let processes = ca::Process::list().ok()?;
        let own_pid = std::process::id() as cidre::sys::Pid;
        for process in processes {
            if process.pid().ok() == Some(own_pid) {
                continue;
            }
            if process.is_running_input().unwrap_or(false) {
                return Some(true);
            }
        }
        Some(false)
    }
}

#[cfg(target_os = "windows")]
mod os {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        eCapture, eMultimedia, AudioSessionStateActive, IAudioSessionControl2,
        IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    /// Walks the WASAPI session list on the default capture endpoint;
    /// any AudioSessionStateActive session means some process holds the mic.
    pub fn mic_in_use() -> bool {
        unsafe {
            // S_OK and S_FALSE both need a balancing CoUninitialize.
            // RPC_E_CHANGED_MODE (Err) means this thread already runs an
            // STA — COM stays usable but must NOT be uninitialized by us.
            let init = CoInitializeEx(None, COINIT_MULTITHREADED);
            // The probe runs in an inner scope so every COM interface is
            // released before CoUninitialize.
            let probe = (|| -> windows::core::Result<bool> {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let device = enumerator.GetDefaultAudioEndpoint(eCapture, eMultimedia)?;
                let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
                let sessions = manager.GetSessionEnumerator()?;
                for i in 0..sessions.GetCount()? {
                    if sessions.GetSession(i)?.GetState()? == AudioSessionStateActive {
                        return Ok(true);
                    }
                }
                Ok(false)
            })();
            if init.is_ok() {
                CoUninitialize();
            }
            probe.unwrap_or(false)
        }
    }

    /// Same WASAPI session walk as `mic_in_use`, but each active session is
    /// attributed via IAudioSessionControl2::GetProcessId so Clauge's own
    /// capture session can be excluded.
    pub fn other_process_uses_mic() -> Option<bool> {
        unsafe {
            let init = CoInitializeEx(None, COINIT_MULTITHREADED);
            let probe = (|| -> windows::core::Result<bool> {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let device = enumerator.GetDefaultAudioEndpoint(eCapture, eMultimedia)?;
                let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
                let sessions = manager.GetSessionEnumerator()?;
                let own_pid = std::process::id();
                for i in 0..sessions.GetCount()? {
                    let session = sessions.GetSession(i)?;
                    if session.GetState()? != AudioSessionStateActive {
                        continue;
                    }
                    let session2: IAudioSessionControl2 = session.cast()?;
                    // GetProcessId fails for cross-process system sessions —
                    // those aren't us, so they count as "other".
                    if session2.GetProcessId().map_or(true, |pid| pid != own_pid) {
                        return Ok(true);
                    }
                }
                Ok(false)
            })();
            if init.is_ok() {
                CoUninitialize();
            }
            probe.ok()
        }
    }
}

#[cfg(target_os = "linux")]
mod os {
    use std::process::Command;
    use std::sync::Once;

    static PACTL_MISSING: Once = Once::new();

    /// `pactl list short source-outputs` lists active capture streams.
    /// Monitor self-captures (fields ending in ".monitor") are skipped when
    /// the source column carries a name; many pactl versions print a numeric
    /// source index instead, in which case any stream counts — acceptable,
    /// since the poller already skips ticks while Clauge itself records
    /// (our monitor capture is the realistic monitor reader).
    pub fn mic_in_use() -> bool {
        let output = match Command::new("pactl")
            .args(["list", "short", "source-outputs"])
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                PACTL_MISSING.call_once(|| {
                    log::warn!("meeting detection: pactl not found; mic-in-use probe inactive");
                });
                return false;
            }
        };
        if !output.status.success() {
            return false;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .any(|l| !l.split_whitespace().any(|field| field.ends_with(".monitor")))
    }

    /// Full `pactl list source-outputs`: every capture stream block carries
    /// `application.process.id = "N"` and a `Source: <index>` line. Streams
    /// from Clauge's own pid and streams reading `.monitor` sources (system
    /// audio loopback — including our own) are skipped; anything left is
    /// another process holding a real mic. pactl missing/failing → None.
    pub fn other_process_uses_mic() -> Option<bool> {
        let output = Command::new("pactl")
            .args(["list", "source-outputs"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        // Indices of monitor sources, so `Source: <idx>` can be classified.
        let monitor_sources: Vec<String> = Command::new("pactl")
            .args(["list", "short", "sources"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| {
                        let mut fields = l.split_whitespace();
                        let idx = fields.next()?;
                        let name = fields.next()?;
                        name.ends_with(".monitor").then(|| idx.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();

        let own_pid = std::process::id().to_string();
        let text = String::from_utf8_lossy(&output.stdout);
        for block in text.split("Source Output #").skip(1) {
            let mut source_idx: Option<&str> = None;
            let mut pid: Option<&str> = None;
            for line in block.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("Source:") {
                    source_idx = Some(rest.trim());
                } else if let Some(rest) = line.strip_prefix("application.process.id =") {
                    pid = Some(rest.trim().trim_matches('"'));
                }
            }
            if source_idx.is_some_and(|idx| monitor_sources.iter().any(|m| m == idx)) {
                continue;
            }
            // No pid property → can't be excluded as ours → counts as other.
            if pid.is_none_or(|p| p != own_pid) {
                return Some(true);
            }
        }
        Some(false)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod os {
    pub fn mic_in_use() -> bool {
        false
    }

    pub fn other_process_uses_mic() -> Option<bool> {
        None
    }
}

// --- Poller ---

#[derive(Clone, Serialize)]
struct CallDetectedPayload {
    app: MeetingApp,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoStoppedPayload<'a> {
    meeting_id: &'a str,
}

fn process_name_snapshot(sys: &mut System) -> Vec<String> {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.processes()
        .values()
        .map(|p| p.name().to_string_lossy().into_owned())
        .collect()
}

pub fn start_poller(app: AppHandle) {
    let poller = tauri::async_runtime::spawn(async move {
        {
            let pool = app.state::<sqlx::SqlitePool>().inner().clone();
            let enabled = settings_repo::get_bool_or(&pool, SETTING_KEY, true).await;
            app.state::<DetectState>().set_enabled(enabled);
            let autostop =
                settings_repo::get_bool_or(&pool, AUTOSTOP_SETTING_KEY, true).await;
            app.state::<DetectState>().set_autostop_enabled(autostop);
        }

        // Reusing one System keeps each refresh incremental instead of a
        // from-scratch process-table build every tick.
        let sys = Arc::new(Mutex::new(System::new()));

        // Change-tracking so the idle steady state logs nothing.
        let mut last_mic = false;
        let mut last_label: Option<MeetingApp> = None;
        let mut suppressed_by_recording = false;

        // Owned by the poller alone: nothing else observes auto-stop
        // progress, so it stays off the managed state.
        let mut autostop = AutoStopTracker::default();
        let mut suppressed_call = SuppressedCallTracker::default();

        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let state = app.state::<DetectState>();
            if !state.enabled() {
                if state.tracker.lock().reset() {
                    log::info!("meeting detection: disabled while episode live — emitting call-ended");
                    let _ = app.emit(EVT_ENDED, ());
                    widget::close_widget(&app);
                }
                continue;
            }

            let status = app.state::<RecorderState>().status();
            if status.recording || status.stopping {
                if !suppressed_by_recording {
                    suppressed_by_recording = true;
                    log::info!("meeting detection: suppressed while recording");
                }
                let _ = state.tracker.lock().tick(Instant::now(), true, false, None);

                // The other-process mic probe feeds two consumers:
                //  • auto-stop — detected-call recordings only, while not
                //    already stopping, only when enabled;
                //  • the suppressed-call notice — any recording, until its
                //    one-shot fired for this recording.
                // The probe never runs when neither wants it.
                let detected = status.source_app.is_some();
                // Auto-stop runs whenever it's ENABLED — not only for call-
                // detected recordings: it arms dynamically once another process
                // is on the mic, so the probe must run for manual recordings
                // too. When disabled it must never fire (see autostop_mic below).
                let autostop_active = !status.stopping && state.autostop_enabled();
                let notice_wanted = !status.stopping
                    && status
                        .meeting_id
                        .as_deref()
                        .is_some_and(|id| suppressed_call.wants_probe(id));
                let other_mic = if autostop_active || notice_wanted {
                    tauri::async_runtime::spawn_blocking(other_process_uses_mic)
                        .await
                        .unwrap_or(None)
                } else {
                    None
                };
                let recording = status
                    .meeting_id
                    .as_deref()
                    .map(|id| (id, detected));
                if suppressed_call.tick(Instant::now(), recording, other_mic) {
                    // One-shot per recording, so the label check (process
                    // snapshot) runs at most once per recording session. It
                    // gates the toast on a meeting app actually being around
                    // — another process on the mic alone could be dictation.
                    let sys = sys.clone();
                    let label = tauri::async_runtime::spawn_blocking(move || {
                        label_meeting_app(&process_name_snapshot(&mut sys.lock()))
                    })
                    .await
                    .unwrap_or(None);
                    match label {
                        Some(meeting_app) => {
                            log::info!(
                                "meeting detection: new call while recording (app={meeting_app:?}) — emitting call-suppressed"
                            );
                            let _ = app.emit(
                                EVT_CALL_SUPPRESSED,
                                CallDetectedPayload { app: meeting_app },
                            );
                        }
                        None => log::info!(
                            "meeting detection: another process took the mic while recording, but no meeting app is running — notice skipped"
                        ),
                    }
                }
                // Feed the probe to auto-stop only when it's enabled; otherwise
                // None, so a disabled auto-stop can never fire (the suppressed
                // notice may still have run the probe above).
                let autostop_mic = if autostop_active { other_mic } else { None };
                if autostop.tick(Instant::now(), recording, autostop_mic) {
                    let meeting_id = status.meeting_id.clone().unwrap_or_default();
                    log::info!(
                        "meeting auto-stop: call ended ({CALL_END_STOP_SECS}s without another process on the mic) — stopping recording {meeting_id}"
                    );
                    match crate::modes::workspace::meetings::recorder::stop_recording(
                        app.clone(),
                    )
                    .await
                    {
                        Ok(stopped_id) => {
                            let _ = app.emit(
                                EVT_AUTOSTOPPED,
                                AutoStoppedPayload {
                                    meeting_id: &stopped_id,
                                },
                            );
                        }
                        Err(e) => {
                            log::warn!("meeting auto-stop: stop failed: {e}")
                        }
                    }
                }
                continue;
            }
            // Recorder idle → any armed auto-stop / notice state is stale.
            let _ = autostop.tick(Instant::now(), None, None);
            let _ = suppressed_call.tick(Instant::now(), None, None);
            if suppressed_by_recording {
                suppressed_by_recording = false;
                log::info!("meeting detection: recording ended, detection resumed");
            }

            // Both probes shell out / hit CoreAudio-COM — keep them off the
            // async runtime threads.
            let mic = tauri::async_runtime::spawn_blocking(mic_in_use)
                .await
                .unwrap_or(false);
            let label = if mic {
                let sys = sys.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    label_meeting_app(&process_name_snapshot(&mut sys.lock()))
                })
                .await
                .unwrap_or(None)
            } else {
                None
            };

            if mic != last_mic || label != last_label {
                log::debug!("meeting detection: mic_in_use={mic} label={label:?}");
                last_mic = mic;
                last_label = label;
            }

            let (event, phase_before, phase_after) = {
                let mut tracker = state.tracker.lock();
                let before = tracker.phase;
                let event = tracker.tick(Instant::now(), false, mic, label);
                (event, before, tracker.phase)
            };
            if phase_after != phase_before {
                log::debug!("meeting detection: phase {phase_before:?} -> {phase_after:?}");
            }
            match event {
                Some(DetectEvent::CallDetected(meeting_app)) => {
                    log::info!(
                        "meeting detection: episode activated (app={meeting_app:?}) — emitting call-detected"
                    );
                    let _ = app.emit(EVT_DETECTED, CallDetectedPayload { app: meeting_app });
                    widget::open_widget(&app);
                }
                Some(DetectEvent::CallEnded) => {
                    log::info!(
                        "meeting detection: episode reset (sustained mic idle) — emitting call-ended"
                    );
                    let _ = app.emit(EVT_ENDED, ());
                    widget::close_widget(&app);
                }
                None => {}
            }
        }
    });
    // The loop itself never returns, so the only way the poller task ends is
    // a panic — which tokio would otherwise swallow silently. Awaiting the
    // handle resolves on either outcome and guarantees a trace.
    tauri::async_runtime::spawn(async move {
        let _ = poller.await;
        log::error!("meeting detection poller exited");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn labels_each_dedicated_app() {
        assert_eq!(
            label_meeting_app(&names(&["zoom.us"])),
            Some(MeetingApp::Zoom)
        );
        assert_eq!(
            label_meeting_app(&names(&["Zoom.exe"])),
            Some(MeetingApp::Zoom)
        );
        assert_eq!(
            label_meeting_app(&names(&["MSTeams"])),
            Some(MeetingApp::Teams)
        );
        assert_eq!(
            label_meeting_app(&names(&["ms-teams.exe"])),
            Some(MeetingApp::Teams)
        );
        assert_eq!(
            label_meeting_app(&names(&["Microsoft Teams"])),
            Some(MeetingApp::Teams)
        );
        assert_eq!(
            label_meeting_app(&names(&["Webex"])),
            Some(MeetingApp::Webex)
        );
        assert_eq!(
            label_meeting_app(&names(&["Discord Helper"])),
            Some(MeetingApp::Discord)
        );
        assert_eq!(
            label_meeting_app(&names(&["Slack"])),
            Some(MeetingApp::Slack)
        );
    }

    #[test]
    fn teamspeak_is_not_teams() {
        assert_eq!(label_meeting_app(&names(&["TeamSpeak3", "ts3client"])), None);
    }

    #[test]
    fn search_daemons_are_not_arc_browser() {
        assert_eq!(
            label_meeting_app(&names(&["searchpartyd", "mdworker"])),
            None
        );
    }

    #[test]
    fn dedicated_app_beats_browser() {
        assert_eq!(
            label_meeting_app(&names(&["Google Chrome", "zoom.us", "Safari"])),
            Some(MeetingApp::Zoom)
        );
    }

    #[test]
    fn browser_fallback() {
        assert_eq!(
            label_meeting_app(&names(&["Google Chrome Helper", "kernel_task"])),
            Some(MeetingApp::Browser)
        );
        assert_eq!(label_meeting_app(&names(&["Arc"])), Some(MeetingApp::Browser));
    }

    #[test]
    fn no_match_is_none() {
        assert_eq!(label_meeting_app(&names(&["kernel_task", "launchd"])), None);
    }

    const T: Duration = Duration::from_secs(3);

    #[test]
    fn detects_once_per_episode() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        assert_eq!(
            tracker.tick(t0, false, true, Some(MeetingApp::Zoom)),
            Some(DetectEvent::CallDetected(MeetingApp::Zoom))
        );
        assert_eq!(tracker.tick(t0 + T, false, true, Some(MeetingApp::Zoom)), None);
        assert_eq!(tracker.tick(t0 + 2 * T, false, true, Some(MeetingApp::Zoom)), None);
    }

    #[test]
    fn idle_for_8s_resets_and_emits_ended() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        tracker.tick(t0, false, true, Some(MeetingApp::Teams));
        assert_eq!(tracker.tick(t0 + T, false, false, None), None);
        // Mic comes back before the reset window — episode survives, no re-emit.
        assert_eq!(tracker.tick(t0 + 2 * T, false, true, Some(MeetingApp::Teams)), None);
        // Now stays idle past 8s.
        assert_eq!(tracker.tick(t0 + 3 * T, false, false, None), None);
        assert_eq!(
            tracker.tick(t0 + 3 * T + ACTIVE_RESET, false, false, None),
            Some(DetectEvent::CallEnded)
        );
        // Fresh episode detects again.
        assert_eq!(
            tracker.tick(t0 + 4 * T + ACTIVE_RESET, false, true, Some(MeetingApp::Teams)),
            Some(DetectEvent::CallDetected(MeetingApp::Teams))
        );
    }

    /// Back-to-back calls: call 1 ends, call 2 joined ~10s later (after the
    /// 8s active reset) must produce a fresh detection — this was the
    /// no-widget-for-call-2 bug under the old 30s window.
    #[test]
    fn back_to_back_calls_both_detected() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        assert_eq!(
            tracker.tick(t0, false, true, Some(MeetingApp::Zoom)),
            Some(DetectEvent::CallDetected(MeetingApp::Zoom))
        );
        // Call 1 hangs up: mic goes idle.
        assert_eq!(tracker.tick(t0 + T, false, false, None), None);
        assert_eq!(
            tracker.tick(t0 + T + ACTIVE_RESET, false, false, None),
            Some(DetectEvent::CallEnded)
        );
        // Call 2 starts ~11s after call 1 ended its mic stream.
        assert_eq!(
            tracker.tick(t0 + T + ACTIVE_RESET + T, false, true, Some(MeetingApp::Teams)),
            Some(DetectEvent::CallDetected(MeetingApp::Teams))
        );
    }

    const DISMISSED_RESET: Duration = Duration::from_secs(DISMISSED_RESET_SECS);

    #[test]
    fn dismiss_suppresses_until_episode_resets() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        tracker.tick(t0, false, true, Some(MeetingApp::Webex));
        tracker.dismiss();
        assert_eq!(tracker.tick(t0 + T, false, true, Some(MeetingApp::Webex)), None);
        let t_idle = t0 + 2 * T;
        assert_eq!(tracker.tick(t_idle, false, false, None), None);
        assert_eq!(
            tracker.tick(t_idle + DISMISSED_RESET, false, false, None),
            Some(DetectEvent::CallEnded)
        );
        assert_eq!(
            tracker.tick(t_idle + DISMISSED_RESET + T, false, true, Some(MeetingApp::Webex)),
            Some(DetectEvent::CallDetected(MeetingApp::Webex))
        );
    }

    /// Dismissing call 1 then joining call 2 shortly after must detect
    /// call 2: a dismissed episode resets after only 5s of mic silence.
    #[test]
    fn dismissed_resets_after_5s_idle_so_back_to_back_call_is_detected() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        tracker.tick(t0, false, true, Some(MeetingApp::Zoom));
        tracker.dismiss();
        // Call 1 ends; 3s of silence is not yet enough.
        assert_eq!(tracker.tick(t0 + T, false, false, None), None);
        assert_eq!(tracker.tick(t0 + 2 * T - Duration::from_secs(1), false, false, None), None);
        // 5s of silence resets the dismissed episode.
        assert_eq!(
            tracker.tick(t0 + T + DISMISSED_RESET, false, false, None),
            Some(DetectEvent::CallEnded)
        );
        // Call 2 joined moments later → detected.
        assert_eq!(
            tracker.tick(t0 + 2 * T + DISMISSED_RESET, false, true, Some(MeetingApp::Teams)),
            Some(DetectEvent::CallDetected(MeetingApp::Teams))
        );
    }

    /// Undismissed episodes keep the longer window: a 7s mic blip must NOT
    /// end the call (no widget flapping), and the 5s dismissed window must
    /// not apply either — only 8s+ of sustained idle resets.
    #[test]
    fn active_episode_still_needs_8s_idle() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        tracker.tick(t0, false, true, Some(MeetingApp::Zoom));
        assert_eq!(tracker.tick(t0 + T, false, false, None), None);
        // 5s idle (the dismissed-window length) is not enough while Active.
        assert_eq!(tracker.tick(t0 + T + DISMISSED_RESET, false, false, None), None);
        // 7s idle still rides it out.
        assert_eq!(
            tracker.tick(t0 + T + ACTIVE_RESET - Duration::from_secs(1), false, false, None),
            None
        );
        // 8s idle resets.
        assert_eq!(
            tracker.tick(t0 + T + ACTIVE_RESET, false, false, None),
            Some(DetectEvent::CallEnded)
        );
    }

    #[test]
    fn snapshot_reflects_phase() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        assert_eq!(tracker.snapshot(), (false, None));
        tracker.tick(t0, false, true, Some(MeetingApp::Slack));
        assert_eq!(tracker.snapshot(), (true, Some(MeetingApp::Slack)));
        tracker.dismiss();
        assert_eq!(tracker.snapshot(), (false, None));
        tracker.tick(t0 + T, false, false, None);
        tracker.tick(t0 + T + DISMISSED_RESET, false, false, None);
        assert_eq!(tracker.snapshot(), (false, None));
    }

    #[test]
    fn recording_suppresses_everything() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        // No detection while recording, even with mic + meeting app present.
        assert_eq!(tracker.tick(t0, true, false, None), None);
        // Active episode + recording starts → behaves like a dismissal.
        assert_eq!(
            tracker.tick(t0 + T, false, true, Some(MeetingApp::Zoom)),
            Some(DetectEvent::CallDetected(MeetingApp::Zoom))
        );
        assert_eq!(tracker.tick(t0 + 2 * T, true, false, None), None);
        // Recording ended, mic still hot in the same call → no re-emit.
        assert_eq!(tracker.tick(t0 + 3 * T, false, true, Some(MeetingApp::Zoom)), None);
        // The recording acted as a dismissal, so the short reset applies.
        let t_idle = t0 + 4 * T;
        assert_eq!(tracker.tick(t_idle, false, false, None), None);
        assert_eq!(
            tracker.tick(t_idle + DISMISSED_RESET, false, false, None),
            Some(DetectEvent::CallEnded)
        );
        assert_eq!(
            tracker.tick(t_idle + DISMISSED_RESET + T, false, true, Some(MeetingApp::Zoom)),
            Some(DetectEvent::CallDetected(MeetingApp::Zoom))
        );
    }

    const STOP_WINDOW: Duration = Duration::from_secs(CALL_END_STOP_SECS);

    #[test]
    fn autostop_manual_recording_with_no_call_never_fires() {
        let mut t = AutoStopTracker::default();
        let t0 = Instant::now();
        // Manual recording (detected = false) where no other app is ever on
        // the mic — a plain voice memo — never arms, so idle never fires.
        assert!(!t.tick(t0, Some(("m1", false)), Some(false)));
        assert!(!t.tick(t0 + STOP_WINDOW, Some(("m1", false)), Some(false)));
        assert!(!t.tick(t0 + 2 * STOP_WINDOW, Some(("m1", false)), Some(false)));
    }

    #[test]
    fn autostop_manual_recording_arms_when_a_call_appears() {
        let mut t = AutoStopTracker::default();
        let t0 = Instant::now();
        // Manual recording (detected = false): not armed at start.
        assert!(!t.tick(t0, Some(("m1", false)), Some(false)));
        // A call appears — another process takes the mic → arms dynamically.
        assert!(!t.tick(t0 + T, Some(("m1", false)), Some(true)));
        // Call ends: 20s of consecutive idle now fires the auto-stop.
        let t1 = t0 + 2 * T;
        assert!(!t.tick(t1, Some(("m1", false)), Some(false)));
        assert!(t.tick(t1 + STOP_WINDOW, Some(("m1", false)), Some(false)));
    }

    #[test]
    fn autostop_fires_once_after_20s_consecutive_idle() {
        let mut t = AutoStopTracker::default();
        let t0 = Instant::now();
        assert!(!t.tick(t0, Some(("m1", true)), Some(false)));
        assert!(!t.tick(t0 + T, Some(("m1", true)), Some(false)));
        assert!(t.tick(t0 + STOP_WINDOW, Some(("m1", true)), Some(false)));
        // One-shot: keeps returning false for the same recording.
        assert!(!t.tick(t0 + STOP_WINDOW + T, Some(("m1", true)), Some(false)));
        assert!(!t.tick(t0 + 3 * STOP_WINDOW, Some(("m1", true)), Some(false)));
    }

    #[test]
    fn autostop_other_mic_active_resets_idle_run() {
        let mut t = AutoStopTracker::default();
        let t0 = Instant::now();
        assert!(!t.tick(t0, Some(("m1", true)), Some(false)));
        // Call still holds the mic — run resets.
        assert!(!t.tick(t0 + 5 * T, Some(("m1", true)), Some(true)));
        // 18s of idle measured from the reset: not yet.
        let t1 = t0 + 6 * T;
        assert!(!t.tick(t1, Some(("m1", true)), Some(false)));
        assert!(!t.tick(t1 + STOP_WINDOW - Duration::from_secs(2), Some(("m1", true)), Some(false)));
        // Full 20s from the reset fires.
        assert!(t.tick(t1 + STOP_WINDOW, Some(("m1", true)), Some(false)));
    }

    #[test]
    fn autostop_unknown_probe_resets_and_never_fires() {
        let mut t = AutoStopTracker::default();
        let t0 = Instant::now();
        assert!(!t.tick(t0, Some(("m1", true)), Some(false)));
        // Probe went unknown mid-run — resets, must not count as idle.
        assert!(!t.tick(t0 + 5 * T, Some(("m1", true)), None));
        assert!(!t.tick(t0 + STOP_WINDOW + T, Some(("m1", true)), Some(false)));
        // Unknown forever never fires.
        let mut t = AutoStopTracker::default();
        for i in 0..20u32 {
            assert!(!t.tick(t0 + i * STOP_WINDOW, Some(("m1", true)), None));
        }
    }

    #[test]
    fn autostop_rearms_for_a_new_recording() {
        let mut t = AutoStopTracker::default();
        let t0 = Instant::now();
        assert!(!t.tick(t0, Some(("m1", true)), Some(false)));
        assert!(t.tick(t0 + STOP_WINDOW, Some(("m1", true)), Some(false)));
        // Recorder goes idle, then a new detected recording starts.
        assert!(!t.tick(t0 + STOP_WINDOW + T, None, None));
        let t1 = t0 + STOP_WINDOW + 2 * T;
        assert!(!t.tick(t1, Some(("m2", true)), Some(false)));
        assert!(t.tick(t1 + STOP_WINDOW, Some(("m2", true)), Some(false)));
    }

    #[test]
    fn autostop_recording_swap_without_idle_gap_rearms() {
        let mut t = AutoStopTracker::default();
        let t0 = Instant::now();
        assert!(!t.tick(t0, Some(("m1", true)), Some(false)));
        // A different meeting id resets the idle run AND the fired guard.
        assert!(!t.tick(t0 + STOP_WINDOW, Some(("m2", true)), Some(false)));
        assert!(t.tick(t0 + 2 * STOP_WINDOW, Some(("m2", true)), Some(false)));
    }

    const NOTICE_QUIET: Duration = Duration::from_secs(NOTICE_QUIET_SECS);

    #[test]
    fn notice_manual_recording_fires_once_on_other_mic_edge() {
        let mut t = SuppressedCallTracker::default();
        let t0 = Instant::now();
        // Mic free, then another process grabs it → call started mid-recording.
        assert!(!t.tick(t0, Some(("m1", false)), Some(false)));
        assert!(t.tick(t0 + T, Some(("m1", false)), Some(true)));
        // One-shot: stays silent for the rest of the recording.
        assert!(!t.tick(t0 + 2 * T, Some(("m1", false)), Some(true)));
        assert!(!t.tick(t0 + 3 * T, Some(("m1", false)), Some(false)));
        assert!(!t.tick(t0 + 4 * T, Some(("m1", false)), Some(true)));
    }

    #[test]
    fn notice_manual_recording_started_during_call_stays_silent() {
        let mut t = SuppressedCallTracker::default();
        let t0 = Instant::now();
        // The user knowingly started recording while already on a call:
        // other_mic is true from the first observation → no toast.
        assert!(!t.tick(t0, Some(("m1", false)), Some(true)));
        assert!(!t.tick(t0 + 10 * T, Some(("m1", false)), Some(true)));
        // That call ends, a NEW call starts → toast.
        assert!(!t.tick(t0 + 11 * T, Some(("m1", false)), Some(false)));
        assert!(t.tick(t0 + 12 * T, Some(("m1", false)), Some(true)));
    }

    #[test]
    fn notice_detected_recording_needs_30s_quiet_before_new_call() {
        let mut t = SuppressedCallTracker::default();
        let t0 = Instant::now();
        // The recorded call holds the mic from the start.
        assert!(!t.tick(t0, Some(("m1", true)), Some(true)));
        // A brief device-handoff blip (6s quiet) is NOT a new call.
        assert!(!t.tick(t0 + T, Some(("m1", true)), Some(false)));
        assert!(!t.tick(t0 + 2 * T, Some(("m1", true)), Some(false)));
        assert!(!t.tick(t0 + 3 * T, Some(("m1", true)), Some(true)));
        // Original call ends: 30s of consecutive quiet…
        let t1 = t0 + 4 * T;
        assert!(!t.tick(t1, Some(("m1", true)), Some(false)));
        assert!(!t.tick(t1 + NOTICE_QUIET, Some(("m1", true)), Some(false)));
        // …then someone is on the mic again → new call, toast.
        assert!(t.tick(t1 + NOTICE_QUIET + T, Some(("m1", true)), Some(true)));
        // One-shot for this recording.
        assert!(!t.tick(t1 + NOTICE_QUIET + 2 * T, Some(("m1", true)), Some(true)));
    }

    #[test]
    fn notice_unknown_probe_resets_quiet_run_and_never_fires() {
        let mut t = SuppressedCallTracker::default();
        let t0 = Instant::now();
        assert!(!t.tick(t0, Some(("m1", true)), Some(false)));
        // Probe goes unknown 15s in — the quiet run must restart.
        assert!(!t.tick(t0 + 5 * T, Some(("m1", true)), None));
        let t1 = t0 + 6 * T;
        assert!(!t.tick(t1, Some(("m1", true)), Some(false)));
        assert!(!t.tick(t1 + NOTICE_QUIET - Duration::from_secs(2), Some(("m1", true)), Some(false)));
        // 28s since the restart → a true here must NOT fire.
        assert!(!t.tick(t1 + NOTICE_QUIET - Duration::from_secs(1), Some(("m1", true)), Some(true)));
        // Unknown forever never fires.
        let mut t = SuppressedCallTracker::default();
        for i in 0..20u32 {
            assert!(!t.tick(t0 + i * NOTICE_QUIET, Some(("m1", false)), None));
        }
    }

    #[test]
    fn notice_rearms_for_a_new_recording_and_resets_when_idle() {
        let mut t = SuppressedCallTracker::default();
        let t0 = Instant::now();
        assert!(!t.tick(t0, Some(("m1", false)), Some(false)));
        assert!(t.tick(t0 + T, Some(("m1", false)), Some(true)));
        assert!(!t.wants_probe("m1"));
        // Recorder idle → state clears.
        assert!(!t.tick(t0 + 2 * T, None, None));
        assert!(t.wants_probe("m2"));
        // New recording can fire again — but quiet must be re-observed first.
        let t1 = t0 + 3 * T;
        assert!(!t.tick(t1, Some(("m2", false)), Some(true)));
        assert!(!t.tick(t1 + T, Some(("m2", false)), Some(false)));
        assert!(t.tick(t1 + 2 * T, Some(("m2", false)), Some(true)));
    }

    #[test]
    fn notice_wants_probe_until_fired() {
        let mut t = SuppressedCallTracker::default();
        let t0 = Instant::now();
        assert!(t.wants_probe("m1"));
        assert!(!t.tick(t0, Some(("m1", false)), Some(false)));
        assert!(t.wants_probe("m1"));
        assert!(t.tick(t0 + T, Some(("m1", false)), Some(true)));
        // Fired → steady state goes probe-free for this recording…
        assert!(!t.wants_probe("m1"));
        // …but a recording swap without an idle gap re-wants the probe.
        assert!(t.wants_probe("m2"));
    }

    /// Manual probe: `cargo test --lib meetings::detect -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_other_process_uses_mic() {
        let started = Instant::now();
        let other = other_process_uses_mic();
        println!("other_process_uses_mic={other:?} ({:?})", started.elapsed());
    }

    /// Manual probe: `cargo test --lib meetings::detect -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_mic_and_processes() {
        let started = Instant::now();
        let mic = mic_in_use();
        let mic_cost = started.elapsed();

        let mut sys = System::new();
        let started = Instant::now();
        let names = process_name_snapshot(&mut sys);
        let cold = started.elapsed();
        let started = Instant::now();
        let names = {
            let _ = names;
            process_name_snapshot(&mut sys)
        };
        let warm = started.elapsed();

        let label = label_meeting_app(&names);
        println!(
            "mic_in_use={mic} ({mic_cost:?}) | {} processes (cold {cold:?}, warm {warm:?}) | label={label:?}",
            names.len()
        );
    }
}

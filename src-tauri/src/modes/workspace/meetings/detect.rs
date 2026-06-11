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
use crate::shared::repos::settings as settings_repo;

pub const SETTING_KEY: &str = "workspace_meeting_detect_enabled";

const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Active episodes ride out long mutes so the widget doesn't flap.
const IDLE_RESET: Duration = Duration::from_secs(30);
/// Dismissed episodes reset fast: dismissing call 1 then joining call 2
/// moments later must still detect call 2.
const DISMISSED_RESET_SECS: u64 = 5;

const EVT_DETECTED: &str = "meetings:call-detected";
const EVT_ENDED: &str = "meetings:call-ended";

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

/// One "episode" spans mic-active → sustained mic silence (30s while
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
            _ => IDLE_RESET,
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

// --- Managed state + commands surface ---

#[derive(Clone, Serialize)]
pub struct DetectStatus {
    pub enabled: bool,
    pub app: Option<MeetingApp>,
    pub active: bool,
}

pub struct DetectState {
    enabled: AtomicBool,
    tracker: Mutex<EpisodeTracker>,
}

impl Default for DetectState {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(true),
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

    pub fn dismiss(&self) {
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

#[cfg(target_os = "macos")]
mod os {
    use cidre::core_audio as ca;

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
}

#[cfg(target_os = "windows")]
mod os {
    use windows::Win32::Media::Audio::{
        eCapture, eMultimedia, AudioSessionStateActive, IAudioSessionManager2,
        IMMDeviceEnumerator, MMDeviceEnumerator,
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
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod os {
    pub fn mic_in_use() -> bool {
        false
    }
}

// --- Poller ---

#[derive(Clone, Serialize)]
struct CallDetectedPayload {
    app: MeetingApp,
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
        }

        // Reusing one System keeps each refresh incremental instead of a
        // from-scratch process-table build every tick.
        let sys = Arc::new(Mutex::new(System::new()));

        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let state = app.state::<DetectState>();
            if !state.enabled() {
                if state.tracker.lock().reset() {
                    let _ = app.emit(EVT_ENDED, ());
                }
                continue;
            }

            let status = app.state::<RecorderState>().status();
            if status.recording || status.stopping {
                let _ = state.tracker.lock().tick(Instant::now(), true, false, None);
                continue;
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

            let event = state.tracker.lock().tick(Instant::now(), false, mic, label);
            match event {
                Some(DetectEvent::CallDetected(meeting_app)) => {
                    let _ = app.emit(EVT_DETECTED, CallDetectedPayload { app: meeting_app });
                }
                Some(DetectEvent::CallEnded) => {
                    let _ = app.emit(EVT_ENDED, ());
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
    fn idle_for_30s_resets_and_emits_ended() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        tracker.tick(t0, false, true, Some(MeetingApp::Teams));
        assert_eq!(tracker.tick(t0 + T, false, false, None), None);
        // Mic comes back before the reset window — episode survives, no re-emit.
        assert_eq!(tracker.tick(t0 + 2 * T, false, true, Some(MeetingApp::Teams)), None);
        // Now stays idle past 30s.
        assert_eq!(tracker.tick(t0 + 3 * T, false, false, None), None);
        assert_eq!(
            tracker.tick(t0 + 3 * T + IDLE_RESET, false, false, None),
            Some(DetectEvent::CallEnded)
        );
        // Fresh episode detects again.
        assert_eq!(
            tracker.tick(t0 + 4 * T + IDLE_RESET, false, true, Some(MeetingApp::Teams)),
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
        // Call 2 joined well inside the old 30s window → detected.
        assert_eq!(
            tracker.tick(t0 + 2 * T + DISMISSED_RESET, false, true, Some(MeetingApp::Teams)),
            Some(DetectEvent::CallDetected(MeetingApp::Teams))
        );
    }

    /// Undismissed episodes keep the long window: a 5s mute must NOT end
    /// the call (no widget flapping on brief mutes).
    #[test]
    fn active_episode_still_needs_30s_idle() {
        let mut tracker = EpisodeTracker::default();
        let t0 = Instant::now();
        tracker.tick(t0, false, true, Some(MeetingApp::Zoom));
        assert_eq!(tracker.tick(t0 + T, false, false, None), None);
        assert_eq!(tracker.tick(t0 + T + DISMISSED_RESET, false, false, None), None);
        assert_eq!(
            tracker.tick(t0 + T + IDLE_RESET - Duration::from_secs(1), false, false, None),
            None
        );
        assert_eq!(
            tracker.tick(t0 + T + IDLE_RESET, false, false, None),
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

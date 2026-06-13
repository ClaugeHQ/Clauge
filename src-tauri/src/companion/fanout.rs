// PTY fan-out: one hub per live terminal, mirroring its byte stream to
// any number of companion WebSocket subscribers while keeping a 256KB
// scrollback ring for replay-on-attach. The hubs live in module-level
// statics (same shape as cloud/scheduler.rs) because the publishers are
// the PTY reader threads / russh tasks, which have no AppHandle.
//
// Resize rule (phone-authoritative when a phone is the active driver):
// while a phone is attached and the desktop isn't focused on this session,
// the shared PTY adopts the PHONE's fit size (smallest fits all phones).
// The desktop reclaims its size when the phone leaves (after a detach
// grace) OR the moment the desktop is focused on the session. All size
// application + the client size-echo flow through `reconcile`, the single
// chokepoint: it diffs the desired size against the applied size and, when
// they differ, resizes the real PTY master through the terminal registry
// and broadcasts a `FanoutEvent::Size` so every client renders at it.
// Debounced both ends (detach grace + blur debounce) so blips/app-switches
// don't thrash. The PTY resize + broadcast NEVER run under the hubs lock.

use parking_lot::Mutex;
use portable_pty::PtySize;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{broadcast, mpsc};

use crate::modes::agent::models::TerminalState;
use crate::modes::ssh::models::{SshCommand, SshTerminalState};

/// Client id the desktop registers its viewport under.
pub const DESKTOP_CLIENT: &str = "desktop";

/// Scrollback ring capacity per terminal — enough for a phone to
/// repaint a full screen plus history without holding the whole
/// session transcript in memory.
pub const SCROLLBACK_CAP: usize = 256 * 1024;

/// Broadcast queue depth. A receiver that falls further behind than
/// this lags out and is dropped by its WS task — the phone reconnects
/// and resyncs from scrollback replay.
const BROADCAST_CAP: usize = 256;

/// How long an exited hub lingers so a late attacher still sees the
/// replayed scrollback + Exit instead of "unknown terminal".
const EXIT_UNREGISTER_GRACE: Duration = Duration::from_secs(30);

/// Attention heuristic: a hub whose tail matched a prompt pattern and
/// which has produced no output for at least this long, with no phone
/// attached, is "waiting for the user". Tunable.
pub const ATTENTION_IDLE: Duration = Duration::from_secs(10);

/// How much of the recent output tail the prompt detector inspects. A
/// shell prompt or a y/N question lands well within this window.
const PROMPT_TAIL_BYTES: usize = 256;

/// Which write/resize internals a subscriber must use for input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    Agent,
    Ssh,
}

#[derive(Debug, Clone)]
pub enum FanoutEvent {
    Out(Vec<u8>),
    Exit,
    /// The reconcile chokepoint resized the PTY: every client must render
    /// at this (cols, rows). ws.rs forwards it as a `{t:"size"}` message —
    /// the same wire shape the desktop emits on its own resizes.
    Size(u16, u16),
}

/// Grace after the last phone detaches before the desktop reclaims its
/// size — absorbs network blips and quick app-switches without flapping.
/// Public so ws.rs can schedule the post-grace reconcile with it.
pub const DETACH_GRACE: Duration = Duration::from_secs(4);

/// How long desktop ownership survives a blur, so a transient focus loss
/// (alt-tab, clicking a dialog) doesn't hand the PTY to the phone.
pub const BLUR_DEBOUNCE: Duration = Duration::from_secs(3);

/// Safe floor for any size we push to a PTY. Phones reporting zero/garbage
/// dimensions are ignored upstream; this clamps the rest.
const MIN_COLS: u16 = 10;
const MIN_ROWS: u16 = 4;

/// Push-dispatch signals fanout hands to `push.rs`. Kept minimal so
/// fanout stays decoupled from the Worker/cloud plumbing: it reports
/// *what happened*, push.rs decides whether to notify.
#[derive(Debug, Clone)]
pub enum PushTrigger {
    /// A terminal's PTY exited. `title` is the session/profile label for
    /// the notification body.
    Exit { terminal_id: String, title: String },
    /// A terminal has been idle at a prompt and wants the user's input.
    Attention { terminal_id: String, title: String },
}

struct TermHub {
    tx: broadcast::Sender<FanoutEvent>,
    scrollback: VecDeque<u8>,
    /// client id → (cols, rows)
    sizes: HashMap<String, (u16, u16)>,
    kind: TermKind,
    /// Session/profile label, surfaced as the push notification body.
    title: String,
    /// When the last output byte arrived — the idle clock for attention.
    last_output: Instant,
    /// The recent output tail matched a prompt pattern at last output.
    prompt_flag: bool,
    /// An attention push has already fired for the current idle stretch.
    /// Reset whenever new output arrives so each fresh prompt notifies
    /// at most once.
    notified: bool,
    /// Client ids of WS connections currently mirroring this terminal —
    /// i.e. phones actively viewing. Tracked separately from `sizes` so
    /// attach/detach gates push without touching the desktop-authoritative
    /// sizing logic.
    viewers: HashSet<String>,
    /// This terminal's agent reports lifecycle hooks (claude/codex Phase 1).
    /// Set on the first `set_hook_event`; once true the output heuristic is
    /// disabled for this hub (the hook is authoritative — the heuristic must
    /// not fight it). Reset to false on `register` for a fresh hub.
    hook_driven: bool,
    /// The desktop is focused on this session right now. While true the
    /// desktop owns the PTY size (the scenario-8 reclaim).
    desktop_focused: bool,
    /// When the desktop last blurred this session. Desktop ownership is
    /// held until `blur_at + BLUR_DEBOUNCE` so a flicker doesn't hand off.
    blur_at: Option<Instant>,
    /// The most recent phone-owned size, held through the detach grace so a
    /// reconnecting phone re-adopts it instantly (no wide→narrow churn).
    last_phone_size: Option<(u16, u16)>,
    /// When the last phone detached (no phones remaining). The detach grace
    /// runs from here.
    last_phone_detach_at: Option<Instant>,
    /// The size currently pushed to the PTY master. `reconcile` is a no-op
    /// when the desired size already equals this.
    applied_size: Option<(u16, u16)>,
    /// Who is driving the PTY size right now.
    /// `None` = unknown/initial (no reconcile has run yet).
    /// `Some(true)` = a phone is the active size driver.
    /// `Some(false)` = the desktop is the active size driver.
    /// Written only by `reconcile_now`; read by `publish_exit` for teardown.
    size_owner: Option<bool>,
}

/// Everything a WS connection needs at attach time, captured under one
/// lock so no byte can fall between the snapshot and the subscription.
pub struct Attached {
    pub scrollback: Vec<u8>,
    pub rx: broadcast::Receiver<FanoutEvent>,
    pub kind: TermKind,
    pub effective_size: Option<(u16, u16)>,
}

static HUBS: OnceLock<Mutex<HashMap<String, TermHub>>> = OnceLock::new();

fn hubs() -> &'static Mutex<HashMap<String, TermHub>> {
    HUBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The live push sink, installed by `push::start` when the companion
/// server starts and cleared on stop. `None` = nobody is listening, so
/// triggers are dropped (server off, or no cloud session — see push.rs).
static PUSH_SINK: OnceLock<Mutex<Option<mpsc::UnboundedSender<PushTrigger>>>> = OnceLock::new();

fn push_sink() -> &'static Mutex<Option<mpsc::UnboundedSender<PushTrigger>>> {
    PUSH_SINK.get_or_init(|| Mutex::new(None))
}

/// Install (or replace) the push sink. Called by `push::start`.
pub fn set_push_sink(tx: mpsc::UnboundedSender<PushTrigger>) {
    *push_sink().lock() = Some(tx);
}

/// Drop the push sink so later triggers are ignored. Called on server stop.
pub fn clear_push_sink() {
    *push_sink().lock() = None;
}

fn emit_push(trigger: PushTrigger) {
    if let Some(tx) = push_sink().lock().as_ref() {
        let _ = tx.send(trigger);
    }
}

/// The desktop AppHandle, stashed in `setup` so fanout can emit
/// frontend events (attention-cleared) WITHOUT routing through the
/// companion-only PUSH_SINK — the desktop dock-bounce/chime must clear
/// even when the companion server is off.
static APP_HANDLE: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();

fn app_handle() -> &'static Mutex<Option<AppHandle>> {
    APP_HANDLE.get_or_init(|| Mutex::new(None))
}

/// Install the desktop AppHandle. Called once from the Tauri `setup` hook.
pub fn set_app_handle(app: AppHandle) {
    *app_handle().lock() = Some(app);
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClearedPayload {
    terminal_id: String,
}

/// Emit `agent-attention-cleared` to the frontend. MUST be called with
/// the hubs lock dropped — never while holding it.
fn emit_cleared(terminal_id: &str) {
    if let Some(handle) = app_handle().lock().as_ref() {
        let _ = handle.emit(
            "agent-attention-cleared",
            ClearedPayload {
                terminal_id: terminal_id.to_string(),
            },
        );
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SizeOwnerPayload {
    terminal_id: String,
    phone_owned: bool,
}

/// Emit `terminal-size-owner` to the frontend. MUST be called with the
/// hubs lock dropped — never while holding it.
fn emit_size_owner(terminal_id: &str, phone_owned: bool) {
    if let Some(handle) = app_handle().lock().as_ref() {
        let _ = handle.emit(
            "terminal-size-owner",
            SizeOwnerPayload {
                terminal_id: terminal_id.to_string(),
                phone_owned,
            },
        );
    }
}

/// True if any phone is currently mirroring this terminal over a WS.
/// Used to suppress push when the user is already looking at the term.
/// The desktop never opens a companion WS to itself, so any viewer is a
/// phone. Tracked independently of `sizes` (a phone only enters `sizes`
/// once it sends a resize, which is too late for gating).
fn phone_attached(hub: &TermHub) -> bool {
    !hub.viewers.is_empty()
}

/// Register a WS viewer (phone) on attach. No-op if the hub is gone.
/// Pure state mutation — the caller fires `reconcile_now` so the PTY
/// adopts the new phone's size.
pub fn add_viewer(terminal_id: &str, client_id: &str) {
    let mut map = hubs().lock();
    if let Some(hub) = map.get_mut(terminal_id) {
        hub.viewers.insert(client_id.to_string());
    }
}

/// Forget a WS viewer on every detach path. If no phone remains, stamp
/// `last_phone_detach_at` so the detach grace runs (keeping
/// `last_phone_size` for an instant reconnect). No-op if the hub is gone.
/// The caller fires `reconcile_now` + `reconcile_after(DETACH_GRACE)`.
pub fn remove_viewer(terminal_id: &str, client_id: &str) {
    let mut map = hubs().lock();
    if let Some(hub) = map.get_mut(terminal_id) {
        hub.viewers.remove(client_id);
        let phones_left = hub
            .viewers
            .iter()
            .any(|c| c.as_str() != DESKTOP_CLIENT);
        if !phones_left {
            hub.last_phone_detach_at = Some(Instant::now());
        }
    }
}

/// Clear the attention state when input arrives from ANY source
/// (desktop keystroke, mobile keystroke, SSH write). Edge-triggered:
/// emits `agent-attention-cleared` only when the hub was actually in an
/// awaiting/notified state, so a stream of keystrokes after it's cleared
/// does not spam events. No-op if the hub is gone.
pub fn note_input(terminal_id: &str) {
    let was_awaiting = {
        let mut map = hubs().lock();
        let Some(hub) = map.get_mut(terminal_id) else {
            return;
        };
        let was_awaiting = hub.prompt_flag || hub.notified;
        hub.prompt_flag = false;
        hub.notified = false;
        hub.last_output = Instant::now();
        was_awaiting
    };
    if was_awaiting {
        emit_cleared(terminal_id);
    }
}

/// Whether this terminal is genuinely waiting for the user: its tail
/// looked like a prompt and it has been idle ≥ ATTENTION_IDLE. No
/// phone-attached / notified gating — this reflects the real waiting
/// state for the mobile list dot. False if the hub is gone.
pub fn is_awaiting(terminal_id: &str) -> bool {
    let map = hubs().lock();
    match map.get(terminal_id) {
        // Hook-driven hubs carry an authoritative signal — no idle debounce.
        // The agent told us it's waiting, so reflect it the instant the dot
        // is queried rather than waiting out ATTENTION_IDLE.
        Some(hub) if hub.hook_driven => hub.prompt_flag,
        Some(hub) => hub.prompt_flag && hub.last_output.elapsed() >= ATTENTION_IDLE,
        None => false,
    }
}

/// Apply an authoritative agent lifecycle event to a terminal's awaiting
/// state (Phase 1: claude/codex). Marks the hub `hook_driven` (which
/// disables the output heuristic for it) and then maps the event:
///   - needs-user events  → awaiting ON  (`prompt_flag = true`)
///   - clear events       → awaiting OFF (like `note_input`)
/// Unknown events are ignored (no state change). Case-insensitive.
/// `agent-attention-cleared` is emitted only on a real awaiting→clear
/// transition, with the hubs lock dropped.
pub fn set_hook_event(terminal_id: &str, event: &str) {
    #[derive(PartialEq)]
    enum Kind {
        NeedsUser,
        Clear,
        Unknown,
    }
    let lower = event.trim().to_ascii_lowercase();
    let kind = match lower.as_str() {
        // Claude: Notification + PreToolUse mean the agent is asking the
        // user (permission prompt / idle notification). Codex emits
        // *_approval_request / request_user_input.
        "notification" | "pretooluse" | "permissionrequest" | "request_user_input" => {
            Kind::NeedsUser
        }
        // Claude lifecycle resume/turn boundaries + Codex task lifecycle —
        // all mean "not waiting on the user".
        "stop" | "start" | "userpromptsubmit" | "posttooluse" | "sessionstart"
        | "sessionend" | "task_complete" | "task_started" => Kind::Clear,
        other => {
            // Codex approval requests carry an agent-specific prefix
            // (`exec_approval_request`, `apply_patch_approval_request`, …).
            if other.ends_with("_approval_request") {
                Kind::NeedsUser
            } else {
                Kind::Unknown
            }
        }
    };
    if kind == Kind::Unknown {
        return;
    }
    let cleared = {
        let mut map = hubs().lock();
        let Some(hub) = map.get_mut(terminal_id) else {
            return;
        };
        hub.hook_driven = true;
        match kind {
            Kind::NeedsUser => {
                hub.prompt_flag = true;
                hub.last_output = Instant::now();
                false
            }
            Kind::Clear => {
                let was_awaiting = hub.prompt_flag || hub.notified;
                hub.prompt_flag = false;
                hub.notified = false;
                was_awaiting
            }
            Kind::Unknown => false,
        }
    };
    if cleared {
        emit_cleared(terminal_id);
    }
}

/// Pure prompt detector: does the output tail look like the terminal is
/// waiting for input? Matches trailing `? `, `[y/N]`/`[Y/n]`, a trailing
/// shell prompt glyph (`❯`/`$ `/`# `), or a BEL (0x07) anywhere in the
/// tail. Kept free of any I/O so the attention unit test can drive it
/// directly with byte sequences.
pub fn looks_like_prompt(tail: &[u8]) -> bool {
    if tail.contains(&0x07) {
        return true;
    }
    // Work on the trimmed-right text so a trailing newline doesn't hide
    // an otherwise-matching prompt.
    let text = String::from_utf8_lossy(tail);
    let trimmed = text.trim_end_matches([' ', '\t', '\r', '\n']);
    if !trimmed.is_empty() {
        let lower = trimmed.to_ascii_lowercase();
        if lower.ends_with("[y/n]") || lower.ends_with("[y/n]?") {
            return true;
        }
        if trimmed.ends_with('?') {
            return true;
        }
        // A bare prompt sigil with the question mark already consumed:
        // the last visual char (pre-trim) was a space following one of
        // these.
        let last = trimmed.chars().last().unwrap();
        if matches!(last, '❯' | '$' | '#' | '>' | ':') {
            return true;
        }
    }
    // Superset: match the desktop frontend's proven patterns against an
    // ANSI-stripped copy of the tail, so the backend's awaiting state is
    // at least as sensitive as the desktop UI (TUI agents paint prompts
    // with cursor moves, so the raw tail heuristic above misses them).
    matches_prompt_phrases(&strip_ansi(&text))
}

/// Strip ANSI CSI sequences (`\x1b[…<letter>`) and OSC sequences
/// (`\x1b]…\x07`) so phrase matching sees the visible text.
fn strip_ansi(s: &str) -> String {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07]*\x07").unwrap()
    });
    re.replace_all(s, "").into_owned()
}

/// Case-insensitive match on the desktop frontend's proven approval /
/// input-waiting phrases.
fn matches_prompt_phrases(text: &str) -> bool {
    let lower = text.to_lowercase();
    let has_allow_deny = lower.contains("allow") && lower.contains("deny");
    has_allow_deny
        || lower.contains("do you want to proceed")
        || lower.contains("(y/n)")
        || lower.contains("[y/n]")
        || lower.contains("press enter")
        || lower.contains("approve this")
        || lower.contains("permission")
        || lower.contains("yes, and don")
}

/// Clamp a candidate size to the safe PTY floor.
fn clamp_size((c, r): (u16, u16)) -> (u16, u16) {
    (c.max(MIN_COLS), r.max(MIN_ROWS))
}

/// A size is usable only if both dimensions are non-zero — phones can
/// briefly report a 0 fit before their container has real width.
fn valid_size((c, r): (u16, u16)) -> bool {
    c > 0 && r > 0
}

/// The min (cols, rows) across every attached phone with a reported,
/// non-garbage size. None if no phone qualifies.
fn min_phone_size(hub: &TermHub) -> Option<(u16, u16)> {
    hub.viewers
        .iter()
        .filter(|c| c.as_str() != DESKTOP_CLIENT)
        .filter_map(|c| hub.sizes.get(c).copied())
        .filter(|&s| valid_size(s))
        .reduce(|(ac, ar), (c, r)| (ac.min(c), ar.min(r)))
}

/// The PTY size the hub should be driven at right now — phone-authoritative
/// when a phone is the active driver.
///
/// - The desktop keeps ownership while focused, and through the blur
///   debounce after losing focus (desktop present/focused wins — scenario 8).
/// - Otherwise an attached phone owns the size: the min over all phones, so
///   the smallest viewport fits all. Cached in `last_phone_size`.
/// - When the last phone has just left, the held phone size is kept through
///   the detach grace so a quick reconnect re-adopts it instantly.
/// - Otherwise the desktop's size drives the PTY (default / fully restored).
///
/// Mutates `last_phone_size` / honours `last_phone_detach_at`, so it takes
/// `&mut TermHub`. Result is clamped to the safe floor.
///
/// Returns `(desired_size, phone_owned)`. `phone_owned` is `true` when the
/// result came from a live phone or the detach-grace hold of a phone size
/// (i.e. the desktop did NOT win the size decision).
fn desired_size(hub: &mut TermHub, now: Instant) -> (Option<(u16, u16)>, bool) {
    let desktop = hub.sizes.get(DESKTOP_CLIENT).copied();

    let effective_focused = hub.desktop_focused
        || hub
            .blur_at
            .map_or(false, |t| now < t + BLUR_DEBOUNCE);

    if effective_focused {
        return (desktop.map(clamp_size), false);
    }

    if let Some(phone) = min_phone_size(hub) {
        hub.last_phone_size = Some(phone);
        return (Some(clamp_size(phone)), true);
    }

    let in_grace = hub
        .last_phone_detach_at
        .map_or(false, |t| now < t + DETACH_GRACE);
    if in_grace {
        if let Some(held) = hub.last_phone_size {
            return (Some(clamp_size(held)), true);
        }
    }

    (desktop.map(clamp_size), false)
}

/// Read-only desired size for snapshot callers (attach). Mirrors
/// `desired_size` but without mutating `last_phone_size`.
fn desired_size_readonly(hub: &TermHub, now: Instant) -> Option<(u16, u16)> {
    let desktop = hub.sizes.get(DESKTOP_CLIENT).copied();
    let effective_focused = hub.desktop_focused
        || hub.blur_at.map_or(false, |t| now < t + BLUR_DEBOUNCE);
    if effective_focused {
        return desktop.map(clamp_size);
    }
    if let Some(phone) = min_phone_size(hub) {
        return Some(clamp_size(phone));
    }
    let in_grace = hub
        .last_phone_detach_at
        .map_or(false, |t| now < t + DETACH_GRACE);
    if in_grace {
        if let Some(held) = hub.last_phone_size {
            return Some(clamp_size(held));
        }
    }
    desktop.map(clamp_size)
}

/// Create the hub for a freshly spawned terminal. Must run before the
/// reader loop starts so the first bytes land in scrollback.
pub fn register(terminal_id: &str, kind: TermKind, title: &str) {
    let (tx, _) = broadcast::channel(BROADCAST_CAP);
    hubs().lock().insert(
        terminal_id.to_string(),
        TermHub {
            tx,
            scrollback: VecDeque::new(),
            sizes: HashMap::new(),
            kind,
            title: title.to_string(),
            last_output: Instant::now(),
            prompt_flag: false,
            notified: false,
            viewers: HashSet::new(),
            hook_driven: false,
            desktop_focused: false,
            blur_at: None,
            last_phone_size: None,
            last_phone_detach_at: None,
            applied_size: None,
            size_owner: None,
        },
    );
}

/// Drop the hub entirely (normally via the post-exit grace timer).
pub fn unregister(terminal_id: &str) {
    hubs().lock().remove(terminal_id);
}

/// Append output to scrollback and fan it out. Sync and non-blocking —
/// this is called from the blocking PTY reader thread, so it must
/// never wait on a subscriber (`broadcast::send` never blocks; with no
/// receivers it just returns Err, which we ignore).
pub fn publish(terminal_id: &str, bytes: &[u8]) {
    let mut map = hubs().lock();
    let Some(hub) = map.get_mut(terminal_id) else {
        return;
    };
    hub.scrollback.extend(bytes.iter().copied());
    let len = hub.scrollback.len();
    if len > SCROLLBACK_CAP {
        hub.scrollback.drain(..len - SCROLLBACK_CAP);
    }
    // Attention bookkeeping: fresh output resets the idle clock and the
    // notified latch. For heuristic-driven hubs it also re-evaluates the
    // prompt flag against the tail. Hook-driven hubs (claude/codex) own
    // their awaiting state via `set_hook_event` / `note_input` / exit, so
    // the heuristic must NOT recompute `prompt_flag` here — otherwise it
    // would fight the authoritative signal and resurrect false positives.
    hub.last_output = Instant::now();
    hub.notified = false;
    if !hub.hook_driven {
        let (a, b) = hub.scrollback.as_slices();
        let combined = [a, b].concat();
        let tail = &combined[combined.len().saturating_sub(PROMPT_TAIL_BYTES)..];
        hub.prompt_flag = looks_like_prompt(tail);
    }
    let _ = hub.tx.send(FanoutEvent::Out(bytes.to_vec()));
}

/// Broadcast Exit, then unregister after a grace window. Uses a plain
/// detached thread for the timer because callers include the PTY
/// reader thread, which has no tokio runtime context.
pub fn publish_exit(terminal_id: &str) {
    let (was_notified, was_phone_owned) = {
        let mut map = hubs().lock();
        let Some(hub) = map.get_mut(terminal_id) else {
            return;
        };
        let _ = hub.tx.send(FanoutEvent::Exit);
        // Notify only when no phone is watching this terminal — if the
        // user is attached they already saw it exit.
        if !phone_attached(hub) {
            emit_push(PushTrigger::Exit {
                terminal_id: terminal_id.to_string(),
                title: hub.title.clone(),
            });
        }
        // Clear attention so the post-exit sweep can't fire a spurious
        // Attention during the 30s lingering window (B7 double-notify).
        let was_notified = hub.notified;
        hub.prompt_flag = false;
        hub.notified = false;
        // If the terminal was phone-owned, clear the size-owner badge.
        let was_phone_owned = hub.size_owner == Some(true);
        hub.size_owner = Some(false);
        (was_notified, was_phone_owned)
    };
    if was_notified {
        emit_cleared(terminal_id);
    }
    if was_phone_owned {
        emit_size_owner(terminal_id, false);
    }
    let id = terminal_id.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(EXIT_UNREGISTER_GRACE);
        unregister(&id);
    });
}

// dead_code: production attaches via `attach` (snapshot + subscribe
// under one lock); the standalone snapshot exists for tests and the
// D4 attention sweep.
#[allow(dead_code)]
pub fn snapshot_scrollback(terminal_id: &str) -> Vec<u8> {
    let map = hubs().lock();
    match map.get(terminal_id) {
        Some(hub) => {
            let (a, b) = hub.scrollback.as_slices();
            [a, b].concat()
        }
        None => Vec::new(),
    }
}

/// Atomic scrollback snapshot + broadcast subscription for a new WS
/// connection. None = unknown terminal.
pub fn attach(terminal_id: &str) -> Option<Attached> {
    let map = hubs().lock();
    let hub = map.get(terminal_id)?;
    let (a, b) = hub.scrollback.as_slices();
    Some(Attached {
        scrollback: [a, b].concat(),
        rx: hub.tx.subscribe(),
        kind: hub.kind,
        effective_size: desired_size_readonly(hub, Instant::now()),
    })
}

// dead_code: callers get the effective size from `attach` /
// `set_client_size` / `remove_client` return values; the direct query
// completes the hub API for tests.
#[allow(dead_code)]
pub fn effective_size(terminal_id: &str) -> Option<(u16, u16)> {
    let map = hubs().lock();
    desired_size_readonly(map.get(terminal_id)?, Instant::now())
}

/// Record a client's viewport. Pure state mutation — application of the
/// PTY resize is the caller's `reconcile_now` (the single chokepoint).
/// No-op if the hub is gone.
pub fn set_client_size(terminal_id: &str, client: &str, cols: u16, rows: u16) {
    let mut map = hubs().lock();
    if let Some(hub) = map.get_mut(terminal_id) {
        hub.sizes.insert(client.to_string(), (cols, rows));
    }
}

/// Forget a detached client's recorded viewport. Pure state mutation;
/// the caller fires `reconcile_now`. No-op if the hub is gone.
pub fn remove_client(terminal_id: &str, client: &str) {
    let mut map = hubs().lock();
    if let Some(hub) = map.get_mut(terminal_id) {
        hub.sizes.remove(client);
    }
}

/// Set the desktop-focus flag for a session. On focus → instant reclaim
/// of the desktop size. On blur → stamp `blur_at` so ownership survives
/// the debounce. The caller fires the reconcile (now on focus,
/// after BLUR_DEBOUNCE on blur). No-op if the hub is gone.
pub fn set_desktop_focused(terminal_id: &str, focused: bool) {
    let mut map = hubs().lock();
    if let Some(hub) = map.get_mut(terminal_id) {
        hub.desktop_focused = focused;
        if !focused {
            hub.blur_at = Some(Instant::now());
        }
    }
}

/// True if the hub exists (so a command can skip its reconcile/spawn for a
/// stale terminal id).
pub fn hub_exists(terminal_id: &str) -> bool {
    hubs().lock().contains_key(terminal_id)
}

// ---------------------------------------------------------------------------
// reconcile — the single size-application chokepoint.
// ---------------------------------------------------------------------------

/// Compute the desired size for a terminal and, if it differs from the
/// size currently applied to the PTY, push the resize and broadcast the
/// new size to every client. Idempotent: desired == applied → no-op.
///
/// Also tracks size ownership (phone-driven vs desktop-driven) and emits
/// a `terminal-size-owner` Tauri event on every ownership transition —
/// with the hubs lock DROPPED (never emit under the lock).
pub fn reconcile_now(terminal_id: &str) {
    let (to_apply, owner_changed) = {
        let mut map = hubs().lock();
        let Some(hub) = map.get_mut(terminal_id) else {
            return;
        };
        let now = Instant::now();
        let (desired, phone_owned) = desired_size(hub, now);

        // Detect ownership transition (only when we have a usable size).
        let owner_changed = if desired.is_some() && hub.size_owner != Some(phone_owned) {
            hub.size_owner = Some(phone_owned);
            Some(phone_owned)
        } else {
            None
        };

        let to_apply = match desired {
            Some(size) if Some(size) != hub.applied_size => {
                hub.applied_size = Some(size);
                Some((hub.kind, size))
            }
            // No desired size yet (no client reported one), or already
            // applied — nothing to do.
            _ => None,
        };
        (to_apply, owner_changed)
    };

    // Emit size-owner transition event (lock already dropped).
    if let Some(phone_owned) = owner_changed {
        emit_size_owner(terminal_id, phone_owned);
    }

    if let Some((kind, (cols, rows))) = to_apply {
        apply_pty_resize(terminal_id, kind, cols, rows);
        broadcast_size(terminal_id, cols, rows);
    }
}

/// `reconcile_now` after `delay`. Spawns a tokio task on the companion
/// runtime (mirrors how push.rs spawns). Used for the detach grace and
/// the blur debounce, where the desired size only changes once a timer
/// elapses. No-op if no tokio runtime / hub is gone by then.
pub fn reconcile_after(terminal_id: &str, delay: Duration) {
    let id = terminal_id.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        reconcile_now(&id);
    });
}

/// Resize the real PTY master through the terminal registry — the same
/// path the desktop resize commands use (agent: `master.resize`; ssh:
/// `SshCommand::Resize`). Reaches the registries via the desktop
/// AppHandle stashed in `set_app_handle`. No-op if the handle/entry is
/// gone. MUST be called with the hubs lock dropped.
fn apply_pty_resize(terminal_id: &str, kind: TermKind, cols: u16, rows: u16) {
    let guard = app_handle().lock();
    let Some(app) = guard.as_ref() else {
        return;
    };
    match kind {
        TermKind::Agent => {
            let state = app.state::<TerminalState>();
            let map = state.terminals.lock();
            if let Some(entry) = map.get(terminal_id) {
                let _ = entry.master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
        TermKind::Ssh => {
            let state = app.state::<SshTerminalState>();
            let map = state.terminals.lock();
            if let Some(entry) = map.get(terminal_id) {
                let _ = entry.handle_tx.send(SshCommand::Resize { cols, rows });
            }
        }
    }
}

/// Broadcast the applied size to every WS client through the fan-out
/// channel; ws.rs turns it into a `{t:"size"}` message (the same wire
/// shape the desktop already emits). MUST be called with the hubs lock
/// dropped (broadcast::send never blocks, but we keep the invariant).
fn broadcast_size(terminal_id: &str, cols: u16, rows: u16) {
    let map = hubs().lock();
    if let Some(hub) = map.get(terminal_id) {
        let _ = hub.tx.send(FanoutEvent::Size(cols, rows));
    }
}

/// Sweep every hub for the attention condition and emit one push per
/// newly-flagged terminal. A hub qualifies when its tail looked like a
/// prompt, it has been idle ≥ ATTENTION_IDLE, no phone is attached, and
/// no attention push has fired for this idle stretch. Latches `notified`
/// so the next sweep won't re-fire until fresh output clears it.
/// Driven by push.rs's tokio interval.
pub fn sweep_attention() {
    let mut map = hubs().lock();
    for (id, hub) in map.iter_mut() {
        // Hook-driven hubs skip the idle debounce: the agent's event is
        // authoritative, so a pending prompt should push on the next sweep.
        let idle_ok = hub.hook_driven || hub.last_output.elapsed() >= ATTENTION_IDLE;
        if hub.prompt_flag && !hub.notified && !phone_attached(hub) && idle_ok {
            hub.notified = true;
            emit_push(PushTrigger::Attention {
                terminal_id: id.clone(),
                title: hub.title.clone(),
            });
        }
    }
}

/// Test hook: rewind a hub's idle clock so the attention sweep can be
/// exercised without sleeping out the real ATTENTION_IDLE window.
#[cfg(test)]
fn backdate_last_output_for_test(terminal_id: &str, by: Duration) {
    let mut map = hubs().lock();
    if let Some(hub) = map.get_mut(terminal_id) {
        hub.last_output = hub.last_output.checked_sub(by).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_subscribe_ordering_and_replay() {
        let id = "fanout-test-order";
        register(id, TermKind::Agent, "test");

        publish(id, b"hello ");
        // Attach mid-stream: replay carries everything so far…
        let mut attached = attach(id).expect("hub registered");
        assert_eq!(attached.scrollback, b"hello ");
        assert_eq!(attached.kind, TermKind::Agent);

        // …and the live feed starts exactly after the snapshot.
        publish(id, b"world");
        publish(id, b"!");
        match attached.rx.try_recv().unwrap() {
            FanoutEvent::Out(b) => assert_eq!(b, b"world"),
            other => panic!("expected Out, got {:?}", other),
        }
        match attached.rx.try_recv().unwrap() {
            FanoutEvent::Out(b) => assert_eq!(b, b"!"),
            other => panic!("expected Out, got {:?}", other),
        }
        assert!(attached.rx.try_recv().is_err()); // nothing else queued

        assert_eq!(snapshot_scrollback(id), b"hello world!");
        unregister(id);
        assert!(attach(id).is_none());
    }

    #[test]
    fn scrollback_ring_truncates_to_cap_keeping_suffix() {
        let id = "fanout-test-ring";
        register(id, TermKind::Agent, "test");

        // 300KB in 1KB chunks, each chunk filled with its index byte so
        // the suffix is verifiable after truncation.
        let total_kb = 300usize;
        for i in 0..total_kb {
            publish(id, &[(i % 251) as u8; 1024]);
        }
        let snap = snapshot_scrollback(id);
        assert_eq!(snap.len(), SCROLLBACK_CAP);

        // The snapshot must equal the SUFFIX of everything published.
        let mut expected = Vec::with_capacity(total_kb * 1024);
        for i in 0..total_kb {
            expected.extend_from_slice(&[(i % 251) as u8; 1024]);
        }
        assert_eq!(snap, expected[expected.len() - SCROLLBACK_CAP..]);
        unregister(id);
    }

    /// Build a bare in-memory hub for pure `desired_size` testing — no
    /// global map, no runtime, no PTY. Just the fields the resolver reads.
    fn test_hub() -> TermHub {
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        TermHub {
            tx,
            scrollback: VecDeque::new(),
            sizes: HashMap::new(),
            kind: TermKind::Agent,
            title: String::new(),
            last_output: Instant::now(),
            prompt_flag: false,
            notified: false,
            viewers: HashSet::new(),
            hook_driven: false,
            desktop_focused: false,
            blur_at: None,
            last_phone_size: None,
            last_phone_detach_at: None,
            applied_size: None,
            size_owner: None,
        }
    }

    /// Record a phone as both a viewer and a sized client (what attach +
    /// resize do together).
    fn attach_phone(hub: &mut TermHub, id: &str, cols: u16, rows: u16) {
        hub.viewers.insert(id.to_string());
        hub.sizes.insert(id.to_string(), (cols, rows));
    }

    #[test]
    fn desired_desktop_only() {
        let now = Instant::now();
        let mut hub = test_hub();
        // No client reported a size yet.
        assert_eq!(desired_size(&mut hub, now), (None, false));
        // Desktop reports → desktop drives the PTY (today's behavior).
        hub.sizes.insert(DESKTOP_CLIENT.to_string(), (120, 40));
        assert_eq!(desired_size(&mut hub, now), (Some((120, 40)), false));
    }

    #[test]
    fn desired_phone_attached_owns_size() {
        let now = Instant::now();
        let mut hub = test_hub();
        hub.sizes.insert(DESKTOP_CLIENT.to_string(), (120, 40));
        attach_phone(&mut hub, "phone-1", 80, 24);
        // Desktop unfocused → the phone's (smaller) size owns the PTY.
        assert_eq!(desired_size(&mut hub, now), (Some((80, 24)), true));
        // And it's cached for the detach grace.
        assert_eq!(hub.last_phone_size, Some((80, 24)));
    }

    #[test]
    fn desired_phone_plus_focused_desktop_wins() {
        let now = Instant::now();
        let mut hub = test_hub();
        hub.sizes.insert(DESKTOP_CLIENT.to_string(), (120, 40));
        attach_phone(&mut hub, "phone-1", 80, 24);
        // Desktop focused on the session → it reclaims (scenario 8).
        hub.desktop_focused = true;
        assert_eq!(desired_size(&mut hub, now), (Some((120, 40)), false));
    }

    #[test]
    fn desired_two_phones_min() {
        let now = Instant::now();
        let mut hub = test_hub();
        hub.sizes.insert(DESKTOP_CLIENT.to_string(), (200, 60));
        // phone-1=(80,24), phone-2=(60,30) → min cols/rows = (60,24).
        attach_phone(&mut hub, "phone-1", 80, 24);
        attach_phone(&mut hub, "phone-2", 60, 30);
        assert_eq!(desired_size(&mut hub, now), (Some((60, 24)), true));
    }

    #[test]
    fn desired_detach_grace_holds_phone_size() {
        let now = Instant::now();
        let mut hub = test_hub();
        hub.sizes.insert(DESKTOP_CLIENT.to_string(), (120, 40));
        // A phone owned the PTY, then detached just now (no phones left).
        hub.last_phone_size = Some((80, 24));
        hub.last_phone_detach_at = Some(now);
        // Within the grace → hold the phone size (quick reconnect = no churn).
        // Ownership is still phone-owned during the grace.
        assert_eq!(desired_size(&mut hub, now), (Some((80, 24)), true));
        // Past the grace → desktop reclaims, ownership flips.
        let later = now + DETACH_GRACE + Duration::from_secs(1);
        assert_eq!(desired_size(&mut hub, later), (Some((120, 40)), false));
    }

    #[test]
    fn desired_blur_debounce_holds_desktop() {
        let now = Instant::now();
        let mut hub = test_hub();
        hub.sizes.insert(DESKTOP_CLIENT.to_string(), (120, 40));
        attach_phone(&mut hub, "phone-1", 80, 24);
        // Desktop just blurred (not focused, but within the debounce).
        hub.desktop_focused = false;
        hub.blur_at = Some(now);
        // Within the debounce → desktop keeps ownership despite the phone.
        assert_eq!(desired_size(&mut hub, now), (Some((120, 40)), false));
        // Past the debounce → the still-attached phone takes over.
        let later = now + BLUR_DEBOUNCE + Duration::from_secs(1);
        assert_eq!(desired_size(&mut hub, later), (Some((80, 24)), true));
    }

    #[test]
    fn desired_clamps_and_ignores_garbage() {
        let now = Instant::now();
        let mut hub = test_hub();
        hub.sizes.insert(DESKTOP_CLIENT.to_string(), (120, 40));
        // A phone reporting a zero dimension is ignored → desktop stays.
        attach_phone(&mut hub, "phone-bad", 0, 24);
        assert_eq!(desired_size(&mut hub, now), (Some((120, 40)), false));
        // A tiny-but-valid phone size is clamped to the safe floor.
        attach_phone(&mut hub, "phone-tiny", 2, 1);
        assert_eq!(desired_size(&mut hub, now), (Some((MIN_COLS, MIN_ROWS)), true));
    }

    #[test]
    fn effective_size_reflects_phone_when_attached() {
        let id = "fanout-test-size-eff";
        register(id, TermKind::Ssh, "test");
        assert_eq!(effective_size(id), None);

        // Desktop-only → desktop size.
        set_client_size(id, DESKTOP_CLIENT, 120, 40);
        assert_eq!(effective_size(id), Some((120, 40)));

        // Phone attaches (viewer + size) → phone owns the size.
        add_viewer(id, "phone-1");
        set_client_size(id, "phone-1", 80, 24);
        assert_eq!(effective_size(id), Some((80, 24)));

        // Phone leaves (viewer + size gone). `last_phone_size` is only
        // cached by the mutating reconcile path, not the read-only
        // `effective_size`, so with no phone present the desktop size is
        // the snapshot value here (the grace-hold is covered by the pure
        // `desired_detach_grace_holds_phone_size` test).
        remove_viewer(id, "phone-1");
        remove_client(id, "phone-1");
        assert_eq!(effective_size(id), Some((120, 40)));

        unregister(id);
    }

    #[test]
    fn exit_event_reaches_subscriber() {
        let id = "fanout-test-exit";
        register(id, TermKind::Agent, "test");
        let mut attached = attach(id).unwrap();

        publish(id, b"bye");
        publish_exit(id);

        match attached.rx.try_recv().unwrap() {
            FanoutEvent::Out(b) => assert_eq!(b, b"bye"),
            other => panic!("expected Out, got {:?}", other),
        }
        assert!(matches!(
            attached.rx.try_recv().unwrap(),
            FanoutEvent::Exit
        ));

        // Hub lingers through the grace window so late attachers can
        // still replay scrollback and observe Exit.
        let late = attach(id).expect("hub alive during grace");
        assert_eq!(late.scrollback, b"bye");
        unregister(id);
    }

    #[test]
    fn prompt_detector_flags_input_waiting_tails() {
        // Positive: trailing question, y/N forms, prompt sigils, BEL.
        assert!(looks_like_prompt(b"Continue? "));
        assert!(looks_like_prompt(b"Overwrite file [y/N] "));
        assert!(looks_like_prompt(b"Proceed [Y/n]"));
        assert!(looks_like_prompt(b"user@host:~$ "));
        assert!(looks_like_prompt(b"root@box:/# "));
        assert!(looks_like_prompt("~/proj ❯ ".as_bytes()));
        assert!(looks_like_prompt(b" pick one: "));
        assert!(looks_like_prompt(b"beep\x07"));

        // Superset phrase matches (desktop-frontend parity), including
        // ANSI-painted TUI prompts whose raw tail wouldn't match.
        assert!(looks_like_prompt(b"Do you want to proceed with this edit"));
        assert!(looks_like_prompt(b"Continue (y/n)"));
        assert!(looks_like_prompt(b"This action requires permission to run"));
        assert!(looks_like_prompt(b"Press Enter to continue"));
        assert!(looks_like_prompt(b"\x1b[1m1. Allow once\x1b[0m   2. Deny"));
        assert!(looks_like_prompt(b"Yes, and don't ask again this session"));

        // Negative: mid-stream output, blank tails, plain text.
        assert!(!looks_like_prompt(b"Building project...\n"));
        assert!(!looks_like_prompt(b"   \n\n"));
        assert!(!looks_like_prompt(b""));
        assert!(!looks_like_prompt(b"compiled 12 files"));
    }

    // The hub map and push sink are process-global, so other tests'
    // hubs may also emit into our sink during a sweep. Count only the
    // Attention triggers addressed to OUR terminal id.
    fn attention_count_for(rx: &mut mpsc::UnboundedReceiver<PushTrigger>, id: &str) -> usize {
        let mut n = 0;
        while let Ok(t) = rx.try_recv() {
            if let PushTrigger::Attention { terminal_id, .. } = t {
                if terminal_id == id {
                    n += 1;
                }
            }
        }
        n
    }

    // Serializes the global-sink tests so they don't drain each other's
    // triggers. (The attention sweep walks every hub under one lock.)
    static SINK_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn attention_sweep_latches_and_resets() {
        let _guard = SINK_TEST_LOCK.lock();
        let id = "fanout-test-attention";
        register(id, TermKind::Agent, "deploy");

        let (tx, mut rx) = mpsc::unbounded_channel();
        set_push_sink(tx);

        // Prompt-looking output, but not idle yet → no trigger for us.
        publish(id, b"Continue? ");
        sweep_attention();
        assert_eq!(attention_count_for(&mut rx, id), 0, "should not fire before idle");

        // Backdate last_output past the idle threshold → fires once.
        backdate_last_output_for_test(id, ATTENTION_IDLE + Duration::from_secs(1));
        sweep_attention();
        assert_eq!(attention_count_for(&mut rx, id), 1, "expected one attention push");

        // Latched: a second idle sweep must not re-fire (last_output is
        // still backdated, but `notified` is set).
        sweep_attention();
        assert_eq!(attention_count_for(&mut rx, id), 0, "notified latch should hold");

        // Fresh non-prompt output clears the latch AND the prompt flag.
        publish(id, b"running...\n");
        backdate_last_output_for_test(id, ATTENTION_IDLE + Duration::from_secs(1));
        sweep_attention();
        assert_eq!(attention_count_for(&mut rx, id), 0, "non-prompt tail must not notify");

        // A phone attached (a WS viewer) suppresses the notification
        // entirely. Viewer tracking is independent of `sizes`.
        publish(id, b"Retry? ");
        add_viewer(id, "phone-1");
        backdate_last_output_for_test(id, ATTENTION_IDLE + Duration::from_secs(1));
        sweep_attention();
        assert_eq!(attention_count_for(&mut rx, id), 0, "phone attached suppresses push");
        remove_viewer(id, "phone-1");

        clear_push_sink();
        unregister(id);
    }

    /// Verify that `size_owner` transitions correctly as ownership changes:
    /// None on fresh register, Some(true) once phone drives, Some(false)
    /// when phone leaves. Pure state test via `desired_size` on a local hub.
    #[test]
    fn size_owner_tracks_phone_transitions() {
        let now = Instant::now();
        let mut hub = test_hub();
        // Fresh hub: no owner yet.
        assert_eq!(hub.size_owner, None);

        // Desktop-only — no phones. Apply desired_size as reconcile would.
        hub.sizes.insert(DESKTOP_CLIENT.to_string(), (120, 40));
        let (size, phone_owned) = desired_size(&mut hub, now);
        assert_eq!(size, Some((120, 40)));
        assert!(!phone_owned);
        // Simulate the reconcile transition logic.
        if hub.size_owner != Some(phone_owned) {
            hub.size_owner = Some(phone_owned);
        }
        assert_eq!(hub.size_owner, Some(false));

        // Phone attaches.
        attach_phone(&mut hub, "phone-1", 80, 24);
        let (size, phone_owned) = desired_size(&mut hub, now);
        assert_eq!(size, Some((80, 24)));
        assert!(phone_owned);
        if hub.size_owner != Some(phone_owned) {
            hub.size_owner = Some(phone_owned);
        }
        assert_eq!(hub.size_owner, Some(true));

        // Desktop focused → reclaims ownership.
        hub.desktop_focused = true;
        let (_, phone_owned) = desired_size(&mut hub, now);
        assert!(!phone_owned);
        if hub.size_owner != Some(phone_owned) {
            hub.size_owner = Some(phone_owned);
        }
        assert_eq!(hub.size_owner, Some(false));
    }

    /// Verify detach-grace is phone_owned=true (ownership stays until grace ends).
    #[test]
    fn size_owner_detach_grace_is_phone_owned() {
        let now = Instant::now();
        let mut hub = test_hub();
        hub.sizes.insert(DESKTOP_CLIENT.to_string(), (120, 40));
        hub.last_phone_size = Some((80, 24));
        hub.last_phone_detach_at = Some(now);
        // In-grace: still phone-owned.
        let (_, phone_owned) = desired_size(&mut hub, now);
        assert!(phone_owned, "grace period must remain phone-owned");
        // Post-grace: desktop reclaims.
        let later = now + DETACH_GRACE + Duration::from_secs(1);
        let (_, phone_owned) = desired_size(&mut hub, later);
        assert!(!phone_owned, "after grace desktop must own");
    }
}

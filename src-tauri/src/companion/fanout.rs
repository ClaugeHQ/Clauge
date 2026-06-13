// PTY fan-out: one hub per live terminal, mirroring its byte stream to
// any number of companion WebSocket subscribers while keeping a 256KB
// scrollback ring for replay-on-attach. The hubs live in module-level
// statics (same shape as cloud/scheduler.rs) because the publishers are
// the PTY reader threads / russh tasks, which have no AppHandle.
//
// Resize rule (the mirror invariant): the PTY is driven at the
// element-wise minimum (min cols, min rows) over every attached client's
// viewport — the desktop counts as client "desktop". A phone attaching
// with a smaller viewport shrinks the PTY (shared reflow), and when it
// detaches the only remaining client is the desktop, so the effective
// size returns to the desktop's and the tab reflows back. A desktop-only
// session is the min over a single client, i.e. the desktop's own size.
// Recomputed on attach/detach/resize; `set_client_size`/`remove_client`
// return the new effective size only when it actually changed, so
// callers never fire redundant resizes.

use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};

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
}

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

/// True if any non-desktop (i.e. phone) client is currently attached.
/// Used to suppress push when the user is already looking at the term.
fn phone_attached(hub: &TermHub) -> bool {
    hub.sizes.keys().any(|k| k != DESKTOP_CLIENT)
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
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with("[y/n]") || lower.ends_with("[y/n]?") {
        return true;
    }
    if trimmed.ends_with('?') {
        return true;
    }
    // A bare prompt sigil with the question mark already consumed: the
    // last visual char (pre-trim) was a space following one of these.
    let last = trimmed.chars().last().unwrap();
    matches!(last, '❯' | '$' | '#' | '>' | ':')
}

/// The PTY size the hub should be driven at, given every client's
/// reported viewport: the element-wise minimum (min cols, min rows) over
/// all attached clients. A smaller phone shrinks the PTY while attached;
/// once it leaves, the desktop is the only client left and the size
/// returns to the desktop's.
fn effective(sizes: &HashMap<String, (u16, u16)>) -> Option<(u16, u16)> {
    sizes.values().fold(None, |acc, &(c, r)| match acc {
        None => Some((c, r)),
        Some((mc, mr)) => Some((mc.min(c), mr.min(r))),
    })
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
    // notified latch, and re-evaluates the prompt flag against the tail.
    hub.last_output = Instant::now();
    hub.notified = false;
    let (a, b) = hub.scrollback.as_slices();
    let combined = [a, b].concat();
    let tail = &combined[combined.len().saturating_sub(PROMPT_TAIL_BYTES)..];
    hub.prompt_flag = looks_like_prompt(tail);
    let _ = hub.tx.send(FanoutEvent::Out(bytes.to_vec()));
}

/// Broadcast Exit, then unregister after a grace window. Uses a plain
/// detached thread for the timer because callers include the PTY
/// reader thread, which has no tokio runtime context.
pub fn publish_exit(terminal_id: &str) {
    {
        let map = hubs().lock();
        let Some(hub) = map.get(terminal_id) else {
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
        effective_size: effective(&hub.sizes),
    })
}

// dead_code: callers get the effective size from `attach` /
// `set_client_size` / `remove_client` return values; the direct query
// completes the hub API for tests.
#[allow(dead_code)]
pub fn effective_size(terminal_id: &str) -> Option<(u16, u16)> {
    let map = hubs().lock();
    effective(&map.get(terminal_id)?.sizes)
}

/// Record a client's viewport. Returns the new effective size only if
/// it differs from the previous effective size — the caller applies
/// the PTY resize exactly then, so a desktop-only terminal sees the
/// same resize cadence it does today.
pub fn set_client_size(terminal_id: &str, client: &str, cols: u16, rows: u16) -> Option<(u16, u16)> {
    let mut map = hubs().lock();
    let hub = map.get_mut(terminal_id)?;
    let before = effective(&hub.sizes);
    hub.sizes.insert(client.to_string(), (cols, rows));
    let after = effective(&hub.sizes);
    if after != before {
        after
    } else {
        None
    }
}

/// Forget a detached client. Returns the new effective size only if it
/// changed AND at least one client remains (nothing to apply when the
/// last client leaves).
pub fn remove_client(terminal_id: &str, client: &str) -> Option<(u16, u16)> {
    let mut map = hubs().lock();
    let hub = map.get_mut(terminal_id)?;
    let before = effective(&hub.sizes);
    hub.sizes.remove(client);
    let after = effective(&hub.sizes);
    if after != before {
        after
    } else {
        None
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
        if hub.prompt_flag
            && !hub.notified
            && !phone_attached(hub)
            && hub.last_output.elapsed() >= ATTENTION_IDLE
        {
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

    #[test]
    fn effective_size_is_min_over_clients_and_restores_on_detach() {
        let id = "fanout-test-size";
        register(id, TermKind::Ssh, "test");
        assert_eq!(effective_size(id), None);

        // First client defines the size (None → Some = change).
        assert_eq!(
            set_client_size(id, DESKTOP_CLIENT, 120, 40),
            Some((120, 40))
        );
        // Same size again → no change → no resize to fire.
        assert_eq!(set_client_size(id, DESKTOP_CLIENT, 120, 40), None);

        // A smaller phone attaches → shared reflow: the PTY shrinks to the
        // element-wise min, so the desktop tab reflows down with it.
        assert_eq!(set_client_size(id, "phone-1", 80, 24), Some((80, 24)));
        assert_eq!(effective_size(id), Some((80, 24)));

        // A second, larger phone never relaxes the min below the smallest.
        assert_eq!(set_client_size(id, "phone-2", 200, 50), None);
        assert_eq!(effective_size(id), Some((80, 24)));

        // The larger phone leaving doesn't change the min…
        assert_eq!(remove_client(id, "phone-2"), None);
        assert_eq!(remove_client(id, "ghost"), None);
        // …but the smallest phone detaching restores the desktop size, so
        // the desktop tab reflows back to its own width.
        assert_eq!(remove_client(id, "phone-1"), Some((120, 40)));
        assert_eq!(effective_size(id), Some((120, 40)));

        // The desktop's own resize still drives the PTY exactly.
        assert_eq!(set_client_size(id, DESKTOP_CLIENT, 100, 30), Some((100, 30)));

        // Desktop leaves last → empty, nothing to apply.
        assert_eq!(remove_client(id, DESKTOP_CLIENT), None);
        assert_eq!(effective_size(id), None);

        // Unknown terminal → None everywhere.
        assert_eq!(set_client_size("nope", "x", 1, 1), None);
        unregister(id);
    }

    #[test]
    fn effective_size_is_phone_size_when_no_desktop() {
        let id = "fanout-test-size-phone-only";
        register(id, TermKind::Ssh, "test");

        // No desktop tab: the phone's size defines the PTY so a
        // phone-spawned session is still usable.
        assert_eq!(set_client_size(id, "phone-1", 80, 24), Some((80, 24)));
        assert_eq!(effective_size(id), Some((80, 24)));

        // A second phone: element-wise minimum.
        assert_eq!(set_client_size(id, "phone-2", 60, 30), Some((60, 24)));
        assert_eq!(effective_size(id), Some((60, 24)));

        // Larger phone leaves → min relaxes.
        assert_eq!(remove_client(id, "phone-2"), Some((80, 24)));
        unregister(id);
    }

    #[test]
    fn phone_drives_min_then_desktop_restores_after_churn() {
        let id = "fanout-test-size-churn";
        register(id, TermKind::Agent, "test");

        assert_eq!(set_client_size(id, DESKTOP_CLIENT, 150, 45), Some((150, 45)));

        // A small phone shrinks the PTY; a later larger size from the same
        // phone relaxes it (still smaller than desktop in cols).
        assert_eq!(set_client_size(id, "phone-1", 40, 20), Some((40, 20)));
        assert_eq!(set_client_size(id, "phone-1", 200, 80), Some((150, 45)));
        // A new smaller phone wins the min again.
        assert_eq!(set_client_size(id, "phone-2", 30, 10), Some((30, 10)));
        assert_eq!(effective_size(id), Some((30, 10)));

        // All phones detach → the lone desktop client restores its size.
        assert_eq!(remove_client(id, "phone-1"), None);
        assert_eq!(remove_client(id, "phone-2"), Some((150, 45)));
        assert_eq!(effective_size(id), Some((150, 45)));

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

        // A phone attached suppresses the notification entirely.
        publish(id, b"Retry? ");
        set_client_size(id, "phone-1", 80, 24);
        backdate_last_output_for_test(id, ATTENTION_IDLE + Duration::from_secs(1));
        sweep_attention();
        assert_eq!(attention_count_for(&mut rx, id), 0, "phone attached suppresses push");

        clear_push_sink();
        unregister(id);
    }
}

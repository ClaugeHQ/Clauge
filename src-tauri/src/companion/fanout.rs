// PTY fan-out: one hub per live terminal, mirroring its byte stream to
// any number of companion WebSocket subscribers while keeping a 256KB
// scrollback ring for replay-on-attach. The hubs live in module-level
// statics (same shape as cloud/scheduler.rs) because the publishers are
// the PTY reader threads / russh tasks, which have no AppHandle.
//
// Resize rule (the mirror invariant): every attached client — the
// desktop counts as client "desktop" — records its viewport here, and
// the PTY is driven at the element-wise minimum (min cols, min rows)
// over all clients, recomputed on attach/detach/resize. With only the
// desktop attached the minimum IS the desktop size, so desktop-only
// behavior is unchanged; `set_client_size`/`remove_client` return the
// new effective size only when it actually changed, so callers never
// fire redundant resizes.

use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::broadcast;

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

struct TermHub {
    tx: broadcast::Sender<FanoutEvent>,
    scrollback: VecDeque<u8>,
    /// client id → (cols, rows)
    sizes: HashMap<String, (u16, u16)>,
    kind: TermKind,
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

fn min_size(sizes: &HashMap<String, (u16, u16)>) -> Option<(u16, u16)> {
    sizes.values().fold(None, |acc, &(c, r)| match acc {
        None => Some((c, r)),
        Some((mc, mr)) => Some((mc.min(c), mr.min(r))),
    })
}

/// Create the hub for a freshly spawned terminal. Must run before the
/// reader loop starts so the first bytes land in scrollback.
pub fn register(terminal_id: &str, kind: TermKind) {
    let (tx, _) = broadcast::channel(BROADCAST_CAP);
    hubs().lock().insert(
        terminal_id.to_string(),
        TermHub {
            tx,
            scrollback: VecDeque::new(),
            sizes: HashMap::new(),
            kind,
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
        effective_size: min_size(&hub.sizes),
    })
}

// dead_code: callers get the effective size from `attach` /
// `set_client_size` / `remove_client` return values; the direct query
// completes the hub API for tests.
#[allow(dead_code)]
pub fn effective_size(terminal_id: &str) -> Option<(u16, u16)> {
    let map = hubs().lock();
    min_size(&map.get(terminal_id)?.sizes)
}

/// Record a client's viewport. Returns the new effective size only if
/// it differs from the previous effective size — the caller applies
/// the PTY resize exactly then, so a desktop-only terminal sees the
/// same resize cadence it does today.
pub fn set_client_size(terminal_id: &str, client: &str, cols: u16, rows: u16) -> Option<(u16, u16)> {
    let mut map = hubs().lock();
    let hub = map.get_mut(terminal_id)?;
    let before = min_size(&hub.sizes);
    hub.sizes.insert(client.to_string(), (cols, rows));
    let after = min_size(&hub.sizes);
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
    let before = min_size(&hub.sizes);
    hub.sizes.remove(client);
    let after = min_size(&hub.sizes);
    if after != before {
        after
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_subscribe_ordering_and_replay() {
        let id = "fanout-test-order";
        register(id, TermKind::Agent);

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
        register(id, TermKind::Agent);

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
    fn effective_size_min_math_across_add_remove() {
        let id = "fanout-test-size";
        register(id, TermKind::Ssh);
        assert_eq!(effective_size(id), None);

        // First client defines the size (None → Some = change).
        assert_eq!(
            set_client_size(id, DESKTOP_CLIENT, 120, 40),
            Some((120, 40))
        );
        // Same size again → no change → no resize to fire.
        assert_eq!(set_client_size(id, DESKTOP_CLIENT, 120, 40), None);

        // Phone smaller in cols, larger in rows → element-wise min.
        assert_eq!(set_client_size(id, "phone-1", 80, 60), Some((80, 40)));
        assert_eq!(effective_size(id), Some((80, 40)));

        // Second phone strictly bigger → min unchanged.
        assert_eq!(set_client_size(id, "phone-2", 200, 50), None);

        // Phone-1 leaves → min relaxes to (120, 40).
        assert_eq!(remove_client(id, "phone-1"), Some((120, 40)));
        // Removing a client that never registered → no change.
        assert_eq!(remove_client(id, "ghost"), None);

        // Remaining clients leave: phone-2 going changes the min…
        assert_eq!(remove_client(id, "phone-2"), None); // (120,40) is still the min
        assert_eq!(remove_client(id, DESKTOP_CLIENT), None); // → empty, nothing to apply
        assert_eq!(effective_size(id), None);

        // Unknown terminal → None everywhere.
        assert_eq!(set_client_size("nope", "x", 1, 1), None);
        unregister(id);
    }

    #[test]
    fn exit_event_reaches_subscriber() {
        let id = "fanout-test-exit";
        register(id, TermKind::Agent);
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
}

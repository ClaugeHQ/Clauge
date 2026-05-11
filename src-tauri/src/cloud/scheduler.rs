// Debounced auto-sync: mutation commands call `bump(kind)` after their SQL
// succeeds; this module collapses bursts into a single push 5s after the last
// bump. Per-kind dirty flags so we only push what actually changed.
//
// Implementation note: the bump path uses module-level statics (no AppHandle
// needed) so call sites are one-liners. The spawn task captures the AppHandle
// once at boot.

use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

const DEBOUNCE: Duration = Duration::from_millis(5_000);

static DIRTY: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
static ENABLED: OnceLock<Mutex<bool>> = OnceLock::new();
static NOTIFY: OnceLock<Arc<Notify>> = OnceLock::new();

fn dirty() -> &'static Mutex<HashSet<&'static str>> {
    DIRTY.get_or_init(|| Mutex::new(HashSet::new()))
}
fn enabled_flag() -> &'static Mutex<bool> {
    ENABLED.get_or_init(|| Mutex::new(false))
}
fn notify() -> Arc<Notify> {
    NOTIFY.get_or_init(|| Arc::new(Notify::new())).clone()
}

/// Tauri-state-managed handle. Methods proxy to the module statics — having
/// the type managed makes the surface explicit and keeps the API symmetric
/// with `AuthState`.
#[derive(Default)]
pub struct Scheduler;

impl Scheduler {
    pub fn enable(&self) {
        *enabled_flag().lock() = true;
    }
    pub fn disable_and_clear(&self) {
        *enabled_flag().lock() = false;
        dirty().lock().clear();
        notify().notify_one();
    }
}

/// Mark a kind dirty. Cheap. Safe to call from any thread. Drops silently if
/// the scheduler isn't enabled yet (e.g. before the user signs in).
pub fn bump(kind: &'static str) {
    if !*enabled_flag().lock() {
        return;
    }
    dirty().lock().insert(kind);
    notify().notify_one();
}

fn drain() -> Vec<&'static str> {
    dirty().lock().drain().collect()
}

/// Spawn the scheduler loop. Lives for the app's lifetime.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Hold the Notify Arc for the loop's lifetime so .notified() futures
        // don't try to borrow from a temporary.
        let n = notify();
        loop {
            n.notified().await;

            // Debounce: keep resetting the timer while more bumps arrive.
            loop {
                let timer = tokio::time::sleep(DEBOUNCE);
                tokio::pin!(timer);
                tokio::select! {
                    _ = &mut timer => break,
                    _ = n.notified() => continue,
                }
            }

            let kinds = drain();
            if kinds.is_empty() {
                continue;
            }

            let pool = match app.try_state::<sqlx::SqlitePool>() {
                Some(p) => p.inner().clone(),
                None => {
                    log::warn!("[cloud:scheduler] SqlitePool state missing; dropping {} kinds", kinds.len());
                    continue;
                }
            };
            let auth_state = app.state::<crate::cloud::auth::AuthState>();
            if !auth_state.is_connected() {
                // Re-mark dirty for when the user comes back.
                for k in &kinds {
                    dirty().lock().insert(k);
                }
                continue;
            }

            let kind_slice: Vec<&str> = kinds.iter().copied().collect();
            match crate::cloud::sync::push_all(&pool, &auth_state, &kind_slice).await {
                Ok(pushed) => {
                    if !pushed.is_empty() {
                        let _ = app.emit("cloud:synced", &pushed);
                        log::info!("[cloud:scheduler] auto-pushed: {:?}", pushed);
                    }
                }
                Err(e) => {
                    log::warn!("[cloud:scheduler] push failed ({}); re-queueing", e);
                    for k in &kinds {
                        dirty().lock().insert(k);
                    }
                }
            }
        }
    });
}

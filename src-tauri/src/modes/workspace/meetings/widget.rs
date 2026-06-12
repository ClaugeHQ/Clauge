//! Floating always-on-top pill window for meeting detection.
//!
//! Opened by the detection poller when a call is detected, and by the
//! recorder when a detected-call recording starts. Manual recordings from
//! the main window never open it, but the stop/error paths close it
//! unconditionally — closing a window that was never opened is a no-op.

use tauri::{AppHandle, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

pub const WIDGET_LABEL: &str = "meeting-widget";

const WIDGET_WIDTH: f64 = 480.0;
const WIDGET_HEIGHT: f64 = 80.0;
const TOP_MARGIN: f64 = 12.0;

pub fn open_widget(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(WIDGET_LABEL) {
        // The app-wide close handler hides windows instead of destroying
        // them, so this instance survives every "close" and may carry
        // geometry from a previous cycle (the compact recording pill
        // shifts it right) — re-center before showing or the drift
        // accumulates until the widget reappears off-screen.
        if let Some((x, y)) = top_center(app) {
            let _ = win.set_position(LogicalPosition::new(x, y));
        }
        let _ = win.show();
        return;
    }
    let mut builder =
        WebviewWindowBuilder::new(app, WIDGET_LABEL, WebviewUrl::App("widget".into()))
            .title("Meeting Notes")
            .inner_size(WIDGET_WIDTH, WIDGET_HEIGHT)
            .decorations(false)
            // On X11 without a compositor this renders as opaque black
            // around the pill — accepted v1 limitation.
            .transparent(true)
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .skip_taskbar(true)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .shadow(false)
            // Buttons must react to the first click even while the window
            // is inactive (macOS otherwise eats it to focus the window).
            .accept_first_mouse(true)
            .focused(false);
    if let Some((x, y)) = top_center(app) {
        builder = builder.position(x, y);
    }
    match builder.build() {
        Ok(win) => apply_content_protection(&win),
        Err(e) => log::warn!("[meetings] failed to open widget window: {e}"),
    }
}

/// Keeps the widget out of screen shares — but only where the OS actually
/// honors it. Windows: WDA_EXCLUDEFROMCAPTURE genuinely excludes the
/// window. macOS < 15: NSWindowSharingType::none still works against the
/// legacy capture APIs (it also hides the widget from the user's own
/// screenshots). On macOS 15+ ScreenCaptureKit ignores sharingType — it is
/// a legacy no-op that protects nothing (tauri#14200) and is implicated in
/// Tahoe rendering glitches — so it stays off there. No-op on Linux.
#[allow(unused_variables)]
fn apply_content_protection(win: &tauri::WebviewWindow) {
    #[cfg(windows)]
    if let Err(e) = win.set_content_protected(true) {
        log::warn!("[meetings] failed to enable widget content protection: {e}");
    }
    #[cfg(target_os = "macos")]
    if crate::shared::platform::macos::macos_version().is_some_and(|(major, _)| major < 15) {
        if let Err(e) = win.set_content_protected(true) {
            log::warn!("[meetings] failed to enable widget content protection: {e}");
        }
    }
}

pub fn close_widget(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(WIDGET_LABEL) {
        let _ = win.close();
    }
}

/// Top-center of the work area of the monitor under the cursor (the one
/// the user is on for the call), falling back to the primary monitor.
/// Returns logical coordinates, which is what `WindowBuilder::position`
/// expects.
fn top_center(app: &AppHandle) -> Option<(f64, f64)> {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let pos = area.position.to_logical::<f64>(scale);
    let size = area.size.to_logical::<f64>(scale);
    Some((pos.x + (size.width - WIDGET_WIDTH) / 2.0, pos.y + TOP_MARGIN))
}

// Centralized timing constants (milliseconds).
//
// Naming: <PURPOSE>_MS for delays/durations, <PURPOSE>_INTERVAL_MS for repeats.
// Group related timings together. Any literal `setTimeout(..., 1234)` outside
// a true micro-tick (0ms) should reference one of these instead.

// --- Toast / transient UI ---
export const TOAST_DURATION_MS = 2500;
export const COPY_FEEDBACK_MS = 1500;
export const STATUS_MESSAGE_MS = 3000;

// --- Debounce / input ---
export const KV_DEBOUNCE_MS = 300;
export const BODY_DEBOUNCE_MS = 300;
export const ENV_SAVE_DEBOUNCE_MS = 400;
export const BLUR_CANCEL_MS = 200;
export const INLINE_INPUT_BLUR_MS = 200;
export const SUGGEST_CLOSE_MS = 150;

// --- Resize / layout ---
export const RESIZE_DEBOUNCE_MS = 100;

// --- Agent panel ---
export const AGENT_NOTIFY_DEBOUNCE_MS = 300;
export const AGENT_NOTIFY_REPEAT_MS = 3000;
export const AGENT_CHIME_STOP_MS = 400;
export const AGENT_ACTIVITY_WINDOW_MS = 500;
export const AGENT_ACTIVITY_DONE_MS = 2000;
export const AGENT_SHELL_LOADER_MS = 3000;
export const AGENT_CONTEXT_USAGE_INTERVAL_MS = 5000;
export const AGENT_SESSION_CAPTURE_INTERVAL_MS = 3000;

// --- Sidebar / window ---
export const FULLSCREEN_POLL_INTERVAL_MS = 1000;

// --- Click-outside guard ---
// Defer attaching the click-outside listener so the click that opened the
// element doesn't immediately close it.
export const CLICK_OUTSIDE_GUARD_MS = 10;

// --- Misc UI ---
export const MICRO_TICK_MS = 0;
export const NAV_EXPAND_MS = 50;
export const ONBOARDING_MOUNT_MS = 50;
export const ONBOARDING_TRANSITION_MS = 320;
export const SCROLL_INTO_VIEW_MS = 10;

// --- Nav hover-reveal edge trigger ---
// Width of the invisible strip on the left edge that arms the hover overlay.
export const NAV_HOVER_TRIGGER_PX = 8;
// Number of consecutive leftward pointer moves required before the overlay
// reveals. Filters out cursor jitter and brief right→left grazes when the
// pointer is just bouncing near the edge of the panel.
export const NAV_HOVER_LEFTWARD_FRAMES = 4;

// --- SSH ---
export const SSH_CAPTURE_TIMEOUT_MS = 15_000;

// --- Periodic background tasks ---
export const PERIODIC_SYNC_INTERVAL_MS = 5 * 60 * 1000;
export const USAGE_LIMITS_POLL_INTERVAL_MS = 5 * 60 * 1000;
export const SPLASH_FADE_OUT_MS = 300;

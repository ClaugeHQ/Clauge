// Canvas mode — spatial workspace surface that displays every open
// tab across every mode as a freely positioned, pan/zoom/drag/resize
// window. Backend ownership is intentionally minimal: it persists
// tile geometry and viewport state, and reconciles tile rows against
// the open-tab list the frontend passes in. It does NOT enumerate
// per-mode tab tables itself; the frontend already knows what's open
// (each mode exposes its open-tab list via its adapter), so it passes
// the union as input to `canvas_resolve_tiles`.

pub mod commands;
pub mod db;
pub mod models;

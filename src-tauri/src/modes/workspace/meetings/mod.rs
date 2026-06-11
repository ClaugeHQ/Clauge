// AI Meeting Notes — persistence for recorded meetings plus the
// recorder orchestrator (capture → whisper → segments). All layers
// write through `repo`.

pub mod commands;
pub mod recorder;
pub mod repo;

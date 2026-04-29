// REST mode — owns collection / request / environment CRUD against app
// SQLite, the HTTP executor that runs saved or ad-hoc requests, the
// request-history aggregate, the Postman / cURL / Clauge-format
// importers and exporters, and the AI tool implementations that drive
// REST flows from the chat surface.
//
// `collections`, `requests`, `environments`, `history`, `http_executor`,
// and `import_export` host `#[tauri::command]` handlers; lib.rs
// references them as `crate::modes::rest::<file>::*`.
// `ai_tools` registers REST-specific tool handlers into the shared
// `crate::shared::ai::dispatch` registry at startup.

pub mod ai_tools;
pub mod collections;
pub mod environments;
pub mod history;
pub mod http_executor;
pub mod import_export;
pub mod requests;

// SQL dialect registry — single source of truth for the per-driver
// metadata that used to be scattered across `client.rs` and the SvelteKit
// layer (default ports, display names, abbreviations, parser profiles).
//
// Per-dialect runtime logic that genuinely differs between drivers
// (connection-string syntax in `client::build_connection_url`, sqlx pool
// construction in `client::create_pool`, dialect-specific SQL in
// `ai_tools.rs`) intentionally stays as match arms — those branches live
// inside the engine, not the metadata. The registry's job is to centralise
// the metadata and let dispatch happen via `descriptor_for_key(...)`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDialect {
    Postgres,
    MySql,
    Sqlite,
}

/// Registry record; some metadata fields are consumed only by the
/// TypeScript mirror today (`src/lib/modes/sql/dialects.ts`) and are kept
/// here so the Rust side stays the source of truth as the registry grows.
#[allow(dead_code)]
#[derive(Debug)]
pub struct SqlDialectDescriptor {
    pub dialect: SqlDialect,
    /// Stable key persisted in the saved-connections table and exchanged
    /// over the Tauri IPC boundary.
    pub key: &'static str,
    pub display_name: &'static str,
    pub abbreviation: &'static str,
    pub default_port: u16,
    pub uses_host_port: bool,
    pub uses_credentials: bool,
    /// Matches the CodeMirror lang-sql dialect name on the frontend.
    pub frontend_parser_profile: Option<&'static str>,
}

const DIALECTS: &[SqlDialectDescriptor] = &[
    SqlDialectDescriptor {
        dialect: SqlDialect::Postgres,
        key: "postgresql",
        display_name: "PostgreSQL",
        abbreviation: "PG",
        default_port: 5432,
        uses_host_port: true,
        uses_credentials: true,
        frontend_parser_profile: Some("PostgreSQL"),
    },
    SqlDialectDescriptor {
        dialect: SqlDialect::MySql,
        key: "mysql",
        display_name: "MySQL",
        abbreviation: "MY",
        default_port: 3306,
        uses_host_port: true,
        uses_credentials: true,
        frontend_parser_profile: Some("MySQL"),
    },
    SqlDialectDescriptor {
        dialect: SqlDialect::Sqlite,
        key: "sqlite",
        display_name: "SQLite",
        abbreviation: "SL",
        default_port: 0,
        uses_host_port: false,
        uses_credentials: false,
        frontend_parser_profile: Some("SQLite"),
    },
];

pub fn descriptor_for_key(key: &str) -> Option<&'static SqlDialectDescriptor> {
    DIALECTS.iter().find(|d| d.key == key)
}

/// Lookup by enum variant. Currently unused at runtime — `client.rs`
/// dispatches via `descriptor_for_key` then matches on `dialect` directly —
/// but kept as part of the registry's public surface for future callers
/// (e.g. ai_tools migration, ClickHouse onboarding).
#[allow(dead_code)]
pub fn descriptor_for(dialect: SqlDialect) -> &'static SqlDialectDescriptor {
    // Safe: the registry always contains every variant of `SqlDialect`.
    DIALECTS
        .iter()
        .find(|d| d.dialect == dialect)
        .expect("SqlDialect variant missing from registry")
}

#[allow(dead_code)]
pub fn all_descriptors() -> &'static [SqlDialectDescriptor] {
    DIALECTS
}

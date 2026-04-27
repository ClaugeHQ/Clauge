use tauri_plugin_sql::{Migration, MigrationKind};

pub fn get_migrations() -> Vec<Migration> {
    vec![Migration {
        version: 1,
        description: "create_initial_schema",
        sql: "
                PRAGMA foreign_keys = ON;

                CREATE TABLE IF NOT EXISTS collections (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    sort_order INTEGER NOT NULL DEFAULT 0,
                    env_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS requests (
                    id TEXT PRIMARY KEY,
                    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    method TEXT NOT NULL DEFAULT 'GET',
                    url TEXT NOT NULL DEFAULT '',
                    body TEXT NOT NULL DEFAULT '',
                    body_type TEXT NOT NULL DEFAULT 'json',
                    auth_type TEXT NOT NULL DEFAULT 'none',
                    auth_data TEXT NOT NULL DEFAULT '{}',
                    pre_script TEXT NOT NULL DEFAULT '',
                    sort_order INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS request_headers (
                    id TEXT PRIMARY KEY,
                    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
                    key TEXT NOT NULL,
                    value TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    sort_order INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS request_params (
                    id TEXT PRIMARY KEY,
                    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
                    key TEXT NOT NULL,
                    value TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    sort_order INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS environments (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    color TEXT NOT NULL DEFAULT '#1dc880',
                    is_default INTEGER NOT NULL DEFAULT 0,
                    sort_order INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS env_variables (
                    id TEXT PRIMARY KEY,
                    environment_id TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
                    key TEXT NOT NULL,
                    value TEXT NOT NULL,
                    is_secret INTEGER NOT NULL DEFAULT 0,
                    sort_order INTEGER NOT NULL DEFAULT 0,
                    UNIQUE(environment_id, key)
                );

                CREATE TABLE IF NOT EXISTS history (
                    id TEXT PRIMARY KEY,
                    request_id TEXT REFERENCES requests(id) ON DELETE SET NULL,
                    method TEXT NOT NULL,
                    url TEXT NOT NULL,
                    resolved_url TEXT NOT NULL,
                    request_body TEXT NOT NULL DEFAULT '',
                    request_headers TEXT NOT NULL DEFAULT '[]',
                    response_status INTEGER,
                    response_body TEXT,
                    response_headers TEXT DEFAULT '[]',
                    response_size_bytes INTEGER,
                    duration_ms INTEGER,
                    environment_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS sync_state (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS sql_connections (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    driver TEXT NOT NULL,
                    host TEXT NOT NULL DEFAULT '',
                    port INTEGER NOT NULL DEFAULT 0,
                    database_name TEXT NOT NULL DEFAULT '',
                    username TEXT NOT NULL DEFAULT '',
                    password TEXT NOT NULL DEFAULT '',
                    ssl INTEGER NOT NULL DEFAULT 0,
                    sort_order INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS nosql_connections (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    driver TEXT NOT NULL,
                    connection_string TEXT NOT NULL DEFAULT '',
                    host TEXT NOT NULL DEFAULT '',
                    port INTEGER NOT NULL DEFAULT 0,
                    database_name TEXT NOT NULL DEFAULT '',
                    username TEXT NOT NULL DEFAULT '',
                    password TEXT NOT NULL DEFAULT '',
                    ssl INTEGER NOT NULL DEFAULT 0,
                    sort_order INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS sql_scripts (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    connection_id TEXT,
                    database_name TEXT NOT NULL DEFAULT '',
                    query TEXT NOT NULL DEFAULT '',
                    sort_order INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
            ",
        kind: MigrationKind::Up,
    },
    Migration {
        version: 2,
        description: "add_direct_connection_to_nosql",
        sql: "ALTER TABLE nosql_connections ADD COLUMN direct_connection INTEGER NOT NULL DEFAULT 0;",
        kind: MigrationKind::Up,
    },
    Migration {
        version: 3,
        description: "create_ai_usage_table",
        sql: "
            CREATE TABLE IF NOT EXISTS ai_usage (
                id TEXT PRIMARY KEY,
                mode TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
        ",
        kind: MigrationKind::Up,
    },
    Migration {
        version: 4,
        description: "create_agent_tables",
        sql: "
            CREATE TABLE IF NOT EXISTS agent_sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                purpose TEXT NOT NULL,
                project_path TEXT NOT NULL,
                project_name TEXT NOT NULL,
                claude_session_id TEXT,
                context_prompt TEXT NOT NULL DEFAULT '',
                worktree_path TEXT,
                worktree_branch TEXT,
                skip_permissions INTEGER NOT NULL DEFAULT 0,
                git_name TEXT,
                git_email TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                last_used_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS agent_contexts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS agent_session_contexts (
                session_id TEXT NOT NULL,
                context_id TEXT NOT NULL,
                PRIMARY KEY (session_id, context_id),
                FOREIGN KEY (session_id) REFERENCES agent_sessions(id) ON DELETE CASCADE,
                FOREIGN KEY (context_id) REFERENCES agent_contexts(id) ON DELETE CASCADE
            );
        ",
        kind: MigrationKind::Up,
    },
    Migration {
        version: 5,
        description: "ssh_profiles_and_known_hosts",
        sql: "
            CREATE TABLE IF NOT EXISTS ssh_profiles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                host TEXT NOT NULL,
                port INTEGER NOT NULL DEFAULT 22,
                username TEXT NOT NULL,
                auth_type TEXT NOT NULL CHECK(auth_type IN ('key','password')),
                key_path TEXT,
                accent_color TEXT,
                last_used_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS ssh_known_hosts (
                profile_id TEXT NOT NULL,
                host TEXT NOT NULL,
                port INTEGER NOT NULL,
                fingerprint_sha256 TEXT NOT NULL,
                accepted_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (profile_id, host, port),
                FOREIGN KEY (profile_id) REFERENCES ssh_profiles(id) ON DELETE CASCADE
            );
        ",
        kind: MigrationKind::Up,
    }]
}

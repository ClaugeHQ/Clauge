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

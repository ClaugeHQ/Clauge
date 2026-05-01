-- Rebuild ssh_profiles to allow auth_type = 'agent'.
--
-- The v5 schema enforced CHECK(auth_type IN ('key','password')), which blocked
-- the 'agent' auth path even though the russh-agent client (modes::ssh::agent)
-- and the auth dispatch in terminal.rs / tunnel.rs are already implemented.
-- SQLite has no ALTER TABLE for CHECK constraints, so we copy into a new
-- table with the relaxed constraint, drop the original, and rename.
--
-- ssh_known_hosts has FOREIGN KEY (profile_id) REFERENCES ssh_profiles(id)
-- ON DELETE CASCADE. The DROP below performs an implicit DELETE which
-- cascades, emptying ssh_known_hosts. This is a one-time cost — fingerprints
-- are a TOFU cache, users will be re-prompted on first connect after this
-- migration runs.
--
-- This file runs once via sqlx::migrate inside a transaction (per-migration
-- atomic). No idempotency dance required — version tracking ensures it
-- runs exactly once per database.

CREATE TABLE ssh_profiles_v7 (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 22,
    username TEXT NOT NULL,
    auth_type TEXT NOT NULL CHECK(auth_type IN ('key','password','agent')),
    key_path TEXT,
    accent_color TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO ssh_profiles_v7
    (id, name, host, port, username, auth_type, key_path, accent_color, last_used_at, created_at)
SELECT id, name, host, port, username, auth_type, key_path, accent_color, last_used_at, created_at
FROM ssh_profiles;

DROP TABLE ssh_profiles;
ALTER TABLE ssh_profiles_v7 RENAME TO ssh_profiles;

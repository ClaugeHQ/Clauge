-- Paired mobile companion devices. The raw device token never touches
-- disk — only its SHA-256 hex digest lands in token_hash, so a leaked
-- database cannot impersonate a phone. Revocation is a soft flag: the
-- row survives for the Settings device list, but auth skips it.

CREATE TABLE IF NOT EXISTS companion_devices (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    platform      TEXT NOT NULL,
    token_hash    TEXT NOT NULL UNIQUE,
    fcm_token     TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen_at  TEXT,
    revoked       INTEGER NOT NULL DEFAULT 0
);

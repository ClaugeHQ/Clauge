-- Clauge telemetry — append-only daily heartbeats from each install.
--
-- Apply locally:  wrangler d1 execute clauge-db --file=migrations/0009_telemetry.sql --local
-- Apply remote:   wrangler d1 execute clauge-db --file=migrations/0009_telemetry.sql --remote
--
-- Design notes (see worker analysis doc):
--   • Insert-only. Each device pings ~once per 24h; we never UPDATE rows.
--     Time-series queries (DAU, version adoption, feature trends) need
--     the time dimension, which an accumulating-row design destroys.
--   • `device_id` is a UUID generated client-side at first launch. It is
--     stable across login state — anonymous pings carry it with
--     `user_id = NULL`; once the user signs in, subsequent pings carry
--     the same `device_id` with `user_id` populated. Drives the
--     anonymous-to-logged-in conversion funnel.
--   • `user_id` FK to users with ON DELETE SET NULL. Account deletion
--     anonymises the user's telemetry rather than removing them, so
--     aggregate signal isn't punished by a single delete.
--   • All counts arrive pre-bucketed as short strings ("0","1-10",
--     "11-100","101-1k","1k+"). The "0" bucket is omitted at the source
--     to keep payloads small, so absence-of-key means zero.

CREATE TABLE IF NOT EXISTS telemetry_pings (
  ping_id       INTEGER PRIMARY KEY AUTOINCREMENT,
  device_id     TEXT    NOT NULL,
  user_id       INTEGER,
  received_at   TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,

  -- Device fingerprint (low-cardinality categories — never PII).
  app_version   TEXT    NOT NULL,
  os            TEXT    NOT NULL,          -- 'macos' | 'win' | 'linux'
  os_version    TEXT,                       -- major only ('15', '11', '22.04')
  arch          TEXT    NOT NULL,          -- 'aarch64' | 'x86_64'
  locale        TEXT,                       -- first 5 chars ('en-US', 'de-DE')
  theme         TEXT,                       -- 'dark' | 'light' | 'auto'
  has_account   INTEGER NOT NULL DEFAULT 0, -- 0/1 — duplicated from user_id presence for cheap filtering

  -- Usage signal.
  modes_active  TEXT    NOT NULL DEFAULT '',   -- comma list, e.g. 'rest,sql,ssh'
  features      TEXT    NOT NULL DEFAULT '{}', -- JSON: {feature_key: bucket}
  errors        TEXT    NOT NULL DEFAULT '{}', -- JSON: {err_key: bucket}
  db_buckets    TEXT    NOT NULL DEFAULT '{}', -- JSON: {db_key: bucket}

  FOREIGN KEY (user_id) REFERENCES users(user_id) ON DELETE SET NULL,
  CHECK (os IN ('macos','win','linux')),
  CHECK (arch IN ('aarch64','x86_64')),
  CHECK (has_account IN (0,1))
);

-- Per-device timeline lookups (most common query: "show me what this
-- install did over the last N days").
CREATE INDEX IF NOT EXISTS idx_telemetry_device_time
  ON telemetry_pings (device_id, received_at DESC);

-- Per-user lookups across their devices. Partial index — anonymous
-- rows (the majority for the first weeks) don't bloat it.
CREATE INDEX IF NOT EXISTS idx_telemetry_user_time
  ON telemetry_pings (user_id, received_at DESC)
  WHERE user_id IS NOT NULL;

-- Daily roll-ups: "DAU", "feature adoption over last 30 days".
CREATE INDEX IF NOT EXISTS idx_telemetry_day
  ON telemetry_pings (DATE(received_at));

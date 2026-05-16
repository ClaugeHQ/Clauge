-- Pro v2 — recurring subscriptions + Clauge AI credits
-- Additive against 0001_init.sql. polar_customer_id already exists.
-- ALTER statements are non-idempotent in D1; run-once discipline.

ALTER TABLE users ADD COLUMN subscription_status TEXT NOT NULL DEFAULT 'inactive'
  CHECK (subscription_status IN ('inactive','active','past_due','canceled','unpaid'));
ALTER TABLE users ADD COLUMN cancel_at_period_end INTEGER NOT NULL DEFAULT 0
  CHECK (cancel_at_period_end IN (0,1));
ALTER TABLE users ADD COLUMN current_period_start TEXT;
ALTER TABLE users ADD COLUMN current_period_end   TEXT;
ALTER TABLE users ADD COLUMN past_due_started_at  TEXT;
ALTER TABLE users ADD COLUMN polar_subscription_id TEXT;
ALTER TABLE users ADD COLUMN credit_allowance_per_cycle INTEGER NOT NULL DEFAULT 0
  CHECK (credit_allowance_per_cycle >= 0);
ALTER TABLE users ADD COLUMN credits_remaining INTEGER NOT NULL DEFAULT 0
  CHECK (credits_remaining >= 0);

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_polar_subscription_id
  ON users(polar_subscription_id) WHERE polar_subscription_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_users_past_due_sweep
  ON users(subscription_status, past_due_started_at)
  WHERE subscription_status = 'past_due';

CREATE TABLE IF NOT EXISTS subscription_history (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id         INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
  event_type      TEXT NOT NULL,
  polar_event_id  TEXT NOT NULL UNIQUE,
  payload_json    TEXT NOT NULL,
  occurred_at     TEXT NOT NULL,
  received_at     TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_sub_history_user_time
  ON subscription_history(user_id, occurred_at DESC);

CREATE TABLE IF NOT EXISTS credit_usage_log (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id             INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
  occurred_at         TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  operation           TEXT NOT NULL,
  clauge_credits      INTEGER NOT NULL CHECK (clauge_credits >= 0),
  cost_usd_micros     INTEGER NOT NULL CHECK (cost_usd_micros >= 0),
  request_id          TEXT NOT NULL,
  UNIQUE (user_id, request_id)
);
CREATE INDEX IF NOT EXISTS idx_credit_usage_user_time
  ON credit_usage_log(user_id, occurred_at DESC);

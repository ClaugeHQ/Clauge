-- Per-session telemetry for the Clauge AI proxy. Each tool round the
-- desktop sends a fresh request_id (replay defense), so credit_usage_log
-- already has one row per round. Grouping by session_id lets us derive
-- rounds-per-session, total credits-per-session, and per-mode rollups —
-- the core signal for validating loop-discipline changes (dedup +
-- introspection budget) and spotting runaway sessions.
--
-- Nullable: historical rows fill NULL; old desktop clients that don't
-- send session_id stay functional (the column just stays NULL for them).
-- Composite index on (user_id, session_id) supports the typical
-- "this user's session-by-session breakdown" query without a full scan.

ALTER TABLE credit_usage_log ADD COLUMN session_id TEXT;
CREATE INDEX IF NOT EXISTS idx_credit_usage_log_user_session
  ON credit_usage_log(user_id, session_id);

-- Track per-call tool-round count alongside token usage so we can measure
-- whether loop-discipline changes (dedup + introspection budget) actually
-- reduce rounds-per-session over time. Default 0 keeps historical rows
-- well-formed; new rows fill it from the streamer's `ai:done` event.
ALTER TABLE ai_usage ADD COLUMN tool_rounds INTEGER NOT NULL DEFAULT 0;

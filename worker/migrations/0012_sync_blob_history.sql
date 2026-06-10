-- Last-N version history for sync blobs. Written on every successful push
-- that REPLACES an existing blob (the old row is archived first), pruned
-- to the newest 5 per (user, kind). Restores are client-driven pulls.
CREATE TABLE sync_blob_history (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id      INTEGER NOT NULL,
  kind         TEXT    NOT NULL,
  payload      BLOB    NOT NULL,
  content_hash TEXT    NOT NULL,
  device_name  TEXT,
  replaced_at  TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (user_id) REFERENCES users(user_id) ON DELETE CASCADE
);
CREATE INDEX idx_sbh_user_kind ON sync_blob_history (user_id, kind, id DESC);

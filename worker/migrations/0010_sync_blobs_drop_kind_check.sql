-- sync_blobs: drop the kind CHECK constraint.
--
-- The original `CHECK (kind IN ('rest','sql','nosql','agent','ssh','explorer'))`
-- in 0001_init.sql rejects every new sync domain (coworkers now, more later)
-- with a SQL constraint failure — even after the JS allowlist accepts it.
--
-- SQLite has no ALTER for CHECK constraints, so recreate WITHOUT the CHECK.
-- Same pattern as 0005_lifetime.sql did for billing_pricing.plan_id.
-- Source of truth for valid kinds stays in worker code (SYNC_KINDS in db.js).

CREATE TABLE sync_blobs_new (
  user_id      INTEGER NOT NULL,
  kind         TEXT    NOT NULL,
  payload      BLOB    NOT NULL,
  content_hash TEXT    NOT NULL,
  updated_at   TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (user_id, kind),
  FOREIGN KEY (user_id) REFERENCES users(user_id) ON DELETE CASCADE
);

INSERT INTO sync_blobs_new (user_id, kind, payload, content_hash, updated_at)
  SELECT user_id, kind, payload, content_hash, updated_at FROM sync_blobs;

DROP TABLE sync_blobs;
ALTER TABLE sync_blobs_new RENAME TO sync_blobs;

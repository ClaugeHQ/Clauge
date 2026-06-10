-- Track which device last wrote each sync blob (display-only metadata).
ALTER TABLE sync_blobs ADD COLUMN device_id   TEXT;
ALTER TABLE sync_blobs ADD COLUMN device_name TEXT;

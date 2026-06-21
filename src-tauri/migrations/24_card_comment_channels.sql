-- Card comment channels + external (GitHub/GitLab) provenance.
--
-- `channel` separates the two card-drawer sections:
--   'ticket'   — the real issue discussion (local, or mirrored from GitHub/GitLab)
--   'coworker' — the local AI persona chat (never synced)
-- `external_id`     — provider comment id for fetched issue comments (NULL = local)
-- `external_author` — GitHub/GitLab login for fetched comments (display name)
--
-- For fetched comments `created_at` carries the ORIGINAL provider timestamp.

ALTER TABLE workspace_card_comments ADD COLUMN channel TEXT NOT NULL DEFAULT 'ticket';
ALTER TABLE workspace_card_comments ADD COLUMN external_id TEXT;
ALTER TABLE workspace_card_comments ADD COLUMN external_author TEXT;

-- Existing AI coworker replies belong in the Coworker section.
UPDATE workspace_card_comments SET channel = 'coworker' WHERE coworker_id IS NOT NULL;

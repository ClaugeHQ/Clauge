-- workspace_id is optional: meetings can be captured before the user
-- assigns them to a workspace, and survive workspace deletion.

CREATE TABLE IF NOT EXISTS workspace_meetings (
    id                 TEXT PRIMARY KEY,
    workspace_id       TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    title              TEXT NOT NULL,
    source_app         TEXT,
    started_at         TEXT NOT NULL,
    ended_at           TEXT,
    language           TEXT NOT NULL DEFAULT 'auto',
    transcript         TEXT NOT NULL DEFAULT '[]',
    notes_md           TEXT,
    notes_provider     TEXT,
    notes_model        TEXT,
    notes_generated_at TEXT,
    status             TEXT NOT NULL DEFAULT 'recording',
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workspace_meetings_started
    ON workspace_meetings(started_at);

CREATE VIRTUAL TABLE IF NOT EXISTS workspace_meetings_fts USING fts5(
    title,
    transcript,
    notes_md,
    workspace_id UNINDEXED,
    meeting_id   UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS workspace_meetings_ai
    AFTER INSERT ON workspace_meetings
BEGIN
    INSERT INTO workspace_meetings_fts (title, transcript, notes_md, workspace_id, meeting_id)
    VALUES (
        new.title,
        (SELECT coalesce(group_concat(json_extract(value,'$.text'), ' '), '')
           FROM json_each(new.transcript)),
        new.notes_md,
        new.workspace_id,
        new.id
    );
END;
CREATE TRIGGER IF NOT EXISTS workspace_meetings_ad
    AFTER DELETE ON workspace_meetings
BEGIN
    DELETE FROM workspace_meetings_fts WHERE meeting_id = old.id;
END;
CREATE TRIGGER IF NOT EXISTS workspace_meetings_au
    AFTER UPDATE OF title, transcript, notes_md, workspace_id ON workspace_meetings
BEGIN
    DELETE FROM workspace_meetings_fts WHERE meeting_id = old.id;
    INSERT INTO workspace_meetings_fts (title, transcript, notes_md, workspace_id, meeting_id)
    VALUES (
        new.title,
        (SELECT coalesce(group_concat(json_extract(value,'$.text'), ' '), '')
           FROM json_each(new.transcript)),
        new.notes_md,
        new.workspace_id,
        new.id
    );
END;

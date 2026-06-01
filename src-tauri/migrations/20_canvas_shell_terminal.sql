-- Extend canvas_tiles.tab_kind CHECK constraint to include 'shell_terminal'
-- (a Canvas-spawned plain shell, started from a user-chosen cwd).
--
-- SQLite cannot ALTER a CHECK constraint, so we recreate the table.

CREATE TABLE IF NOT EXISTS canvas_tiles_v20 (
    workspace_id   TEXT NOT NULL,
    -- no FK: orphan cleanup happens at app level via canvas_resolve_tiles.
    tab_id         TEXT NOT NULL,
    tab_kind       TEXT NOT NULL CHECK(tab_kind IN (
        'agent_terminal', 'ssh_terminal',
        'sql_editor', 'rest_request',
        'mongo_query', 'redis_query',
        'workspace_note', 'workspace_board',
        'shell_terminal'
    )),
    x              REAL NOT NULL,
    y              REAL NOT NULL,
    width          REAL NOT NULL,
    height         REAL NOT NULL,
    z_order        INTEGER NOT NULL DEFAULT 0,
    minimized      INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at     TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (workspace_id, tab_id)
);

INSERT INTO canvas_tiles_v20
    (workspace_id, tab_id, tab_kind, x, y, width, height, z_order, minimized, created_at, updated_at)
SELECT
    workspace_id, tab_id, tab_kind, x, y, width, height, z_order, minimized, created_at, updated_at
FROM canvas_tiles;

DROP TABLE canvas_tiles;
ALTER TABLE canvas_tiles_v20 RENAME TO canvas_tiles;

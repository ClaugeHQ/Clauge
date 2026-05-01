CREATE TABLE IF NOT EXISTS agent_sessions (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    purpose TEXT NOT NULL,
    project_path TEXT NOT NULL,
    project_name TEXT NOT NULL,
    claude_session_id TEXT,
    context_prompt TEXT NOT NULL DEFAULT '',
    worktree_path TEXT,
    worktree_branch TEXT,
    skip_permissions INTEGER NOT NULL DEFAULT 0,
    git_name TEXT,
    git_email TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_used_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS agent_contexts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS agent_session_contexts (
    session_id TEXT NOT NULL,
    context_id TEXT NOT NULL,
    PRIMARY KEY (session_id, context_id),
    FOREIGN KEY (session_id) REFERENCES agent_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (context_id) REFERENCES agent_contexts(id) ON DELETE CASCADE
);

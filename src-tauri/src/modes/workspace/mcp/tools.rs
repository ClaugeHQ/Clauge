// MCP `tools/list` schema. Each entry advertises one tool with its
// JSON-Schema input shape. Split into per-category helpers because
// the consolidated `json!([...])` previously needed a global
// `recursion_limit = "256"` bump just to compile. Each helper here
// stays well under the default 128.

use serde_json::{json, Value};

pub(super) fn tool_descriptors() -> Value {
    let mut tools: Vec<Value> = Vec::new();
    tools.extend(workspace_schemas());
    tools.extend(note_schemas());
    tools.extend(board_schemas());
    tools.extend(card_schemas());
    tools.extend(shipping_schemas());
    tools.extend(meta_schemas());
    Value::Array(tools)
}

/// Workspace-level CRUD + project linking + summary.
fn workspace_schemas() -> Vec<Value> {
    vec![
        json!(
        {
            "name": "workspaces_list",
            "description": "List workspaces (containers of notes and boards), most-recently-updated first. Paginated — see `limit` (default 50, max 200) and `offset`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit":  { "type": "integer", "description": "Page size. Default 50, capped at 200." },
                    "offset": { "type": "integer", "description": "Skip this many rows before returning. Default 0." }
                },
                "required": []
            }
        }
        ),
        json!(
        {
            "name": "workspaces_upsert_for_project",
            "description": "Find a workspace bound to the given project path. If none exists, create one named after the folder with a default 5-column board, and return it. Use this whenever you have a project path (e.g. cwd) and want a workspace to put notes or cards in — it's the canonical way to resolve 'this project' to a workspace id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectPath": { "type": "string", "description": "Absolute path to the project's root folder." }
                },
                "required": ["projectPath"]
            }
        }
        ),
        json!(
        {
            "name": "workspace_summary",
            "description": "Workspace-level snapshot: note count, card count, board count, review backlog, and a per-actor edit count breakdown (helps see who is contributing — user vs agent).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" }
                },
                "required": ["workspaceId"]
            }
        }
        ),
        json!(
        {
            "name": "workspace_link_to_repo",
            "description": "Set or clear the workspace's GitHub/GitLab URL. Used as the default remote when a board has no per-board override. Pass `null` (or omit `repoUrl`) to clear.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" },
                    "repoUrl": { "type": ["string", "null"] }
                },
                "required": ["workspaceId"]
            }
        }
        ),
    ]
}

/// Note CRUD, search, surgical edits, freeze, session linking.
fn note_schemas() -> Vec<Value> {
    vec![
        json!(
        {
            "name": "notes_list",
            "description": "List notes inside a workspace, most-recently-updated first. Paginated — see `limit` (default 50, max 200) and `offset`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" },
                    "limit":       { "type": "integer", "description": "Page size. Default 50, capped at 200." },
                    "offset":      { "type": "integer", "description": "Skip this many rows before returning. Default 0." }
                },
                "required": ["workspaceId"]
            }
        }
        ),
        json!(
        {
            "name": "notes_read",
            "description": "Read a note by id (returns title, content, tags, linked session).",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "notes_create",
            "description": "Create a new note inside an existing workspace (when you already know the workspaceId; otherwise prefer notes_create_for_project). Returns the new note. Intent cues — use this tool, NOT the filesystem Write tool, for any request to 'create a note / doc / page / md / markdown file in the workspace / in the notes / in Clauge', or to 'record / capture / jot down / save X in the notes'. Notes live in the Clauge SQLite DB, not on disk; only fall back to filesystem writes if the user explicitly says 'on disk' or names a path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "coworkerId": { "type": "string", "description": "Optional. The coworker (persona) id this write is attributed to. When set, the note is recorded as @<coworker> and is NOT auto-linked to the user's manual Agent session." }
                },
                "required": ["workspaceId", "title"]
            }
        }
        ),
        json!(
        {
            "name": "notes_create_for_project",
            "description": "Create a NEW note for the given project (always creates, never replaces). If no workspace exists for the project path, one is auto-created (named after the folder, with a default board) before the note is added. Returns { workspace, note }. Intent cues — use this tool, NOT the filesystem Write tool, for any request to 'create a note / doc / page / md / markdown file in the workspace / in the notes / in Clauge', or to 'record / capture / jot down / save X in the notes'. Notes live in the Clauge SQLite DB, not on disk; only fall back to filesystem writes if the user explicitly says 'on disk' or names a path. Prefer notes_upsert_for_project when the user is asking to record/refresh information on a topic — it'll update an existing same-titled note instead of stacking duplicates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectPath": { "type": "string", "description": "Absolute path to the project's root folder." },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "coworkerId": { "type": "string", "description": "Optional. The coworker (persona) id this write is attributed to. When set, the note is recorded as @<coworker> and is NOT auto-linked to the user's manual Agent session." }
                },
                "required": ["projectPath", "title"]
            }
        }
        ),
        json!(
        {
            "name": "notes_upsert_for_project",
            "description": "Find-or-create a note in the project's workspace (workspace itself is auto-created if missing). Match is by case-insensitive title within that one workspace. If the note exists, content/tags are UPDATED (replace by default; pass mode='append' to add to the bottom). Returns { workspace, note, created: bool }. Intent cues — use this tool, NOT the filesystem Write tool, for any request to 'create / update a note / doc / page / md / markdown file in the workspace / in the notes / in Clauge', or to 'record / capture / refresh / update X in the notes'. Notes live in the Clauge SQLite DB, not on disk; only fall back to filesystem writes if the user explicitly says 'on disk' or names a path. This is the right tool for evolving topical docs ('Overview', 'Architecture', 'TODO', etc.) — calling it twice with the same title edits the same note instead of duplicating.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectPath": { "type": "string", "description": "Absolute path to the project's root folder." },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "mode": {
                        "type": "string",
                        "enum": ["replace", "append"],
                        "description": "How to apply content when the note already exists. 'replace' (default) overwrites; 'append' adds the new content as a new section at the bottom separated by ---."
                    },
                    "coworkerId": { "type": "string", "description": "Optional. The coworker (persona) id this write is attributed to. When set, the note is recorded as @<coworker> and is NOT auto-linked to the user's manual Agent session." }
                },
                "required": ["projectPath", "title"]
            }
        }
        ),
        json!(
        {
            "name": "notes_update",
            "description": "Update an existing note. Pass any of title, content, tags. Pass the note's current `updatedAt` as `expectedUpdatedAt` to refuse the write if the note was modified concurrently. Intent cues — use this tool, NOT the filesystem Write tool, for any request to 'edit / update / append to / rewrite a note / doc / page / md / markdown file in the workspace / in the notes / in Clauge'. Notes live in the Clauge SQLite DB, not on disk; only fall back to filesystem writes if the user explicitly says 'on disk' or names a path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id":                 { "type": "string" },
                    "title":              { "type": "string" },
                    "content":            { "type": "string" },
                    "tags":               { "type": "array", "items": { "type": "string" } },
                    "expectedUpdatedAt":  { "type": "string", "description": "Optional. The `updatedAt` you read on this note. If it no longer matches, the call returns a conflict error so you can re-read and retry." },
                    "coworkerId":         { "type": "string", "description": "Optional. The coworker (persona) id this write is attributed to. When set, the note is recorded as @<coworker> and is NOT auto-linked to the user's manual Agent session." }
                },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "notes_search",
            "description": "Full-text search notes by title and content. Optional workspaceId scopes the search; otherwise searches across all workspaces. Returns ranked notes (best match first). Use this before notes_upsert_for_project to check whether a topic already has a note under a different title.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "FTS5 query string. Plain words match prefix-tokens; quote phrases for exact matches." },
                    "workspaceId": { "type": "string" },
                    "limit": { "type": "integer", "description": "Max rows to return (default 20)." }
                },
                "required": ["query"]
            }
        }
        ),
        json!(
        {
            "name": "notes_append_section",
            "description": "Append a new markdown section (heading + body) to the bottom of an existing note. Less destructive than notes_update — preserves all prior content. Use this for incremental log/journal style notes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "heading": { "type": "string", "description": "Section heading text. A '## ' prefix is added automatically." },
                    "content": { "type": "string", "description": "Body of the new section." }
                },
                "required": ["id", "heading", "content"]
            }
        }
        ),
        json!(
        {
            "name": "notes_apply_diff",
            "description": "Surgical find/replace on a note's content (literal string match — not a regex). The `find` text must appear exactly once or the call errors. Prefer this over notes_update for small edits where you don't want to risk overwriting the rest of the note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "find": { "type": "string" },
                    "replace": { "type": "string" }
                },
                "required": ["id", "find", "replace"]
            }
        }
        ),
        json!(
        {
            "name": "notes_link_to_session",
            "description": "Attach an agent session id to a note. Lets the UI jump from a note straight back to the conversation that produced it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "sessionId": { "type": ["string", "null"] }
                },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "notes_freeze",
            "description": "Mark a note as frozen — agents (including this one) can no longer mutate it via MCP until unfrozen. UI edits are unaffected. Use to lock down canonical docs (e.g. 'Architecture Overview').",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "notes_unfreeze",
            "description": "Reverse of notes_freeze.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        }
        ),
    ]
}

/// Board CRUD + summary (cards come back via boards_read).
fn board_schemas() -> Vec<Value> {
    vec![
        json!(
        {
            "name": "boards_list",
            "description": "List boards inside a workspace, ordered by display position. Paginated — see `limit` (default 50, max 200) and `offset`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" },
                    "limit":       { "type": "integer", "description": "Page size. Default 50, capped at 200." },
                    "offset":      { "type": "integer", "description": "Skip this many rows before returning. Default 0." }
                },
                "required": ["workspaceId"]
            }
        }
        ),
        json!(
        {
            "name": "boards_read",
            "description": "Read a board's columns and cards. Returns { columns, cards } in one payload.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "boards_summary",
            "description": "One-shot board health snapshot: per-column card counts, total cards, review backlog. Cheaper than boards_read when you only need numbers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "boardId": { "type": "string" }
                },
                "required": ["boardId"]
            }
        }
        ),
    ]
}

/// Card CRUD, move, search, review workflow, freeze, claim/release, comments, drawer chat.
fn card_schemas() -> Vec<Value> {
    vec![
        json!(
        {
            "name": "cards_create",
            "description": "Create a new card inside a column. Pass `coworkerId` to attribute the card to a persona (the persona's avatar/name will appear on the card foot). When you spin off a card while discussing another card, drop a comment on the source card linking to the new one — that's how lineage is communicated, there's no first-class parent/child relationship.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "columnId":    { "type": "string" },
                    "title":       { "type": "string" },
                    "description": { "type": "string" },
                    "priority":    { "type": "string", "enum": ["P0", "P1", "P2", "P3"] },
                    "tags":        { "type": "array", "items": { "type": "string" } },
                    "coworkerId":  { "type": "string", "description": "Persona that's creating this card. Use your declared coworker_id when acting as a persona." }
                },
                "required": ["columnId", "title"]
            }
        }
        ),
        json!(
        {
            "name": "cards_update",
            "description": "Update a card's title, description, priority, tags, or review checklist. Pass `coworkerId` to record which persona made the change. Pass the card's current `updatedAt` as `expectedUpdatedAt` to refuse the write if the card was modified concurrently.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id":                { "type": "string" },
                    "title":             { "type": "string" },
                    "description":       { "type": "string" },
                    "priority":          { "type": "string" },
                    "tags":              { "type": "array", "items": { "type": "string" } },
                    "reviewChecklist":   { "type": "string" },
                    "coworkerId":        { "type": "string" },
                    "expectedUpdatedAt": { "type": "string", "description": "Optional. The `updatedAt` you read on this card. If it no longer matches, the call returns a conflict error so you can re-read and retry." }
                },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "cards_move",
            "description": "Move a card to a column / position. Moving an agent's card to a Review-class column auto-flags it as Pending review.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "columnId": { "type": "string" },
                    "position": { "type": "integer" }
                },
                "required": ["id", "columnId"]
            }
        }
        ),
        json!(
        {
            "name": "cards_search",
            "description": "Full-text search board cards by title and description. Returns ranked cards (best match first). Useful for de-duplicating: search before cards_create.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "workspaceId": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }
        }
        ),
        json!(
        {
            "name": "cards_approve",
            "description": "Clear the Pending-review flag on a card (optionally appending a short approval comment to the description). Use when the human work the agent submitted has been validated and the card should leave the review queue.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "comment": { "type": "string", "description": "Optional approval note appended to the card description." }
                },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "cards_request_changes",
            "description": "Send a card back from Review with structured feedback. Appends the feedback to the description (under a 'Review feedback' marker), clears the Pending-review flag, and (if columnId is provided) moves the card there — typically the 'In Review' column.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "feedback": { "type": "string" },
                    "columnId": { "type": "string", "description": "Optional column to move the card to (e.g. the In Review column)." }
                },
                "required": ["id", "feedback"]
            }
        }
        ),
        json!(
        {
            "name": "cards_list_pending_review",
            "description": "List cards currently flagged as Pending review (i.e. an agent moved them into a Review column). Optionally scope to one workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspaceId": { "type": "string" }
                },
                "required": []
            }
        }
        ),
        json!(
        {
            "name": "cards_create_from_branch",
            "description": "Convenience: create a card titled after a git branch name (e.g. 'feature/add-login' → 'Add login'). The branch is stored as `externalId` so future tooling can link card ↔ branch. If projectPath is given, the card lands in the project's workspace's first board (Todo column when present, else first column). Otherwise pass columnId explicitly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "branchName": { "type": "string" },
                    "projectPath": { "type": "string" },
                    "columnId": { "type": "string", "description": "Override target column. Wins over projectPath inference." },
                    "description": { "type": "string" }
                },
                "required": ["branchName"]
            }
        }
        ),
        json!(
        {
            "name": "cards_link_to_session",
            "description": "Attach an agent session id to a card. Same purpose as notes_link_to_session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "sessionId": { "type": ["string", "null"] }
                },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "cards_freeze",
            "description": "Mark a card as frozen — blocks agent updates/moves. UI edits unaffected.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "cards_unfreeze",
            "description": "Reverse of cards_freeze.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "cards_add_comment",
            "description": "Post a comment on a card. Comments live in the card's thread (visible in the drawer). Pass `coworkerId` when you're acting as a persona — your comment renders with that persona's avatar + name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id":         { "type": "string" },
                    "body":       { "type": "string" },
                    "coworkerId": { "type": "string", "description": "Persona authoring the comment, when applicable." }
                },
                "required": ["id", "body"]
            }
        }
        ),
        json!(
        {
            "name": "cards_call_coworker",
            "description": "Invoke a specific coworker (persona) to chat on a card. The system: (1) posts your message as a comment from the calling agent, (2) creates-or-reuses the card's hidden session for that coworker, (3) runs the coworker as an agent with their persona prompt, (4) posts the response as a comment from the coworker, and (5) returns the response text so you can summarise back to the user. Use this when the user asks you to involve a specific coworker on a card without leaving your own session. The card becomes claimed by the coworker after this call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id":         { "type": "string", "description": "Card id." },
                    "coworkerId": { "type": "string", "description": "Coworker to invoke." },
                    "message":    { "type": "string", "description": "What to ask them — e.g. 'Brainstorm OAuth approaches for this card'." }
                },
                "required": ["id", "coworkerId", "message"]
            }
        }
        ),
        json!(
        {
            "name": "cards_claim",
            "description": "Claim ownership of a card for this agent's calling session — the agent becomes the single work-stream allowed to drive the card going forward. The drawer in the UI will show 'Active in <session-title>' and disable its in-drawer chat. Use this when the user tells you to 'work on card X' from the terminal: claim, then proceed to add comments / move columns / write code as normal. Errors when the card is already claimed by a different session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Card id." }
                },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "cards_release",
            "description": "Release the claim this session holds on a card. Use when finished working — the card unlocks and the drawer can host new chats again. No-op when the card isn't claimed by this session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Card id." }
                },
                "required": ["id"]
            }
        }
        ),
    ]
}

/// Shipping ops — only fire on explicit user request (start_work, push, commit, PR).
fn shipping_schemas() -> Vec<Value> {
    vec![
        json!(
        {
            "name": "cards_start_work",
            "description": "Create an isolated git worktree + branch for this card and attach it to the active hidden session. Call this BEFORE you make file edits in a card-driven chat — your subsequent edits go into the worktree, keeping the user's main checkout clean. No-op if the card already has a worktree. Errors when the card is owned by a manual terminal session (those manage their own worktrees).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Card id." }
                },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "cards_push_to_repo",
            "description": "Create a real GitHub/GitLab issue from a local card. Requires the workspace to have a repo URL set (workspace_link_to_repo) — otherwise the call errors. The card must currently be local (no externalId). On success the card's externalId/externalUrl are populated; the local→repo badge updates automatically. This call SHELLS OUT to `gh` or `glab` and so requires the user to have those CLIs installed and authenticated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" }
                },
                "required": ["id"]
            }
        }
        ),
        json!(
        {
            "name": "cards_commit",
            "description": "Stage and commit any pending changes in the card's worktree with the given message. Requires an active claim + worktree (call cards_start_work first if needed). Errors with 'no changes' when the worktree is clean. ONLY call this when the user explicitly asks to commit — never on your own initiative. Drops a 'Commit on <branch>' bubble on the card thread so the user sees the activity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cardId":  { "type": "string", "description": "Card id." },
                    "message": { "type": "string", "description": "Commit message. Should describe the change in present tense — e.g. 'Add login rate-limit guard'." }
                },
                "required": ["cardId", "message"]
            }
        }
        ),
        json!(
        {
            "name": "cards_raise_pr",
            "description": "Push the card's branch and (if no PR exists yet) open a GitHub PR / GitLab MR for it. Idempotent — when the card already has a pr_url, this just pushes new commits to the existing PR's branch (no second PR is opened). Requires worktree + branch + workspace repo URL. ONLY call when the user explicitly asks ('raise a PR', 'ship it', 'push it') — never autonomously. Returns { prUrl, alreadyExisted, branch }.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cardId": { "type": "string", "description": "Card id." },
                    "title":  { "type": "string", "description": "Optional PR title. Defaults to the card title." },
                    "body":   { "type": "string", "description": "Optional PR body. Defaults to a one-line reference to the card thread." }
                },
                "required": ["cardId"]
            }
        }
        ),
        json!(
        {
            "name": "cards_link_pr",
            "description": "Stamp a PR / MR URL onto a card without running any CLI. Use when you raised a PR via raw bash (legacy path) and want the card's UI to show the link. The preferred path is cards_raise_pr — it does the push + open + link in one step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cardId": { "type": "string", "description": "Card id." },
                    "prUrl":  { "type": "string", "description": "Full PR / MR URL." }
                },
                "required": ["cardId", "prUrl"]
            }
        }
        ),
    ]
}

/// Activity feed + coworker list.
fn meta_schemas() -> Vec<Value> {
    vec![
        json!(
        {
            "name": "activity_feed",
            "description": "Recent agent activity — notes and cards mutated by non-user actors. Filter by actor and a since-timestamp. Same data the Inbox UI surfaces; useful for an agent to catch up on what other agents have been doing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "since": { "type": "string", "description": "RFC3339 timestamp; only entries newer than this are returned." },
                    "actor": { "type": "string", "description": "Filter to one actor slug (e.g. 'claude', 'codex')." },
                    "limit": { "type": "integer", "description": "Default 50." }
                },
                "required": []
            }
        }
        ),
        json!(
        {
            "name": "coworkers_list",
            "description": "List all coworkers (personas) the user has set up. Each coworker has a name, role, system_prompt that's appended at agent spawn, avatar, and underlying provider. Use this to know who's on the team — e.g. when the user asks 'who's working with me?'.",
            "inputSchema": { "type": "object", "properties": {}, "required": [] }
        }
        ),
    ]
}

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Project issue scan — used by `workspace_scan_project_issues` to fetch
// open issues from the workspace's bound project (GitHub via `gh`,
// GitLab via `glab`) so the kanban can pre-populate cards.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIssue {
    pub external_id: String,
    pub title: String,
    pub body: String,
    pub url: String,
    /// 'github' | 'gitlab' — drives the icon shown on the imported card.
    pub source: String,
    pub labels: Vec<String>,
}

/// Result of a project-issue scan. Each variant maps 1:1 to a UI banner
/// state with its own action button (install tool, run auth, retry, …).
/// Frontend matches on `kind` and renders accordingly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProjectScanResult {
    #[serde(rename_all = "camelCase")]
    Success {
        issues: Vec<ProjectIssue>,
        remote: String,
        source: String,
    },
    NotGitRepo,
    NoRemote,
    #[serde(rename_all = "camelCase")]
    UnsupportedRemote { url: String },
    #[serde(rename_all = "camelCase")]
    ToolNotInstalled { tool: String, install_url: String },
    #[serde(rename_all = "camelCase")]
    NotAuthenticated { tool: String, login_command: String },
    /// CLI is authenticated but the active account can't access this
    /// repo. Common case: user has multiple `gh` accounts and the
    /// wrong one is active. Banner copy points at `gh auth switch`
    /// (or `gh auth login` for first-time multi-account setup).
    #[serde(rename_all = "camelCase")]
    NoAccess { tool: String, repo: String, login_command: String },
    /// Network / DNS / connectivity failure — transient, retry.
    #[serde(rename_all = "camelCase")]
    NetworkError { message: String },
    #[serde(rename_all = "camelCase")]
    ApiError { message: String },
}


#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub project_path: Option<String>,
    pub project_name: Option<String>,
    pub color: Option<String>,
    pub created_at: String,
    pub created_by: String,
    pub updated_at: String,
    pub updated_by: String,
    /// Workspace-level GitHub/GitLab URL. Used as the agent's default
    /// remote when no per-board override is set. Migration 12 added.
    pub repo_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceNote {
    pub id: String,
    pub workspace_id: String,
    pub title: String,
    pub content: String,
    /// JSON-encoded `string[]`. Kept as a string at the SQL boundary so
    /// FromRow stays trivial; frontends parse on receive.
    pub tags: String,
    pub linked_session_id: Option<String>,
    pub created_at: String,
    pub created_by: String,
    pub updated_at: String,
    pub updated_by: String,
    /// `1` = blocked from agent edits; UI is free to edit. Migration
    /// 12 added. Tools that mutate must check this and return an
    /// error explaining the row is frozen.
    pub frozen: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBoard {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    /// `'manual' | 'github_issues'` etc. Currently always `manual` —
    /// non-manual sources land with the v1.5 issue-sync work.
    pub source: String,
    pub source_config: Option<String>,
    pub position: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBoardColumn {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub color: Option<String>,
    pub position: i32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCardComment {
    pub id: String,
    pub card_id: String,
    pub actor: String,
    /// Coworker (persona) this reply belongs to. NULL for plain user
    /// comments and for any pre-coworker agent comments.
    pub coworker_id: Option<String>,
    pub body: String,
    /// Reserved for threaded replies; always None in v1.
    pub parent_id: Option<String>,
    pub created_at: String,
}

/// Persona built on top of an underlying agent CLI. Global to the
/// user — not workspace-scoped — so personas follow you across
/// projects. Drives the @<name> chat experience: pick Alex
/// (Brainstormer) or Matt (Developer) and the agent gets their
/// system_prompt appended at spawn time.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCoworker {
    pub id: String,
    pub name: String,
    /// One-line role/skill — drives the chip caption ("Brainstormer",
    /// "Developer", "Reviewer", …).
    pub role: String,
    /// Free-form prompt added to the agent's system prompt on every
    /// run for this coworker. Keeps the persona consistent.
    pub system_prompt: String,
    /// CLI provider id ('claude', 'codex', 'gemini', 'opencode').
    /// v1 wires only 'claude'; the column is here so other arms slot
    /// in later without a migration.
    pub provider: String,
    /// dicebear seed (defaults to the name; user can re-roll). Same
    /// seed → same avatar deterministically.
    pub avatar_seed: String,
    /// dicebear collection name ('personas', 'bottts', 'avataaars',
    /// …). Default 'personas'.
    pub avatar_style: String,
    pub created_at: String,
    pub created_by: String,
    pub disabled_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBoardCard {
    pub id: String,
    pub column_id: String,
    pub title: String,
    pub description: String,
    pub priority: Option<String>,
    pub tags: String,
    pub position: i32,
    pub external_id: Option<String>,
    pub external_url: Option<String>,
    /// PR / MR URL once `cards_raise_pr` has been called for this card.
    /// `None` until the first PR is raised. Subsequent raises detect
    /// this and push commits to the same branch instead of opening
    /// a new PR.
    pub pr_url: Option<String>,
    pub linked_session_id: Option<String>,
    /// `1` when an agent moved this card into a Review column. Surfaced
    /// as a "Pending review" badge in the UI; user clears by approving
    /// (move to Done) or requesting changes (move elsewhere).
    pub review_pending: i32,
    pub review_checklist: Option<String>,
    pub created_at: String,
    pub created_by: String,
    pub updated_at: String,
    pub updated_by: String,
    /// Same semantics as WorkspaceNote.frozen — agent mutations
    /// blocked, UI edits allowed. Migration 12 added.
    pub frozen: i32,
    /// The single session currently allowed to drive the agent on
    /// this card (drawer chat or terminal). `None` = unclaimed; any
    /// surface can start a chat. Non-null = locked; other surfaces
    /// see a banner directing them to the active session.
    pub claimed_session_id: Option<String>,
    /// Persona that owns the active conversation. Always set together
    /// with `claimed_session_id` from drawer-driven chat; may be NULL
    /// even when `claimed_session_id` is set if a manual terminal
    /// session claimed the card (terminal sessions don't have a
    /// persona today — that's a future enhancement).
    pub claimed_coworker_id: Option<String>,
    /// Persona that created this card, when known. The agent slug is
    /// always in `created_by`; this column resolves the *display name*
    /// safely against renames (we look up the coworker row at render).
    pub created_by_coworker_id: Option<String>,
    /// Persona behind the most-recent mutation. Same semantics as
    /// `created_by_coworker_id`.
    pub updated_by_coworker_id: Option<String>,
    /// Total comments on this card. Computed via subquery in
    /// `list_cards_in_board`; 0 by default for SELECT * paths that
    /// don't include it (single-card getters).
    #[sqlx(default)]
    pub comment_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMeeting {
    pub id: String,
    /// Optional — meetings can be captured before being assigned to a
    /// workspace, and survive workspace deletion (ON DELETE SET NULL).
    pub workspace_id: Option<String>,
    pub title: String,
    /// App the audio was captured from ('zoom', 'meet', …), when the
    /// call detector identified one.
    pub source_app: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Whisper language hint; 'auto' = detect.
    pub language: String,
    /// JSON-encoded `TranscriptSegment[]`. Kept as a string at the SQL
    /// boundary so FromRow stays trivial; frontends parse on receive.
    pub transcript: String,
    pub notes_md: Option<String>,
    /// Provider/model that generated `notes_md`. All three notes_*
    /// columns stay NULL (or keep their last values) on manual edits —
    /// only AI generation stamps them.
    pub notes_provider: Option<String>,
    pub notes_model: Option<String>,
    pub notes_generated_at: Option<String>,
    /// 'recording' | 'transcribed' | 'notes_ready'.
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// One transcribed chunk of meeting audio. Stored as elements of the
/// JSON array in `WorkspaceMeeting.transcript`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Audio origin: 'mic' | 'system'.
    pub source: String,
    pub text: String,
}

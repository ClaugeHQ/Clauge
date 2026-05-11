// Centralized event-name constants.
//
// Two distinct event channels are used in this app:
//   • Tauri backend → frontend events via `@tauri-apps/api/event` (`listen`).
//     These names are mirrored on the Rust side. Many are session-scoped via
//     a `:<sessionId>` suffix — use the helper builders below.
//   • DOM CustomEvents on `window` for cross-component coordination
//     (avoids circular imports between SvelteKit components).

// --- Tauri events (Rust → frontend) ---
//
// Session-scoped: the Rust emitter appends `:<sessionId>` to the base name.
// Use the builder helpers — never concatenate inline.

export const AI_EVENT = {
  TEXT: 'ai:text',
  TOOL_START: 'ai:tool_start',
  TOOL_END: 'ai:tool_end',
  TOOL_PENDING: 'ai:tool_pending',
  ACTION: 'ai:action',
  DONE: 'ai:done',
  ERROR: 'ai:error',
} as const;

export const aiEvent = {
  text: (sessionId: string) => `${AI_EVENT.TEXT}:${sessionId}`,
  toolStart: (sessionId: string) => `${AI_EVENT.TOOL_START}:${sessionId}`,
  toolEnd: (sessionId: string) => `${AI_EVENT.TOOL_END}:${sessionId}`,
  toolPending: (sessionId: string) => `${AI_EVENT.TOOL_PENDING}:${sessionId}`,
  action: (sessionId: string) => `${AI_EVENT.ACTION}:${sessionId}`,
  done: (sessionId: string) => `${AI_EVENT.DONE}:${sessionId}`,
  error: (sessionId: string) => `${AI_EVENT.ERROR}:${sessionId}`,
} as const;

// --- DOM CustomEvents (cross-component coordination) ---

export const SSH_EVENT = {
  OPEN_TAB: 'ssh:open-tab',
  CLOSE_TAB: 'ssh:close-tab',
  ADD_TAB: 'ssh:add-tab',
  INSERT_COMMAND: 'ssh:insert-command',
  EXECUTE_CAPTURE_REQUEST: 'ssh:execute-capture-request',
  PROFILE_CREATED: 'ssh:profile-created',
  PROFILE_UPDATED: 'ssh:profile-updated',
  NEW_PROFILE: 'ssh:new-profile',
  /** Open a NEW independent SSH tab for the given profile (always creates,
   * never activates an existing tab). Detail: SshProfile. */
  DUPLICATE_SESSION: 'ssh:duplicate-session',
} as const;

export const EXPLORER_EVENT = {
  ADD_TAB: 'explorer:add-tab',
  ADD_CONNECTION: 'explorer:add-connection',
  OPEN_TAB: 'explorer:open-tab',
} as const;

export const AGENT_EVENT = {
  ADD_TAB: 'agent:add-tab',
  CLOSE_TAB_SESSION: 'agent:close-tab-session',
  RESET_SESSION: 'agent:reset-session',
  DELETE_SESSION: 'agent:delete-session',
  RELAUNCH_SESSION: 'agent:relaunch-session',
  EDIT_SESSION: 'agent:edit-session',
  NEW_SESSION: 'agent:new-session',
  SELECT_SESSION: 'agent:select-session',
  SHOW_USAGE_DASHBOARD: 'agent:show-usage-dashboard',
} as const;

export const WORKSPACE_EVENT = {
  ADD_TAB: 'workspace:add-tab',
  NEW_WORKSPACE: 'workspace:new-workspace',
  EDIT_WORKSPACE: 'workspace:edit-workspace',
  DELETE_WORKSPACE: 'workspace:delete-workspace',
  NEW_NOTE: 'workspace:new-note',
  NEW_BOARD: 'workspace:new-board',
} as const;

export const APP_EVENT = {
  TAB_CLOSE_PROMPT: 'clauge:tab-close-prompt',
  SQL_SAVE: 'clauge:sql-save',
  SAVE_NEW_REQUEST: 'clauge:save-new-request',
  /** Detail: `{ provider: 'github' | 'google'; code: string }`.
   *  The receiver calls `cloudExchangeCode(provider, code)`. */
  OAUTH_CALLBACK: 'clauge:oauth-callback',
} as const;

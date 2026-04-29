# Clauge — Architecture Rules

> Read this BEFORE editing any file in this repo. These rules govern every change, regardless of who or what is editing.
>
> Pair docs (read alongside this file):
> - `/Users/macbook/Personal/Ctx/PROJECT.md` — current architecture snapshot. **Lives outside this repo**, in the user's personal context store. It describes the post-refactor reality (modes/, shared/, repos, registries, CLI trait, credential store, schema, behaviors).

This file describes the codebase **as it stands** after the Wave 1–4A refactor completed 2026-04-29. The refactor is done; the structure is settled. New work extends what's here, it doesn't re-shape it.

---

## 1. The five properties of a scalable codebase

The operating definition of "scalable" here. Every change should reinforce these. Anything that breaks them is debt.

1. **Locality** — a feature's code lives in one place. Editing it touches that place, nothing else.
2. **Stable seams** — layers communicate through narrow, explicit contracts. Implementation can change behind a seam without rippling outward.
3. **Composition over duplication / bespoke giants** — small primitives composed many ways, not large one-off files. CSS, constants, and logic don't co-habit.
4. **Open / closed for extension** — core code is closed to modification, open to extension via registry / interface / configuration. New things slot in without editing existing code.
5. **Cost-of-change scales with the change, not the codebase** — a small edit reads a small amount of code.

---

## 2. The four standing rules (apply to every PR / commit / edit)

1. **Folder by feature, not by layer.** Each mode owns *all* of its code:
   - Frontend: `src/lib/modes/<mode>/` — components, AI files (`ai/{prompt,context,...}.ts`), command invokers, types, stores.
   - Backend: `src-tauri/src/modes/<mode>/` — commands, models, terminal/client, `ai_tools.rs`.
   - `src/lib/shared/` and `src-tauri/src/shared/` are only for things genuinely used by ≥3 features.
2. **Cross-cutting concerns sit behind a registry or interface — never a hardcoded match arm.**
   - Provider × model → `src/lib/shared/ai/providers.ts` (TS) and `src-tauri/src/shared/ai/providers.rs` (Rust).
   - Tool dispatch → `src-tauri/src/shared/ai/dispatch.rs`. Modes register their tools at startup via `register_tools()` in `src-tauri/src/modes/<mode>/ai_tools.rs`.
   - CLI specifics → `CliRunner` trait in `src-tauri/src/shared/cli/runner.rs`; concrete implementations in `claude.rs` (and future `codex.rs`/`gemini.rs`).
   - Credential storage → `CredentialStore` trait in `src-tauri/src/shared/platform/credential_store.rs`.
   - DB persistence → `src-tauri/src/shared/repos/<aggregate>.rs`.
3. **Soft cap ~500 LOC per `.svelte` / `.rs` / `.ts`. No file mixes CSS + constants + logic at scale.**
   - Past 500 LOC: refactor before adding.
   - CSS extracts to co-located `*.svelte.css` modules imported via `@import` (preserves Svelte CSS scoping). Currently in use for `SettingsModal`, `AIPanel`, `DocumentViewer`.
4. **No magic strings or numbers at call sites.** Every storage key, event name, timeout/interval/debounce, color/hex/rgba is a typed constant in `src/lib/shared/constants/`:
   - `storage.ts` — all `localStorage` keys (`clauge_*` prefix).
   - `events.ts` — all Tauri/window event names.
   - `timings.ts` — all timeouts, intervals, debounces.
   - `colors.ts` — all hex/rgba and CSS-var color tokens.
   - Import — never hardcode at the call site.

---

## 3. Repository structure (post-refactor reality)

```
src/lib/
├── modes/
│   ├── agent/   index.ts cmds.ts stores.ts types.ts ai/{prompt}.ts components/
│   ├── rest/    index.ts cmds.ts stores.ts types.ts ai/{prompt,context}.ts components/ utils/
│   ├── sql/     (same shape) utils/splitter.ts
│   ├── nosql/   (same shape) components/DocumentViewer.svelte + .svelte.css
│   └── ssh/     index.ts cmds.ts stores.ts types.ts ai/{prompt,execute,safety}.ts components/
├── shared/
│   ├── constants/  storage.ts events.ts timings.ts colors.ts
│   ├── ai/         providers.ts
│   ├── primitives/ Modal.svelte ConfirmDialog.svelte Toast.svelte ContextMenu.svelte Dropdown.svelte EnvInput.svelte ImportExportModal.svelte SaveRequestDialog.svelte ShortcutsOverlay.svelte UpdateNotification.svelte WhatsNewModal.svelte WindowControls.svelte
│   ├── stores/     tabs.ts
│   └── utils/      json-highlight.ts import-parser.ts
├── components/   (shell-level, non-mode)
│   ├── ai/AIPanel.svelte + .svelte.css
│   ├── settings/SettingsModal.svelte + .svelte.css
│   ├── sidebar/  topbar/  nav/  statusbar/  env/  github/  onboarding/
├── commands/     ai.ts github.ts settings.ts index.ts   (cross-mode invokers)
├── stores/       app.ts github.ts settings.ts index.ts  (cross-mode shell state)
├── services/     ai-chat.ts                              (AI streaming + tool dispatch event glue)
├── types/        ai.ts settings.ts index.ts
├── utils/        theme.ts shortcuts.ts updater.ts errors.ts variable-resolver.ts
└── assets/

src-tauri/src/
├── modes/
│   ├── agent/   mod.rs commands.rs models.rs terminal.rs git.rs worktree.rs plugins.rs usage.rs ai_tools.rs
│   ├── rest/    mod.rs collections.rs requests.rs environments.rs http_executor.rs history.rs import_export.rs ai_tools.rs
│   ├── sql/     mod.rs client.rs ai_tools.rs
│   ├── nosql/   mod.rs client.rs ai_tools.rs
│   └── ssh/     mod.rs models.rs profiles.rs terminal.rs ai_tools.rs
├── shared/
│   ├── ai/      providers.rs dispatch.rs
│   ├── cli/     runner.rs claude.rs
│   ├── platform/ credential_store.rs
│   └── repos/   sessions.rs collections.rs requests.rs environments.rs history.rs settings.rs sql_connections.rs nosql_connections.rs ssh_profiles.rs ai_usage.rs
├── commands/    (legacy holdouts — see below)
│   ├── settings.rs
│   └── ai/      mod.rs types.rs context.rs anthropic.rs openai.rs usage.rs
├── db/          migrations.rs models.rs
├── github/      oauth.rs gist.rs
└── appearance/  vibrancy.rs
```

**Legacy holdouts (deliberate, deferred):**
- `src-tauri/src/commands/ai/{mod,types,anthropic,openai,context,usage}.rs` — provider/streaming clients still live here. Provider × model config and tool dispatch already moved to `shared/ai/`. Future cleanup may relocate the streaming clients themselves into `shared/ai/`.
- `src-tauri/src/modes/sql/client.rs` retains inline driver-specific queries against **user-supplied** databases. These are NOT app-DB persistence — the repository pattern doesn't apply.
- `src-tauri/src/modes/rest/import_export.rs` was moved out of legacy `commands/` but remains tangled (cross-aggregate transactions across collections/requests/environments). Deferred for a future targeted pass.
- DB column `agent_sessions.claude_session_id` is unchanged; the `CliRunner` trait abstracts CLI specifics, but renaming the column to `cli_session_id` would require a risky migration and isn't blocking. Live with the name.

---

## 4. Cheat sheet — ask before every edit

1. **Locality** — does this code live in one folder? If it spans shared layer-folders, fix that first.
2. **Seam** — am I crossing transport / business-logic / persistence inside one function? Split it.
3. **Composition** — is this file > 500 LOC? Mixing CSS + logic + constants? Extract before adding.
4. **Open/closed** — am I editing an N-arm match to add the (N+1)th case? Make it a registry.
5. **Cost-of-change** — how much surrounding code does an AI need to read for this change? If much, the structure is wrong; fix the structure first.

---

## 5. Verification before commit (mandatory)

A commit that fails any of these is broken — fix it, don't bypass:

- `bun run check` (frontend type-check) — must pass with 0 errors.
- `bun run build` (frontend production build) — must pass with 0 errors.
- `cargo check --manifest-path src-tauri/Cargo.toml` — must pass with 0 errors.
- For non-trivial Rust changes: `cargo build --release --manifest-path src-tauri/Cargo.toml` — must pass.
- For UI-affecting changes: manually verify the affected mode in `bunx tauri dev`.

Never use `--no-verify`, `--no-gpg-sign`, or skip hooks.

---

## 6. Tauri v2 invariants (don't break these)

- All Rust structs serialized to JS need `#[serde(rename_all = "camelCase")]`.
- JS `invoke()` params use camelCase keys (`{ projectPath }` not `{ project_path }`).
- Modals render at root level in `+layout.svelte`, NOT inside `NavPanel` (clipping).
- Glass mode: `--c` and `--e` are rgba — don't stack backgrounds.
- Never use `document.body.style.zoom` (breaks WebKit).
- Production builds block right-click, Cmd+R, F5, dev tools.
- WKWebView Date parsing: timestamps must be `chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)` (3-digit fractional seconds), never plain `to_rfc3339()` (microseconds make `new Date(iso)` return Invalid Date).
- Tauri updater pubkey in `tauri.conf.json` must match the CI signing key — never edit `pubkey` without verifying the matching `TAURI_SIGNING_PRIVATE_KEY` is in CI. Mismatch orphans every existing user's auto-update.

---

## 7. Commit / PR / git rules

- Use git identity `ansxuman@gmail.com` (already configured).
- Branch → PR → merge; never push directly to `main`.
- **No co-author lines, no AI/Claude/tool attribution** in commit messages, PR descriptions, or code/comments. Commits describe *what* changed and *why* — not who or what wrote them. (The file `src-tauri/src/shared/cli/claude.rs` is fine — that's a CLI binary reference, not attribution.)
- Prefer many small commits over one large one.
- Commit message style: `<scope>: <imperative>` (e.g., `ssh: extract execute_shell into ai_tools.rs`, `shared: introduce timings constants module`).

---

## 8. In-place tags & references

- Git tag `pre-refactor-baseline` marks the commit before any Wave 1–4A refactor began. It is the rollback anchor for the refactor. **Do not delete or retag it.**
- The `feat/qorix-merge` branch carries the super-app merge + the entire refactor; nothing pushed yet.

---

## 9. When in doubt

1. Read `/Users/macbook/Personal/Ctx/PROJECT.md` — current architecture snapshot.
2. Open the active mode folder (`src/lib/modes/<mode>/` and `src-tauri/src/modes/<mode>/`) — the slice tells you the patterns in use.
3. If a new pattern is needed (new top-level folder, new registry, new shared concept), **ask before introducing it.** The shared kernel is intentionally thin.

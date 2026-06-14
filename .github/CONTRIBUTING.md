# Contributing to Clauge

Thanks for taking the time to contribute. Clauge is a desktop app built with Tauri + SvelteKit, and we welcome patches, bug reports, and feature ideas from the community.

Before opening a pull request, please read this document end-to-end — especially the **Branch Naming** and **Commit Authorship** sections.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/your-username/Clauge.git`
3. Install dependencies: `bun install`
4. Run in development: `bun run tauri dev`

## Prerequisites

- [Bun](https://bun.sh) (latest)
- [Rust](https://rustup.rs) (1.77+)
- [Tauri CLI](https://tauri.app) v2

## Project Structure

- `src/` — SvelteKit frontend (Svelte 5)
- `src-tauri/` — Rust backend (Tauri v2)
- `src-tauri/src/lib.rs` — Core logic (PTY, sessions, commands)
- `src-tauri/scripts/` — Helper binaries (usage fetcher)

## Branch Naming

Branch off `main`. Use a prefix that matches the kind of work, followed by a short kebab-case description:

| Prefix | When to use | Example |
|---|---|---|
| `feature/` | A new user-facing capability | `feature/sql-clickhouse-driver` |
| `fix/` | A bug fix | `fix/agent-session-leak` |
| `chore/` | Build, tooling, deps, infra | `chore/bump-tauri-2.1` |
| `docs/` | README, contributing, comments | `docs/contributing-branch-naming` |
| `refactor/` | Behavior-preserving cleanup | `refactor/extract-pty-manager` |
| `perf/` | Performance-only changes | `perf/board-card-render` |
| `test/` | Tests only, no production code | `test/rest-runner-collection` |

Keep the description short (≤ 5 words), lowercase, hyphens between words. One branch per logical change — don't pile unrelated work into a single PR.

## Commit Authorship

AI coding assistants are welcome here — many of us use them every day. But **every commit must be authored solely by a human contributor**, and you must not add AI co-author trailers.

**Do not add lines like:**

- `Co-Authored-By: Claude <noreply@anthropic.com>`
- `Co-Authored-By: Cursor`
- `Co-Authored-By: Codex`
- Any similar attribution for an AI agent or coding assistant

**Why this rule exists.** AI is a tool — the same category as an IDE, a linter, or autocomplete. We don't credit our text editor in commits, and the same applies to coding agents. A signed commit is a statement that **you read the diff, you understand the change, and you stand behind it.** When an AI co-author is named on a commit, that line blurs: responsibility splits between something that can't be held accountable and the person who actually pressed merge.

We want a human in the pilot seat. The agent is a copilot. If your name is on the commit, that's the assurance reviewers need that someone made deliberate engineering choices, read the output, and is willing to defend it in review. Use AI as much as it helps you ship — just own the result.

If your tooling adds the trailer automatically (Claude Code, Cursor, etc.), strip it before pushing. A quick `git commit --amend` will do.

## Submitting Changes

1. Create a branch from `main` following the [Branch Naming](#branch-naming) convention
2. Make focused, well-scoped changes
3. Ensure `bun run build` passes
4. Ensure `cargo check` passes in `src-tauri/`
5. Author your commits as a human (see [Commit Authorship](#commit-authorship))
6. Open a pull request using the PR template
7. Sign the CLA on your first PR (the CLA Assistant bot will prompt you)

## Code Style

- Frontend: JavaScript with Svelte 5 runes (`$state`, `$derived`, `$effect`)
- Backend: Rust with standard formatting (`cargo fmt`)
- Follow the existing patterns in the codebase — don't introduce new abstractions for a one-off change

## Reporting Issues

Open issues through the [issue chooser](https://github.com/ClaugeHQ/Clauge/issues/new/choose) — it will route you into the correct template (bug report, feature request, etc.). Blank issues are disabled on purpose: a structured report saves everyone time.

## License & CLA

This project is licensed under the [PolyForm Noncommercial License 1.0.0](../LICENSE). Contributions are governed by the [Contributor License Agreement](../CLA.md), which you accept by opening a pull request.

<p align="center">
  <img src="src-tauri/icons/clauge-mark.svg" alt="Clauge" width="96" />
</p>

<h1 align="center">Clauge</h1>

<p align="center">
  <strong>One window. Every dev tool.</strong>
</p>

<p align="center">
  Coding agents · workspace · REST · SQL · NoSQL · SSH · file explorer<br/>
  — every tool, one shell, an AI tuned to each.
</p>

<p align="center">
  <a href="https://github.com/ClaugeHQ/Clauge/releases/latest"><img src="https://img.shields.io/github/v/release/ClaugeHQ/Clauge?style=flat-square&color=ff5436&label=latest" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-PolyForm%20Noncommercial-7c5cf8?style=flat-square" alt="License"></a>
  <a href="https://github.com/ClaugeHQ/Clauge/stargazers"><img src="https://img.shields.io/github/stars/ClaugeHQ/Clauge?style=flat-square&color=f5a623" alt="Stars"></a>
  <img src="https://img.shields.io/badge/built%20with-Rust%20%2B%20Tauri-CE422B?style=flat-square" alt="Rust + Tauri">
  <img src="https://img.shields.io/badge/binary-~25%20MB-4f94d4?style=flat-square" alt="~20 MB">
  <img src="https://img.shields.io/badge/MCP-built--in-1dc880?style=flat-square" alt="MCP built-in">
  <a href="https://x.com/clauge_in"><img src="https://img.shields.io/badge/X%2FTwitter-white?logo=x&style=flat&logoColor=black"></a>
  

</p>

<p align="center">
  <a href="https://clauge.in">Website</a> ·
  <a href="https://clauge.in/changelog.html">Changelog</a> ·
  <a href="https://github.com/ClaugeHQ/Clauge/releases/latest"><strong>Download →</strong></a> ·
  <a href="https://github.com/ClaugeHQ/Clauge/issues">Report a bug</a>
</p>

<p align="center">
<b>Clauge has no associated cryptocurrency or token. Any tokens using our name are scams </b>
</p>

---

You've been flipping between multiple apps to do one job. Clauge runs all of them in one shell — coding agents, an API client, SQL and NoSQL editors, SSH sessions, a remote file browser, and a project workspace — and gives each mode its own AI, tuned for that workflow. Every mode is one keystroke away. Your data stays on your machine.

---

## Modes

| | Mode | What it does | Key capabilities |
|---|---|---|---|
| `01` | **Agent** | Run multiple coding agents in parallel — each with its own purpose, git worktree, and context. | Claude · Codex · Gemini · OpenCode · purpose-pinned sessions · Context Manager · Plugin Manager · per-session git identity · usage analytics |
| `02` | **Workspace** | Boards + notes your agents can read, write, and act on. | Kanban boards · markdown notes · AI coworkers · GitHub & GitLab issue import · full MCP integration |
| `03` | **REST** | An API client your AI — and any external agent — can drive. | Collections · environments · AI batch runner · MCP-exposed (agents create / read / update / delete collections and requests) |
| `04` | **SQL** | One client, every engine. | PostgreSQL · MySQL · ClickHouse · SQLite · Cloudflare D1 · schema-aware AI · cross-dialect translation · SSH tunnels shared with other modes |
| `05` | **NoSQL** | Document and key-value stores, side by side. | MongoDB · Redis · aggregation pipeline builder · interactive Redis console · engine-aware AI |
| `06` | **SSH** | A terminal with an AI co-pilot. | Profiles · multi-tab per host · port forwarding · keychain-backed creds · **two AI modes**: permission-gated and auto |
| `07` | **Explorer** | Every storage, one browser. | Local FS · S3 (and S3-compatibles) · Azure Blob · SFTP · FTP · drag-and-drop transfers · AI scan |
| `08` | **Atlas** | Drag, resize, and snap your open tabs into a free-form spatial workspace. | Pan / zoom canvas · per-workspace layouts · Atlas-spawned shells · hosts Agent, SSH, REST, SQL, NoSQL, and Explorer tabs |

Plus a cross-mode **History** layer — a queryable log of every session, request, query, and command across modes.

---

## Inside each mode

### Agent

Spawn coding agents in parallel — **Claude**, **Codex**, **Gemini**, **OpenCode** — without ever leaving the editor. Each session is independent: its own git worktree, its own context, its own model.

Per-session controls:

- **Purpose** — pin the session's intent up front (see table below)
- **Git identity** — commit as a coworker, not yourself
- **Skip permissions** — for fully-autonomous runs when you trust the prompt
- **Context injection** — pin files, folders, or MCP servers via the **Context Manager** (save sets, reuse across sessions)
- **Plugins** — load extra tools from the **Plugin Manager**

#### Purposes

Every Clauge session has a **purpose** that shapes the agent's focus from the first message. No more prompting it to "act like a code reviewer" mid-conversation.

| Purpose | What the agent focuses on |
|---|---|
| **Brainstorming** | Architecture, tradeoffs, multiple approaches — before writing a line |
| **Development** | Clean, tested, pattern-consistent code shipped in small verified steps |
| **Code Review** | Bugs, security holes, edge cases — with file and line references |
| **PR Review** | End-to-end pull-request analysis: what changed, what's good, what needs work |
| **Debugging** | Root cause, not band-aids — reproduce, trace, verify the fix actually works |
| **Custom** | Import an existing Claude Code session or define your own mode |

Every session emits **usage analytics** — tokens in / out, cost, cache hit, model mix — per session and aggregated across your day.

### Workspace

A shared layer your agents can read, write, and act on through MCP.

- **Kanban boards** — Backlog → Todo → In Progress → In Review → Done. *Review* is a safety gate, not a column you skip past.
- **Two-way Git integration** — pull open issues from **GitHub** and **GitLab** into a board, push finished cards back as pull requests.
- **Project linking** — point a workspace at a git repo and Clauge auto-spawns one board per subproject.
- **Notion-style notes** — a real WYSIWYG markdown editor, not a textarea. Per-project, linked to sessions, exposed through MCP.
- **AI coworkers** — named personas (Tech Lead, Brainstormer, Developer, Reviewer, QA) with their own prompts and providers. They read cards, comment, request changes, claim work, commit, and raise PRs.
- **Single-owner lock** — only one coworker (or one terminal session) owns a card at any moment. Switching owners, claiming from a terminal, and chatting in the card drawer all go through the same lock — no race conditions, no lost work.
- **Pinned Inbox** — mentions, review requests, and approval queues across every board, in one pane.
- **Cross-board search** — instant full-text search across every note and card.
- **Default-on MCP** — boards, cards, notes, and `@-mention a coworker` are all MCP tools an external agent can call. Auto-starts when the app launches — no flag, no setup.

### REST

An API client driven by AI and exposed over MCP.

- **MCP integration** — external agents can list, read, create, update, and delete collections and requests.
- **AI assistance** — describe an endpoint and the AI writes the request; ask *"run the smoke tests in staging"* and the AI fires the whole collection.
- **Execution reports** — pass / fail summary with the failing request inline.
- Collections, environments, request history, Postman v2 / v2.1 import.

### SQL

One editor, multiple engines, schema-aware AI.

- **Engines** — PostgreSQL · MySQL · ClickHouse · SQLite · Cloudflare D1
- **AI assistance** — natural language → schema-aware SQL, ready to run
- **Cross-dialect translation** — Postgres → MySQL → ClickHouse; AI rewrites the query for the target engine
- **SSH tunneling** shared with other modes — wire your bastion once, all modes use it

### NoSQL

Document stores and key-value engines, side by side.

- **MongoDB** — collections, JSON query editor, stage-by-stage aggregation pipeline builder with previews
- **Redis** — keys, TTL, streams, pub-sub, interactive console
- **Engine-aware AI** — never confuses a `find` with a `SCAN`

### SSH

Persistent SSH with two AI modes:

- **Permission mode** — every command the AI proposes is gated; you approve before it touches the wire.
- **Auto mode** — describe what you need; the AI executes and streams the output back.

Plus: reusable profiles, **multiple tabs against the same host** (no re-authenticating), port forwarding, ed25519 and agent forwarding, keychain-backed credentials.

### Explorer

Every backend, one browser, one set of shortcuts.

- **Backends** — Local FS · Amazon S3 (and S3-compatibles like R2, MinIO, Wasabi) · Azure Blob · SFTP · FTP
- **One-click presets** — pre-configured profiles for the common S3-compatible providers, so connecting takes seconds
- **Drag-and-drop transfers** — drop files into any backend, right-click "Download to…", with a live transfer panel showing progress and cancel
- **AI assistance** — *"what grew today?"*, *"find images larger than 5 MB"* — natural-language file ops
- **Inline preview** — text, JSON, CSV without a round-trip download

### Atlas

A free-form spatial workspace where every open tab becomes a draggable window. Pan, zoom, snap to neighbors, and stop alt-tabbing.

- **Universal tile** — Agent sessions, SSH terminals, REST requests, SQL editors, NoSQL queries, Explorer file browsers, and Atlas-spawned shells all coexist as resizable windows on one canvas.
- **State preserved across modes** — switching to Atlas and back doesn't reset anything; CodeMirror undo stacks, scroll positions, terminal scrollback, and connection state survive the round-trip.
- **In-tile control surfaces** — SQL tiles get a Run button + results table + connection picker; NoSQL tiles get a connection/collection picker; REST tiles get the full headers/auth/params/body editor + env picker + Send.
- **Snap guides + per-workspace layouts** — tiles snap to the edges of neighbors while dragging; tile positions are persisted per workspace, so each project has its own canvas memory.
- **Spawn shells anywhere** — a shell-spawn button drops a fresh terminal onto the canvas, scoped to that workspace.

---

## What makes Clauge different

**One window, not many.** Every developer's day is fragmented across a code editor, a REST client, a SQL GUI, a Mongo shell, a terminal, and a project board. Clauge collapses all of them into one shell with shared sessions, shared SSH tunnels, and shared AI context — switch modes with one keystroke.

**An AI per workflow, not a generic chat.** REST's AI understands your collections. SQL's AI knows your schema. SSH's AI refuses destructive commands without confirmation. The Workspace AI moves cards, leaves comments, and raises pull requests. Each one is tuned for what it sits next to — not a single chatbot bolted onto the side.

**Built-in MCP server, not a plug-in.** Clauge runs an MCP server out of the box, exposing 45+ tools across boards, cards, notes, REST collections, and coworker coordination. Claude Desktop, Cursor, Cline, Continue, or any MCP-speaking client can read, edit, and add to your workspace from the outside.

**Local-first by default.** Your sessions, your notes, your queries, your keys live on your disk. The desktop app is the source of truth. Sync is opt-in, per-feature.

**Native.** Rust + Tauri. ~20 MB binary. Sub-second cold start. No Electron tax.

---

## MCP

Clauge ships an MCP server with **45+ tools** — `boards_*`, `cards_*`, `notes_*`, `rest_collection_*`, `coworkers_*`, `workspace_*`, `activity_feed`, and `cards_call_coworker` (so the agent in your terminal can `@-mention` a coworker on a card without leaving the shell). The server **auto-starts on launch** — no flag, no extra setup. Any MCP-speaking client can drive your workspace.

### From Claude Desktop

Add to your `claude_desktop_config.json`:

```jsonc
{
  "mcpServers": {
    "clauge": {
      "command": "clauge",
      "args": ["mcp", "serve", "--stdio"]
    }
  }
}
```

### From Cursor / Cline / Continue

Point your MCP client at the local Clauge process (`stdio` or `http://localhost:7421/mcp`). The agent can now list boards, create cards, append notes, raise PRs, search REST collections, and coordinate with the named coworkers you defined inside the app — all without leaving its own UI.

---

## AI assistance — bring your own key, or use Clauge AI

Every mode's AI runs on the provider you choose.

| Option | How it works |
|---|---|
| **BYOK** | Drop in your own Anthropic, OpenAI, Google, or OpenCode key — Clauge talks directly to the provider. No middleman. |
| **Clauge AI credits** | Subscribe and use Clauge-managed credits across every mode and every provider — no separate billing. |

Set keys in **Settings → AI Providers**. Per-mode model choice; per-session model override.

## Cloud sync

Opt-in, encrypted.Agent Context, Co-Worker Profil, REST collections,SQL/NoSQL and SSH profiles sync across machines — or stay local, your call. Sessions and credentials never leave the device unless you explicitly enable sync for them.

---

## Mobile

Drive your desktop from your phone. The **Clauge** companion app attaches to any running **Agent** (Claude · Codex · Gemini · OpenCode) or **SSH** session — watch it live, send prompts, and approve actions — over your local network or Tailscale.

- **Android** — [download](https://github.com/ClaugeHQ/clauge-android/releases/latest) · [source](https://github.com/ClaugeHQ/clauge-android)
- **iOS** — [download](https://github.com/ClaugeHQ/clauge-ios/releases/latest) · [source](https://github.com/ClaugeHQ/clauge-ios)

---

## Built with

- **Frontend** — SvelteKit + Svelte 5
- **Native shell** — Tauri v2 (Rust)
- **Persistence** — SQLite (local), optional encrypted cloud sync
- **Terminals** — `xterm.js` + cross-platform PTY
- **MCP** — built-in JSON-RPC server, stdio and HTTP transports

---

## Star History

<a href="https://www.star-history.com/?repos=ansxuman%2FClauge&type=date&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=ClaugeHQ/Clauge&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=ClaugeHQ/Clauge&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=ClaugeHQ/Clauge&type=date&legend=top-left" />
 </picture>
</a>

---

## License

This project is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE) .

Contributions require signing a Contributor License Agreement (CLA). See [Contributor License Agreement](CLA.md) for details.

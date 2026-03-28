<p align="center">
  <img src="src-tauri/icons/clauge-mark.svg" alt="Clauge" width="120" />
</p>

<h1 align="center">Clauge</h1>

<p align="center">
  Run multiple Claude Code sessions in parallel — organized by project, each with its own purpose and terminal.
</p>

<p align="center">
  <a href="https://github.com/ansxuman/Clauge/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-7c5cf8?style=flat-square" alt="License"></a>
  <a href="https://github.com/ansxuman/Clauge/stargazers"><img src="https://img.shields.io/github/stars/ansxuman/Clauge?style=flat-square&color=f5a623" alt="Stars"></a>
  <a href="https://github.com/ansxuman/Clauge/issues"><img src="https://img.shields.io/github/issues/ansxuman/Clauge?style=flat-square&color=4f94d4" alt="Issues"></a>
  <a href="https://github.com/ansxuman/Clauge/releases/latest"><img src="https://img.shields.io/github/v/release/ansxuman/Clauge?style=flat-square&color=1dc880" alt="Release"></a>
</p>

<p align="center">
  <a href="https://github.com/ansxuman/Clauge/issues">Report Bug</a> ·
  <a href="https://github.com/ansxuman/Clauge/issues">Request Feature</a> ·
  <a href="https://buymeacoffee.com/ansxuman">Buy me a coffee</a>
</p>

---

## Session Purposes

Each session stays focused on what you need right now. The purpose persists throughout the entire conversation.

| Purpose | What you get |
|---------|-------------|
| **Brainstorming** | Think through architecture before writing a line of code. Get multiple approaches with tradeoffs so you pick the right one. |
| **Development** | Ship features with clean, tested code that follows your existing patterns. Small changes, verified one at a time. |
| **Code Review** | Catch bugs, security holes, and edge cases before they reach production. Specific feedback with file and line references. |
| **PR Review** | Review pull requests end-to-end before merging. Clear summary of what changed, what's good, and what needs work. |
| **Debugging** | Find the root cause, not a band-aid. Reproduces the issue, traces it methodically, and verifies the fix actually works. |

## Features

**Parallel sessions, zero conflicts** — Run multiple sessions on the same project. Each one is automatically isolated so they don't overwrite each other's work.

**Embedded terminal** — Full interactive terminal built in. Colors, scrollback, resize. Switch between sessions instantly without re-spawning Claude.

**~10MB, low resource usage** — Built with Rust and Tauri. No Electron, no bundled Chromium. Starts fast, stays light.

**Organized by project** — Sessions grouped by project folder with expand/collapse. Auto-discovers your existing Claude Code sessions.

**Usage tracking** — Session and weekly usage limits visible in the menu bar. Know how much headroom you have without leaving the app.

**Themes and shortcuts** — Dark/light themes, accent colors. `Cmd+N` new session, `Cmd+1-9` switch, `Cmd+B` toggle sidebar.

## Download

<a href="https://github.com/ansxuman/Clauge/releases/latest"><strong>Download for macOS →</strong></a>

## Development

**Requires:** [Bun](https://bun.sh), [Rust](https://rustup.rs) 1.77+, [Tauri CLI](https://tauri.app) v2

```bash
git clone https://github.com/ansxuman/Clauge.git
cd Clauge
bun install
bun run tauri dev
```

## Tech Stack

| | |
|---|---|
| **Frontend** | SvelteKit, Svelte 5 |
| **Backend** | Rust, Tauri v2 |
| **Terminal** | xterm.js, portable-pty |

## Contributing

See [CONTRIBUTING.md](.github/CONTRIBUTING.md).

## Support

<a href="https://www.buymeacoffee.com/ansxuman" target="_blank"><img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy Me A Coffee" height="40"></a>

## License

[Apache License 2.0](LICENSE)

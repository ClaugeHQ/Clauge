<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import '@xterm/xterm/css/xterm.css';
  import { Channel } from '@tauri-apps/api/core';
  import {
    activeSshProfile,
    sshProfiles,
    sshTerminalIds,
    sshConnStates,
    sshTerminalMap,
    loadSshProfiles,
  } from '../stores';
  import {
    sshSpawnTerminal,
    sshWriteToTerminal,
    sshResizeTerminal,
    sshKillTerminal,
    sshTouchProfile,
  } from '../commands';
  import { phoneOwnedTerminals, phoneDrivenSizes } from '$lib/stores/sizeOwner';
  import { tabs as tabsStore, activeTabId, addTab, activateTab, closeTab } from '$lib/shared/stores/tabs';
  import { newSshTabKey, profileIdFromTabKey } from '../tabkey';
  import { getTerminalTheme } from '$lib/utils/theme';
  import { appearance } from '$lib/stores/settings';
  import { mode } from '$lib/stores/app';
  import { showToast } from '$lib/shared/primitives/toast';
  import { base64ToBytes, deferUntilFrame, loadWebGLAddon } from '$lib/shared/primitives/terminal-utils';
  import { resolveSshCapture, rejectAllSshCaptures, type SshCaptureRequest } from '../ai/execute';
  import type { SshProfile, SshTerminalPayload } from '../types';
  import { SSH_EVENT } from '$lib/shared/constants/events';
  import { RESIZE_DEBOUNCE_MS, SSH_CAPTURE_TIMEOUT_MS } from '$lib/shared/constants/timings';

  let terminalEl: HTMLDivElement;

  // Reparent guard: if the mode becomes 'ssh' and the active tab's xterm
  // container has been moved elsewhere (e.g., into a Canvas tile), pull it
  // back into terminalEl. Without this, switching Canvas → SSH leaves the
  // panel blank because nothing returns the DOM to terminalEl.
  $effect(() => {
    if ($mode !== 'ssh') return;
    const tabKey = currentTabKey;
    if (!tabKey || !terminalEl) return;
    const entry = $sshTerminalMap.get(tabKey);
    if (!entry?.container) return;
    if (entry.container.parentElement !== terminalEl) {
      terminalEl.appendChild(entry.container);
      try {
        entry.fitAddon?.fit();
      } catch {
        // Layout not ready yet; ResizeObserver will fit shortly.
      }
    }
  });

  // Per-tab xterm entry. Keyed by tab.key (== profile.id-based key).
  type TermEntry = {
    term: Terminal;
    fitAddon: FitAddon;
    searchAddon: SearchAddon;
    container: HTMLDivElement;
    terminalId: string | null;
    profileId: string;
    tabKey: string;
    generation: number;
    // Active capture for execute_shell tool. Null when no AI command pending.
    capture: {
      requestId: string;
      buffer: string;
      timeoutId: ReturnType<typeof setTimeout>;
    } | null;
  };

  // Heuristic shell prompt detector: matches `$ `, `# `, `> `, `% `, `❯ ` at the
  // end of the cleaned (ANSI-stripped) buffer. Imperfect — some PS1 setups omit
  // the trailing space — but the 15s timeout is the hard backstop.
  const SHELL_PROMPT_RE = /[\$#>❯%]\s*$/m;
  const ANSI_RE = /\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07]*(?:\x07|\x1b\\)/g;
  const CAPTURE_TIMEOUT_MS = SSH_CAPTURE_TIMEOUT_MS;
  const CAPTURE_MAX_CHARS = 100_000;

  function stripAnsi(text: string): string {
    return text.replace(ANSI_RE, '');
  }

  function finishCapture(entry: TermEntry, reason: 'prompt' | 'timeout' | 'cleanup' = 'prompt') {
    const cap = entry.capture;
    if (!cap) return;
    clearTimeout(cap.timeoutId);
    entry.capture = null;
    const cleaned = stripAnsi(cap.buffer).trim();
    const note = reason === 'timeout' ? '\n[NOTE] Capture timed out after 15s; output may be incomplete.' : '';
    resolveSshCapture(cap.requestId, cleaned + note);
  }

  // Per-tab xterm entries lifted into sshTerminalMap store for Canvas reparenting.
  // Local helpers shadow plain-Map calls with copy-on-mutate store operations.
  let activeEntry: TermEntry | null = null;

  // Track which tabs have an exited terminal (for reconnect banner).
  let exitedTabs = $state<Set<string>>(new Set());

  // Loading state — gated by first-data-received flag for current spawn.
  let spawning = $state(false);
  let termReady = $state(false);

  // Terminal background color (synced with theme).
  let termBg = $state('#0d0d18');

  // Last-active tab.key tracked here so the activeAgent-style subscriber
  // knows whether re-entry is a real switch or noop. Must be `$state`
  // because `activeIsExited` derives off it — non-reactive assignments
  // wouldn't drive the reconnect banner / term-hidden CSS toggle.
  let currentTabKey = $state<string | null>(null);

  // Find-in-terminal bar state
  let findOpen          = $state(false);
  let findQuery         = $state('');
  let findNoMatch       = $state(false);
  let findResultIndex   = $state(-1);
  let findResultCount   = $state(0);
  let findCaseSensitive = $state(false);
  let findRegex         = $state(false);
  let findWholeWord     = $state(false);
  let findInputEl: HTMLInputElement;

  const FIND_DECORATIONS = {
    matchBackground:               '#1c3d6b',
    matchBorder:                   '#2a5a9e',
    matchOverviewRuler:            '#4488cc',
    activeMatchBackground:         '#7a3d00',
    activeMatchBorder:             '#e07000',
    activeMatchColorOverviewRuler: '#ff8c00',
  };

  function searchOpts() {
    return {
      regex:         findRegex,
      caseSensitive: findCaseSensitive,
      wholeWord:     findWholeWord,
      decorations:   FIND_DECORATIONS,
    };
  }

  function openFind() {
    findOpen = true;
    findNoMatch = false;
    findResultIndex = -1;
    findResultCount = 0;
    requestAnimationFrame(() => findInputEl?.focus());
  }

  function closeFind() {
    findOpen = false;
    findQuery = '';
    findNoMatch = false;
    findResultIndex = -1;
    findResultCount = 0;
    try { activeEntry?.searchAddon.clearDecorations(); } catch { /* ignore */ }
    requestAnimationFrame(() => { try { activeEntry?.term.focus(); } catch { /* ignore */ } });
  }

  function doFindNext() {
    if (!activeEntry || !findQuery) return;
    try {
      const found = activeEntry.searchAddon.findNext(findQuery, searchOpts());
      findNoMatch = !found;
    } catch { findNoMatch = true; }
  }

  function doFindPrev() {
    if (!activeEntry || !findQuery) return;
    try {
      const found = activeEntry.searchAddon.findPrevious(findQuery, searchOpts());
      findNoMatch = !found;
    } catch { findNoMatch = true; }
  }

  // Read query from DOM directly so we're never racing bind:value's update.
  function onFindInput(e: Event) {
    const query = (e.currentTarget as HTMLInputElement).value;
    findQuery = query;
    findNoMatch = false;
    if (!activeEntry) return;
    if (!query) {
      findResultIndex = -1;
      findResultCount = 0;
      try { activeEntry.searchAddon.clearDecorations(); } catch { /* ignore */ }
      return;
    }
    try {
      const found = activeEntry.searchAddon.findNext(query, searchOpts());
      findNoMatch = !found;
    } catch { findNoMatch = true; }
  }

  function reRunSearch() {
    if (!activeEntry || !findQuery) return;
    try {
      const found = activeEntry.searchAddon.findNext(findQuery, searchOpts());
      findNoMatch = !found;
    } catch { findNoMatch = true; }
  }

  function toggleCase()  { findCaseSensitive = !findCaseSensitive; reRunSearch(); }
  function toggleRegex() { findRegex         = !findRegex;         reRunSearch(); }
  function toggleWord()  { findWholeWord     = !findWholeWord;     reRunSearch(); }

  function onFindKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (e.shiftKey) doFindPrev(); else doFindNext();
    } else if (e.key === 'Escape') {
      closeFind();
    }
  }

  // Per-tab generation: invalidates stale Channel writes after reconnect.
  const generations = new Map<string, number>();

  function getCurrentTermTheme(): Record<string, string> {
    const app = get(appearance);
    return getTerminalTheme(app.theme, app.accentColor);
  }

  function createEntry(tabKey: string, profile: SshProfile): TermEntry {
    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", "SF Mono", "Menlo", monospace',
      theme: getCurrentTermTheme(),
      allowTransparency: true,
      scrollback: 10000,
      lineHeight: 1.35,
      smoothScrollDuration: 100,
      rescaleOverlappingGlyphs: true,
      cursorStyle: 'bar',
      cursorInactiveStyle: 'outline',
      rightClickSelectsWord: true,
    });
    const fitAddon = new FitAddon();
    const searchAddon = new SearchAddon();
    term.loadAddon(fitAddon);
    term.loadAddon(searchAddon);

    const container = document.createElement('div');
    container.style.cssText = 'width:100%;height:100%;display:none;';
    terminalEl.appendChild(container);
    term.open(container);
    loadWebGLAddon(term);

    // Intercept Cmd/Ctrl+F before xterm handles it, and Escape to close the bar.
    term.attachCustomKeyEventHandler((e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'f' && e.type === 'keydown') {
        openFind();
        return false;
      }
      if (e.key === 'Escape' && e.type === 'keydown' && findOpen) {
        closeFind();
        return false;
      }
      return true;
    });

    const entry: TermEntry = {
      term,
      fitAddon,
      searchAddon,
      container,
      terminalId: null,
      profileId: profile.id,
      tabKey,
      generation: 0,
      capture: null,
    };

    searchAddon.onDidChangeResults(({ resultIndex, resultCount }) => {
      if (activeEntry?.tabKey !== entry.tabKey) return;
      findResultIndex = resultIndex;
      findResultCount = resultCount;
    });

    term.onData((data) => {
      const id = entry.terminalId;
      if (!id) return;
      sshWriteToTerminal(id, data).catch(() => {
        // PTY/channel dead — mark exited and surface reconnect banner
        markExited(tabKey);
      });
    });

    let resizeTimer: ReturnType<typeof setTimeout> | null = null;
    new ResizeObserver(() => {
      if (!fitAddon || container.offsetWidth <= 0) return;
      if (resizeTimer) clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        resizeTimer = null;
        try {
          // Phone-owned: the phone drives this terminal's size. Adopt it
          // (handled by the adopt $effect) and skip the local fit + PTY-resize
          // drive so the desktop doesn't fight the phone (no resize war).
          if (entry.terminalId && get(phoneDrivenSizes).has(entry.terminalId)) return;
          fitAddon.fit();
          if (entry.terminalId) {
            const dims = fitAddon.proposeDimensions();
            if (dims) sshResizeTerminal(entry.terminalId, dims.cols, dims.rows).catch(() => {});
          }
        } catch {
          /* ignore */
        }
      }, RESIZE_DEBOUNCE_MS);
    }).observe(container);

    sshTerminalMap.update((m) => { const next = new Map(m); next.set(tabKey, entry); return next; });
    return entry;
  }

  function showEntry(entry: TermEntry) {
    if (activeEntry && activeEntry !== entry) {
      activeEntry.container.style.display = 'none';
      try { activeEntry.term.options.scrollback = 1000; } catch { /* ignore */ }
      // Clear search decorations on the outgoing terminal
      if (findOpen) try { activeEntry.searchAddon.findNext(''); } catch { /* ignore */ }
    }
    entry.container.style.display = 'block';
    try { entry.term.options.scrollback = 10000; } catch { /* ignore */ }
    activeEntry = entry;
    requestAnimationFrame(() => {
      try { entry.fitAddon.fit(); } catch { /* ignore */ }
      if (findOpen) {
        findInputEl?.focus();
        if (findQuery) {
          findResultIndex = -1;
          findResultCount = 0;
          try {
            const found = entry.searchAddon.findNext(findQuery, searchOpts());
            findNoMatch = !found;
          } catch { findNoMatch = true; }
        }
      } else {
        try { entry.term.focus(); } catch { /* ignore */ }
      }
    });
  }

  function markExited(tabKey: string) {
    const entry = get(sshTerminalMap).get(tabKey);
    if (entry) entry.terminalId = null;
    sshTerminalIds.update((m) => {
      m.delete(tabKey);
      return new Map(m);
    });
    sshConnStates.update((m) => {
      m.set(tabKey, 'disconnected');
      return new Map(m);
    });
    exitedTabs = new Set([...exitedTabs, tabKey]);
  }

  async function spawnFor(entry: TermEntry, profile: SshProfile) {
    // Bump generation so older stale Channel callbacks no-op.
    const gen = (generations.get(entry.tabKey) ?? 0) + 1;
    generations.set(entry.tabKey, gen);
    entry.generation = gen;

    spawning = true;
    termReady = false;
    sshConnStates.update((m) => {
      m.set(entry.tabKey, 'connecting');
      return new Map(m);
    });
    exitedTabs = new Set([...exitedTabs].filter((k) => k !== entry.tabKey));

    let firstDataSeen = false;
    const channel = new Channel<SshTerminalPayload>();
    channel.onmessage = (payload) => {
      if (entry.generation !== gen) return; // stale spawn
      if (payload.exit === true) {
        // If exit arrives DURING spawn (e.g. auth ok but shell died instantly,
        // or the server closed the channel right after request_shell), the
        // loader would otherwise stay forever — flip flags so only the banner
        // is shown, not loader-and-banner together.
        if (spawning) {
          spawning = false;
          termReady = false;
        }
        markExited(entry.tabKey);
        try {
          entry.term.write('\r\n\x1b[33m[connection closed]\x1b[0m\r\n');
        } catch { /* ignore */ }
        return;
      }
      if (!firstDataSeen && payload.data) {
        firstDataSeen = true;
        // 'connected' is gated on first actual byte from the server, NOT on
        // ssh_spawn_terminal resolving. The Rust command returns the
        // terminal_id instantly after pre-minting a UUID and spawning the
        // run_ssh_session task in the background — by the time the await
        // resolves, the SSH handshake hasn't even started. Writing 'connected'
        // at that point paints a green dot for ~1.7s while auth is still in
        // flight; on auth failure the dot then snaps to disconnected. Tying
        // it to firstDataSeen instead means the nav dot only goes green when
        // we have evidence the server actually accepted us.
        sshConnStates.update((m) => {
          m.set(entry.tabKey, 'connected');
          return new Map(m);
        });
        deferUntilFrame(() => {
          if (spawning) {
            spawning = false;
            termReady = true;
          }
        });
      }
      if (payload.data) {
        try {
          const bytes = base64ToBytes(payload.data);
          entry.term.write(bytes);
          // Capture buffer for execute_shell tool: append decoded text and
          // check for shell prompt at end (heuristic stop).
          if (entry.capture) {
            const text = new TextDecoder().decode(bytes);
            entry.capture.buffer += text;
            if (entry.capture.buffer.length > CAPTURE_MAX_CHARS) {
              entry.capture.buffer = entry.capture.buffer.slice(-CAPTURE_MAX_CHARS);
            }
            const cleaned = stripAnsi(entry.capture.buffer);
            if (SHELL_PROMPT_RE.test(cleaned)) {
              finishCapture(entry, 'prompt');
            }
          }
        } catch { /* ignore decode errors */ }
      }
    };

    try {
      const terminalId = await sshSpawnTerminal(profile.id, channel);
      // Stale-spawn guard: cancelConnect bumps the generation while we're
      // awaiting. If the user cancelled before Rust finished connecting,
      // drop the terminal we just got — claiming it would resurrect a
      // 'connected' state in the nav after the user already cancelled.
      if (entry.generation !== gen) {
        sshKillTerminal(terminalId).catch(() => {});
        return;
      }
      entry.terminalId = terminalId;
      sshTerminalIds.update((m) => {
        m.set(entry.tabKey, terminalId);
        return new Map(m);
      });
      // sshConnStates is NOT set to 'connected' here. ssh_spawn_terminal
      // resolves before the SSH handshake even begins (it just pre-mints a
      // UUID and spawns a background task). 'connected' is now set inside
      // the channel.onmessage firstDataSeen branch above, when the server
      // actually sends bytes — the only reliable signal that auth + PTY
      // succeeded. Until then state stays 'connecting' (yellow pulse).
      // Rust just bumped last_used_at as part of spawn — refresh the store so
      // the SshNav list reflects the new "Xs ago" instead of "never".
      loadSshProfiles().catch(() => {});

      // Send initial fit — unless the phone already drives this terminal's
      // size, in which case the adopt $effect owns it.
      requestAnimationFrame(() => {
        if (get(phoneDrivenSizes).has(terminalId)) return;
        try {
          entry.fitAddon.fit();
          const dims = entry.fitAddon.proposeDimensions();
          if (dims) sshResizeTerminal(terminalId, dims.cols, dims.rows).catch(() => {});
        } catch { /* ignore */ }
      });
    } catch (e) {
      spawning = false;
      termReady = false;
      const msg = String(e);
      // Distinguish timeout from other failures so the user gets clear feedback.
      const isTimeout = msg.toLowerCase().includes('timed out') || msg.toLowerCase().includes('timeout');
      // Clean up the local entry — connection never came up, no PTY to keep around.
      try { entry.container.remove(); } catch { /* ignore */ }
      try { entry.term.dispose(); } catch { /* ignore */ }
      sshTerminalMap.update((m) => { const next = new Map(m); next.delete(entry.tabKey); return next; });
      sshConnStates.update((m) => { m.set(entry.tabKey, 'disconnected'); return new Map(m); });
      // Close the tab and return to home so the user can pick a profile and retry.
      const allTabs = get(tabsStore);
      const tab = allTabs.find((t) => t.mode === 'ssh' && t.key === entry.tabKey);
      if (tab) closeTab(tab.id);
      activeSshProfile.set(null);
      showToast(isTimeout ? `Connection to ${profile.host} timed out` : `Failed to connect: ${msg}`, 'error');
    }
  }

  /**
   * Bring a tab into view. Two call shapes:
   *
   *  - `activateProfile(profile)` — used when the user picks a profile from
   *    the nav. Reuses the most recent active tab for that profile if one
   *    exists (existing UX). Creates a new tab + spawn if none.
   *
   *  - `activateProfile(profile, { tabKey })` — used when a specific tab is
   *    activated (tab click, duplicate-session, OPEN_TAB with explicit key).
   *    Finds or recreates the entry for that exact tabKey.
   */
  async function activateProfile(profile: SshProfile, opts: { tabKey?: string } = {}) {
    if (!terminalEl) return;

    let tabKey = opts.tabKey ?? null;
    if (!tabKey) {
      // No explicit key — find the most recent existing tab for this profile.
      const all = get(tabsStore);
      const matching = all.filter((t) => t.mode === 'ssh' && t.key && profileIdFromTabKey(t.key) === profile.id);
      if (matching.length > 0) {
        const last = matching[matching.length - 1];
        tabKey = last.key as string;
      } else {
        tabKey = newSshTabKey(profile.id);
      }
    }

    // Re-attach existing entry if still alive
    let entry = get(sshTerminalMap).get(tabKey);
    if (entry && entry.terminalId) {
      if (entry.container.parentElement !== terminalEl) {
        terminalEl.appendChild(entry.container);
      }
      currentTabKey = tabKey;
      spawning = false;
      termReady = true;
      showEntry(entry);
      return;
    }

    if (entry) {
      // Stale — recreate xterm to avoid showing prior buffer for a new connection
      try { entry.container.remove(); } catch { /* ignore */ }
      try { entry.term.dispose(); } catch { /* ignore */ }
      sshTerminalMap.update((m) => { const next = new Map(m); next.delete(tabKey); return next; });
    }

    currentTabKey = tabKey;
    entry = createEntry(tabKey, profile);
    showEntry(entry);
    await spawnFor(entry, profile);
  }

  // Cancel an in-flight connect attempt. We don't have a Rust-side abort
  // handle (russh's connect future isn't easily abortable from outside), so
  // the strategy is: kill the terminal id if it was issued, otherwise just
  // tear down the local tab so the user is unblocked. Backend timeout (15s)
  // is the hard floor.
  //
  // Tab keys are `<profileId>#<timestamp>-<counter>`, NOT `profile.id`. Use
  // `currentTabKey` to match what sshTerminalMap / sshTerminalIds / sshConnStates
  // are actually keyed by, otherwise this whole function operates on a key
  // nobody else has and the in-flight spawn happily completes and writes
  // 'connected' to the real key.
  async function cancelConnect() {
    if (!currentTabKey) return;
    const tabKey = currentTabKey;
    const entry = get(sshTerminalMap).get(tabKey);

    // Bump generation BEFORE anything else so the in-flight spawnFor's
    // post-await success block sees a stale gen and skips writing
    // 'connected' / claiming the terminal.
    const nextGen = (generations.get(tabKey) ?? 0) + 1;
    generations.set(tabKey, nextGen);
    if (entry) entry.generation = nextGen;

    if (entry?.terminalId) {
      sshKillTerminal(entry.terminalId).catch(() => {});
    }
    if (entry) {
      try { entry.container.remove(); } catch { /* ignore */ }
      try { entry.term.dispose(); } catch { /* ignore */ }
      sshTerminalMap.update((m) => { const next = new Map(m); next.delete(tabKey); return next; });
    }
    spawning = false;
    termReady = false;
    // Delete (don't just write 'disconnected') so SshNav's connected check
    // can't see a stale 'connected' value if a late spawn-success callback
    // somehow slips through.
    sshTerminalIds.update((m) => { m.delete(tabKey); return new Map(m); });
    sshConnStates.update((m) => { m.delete(tabKey); return new Map(m); });
    // Close the tab and unset active profile — return user to home screen.
    const allTabs = get(tabsStore);
    const tab = allTabs.find((t) => t.mode === 'ssh' && t.key === tabKey);
    if (tab) closeTab(tab.id);
    currentTabKey = null;
    activeSshProfile.set(null);
    showToast('Connection cancelled', 'info');
  }

  async function reconnectActive() {
    const profile = get(activeSshProfile);
    if (!profile) return;
    const tabKey = profile.id;
    const entry = get(sshTerminalMap).get(tabKey);
    if (!entry) {
      activateProfile(profile);
      return;
    }
    // Kill any lingering remote terminal first
    if (entry.terminalId) {
      sshKillTerminal(entry.terminalId).catch(() => {});
      entry.terminalId = null;
    }
    try { entry.term.clear(); } catch { /* ignore */ }
    await spawnFor(entry, profile);
  }

  // ── Event listeners ─────────────────────────────────────────────────────────

  function handleOpenTab(e: Event) {
    const profile = (e as CustomEvent<SshProfile>).detail;
    if (!profile) return;

    const all = get(tabsStore);
    const existing = all.find((t) => t.mode === 'ssh' && t.key && profileIdFromTabKey(t.key) === profile.id);
    if (existing) {
      activateTab(existing.id);
    } else {
      addTab(profile.name, 'ssh', newSshTabKey(profile.id), 'var(--ssh)');
    }
    activeSshProfile.set(profile);
    // Bump last_used_at on every open path (NewProfileModal save, Topbar +
    // picker, SshNav click) so "last used" reflects reality everywhere.
    sshTouchProfile(profile.id)
      .then(() => loadSshProfiles())
      .catch(() => {});
  }

  /** Always-create flow: a new tab + new session for the same profile. */
  function handleDuplicateSession(e: Event) {
    const profile = (e as CustomEvent<SshProfile>).detail;
    if (!profile) return;
    // Compute next "session number" for the label so the user can tell tabs
    // apart at a glance. Counts existing tabs for this profile + 1.
    const all = get(tabsStore);
    const existing = all.filter((t) => t.mode === 'ssh' && t.key && profileIdFromTabKey(t.key) === profile.id);
    const n = existing.length + 1;
    const label = `${profile.name} #${n}`;
    addTab(label, 'ssh', newSshTabKey(profile.id), 'var(--ssh)');
    activeSshProfile.set(profile);
    sshTouchProfile(profile.id).then(() => loadSshProfiles()).catch(() => {});
  }

  function handleCloseTab(e: Event) {
    const detail = (e as CustomEvent).detail;
    const tabKey = detail?.tabKey as string | undefined;
    if (!tabKey) return;

    // Bump generation so any in-flight Channel callbacks for this tabKey
    // (mid-connection writes/exit) no-op against a removed entry.
    generations.set(tabKey, (generations.get(tabKey) ?? 0) + 1);

    const wasActive = activeEntry?.tabKey === tabKey
      || get(activeSshProfile)?.id === tabKey;

    const entry = get(sshTerminalMap).get(tabKey);
    if (entry) {
      if (entry.terminalId) {
        sshKillTerminal(entry.terminalId).catch(() => {});
      }
      try { entry.container.remove(); } catch { /* ignore */ }
      try { entry.term.dispose(); } catch { /* ignore */ }
      sshTerminalMap.update((m) => { const next = new Map(m); next.delete(tabKey); return next; });
    }
    sshTerminalIds.update((m) => {
      m.delete(tabKey);
      return new Map(m);
    });
    sshConnStates.update((m) => {
      m.delete(tabKey);
      return new Map(m);
    });
    exitedTabs = new Set([...exitedTabs].filter((k) => k !== tabKey));

    if (wasActive) {
      activeEntry = null;
      currentTabKey = null;
      // Reset loader/banner flags so whatever Topbar activates next (or the
      // empty home screen) renders cleanly. Topbar owns the active-profile
      // switch — we don't double-set it here to avoid racing with Topbar's
      // own next-tab selection.
      spawning = false;
      termReady = false;
    }
  }

  function handleInsertCommand(e: Event) {
    const cmd = (e as CustomEvent<string>).detail;
    if (!cmd || typeof cmd !== 'string') return;
    if (!activeEntry?.terminalId) {
      showToast('No active SSH terminal', 'info');
      return;
    }
    // Insert command at cursor without trailing newline. User presses Enter
    // themselves. Used as a fallback when AI sends a code block instead of
    // calling execute_shell (e.g. for interactive commands the system prompt
    // says shouldn't be auto-run).
    sshWriteToTerminal(activeEntry.terminalId, cmd).catch(() => {
      showToast('Failed to write to terminal', 'error');
    });
    try {
      activeEntry.term.focus();
    } catch { /* ignore */ }
  }

  // ── Reactive subscriptions ──────────────────────────────────────────────────

  // Activate the right TermEntry whenever the tabs store's activeTabId
  // points at an SSH tab. This handles:
  //   - Initial mount (existing SSH tab is active)
  //   - User clicks an SSH tab in the topbar
  //   - DUPLICATE_SESSION creates a new tab (addTab also activates it)
  // Multiple tabs for the same profile each have a distinct tabKey, so we
  // key off the tab's `key` field rather than the profile id.
  const unsubActiveTab = activeTabId.subscribe((tabId) => {
    if (tabId < 0) return;
    const all = get(tabsStore);
    const tab = all.find((t) => t.id === tabId);
    if (!tab || tab.mode !== 'ssh' || !tab.key) return;
    if (tab.key === currentTabKey) return;
    const profileId = profileIdFromTabKey(tab.key);
    const profile = get(sshProfiles).find((p) => p.id === profileId);
    if (!profile) return;
    activeSshProfile.set(profile);
    requestAnimationFrame(() => activateProfile(profile, { tabKey: tab.key as string }));
  });

  // Hide the panel when there's no active SSH profile (e.g. user closed the
  // last SSH tab). The activeTabId subscriber handles the "switch in" case.
  const unsubProfile = activeSshProfile.subscribe((profile) => {
    if (!profile) {
      currentTabKey = null;
      if (activeEntry) {
        activeEntry.container.style.display = 'none';
        activeEntry = null;
      }
    }
  });

  const unsubAppearance = appearance.subscribe((app) => {
    if (!app) return;
    const theme = getTerminalTheme(app.theme, app.accentColor);
    termBg = theme.background || '#0d0d18';
    for (const entry of get(sshTerminalMap).values()) {
      try { entry.term.options.theme = theme; } catch { /* ignore */ }
    }
  });

  function handleExecuteCaptureRequest(e: Event) {
    const detail = (e as CustomEvent<SshCaptureRequest>).detail;
    if (!detail) return;
    const { requestId, profileId, command } = detail;
    // Find the entry whose profile matches. Prefer the active entry.
    let target: TermEntry | null = null;
    if (activeEntry && activeEntry.profileId === profileId && activeEntry.terminalId) {
      target = activeEntry;
    } else {
      for (const entry of get(sshTerminalMap).values()) {
        if (entry.profileId === profileId && entry.terminalId) { target = entry; break; }
      }
    }
    if (!target || !target.terminalId) {
      resolveSshCapture(requestId, '[ERROR] No live SSH terminal for the requested profile.');
      return;
    }
    // Already capturing for another tool call — reject the older one.
    if (target.capture) {
      finishCapture(target, 'cleanup');
    }
    target.capture = {
      requestId,
      buffer: '',
      timeoutId: setTimeout(() => {
        const e2 = target!;
        if (e2.capture && e2.capture.requestId === requestId) {
          finishCapture(e2, 'timeout');
        }
      }, CAPTURE_TIMEOUT_MS),
    };
    // Write the command followed by Enter.
    sshWriteToTerminal(target.terminalId, command + '\r').catch(() => {
      finishCapture(target!, 'cleanup');
      resolveSshCapture(requestId, '[ERROR] Failed to write command to SSH terminal.');
    });
  }

  onMount(async () => {
    window.addEventListener(SSH_EVENT.OPEN_TAB, handleOpenTab);
    window.addEventListener(SSH_EVENT.CLOSE_TAB, handleCloseTab);
    window.addEventListener(SSH_EVENT.DUPLICATE_SESSION, handleDuplicateSession);
    window.addEventListener(SSH_EVENT.INSERT_COMMAND, handleInsertCommand);
    window.addEventListener(SSH_EVENT.EXECUTE_CAPTURE_REQUEST, handleExecuteCaptureRequest);

    // First mount: load profiles + auto-attach if there's a tab waiting.
    await loadSshProfiles();

    // If a tab is already active for the current mode, restore the session.
    const profile = get(activeSshProfile);
    if (profile) {
      currentTabKey = null; // force activate
      requestAnimationFrame(() => activateProfile(profile));
    } else {
      // If there is an SSH tab but no active profile, hydrate it.
      const all = get(tabsStore);
      const sshTab = all.find((t) => t.mode === 'ssh' && t.key);
      if (sshTab?.key) {
        const profiles = get(sshProfiles);
        const match = profiles.find((p) => p.id === sshTab.key);
        if (match) activeSshProfile.set(match);
      }
    }
  });

  onDestroy(() => {
    unsubProfile();
    unsubActiveTab();
    unsubAppearance();
    window.removeEventListener(SSH_EVENT.OPEN_TAB, handleOpenTab);
    window.removeEventListener(SSH_EVENT.CLOSE_TAB, handleCloseTab);
    window.removeEventListener(SSH_EVENT.DUPLICATE_SESSION, handleDuplicateSession);
    window.removeEventListener(SSH_EVENT.INSERT_COMMAND, handleInsertCommand);
    window.removeEventListener(SSH_EVENT.EXECUTE_CAPTURE_REQUEST, handleExecuteCaptureRequest);
    rejectAllSshCaptures('SSH panel unmounted');
  });

  // Reconnect banner state for the currently active SSH tab.
  // Lookup MUST use the tab key (profileId#timestamp-N), not profile.id —
  // exitedTabs/sshTerminalIds/sshConnStates have all been keyed by tabKey
  // since the multi-tab-per-profile refactor (2026-05-01).
  let activeIsExited = $derived(
    !!currentTabKey && exitedTabs.has(currentTabKey),
  );

  // Ambient "Controlled from phone" hint: true while the on-screen tab's
  // terminal is being size-driven by a paired phone. Resolves the visible
  // terminalId through sshTerminalMap keyed by currentTabKey, the same way
  // focus-reporting does.
  let activePhoneOwned = $derived.by(() => {
    const termId = currentTabKey
      ? $sshTerminalMap.get(currentTabKey)?.terminalId ?? null
      : null;
    return !!termId && $phoneOwnedTerminals.has(termId);
  });

  // Yield phone-owned terminals. While the phone owns a terminal's size, the
  // desktop does NOTHING to that PTY — it neither resizes its xterm nor fits/
  // drives the PTY (the phone is the sole renderer). The "Controlled from
  // phone" panel covers the now-irrelevant desktop xterm. When a terminal
  // stops being phone-owned, the desktop reclaims it: re-fit to its pane and
  // resume driving. Each sshTerminalMap entry carries its own terminalId, so
  // we resolve phone-driven terminalIds directly to their xterm entry.
  let _lastDrivenTermIds = new Set<string>();
  $effect(() => {
    const driven = $phoneDrivenSizes;
    const termMap = $sshTerminalMap;
    // terminalId → entry lookup.
    const entryByTerm = new Map<string, TermEntry>();
    for (const entry of termMap.values()) {
      if (entry.terminalId) entryByTerm.set(entry.terminalId, entry);
    }

    const nowDriven = new Set<string>();
    for (const termId of driven.keys()) nowDriven.add(termId);

    // Terminals that just stopped being phone-owned → re-fit to their pane and
    // resume desktop driving (reclaim on detach).
    for (const termId of _lastDrivenTermIds) {
      if (nowDriven.has(termId)) continue;
      const entry = entryByTerm.get(termId);
      if (entry?.fitAddon && entry.terminalId) {
        try {
          entry.fitAddon.fit();
          const dims = entry.fitAddon.proposeDimensions();
          if (dims) sshResizeTerminal(entry.terminalId, dims.cols, dims.rows).catch(() => {});
        } catch { /* ignore */ }
      }
    }
    _lastDrivenTermIds = nowDriven;
  });
</script>

{#if $activeSshProfile}
  <div class="ssh-panel">
    {#if activePhoneOwned}
      <div class="phone-owned-panel">
        <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <rect x="6" y="2" width="12" height="20" rx="2.5"/>
          <line x1="11" y1="18" x2="13" y2="18"/>
        </svg>
        <span class="phone-owned-title">Controlled from phone</span>
        <span class="phone-owned-sub">This session is being driven from your mobile. It'll return when you detach.</span>
      </div>
    {/if}
    {#if spawning}
      <div class="ssh-loading">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="var(--ssh)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <rect x="2" y="4" width="20" height="6" rx="1"/>
          <rect x="2" y="14" width="20" height="6" rx="1"/>
          <line x1="6" y1="7" x2="6.01" y2="7"/>
          <line x1="6" y1="17" x2="6.01" y2="17"/>
        </svg>
        <div class="loading-text">
          <span class="loading-title">Connecting to {$activeSshProfile.host}</span>
          <span class="loading-sub">{$activeSshProfile.username}@{$activeSshProfile.host}:{$activeSshProfile.port}<span class="loading-dots"></span></span>
        </div>
        <button class="ssh-cancel-btn" onclick={cancelConnect}>Cancel</button>
      </div>
    {/if}

    {#if activeIsExited}
      <div class="ssh-banner">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="var(--err)" stroke-width="2" stroke-linecap="round">
          <circle cx="12" cy="12" r="10"/>
          <line x1="15" y1="9" x2="9" y2="15"/>
          <line x1="9" y1="9" x2="15" y2="15"/>
        </svg>
        <span class="ssh-banner-text">Connection closed</span>
        <button class="ssh-banner-btn" onclick={reconnectActive}>Reconnect</button>
      </div>
    {/if}

    {#if findOpen}
      <div class="ssh-find-bar">
        <!-- svelte-ignore a11y_autofocus -->
        <input
          bind:this={findInputEl}
          value={findQuery}
          oninput={onFindInput}
          onkeydown={onFindKeydown}
          onblur={() => { try { activeEntry?.searchAddon.clearActiveDecoration(); } catch { /* ignore */ } }}
          class="ssh-find-input"
          class:no-match={findNoMatch}
          placeholder="Find in terminal…"
          spellcheck={false}
          autocomplete="off"
        />

        <div class="ssh-find-sep"></div>

        <button class="ssh-find-toggle" class:active={findCaseSensitive} onclick={toggleCase} title="Case sensitive (Aa)">Aa</button>
        <button class="ssh-find-toggle" class:active={findRegex}         onclick={toggleRegex} title="Use regular expression (.*)">.*</button>
        <button class="ssh-find-toggle" class:active={findWholeWord}     onclick={toggleWord}  title="Match whole word">W</button>

        <div class="ssh-find-sep"></div>

        {#if findQuery}
          <span class="ssh-find-count" class:no-results={findNoMatch}>
            {#if findNoMatch}
              No results
            {:else if findResultCount > 0}
              {findResultIndex === -1 ? `${findResultCount}+` : `${findResultIndex + 1} / ${findResultCount}`}
            {/if}
          </span>
        {/if}

        <button class="ssh-find-btn" onclick={doFindPrev} title="Previous (Shift+Enter)">
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="18 15 12 9 6 15"/></svg>
        </button>
        <button class="ssh-find-btn" onclick={doFindNext} title="Next (Enter)">
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="6 9 12 15 18 9"/></svg>
        </button>

        <div class="ssh-find-sep"></div>

        <button class="ssh-find-close" onclick={closeFind} title="Close (Esc)">
          <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>
    {/if}

    <div class="ssh-terminal-container" class:term-hidden={(!termReady && !activeIsExited) || activePhoneOwned} bind:this={terminalEl} style="background:{termBg}"></div>
  </div>
{:else}
  <div class="ssh-empty">
    <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="var(--t4)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <rect x="2" y="4" width="20" height="6" rx="1"/>
      <rect x="2" y="14" width="20" height="6" rx="1"/>
      <line x1="6" y1="7" x2="6.01" y2="7"/>
      <line x1="6" y1="17" x2="6.01" y2="17"/>
    </svg>
    <p class="empty-title">No active SSH session</p>
    <p class="empty-sub">Pick a profile from the sidebar or create a new one</p>
  </div>
{/if}

<style>
  .ssh-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    position: relative;
  }

  .phone-owned-panel {
    position: absolute;
    inset: 0;
    z-index: 6;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    text-align: center;
    padding: 24px;
    background: var(--n);
    color: var(--acc);
    user-select: none;
  }
  .phone-owned-panel svg {
    flex: none;
    opacity: 0.85;
  }
  .phone-owned-title {
    font-family: var(--ui);
    font-size: 15px;
    font-weight: 600;
    line-height: 1.2;
    color: var(--acc);
  }
  .phone-owned-sub {
    font-family: var(--ui);
    font-size: 12px;
    line-height: 1.4;
    max-width: 320px;
    color: var(--fg-muted, color-mix(in srgb, var(--fg) 55%, transparent));
  }
  .ssh-terminal-container {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    transition: opacity 0.15s ease;
  }
  .ssh-terminal-container.term-hidden { opacity: 0; }
  .ssh-terminal-container :global(.xterm) { height: 100% !important; padding: 0 !important; }
  /* xterm.css hardcodes viewport background to #000 (legacy macOS scrollbar
     workaround). Override so the wrapper's themed background shows through
     at the canvas-cell-rounding gap on the right/bottom edges. */
  .ssh-terminal-container :global(.xterm-viewport) { height: 100% !important; scrollbar-gutter: auto; background-color: transparent !important; }
  .ssh-terminal-container :global(.xterm-screen) { height: 100% !important; }
  .ssh-terminal-container :global(.xterm-viewport::-webkit-scrollbar) { width: 3px; }
  .ssh-terminal-container :global(.xterm-viewport::-webkit-scrollbar-track) { background: transparent; }
  .ssh-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb) { background: var(--surface-hover); border-radius: 3px; }
  .ssh-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb:hover) { background: var(--surface-hover); }

  .ssh-loading {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 16px;
    z-index: 2;
    animation: loadFadeIn 0.3s ease;
    /* Block xterm's text cursor from bleeding through the loader area */
    background: var(--n);
    cursor: default;
  }
  .loading-text { display: flex; flex-direction: column; align-items: center; gap: 4px; }
  .loading-title { font-size: 14px; font-weight: 500; color: var(--t2); font-family: var(--ui); }
  .loading-sub { font-size: 11px; color: var(--t4); font-family: var(--mono); }
  .loading-dots::after { content: ''; animation: dots 1.5s steps(4, end) infinite; }
  @keyframes dots { 0% { content: ''; } 25% { content: '.'; } 50% { content: '..'; } 75% { content: '...'; } }
  @keyframes loadFadeIn { from { opacity: 0; transform: scale(0.97); } to { opacity: 1; transform: scale(1); } }

  .ssh-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    background: color-mix(in srgb, var(--err) 12%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--err) 25%, var(--b1));
    flex-shrink: 0;
    z-index: 3;
  }
  .ssh-banner-text {
    font-family: var(--ui);
    font-size: 12px;
    color: var(--t2);
    flex: 1;
  }
  .ssh-banner-btn {
    padding: 4px 12px;
    border-radius: 5px;
    border: 1px solid var(--ssh);
    background: var(--ssh);
    color: #fff;
    font-size: 11px;
    font-weight: 600;
    font-family: var(--ui);
    cursor: pointer;
  }
  .ssh-banner-btn:hover { filter: brightness(1.1); }

  .ssh-cancel-btn {
    margin-top: 14px;
    padding: 6px 16px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    pointer-events: auto;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .ssh-cancel-btn:hover {
    background: var(--surface-hover);
    border-color: var(--b2);
    color: var(--t1);
    cursor: pointer;
  }

  .ssh-find-bar {
    position: absolute;
    top: 8px;
    right: 14px;
    z-index: 10;
    display: flex;
    align-items: center;
    gap: 1px;
    background: var(--n2, #1a1a2e);
    border: 1px solid var(--b1);
    border-radius: 7px;
    padding: 3px 5px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.5);
    animation: findSlideIn 0.12s ease;
  }
  @keyframes findSlideIn {
    from { opacity: 0; transform: translateY(-4px); }
    to   { opacity: 1; transform: translateY(0); }
  }
  .ssh-find-input {
    background: transparent;
    border: none;
    outline: none;
    color: var(--t1);
    font-family: var(--mono);
    font-size: 12px;
    width: 180px;
    padding: 2px 4px;
    caret-color: var(--acc);
  }
  .ssh-find-input.no-match { color: var(--err, #f55); }
  .ssh-find-input::placeholder { color: var(--t4); }
  .ssh-find-sep {
    width: 1px;
    height: 16px;
    background: var(--b1);
    flex-shrink: 0;
    margin: 0 3px;
  }
  .ssh-find-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 22px;
    min-width: 22px;
    padding: 0 4px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--t4);
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    flex-shrink: 0;
    transition: background 0.1s, color 0.1s;
  }
  .ssh-find-toggle:hover { background: var(--surface-hover); color: var(--t2); }
  .ssh-find-toggle.active {
    background: color-mix(in srgb, var(--acc) 20%, transparent);
    color: var(--acc);
  }
  .ssh-find-count {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--t3);
    white-space: nowrap;
    padding: 0 4px;
    min-width: 52px;
    text-align: center;
    flex-shrink: 0;
  }
  .ssh-find-count.no-results { color: var(--err, #f55); }
  .ssh-find-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--t3);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }
  .ssh-find-btn:hover { background: var(--surface-hover); color: var(--t1); }
  .ssh-find-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--t4);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }
  .ssh-find-close:hover { background: var(--surface-hover); color: var(--t2); }

  .ssh-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }
  .empty-title {
    font-size: 16px;
    font-weight: 500;
    color: var(--t2);
    font-family: var(--ui);
    margin: 0;
  }
  .empty-sub {
    font-size: 13px;
    color: var(--t3);
    font-family: var(--ui);
    margin: 0;
  }
</style>

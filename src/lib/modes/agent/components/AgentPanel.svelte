<script lang="ts">
  import { onMount, onDestroy, untrack } from 'svelte';
  import { get } from 'svelte/store';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import '@xterm/xterm/css/xterm.css';
  import { Channel } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import {
    activeAgentSession,
    agentTerminalMap,
    agentShellMap,
    agentTerminalIds,
    agentShellIds,
    agentShellOpen,
    agentSessionActivity,
    agentSessionAwaiting,
    agentSessions,
    agentSoundEnabled,
    agentDockBounceEnabled,
  } from '../stores';
  import { getSetting, setSetting } from '$lib/commands/settings';
  import { phoneOwnedTerminals, phoneDrivenSizes } from '$lib/stores/sizeOwner';
  import { tabs as tabsStore, closeTab, activateTab } from '$lib/shared/stores/tabs';
  import {
    agentSpawnTerminal,
    agentSpawnShell,
    agentWriteToTerminal,
    agentResizeTerminal,
    agentUpdateSessionId,
    agentUpdateLastUsed,
    agentDiscoverSessions,
    agentIsGitRepo,
    agentCreateWorktree,
    agentUpdateWorktree,
    agentListSessions,
    agentGetSessionContexts,
    agentInjectContexts,
    agentInjectPurpose,
    agentKillTerminal,
    agentRemoveWorktree,
    agentDeleteSession,
    agentCheckClaudeInstalled,
    agentCheckCliInstalled,
  } from '../commands';
  import ProviderNotInstalledModal from '$lib/shared/agent/ProviderNotInstalledModal.svelte';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import { refreshAgentGitStatus, refreshAgentContextUsage, loadAgentSessions, agentGitBranchName, agentGitFiles, agentGitAhead, agentGitBehind } from '../stores';
  import { getTerminalTheme } from '$lib/utils/theme';
  import { appearance } from '$lib/stores/settings';
  import { mode } from '$lib/stores/app';
  import { base64ToBytes, deferUntilFrame, loadWebGLAddon } from '$lib/shared/primitives/terminal-utils';
  import { getPurposePrompt } from '../ai/prompt';
  import { AGENT_EVENT } from '$lib/shared/constants/events';
  import {
    AGENT_NOTIFY_REPEAT_MS,
    AGENT_CHIME_STOP_MS,
    AGENT_ACTIVITY_WINDOW_MS,
    AGENT_ACTIVITY_DONE_MS,
    AGENT_SHELL_LOADER_MS,
    AGENT_CONTEXT_USAGE_INTERVAL_MS,
    AGENT_SESSION_CAPTURE_INTERVAL_MS,
    RESIZE_DEBOUNCE_MS,
  } from '$lib/shared/constants/timings';

  let terminalEl: HTMLDivElement;
  let shellEl: HTMLDivElement;
  let wrapperEl: HTMLDivElement;

  $effect(() => {
    if ($mode !== 'agent') return;
    // Read activeAgentSession non-reactively — this $effect should only
    // re-run when $mode changes (e.g., Canvas → Agent return), NOT when
    // the active session changes via tab click. Tab click activations go
    // through the unsubSession subscriber → selectSession → showTermEntry,
    // which has sole authority over visibility. Adding a parallel call
    // from this $effect races and corrupts the activeTermEntry pointer.
    const session = untrack(() => $activeAgentSession);
    if (!session?.id) return;

    // Active session's container may be orphaned after a Canvas round-trip
    // (CanvasTile.detachAgentTerminal moved it into a canvas tile slot,
    // then the tile unmounted). Re-home it to terminalEl/shellEl, then let
    // showTermEntry / showShellEntry apply the correct visibility class.
    const entry = untrack(() => $agentTerminalMap).get(session.id);
    if (entry && terminalEl) {
      if (entry.container.parentElement !== terminalEl) {
        terminalEl.appendChild(entry.container);
      }
      showTermEntry(entry);
    }

    const shellEntry = untrack(() => $agentShellMap).get(session.id);
    if (shellEntry && shellEl) {
      if (shellEntry.container.parentElement !== shellEl) {
        shellEl.appendChild(shellEntry.container);
      }
      showShellEntry(shellEntry);
    }
  });

  // Re-fit when mode returns to agent (e.g. Canvas → Agent round-trip).
  $effect(() => {
    const m = $mode;
    if (m === 'agent') scheduleFit();
  });

  // Active terminal entry refs
  let activeTermEntry: { term: Terminal; fitAddon: FitAddon; searchAddon: SearchAddon; container: HTMLDivElement; terminalId: string | null; _exitBuffer?: string } | null = null;
  let activeShellEntry: { term: Terminal; fitAddon: FitAddon; searchAddon: SearchAddon; container: HTMLDivElement; terminalId: string | null } | null = null;

  // Outer-container ResizeObserver (set up in onMount, torn down in onDestroy)
  let _containerResizeObserver: ResizeObserver | null = null;

  // Divider drag state
  let dragging = $state(false);
  let mainWidth = $state(55); // percentage (left terminal width)

  // Terminal background color (synced with theme to fill gaps)
  let termBg = $state('#0d0d18');

  // Main terminal find bar
  let termFindOpen          = $state(false);
  let termFindQuery         = $state('');
  let termFindNoMatch       = $state(false);
  let termFindResultIndex   = $state(-1);
  let termFindResultCount   = $state(0);
  let termFindCaseSensitive = $state(false);
  let termFindRegex         = $state(false);
  let termFindWholeWord     = $state(false);
  let termFindInputEl: HTMLInputElement | null = null;

  // Shell terminal find bar
  let shellFindOpen          = $state(false);
  let shellFindQuery         = $state('');
  let shellFindNoMatch       = $state(false);
  let shellFindResultIndex   = $state(-1);
  let shellFindResultCount   = $state(0);
  let shellFindCaseSensitive = $state(false);
  let shellFindRegex         = $state(false);
  let shellFindWholeWord     = $state(false);
  let shellFindInputEl: HTMLInputElement | null = null;

  const FIND_DECORATIONS = {
    matchBackground:               '#1c3d6b',
    matchBorder:                   '#2a5a9e',
    matchOverviewRuler:            '#4488cc',
    activeMatchBackground:         '#7a3d00',
    activeMatchBorder:             '#e07000',
    activeMatchColorOverviewRuler: '#ff8c00',
  };

  function searchOpts(caseSensitive: boolean, regex: boolean, wholeWord: boolean) {
    return { regex, caseSensitive, wholeWord, decorations: FIND_DECORATIONS };
  }

  // Main terminal find functions
  function openTermFind() {
    termFindOpen = true;
    termFindNoMatch = false;
    termFindResultIndex = -1;
    termFindResultCount = 0;
    requestAnimationFrame(() => termFindInputEl?.focus());
  }
  function closeTermFind() {
    termFindOpen = false;
    termFindQuery = '';
    termFindNoMatch = false;
    termFindResultIndex = -1;
    termFindResultCount = 0;
    try { activeTermEntry?.searchAddon.clearDecorations(); } catch { /* ignore */ }
    requestAnimationFrame(() => { try { activeTermEntry?.term.focus(); } catch { /* ignore */ } });
  }
  function doTermFindNext() {
    if (!activeTermEntry || !termFindQuery) return;
    try {
      const found = activeTermEntry.searchAddon.findNext(termFindQuery, searchOpts(termFindCaseSensitive, termFindRegex, termFindWholeWord));
      termFindNoMatch = !found;
    } catch { termFindNoMatch = true; }
  }
  function doTermFindPrev() {
    if (!activeTermEntry || !termFindQuery) return;
    try {
      const found = activeTermEntry.searchAddon.findPrevious(termFindQuery, searchOpts(termFindCaseSensitive, termFindRegex, termFindWholeWord));
      termFindNoMatch = !found;
    } catch { termFindNoMatch = true; }
  }
  function onTermFindInput(e: Event) {
    const query = (e.currentTarget as HTMLInputElement).value;
    termFindQuery = query;
    termFindNoMatch = false;
    if (!activeTermEntry) return;
    if (!query) {
      termFindResultIndex = -1;
      termFindResultCount = 0;
      try { activeTermEntry.searchAddon.clearDecorations(); } catch { /* ignore */ }
      return;
    }
    try {
      const found = activeTermEntry.searchAddon.findNext(query, searchOpts(termFindCaseSensitive, termFindRegex, termFindWholeWord));
      termFindNoMatch = !found;
    } catch { termFindNoMatch = true; }
  }
  function reRunTermSearch() {
    if (!activeTermEntry || !termFindQuery) return;
    try {
      const found = activeTermEntry.searchAddon.findNext(termFindQuery, searchOpts(termFindCaseSensitive, termFindRegex, termFindWholeWord));
      termFindNoMatch = !found;
    } catch { termFindNoMatch = true; }
  }
  function toggleTermCase()  { termFindCaseSensitive = !termFindCaseSensitive; reRunTermSearch(); }
  function toggleTermRegex() { termFindRegex         = !termFindRegex;         reRunTermSearch(); }
  function toggleTermWord()  { termFindWholeWord     = !termFindWholeWord;     reRunTermSearch(); }
  function onTermFindKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter')  { e.preventDefault(); if (e.shiftKey) doTermFindPrev(); else doTermFindNext(); }
    else if (e.key === 'Escape') { closeTermFind(); }
  }

  // Shell terminal find functions
  function openShellFind() {
    shellFindOpen = true;
    shellFindNoMatch = false;
    shellFindResultIndex = -1;
    shellFindResultCount = 0;
    requestAnimationFrame(() => shellFindInputEl?.focus());
  }
  function closeShellFind() {
    shellFindOpen = false;
    shellFindQuery = '';
    shellFindNoMatch = false;
    shellFindResultIndex = -1;
    shellFindResultCount = 0;
    try { activeShellEntry?.searchAddon.clearDecorations(); } catch { /* ignore */ }
    requestAnimationFrame(() => { try { activeShellEntry?.term.focus(); } catch { /* ignore */ } });
  }
  function doShellFindNext() {
    if (!activeShellEntry || !shellFindQuery) return;
    try {
      const found = activeShellEntry.searchAddon.findNext(shellFindQuery, searchOpts(shellFindCaseSensitive, shellFindRegex, shellFindWholeWord));
      shellFindNoMatch = !found;
    } catch { shellFindNoMatch = true; }
  }
  function doShellFindPrev() {
    if (!activeShellEntry || !shellFindQuery) return;
    try {
      const found = activeShellEntry.searchAddon.findPrevious(shellFindQuery, searchOpts(shellFindCaseSensitive, shellFindRegex, shellFindWholeWord));
      shellFindNoMatch = !found;
    } catch { shellFindNoMatch = true; }
  }
  function onShellFindInput(e: Event) {
    const query = (e.currentTarget as HTMLInputElement).value;
    shellFindQuery = query;
    shellFindNoMatch = false;
    if (!activeShellEntry) return;
    if (!query) {
      shellFindResultIndex = -1;
      shellFindResultCount = 0;
      try { activeShellEntry.searchAddon.clearDecorations(); } catch { /* ignore */ }
      return;
    }
    try {
      const found = activeShellEntry.searchAddon.findNext(query, searchOpts(shellFindCaseSensitive, shellFindRegex, shellFindWholeWord));
      shellFindNoMatch = !found;
    } catch { shellFindNoMatch = true; }
  }
  function reRunShellSearch() {
    if (!activeShellEntry || !shellFindQuery) return;
    try {
      const found = activeShellEntry.searchAddon.findNext(shellFindQuery, searchOpts(shellFindCaseSensitive, shellFindRegex, shellFindWholeWord));
      shellFindNoMatch = !found;
    } catch { shellFindNoMatch = true; }
  }
  function toggleShellCase()  { shellFindCaseSensitive = !shellFindCaseSensitive; reRunShellSearch(); }
  function toggleShellRegex() { shellFindRegex         = !shellFindRegex;         reRunShellSearch(); }
  function toggleShellWord()  { shellFindWholeWord     = !shellFindWholeWord;     reRunShellSearch(); }
  function onShellFindKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter')  { e.preventDefault(); if (e.shiftKey) doShellFindPrev(); else doShellFindNext(); }
    else if (e.key === 'Escape') { closeShellFind(); }
  }

  // Track current session to detect changes
  let currentSessionId: string | null = null;

  // Loading state for terminal spawn
  let spawning = $state(false);
  let termReady = $state(false);
  // Shell-panel loading: tracks session IDs currently spawning a shell
  let shellLoadingSessions = $state<string[]>([]);
  let activeShellLoading = $derived(
    $activeAgentSession ? shellLoadingSessions.includes($activeAgentSession.id) : false
  );

  // Ambient "Controlled from phone" hint: true while the on-screen session's
  // terminal is being size-driven by a paired phone (phone-authoritative
  // sizing). Resolves the visible terminalId the same way focus-reporting does.
  let activePhoneOwned = $derived.by(() => {
    const termId = $activeAgentSession
      ? $agentTerminalIds.get($activeAgentSession.id) ?? null
      : null;
    return !!termId && $phoneOwnedTerminals.has(termId);
  });

  // Yield phone-owned terminals. While the phone owns a terminal's size, the
  // desktop does NOTHING to that PTY — it neither resizes its xterm nor fits/
  // drives the PTY (the phone is the sole renderer). The "Controlled from
  // phone" panel covers the now-irrelevant desktop xterm. When a terminal
  // stops being phone-owned, the desktop reclaims it: re-fit to its pane and
  // resume driving. Keyed off agentTerminalIds (sessionId → terminalId) so we
  // can resolve each phone-driven terminalId back to its xterm entry in
  // agentTerminalMap (keyed by sessionId).
  let _lastDrivenTermIds = new Set<string>();
  $effect(() => {
    const driven = $phoneDrivenSizes;
    const idMap = $agentTerminalIds;
    const termMap = get(agentTerminalMap);
    // termId → sessionId reverse lookup.
    const sessionByTerm = new Map<string, string>();
    for (const [sid, tid] of idMap) sessionByTerm.set(tid, sid);

    const nowDriven = new Set<string>();
    for (const termId of driven.keys()) nowDriven.add(termId);

    // Terminals that just stopped being phone-owned → re-fit to their pane and
    // resume desktop driving (reclaim on detach).
    for (const termId of _lastDrivenTermIds) {
      if (nowDriven.has(termId)) continue;
      const sid = sessionByTerm.get(termId);
      const entry = sid ? termMap.get(sid) : undefined;
      if (entry?.fitAddon) {
        try {
          entry.fitAddon.fit();
          if (entry.terminalId) {
            const dims = entry.fitAddon.proposeDimensions();
            if (dims) agentResizeTerminal(entry.terminalId, dims.cols, dims.rows).catch(() => {});
          }
        } catch (_) {}
      }
    }
    _lastDrivenTermIds = nowDriven;
  });

  // Context usage polling interval
  let contextUsageInterval: ReturnType<typeof setInterval> | null = null;

  // Terminal IDs whose PTY exit was initiated by Clauge (reset / relaunch /
  // close-tab / delete-session). The exit handler removes the id from this
  // set when the exit event arrives and routes through the suppressed-path
  // cleanup instead of the natural-exit path. Per-terminal scoping (vs a
  // single boolean flag) eliminates the async-race window — each kill is
  // paired 1:1 with its own exit. Termids are unique per spawn, so a stale
  // entry (exit event that never arrives) has no collision risk.
  const _suppressedTerminalIds = new Set<string>();

  let showNotInstalled = $state(false);
  let notInstalledProvider = $state<'claude' | 'codex' | 'gemini' | 'opencode'>('claude');

  // --- Notification system for action-required prompts (per-session) ---
  // Each session detects its own prompts and tracks its own re-alert guard so
  // one session's output can never silence or storm another's. The dock-bounce
  // + chime alert is shared (one window-level effect) but is driven by the
  // union of awaiting sessions in `agentSessionAwaiting`.
  let notifySoundInterval: ReturnType<typeof setInterval> | null = null;
  let dockBounceActive = false;
  let unlistenFileDrop: (() => void) | null = null;
  let unlistenAttentionCleared: UnlistenFn | null = null;

  // Per-session rolling ANSI-stripped output buffer (for prompt detection).
  const notifyBuffers = new Map<string, string>();
  // Per-session last-notify timestamp (10s re-alert guard, per session).
  const notifyLastTimes = new Map<string, number>();

  const actionPatterns = [
    /Do you want to proceed/i,
    /1\.\s*Yes/,
    /\(y\/n\)/i,
    /\[Y\/n\]/i,
    /\[y\/N\]/i,
    /Press Enter/i,
    /Allow.*Deny/i,
    /approve this/i,
    /permission/i,
    /Yes, and don.t ask/i,
  ];

  // Start (or refresh) the shared dock-bounce + chime alert. Called when any
  // session enters the awaiting state while the window is unfocused. Both
  // surfaces are gated by their settings and stop automatically once the
  // window regains focus or no session is awaiting.
  function startAlert() {
    if (document.hasFocus()) return;

    if (get(agentDockBounceEnabled) && !dockBounceActive) {
      import('@tauri-apps/api/window').then(({ getCurrentWindow, UserAttentionType }) => {
        getCurrentWindow().requestUserAttention(UserAttentionType.Critical);
      }).catch(() => {});
      dockBounceActive = true;
    }

    if (get(agentSoundEnabled)) {
      playChime();
      if (notifySoundInterval) clearInterval(notifySoundInterval);
      notifySoundInterval = setInterval(() => {
        if (document.hasFocus() || get(agentSessionAwaiting).size === 0) {
          stopAlert();
          return;
        }
        playChime();
      }, AGENT_NOTIFY_REPEAT_MS);
    }
  }

  // Stop the shared alert: clear the chime repeat and cancel any active dock
  // bounce. Safe to call when nothing is active.
  function stopAlert() {
    if (notifySoundInterval) { clearInterval(notifySoundInterval); notifySoundInterval = null; }
    if (dockBounceActive) {
      dockBounceActive = false;
      import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
        getCurrentWindow().requestUserAttention(null);
      }).catch(() => {});
    }
  }

  // Mark a session as awaiting input. Adds it to the per-session store and, if
  // the window is unfocused, raises the shared alert. The 10s per-session guard
  // prevents a single redraw-happy session from re-firing the chime.
  function markAwaiting(sessionId: string) {
    const now = Date.now();
    const last = notifyLastTimes.get(sessionId) ?? 0;
    if (now - last < 10000) return;
    notifyLastTimes.set(sessionId, now);
    agentSessionAwaiting.update(s => {
      if (s.has(sessionId)) return s;
      const next = new Set(s);
      next.add(sessionId);
      return next;
    });
    startAlert();
  }

  // Clear a session's awaiting state. Removes it from the store, resets its
  // re-alert guard, and stops the shared alert if no session is left awaiting.
  // This is the per-session version of the old maybeClear backstop AND the
  // landing point for the authoritative `agent-attention-cleared` event.
  function clearAwaiting(sessionId: string) {
    notifyLastTimes.set(sessionId, 0);
    let removed = false;
    agentSessionAwaiting.update(s => {
      if (!s.has(sessionId)) return s;
      removed = true;
      const next = new Set(s);
      next.delete(sessionId);
      return next;
    });
    if (!removed) return;
    if (get(agentSessionAwaiting).size === 0) stopAlert();
  }

  function playChime() {
    try {
      const ctx = new (window.AudioContext || (window as any).webkitAudioContext)();
      const osc1 = ctx.createOscillator();
      const osc2 = ctx.createOscillator();
      const gain = ctx.createGain();
      osc1.frequency.value = 880;
      osc2.frequency.value = 1318.5;
      osc1.type = 'sine';
      osc2.type = 'sine';
      gain.gain.value = 0.15;
      osc1.connect(gain);
      osc2.connect(gain);
      gain.connect(ctx.destination);
      osc1.start();
      osc2.start();
      gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.3);
      setTimeout(() => { osc1.stop(); osc2.stop(); ctx.close(); }, AGENT_CHIME_STOP_MS);
    } catch (_) {}
  }

  function handleTerminalOutput(sessionId: string, base64Data: string) {
    try {
      const raw = atob(base64Data);
      const text = raw.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '').replace(/\x1b\][^\x07]*\x07/g, '');

      // Maintain a small rolling per-session buffer so a prompt split across
      // chunks is still matched. Bounded so it can't grow unboundedly.
      let buf = (notifyBuffers.get(sessionId) ?? '') + text;
      if (buf.length > 2000) buf = buf.slice(-2000);
      notifyBuffers.set(sessionId, buf);

      const isPrompt = actionPatterns.some(p => p.test(buf));
      if (isPrompt) {
        // A prompt for THIS session — flag it and (if unfocused) alert. The
        // buffer is reset so the same prompt text doesn't keep re-matching on
        // every redraw chunk; markAwaiting's per-session guard handles storms.
        notifyBuffers.set(sessionId, '');
        markAwaiting(sessionId);
      } else if (get(agentSessionAwaiting).has(sessionId) && text.trim().length > 8) {
        // Backstop to the backend `agent-attention-cleared` event: if a session
        // that was awaiting produces substantial non-prompt output, the agent
        // has continued — clear it locally even if the event hasn't arrived.
        clearAwaiting(sessionId);
      }
    } catch (_) {}
  }

  function getCurrentTermTheme(): Record<string, string> {
    const app = get(appearance);
    return getTerminalTheme(app.theme, app.accentColor);
  }

  type ImeCoalesceState = { last: string; at: number };

  const hasNonAscii = (data: string | null | undefined): boolean => !!data && /[^\x00-\x7F]/.test(data);

  function coalesceGrowingImeCommit(data: string, state: ImeCoalesceState): string {
    // Some Linux IME + WebKitGTK/xterm.js combinations can resend the
    // whole committed composition buffer through xterm.onData(), e.g.
    // "你" then "你好".  Do not intercept DOM composition/input events here:
    // preventing xterm's hidden textarea events can stop IME commits entirely.
    // Keep this workaround at the PTY bridge only and only trim the observed
    // growing-buffer shape. Plain ASCII/control input is always passed through.
    const now = Date.now();
    if (!hasNonAscii(data)) {
      if (/\x1b|[\x00-\x08\x0e-\x1f\x7f]/.test(data) || data.length !== 1) {
        state.last = '';
        state.at = 0;
      }
      return data;
    }

    if (state.last && now - state.at < 500 && data.length > state.last.length && data.startsWith(state.last)) {
      const suffix = data.slice(state.last.length);
      state.last = data;
      state.at = now;
      return suffix;
    }

    state.last = data;
    state.at = now;
    return data;
  }

  function clearXtermTextarea(term: Terminal): void {
    // WebKitGTK can leave committed IME text in xterm's hidden textarea; the
    // next commit is then emitted as previous+new ("你好" then
    // "你好今天星期几").  Clear after xterm has already produced onData so the
    // current commit is preserved while the next diff starts from empty.
    const textarea = term.textarea;
    if (textarea && textarea.value) textarea.value = '';
  }

  function createTermEntry(sessionId: string): { term: Terminal; fitAddon: FitAddon; searchAddon: SearchAddon; container: HTMLDivElement; terminalId: string | null; _exitBuffer?: string } {
    const t = new Terminal({
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
    const fa = new FitAddon();
    const sa = new SearchAddon();
    t.loadAddon(fa);
    t.loadAddon(sa);

    const container = document.createElement('div');
    container.style.cssText = 'width:100%;height:100%;';
    terminalEl.appendChild(container);
    t.open(container);
    loadWebGLAddon(t);

    sa.onDidChangeResults(({ resultIndex, resultCount }) => {
      if (activeTermEntry?.container !== container) return;
      termFindResultIndex = resultIndex;
      termFindResultCount = resultCount;
    });

    t.attachCustomKeyEventHandler((e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'f' && e.type === 'keydown') {
        openTermFind();
        return false;
      }
      if (e.key === 'Escape' && e.type === 'keydown' && termFindOpen) {
        closeTermFind();
        return false;
      }
      return true;
    });

    const writeAgentTerminalData = (data: string) => {
      if (!data) return;
      const tIds = get(agentTerminalIds);
      const termId = tIds.get(sessionId);
      if (termId) {
        agentWriteToTerminal(termId, data).catch(() => {
          // PTY dead (I/O error) — treat as session exit. Mirror the
          // natural-exit path: clear the live termId, mark activity done,
          // and close the tab so the user isn't stuck typing into a
          // frozen terminal. Re-opening from the sidebar will auto-spawn
          // a fresh CLI (with --resume).
          agentTerminalIds.update(m => { m.delete(sessionId); return new Map(m); });
          agentSessionActivity.update(m => { m.set(sessionId, 'done'); return new Map(m); });
          const tMapErr = get(agentTerminalMap);
          const errEntry = tMapErr.get(sessionId);
          if (errEntry) {
            try { errEntry.container.remove(); } catch (_) {}
            try { errEntry.term.dispose(); } catch (_) {}
            agentTerminalMap.update(m => { m.delete(sessionId); return new Map(m); });
          }
          if (activeTermEntry === errEntry) activeTermEntry = null;
          if (currentSessionId === sessionId) currentSessionId = null;
          const allTabsErr = get(tabsStore);
          const errTab = allTabsErr.find(t => t.mode === 'agent' && t.key === sessionId);
          if (errTab) closeTab(errTab.id);
        });
      }
    };
    const termImeState: ImeCoalesceState = { last: '', at: 0 };
    t.onData((rawData: string) => {
      const data = coalesceGrowingImeCommit(rawData, termImeState);
      clearXtermTextarea(t);
      writeAgentTerminalData(data);
    });

    let resizeTimer: ReturnType<typeof setTimeout> | null = null;
    new ResizeObserver(() => {
      if (!fa || container.offsetWidth <= 0) return;
      if (resizeTimer) clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        resizeTimer = null;
        try {
          const tIds = get(agentTerminalIds);
          const termId = tIds.get(sessionId);
          // Phone-owned: the phone drives the size. Adopt it onto our xterm
          // (handled by the adopt $effect) and skip both the local fit and
          // the PTY-resize drive so the desktop doesn't fight the phone.
          if (termId && get(phoneDrivenSizes).has(termId)) return;
          fa.fit();
          // Skip PTY resize during drag — only resize on mouseup via refitAll
          if (dragging) return;
          if (termId) {
            const dims = fa.proposeDimensions();
            if (dims) agentResizeTerminal(termId, dims.cols, dims.rows).catch(() => {});
          }
        } catch (_) {}
      }, RESIZE_DEBOUNCE_MS);
    }).observe(container);

    const entry = { term: t, fitAddon: fa, searchAddon: sa, container, terminalId: null as string | null, _exitBuffer: '' };

    agentTerminalMap.update(m => { m.set(sessionId, entry); return new Map(m); });
    return entry;
  }

  function showTermEntry(entry: { term: Terminal; fitAddon: FitAddon; searchAddon: SearchAddon; container: HTMLDivElement; terminalId: string | null }) {
    if (activeTermEntry && activeTermEntry !== entry) {
      activeTermEntry.container.classList.add('agent-term-hidden');
      try { activeTermEntry.term.options.scrollback = 1000; } catch (_) {}
      if (termFindOpen) try { activeTermEntry.searchAddon.clearDecorations(); } catch { /* ignore */ }
    }
    entry.container.classList.remove('agent-term-hidden');
    try { entry.term.options.scrollback = 10000; } catch (_) {}
    activeTermEntry = entry;
    requestAnimationFrame(() => {
      try { entry.fitAddon.fit(); } catch (_) {}
      if (termFindOpen) {
        termFindInputEl?.focus();
        if (termFindQuery) {
          termFindResultIndex = -1;
          termFindResultCount = 0;
          try {
            const found = entry.searchAddon.findNext(termFindQuery, searchOpts(termFindCaseSensitive, termFindRegex, termFindWholeWord));
            termFindNoMatch = !found;
          } catch { termFindNoMatch = true; }
        }
      } else {
        try { entry.term.focus(); } catch (_) {}
      }
    });
    scheduleFitRetries();
  }

  function createShellEntry(sessionId: string): { term: Terminal; fitAddon: FitAddon; searchAddon: SearchAddon; container: HTMLDivElement; terminalId: string | null } {
    const t = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", "SF Mono", "Menlo", monospace',
      theme: getCurrentTermTheme(),
      scrollback: 5000,
      lineHeight: 1.35,
      smoothScrollDuration: 100,
      rescaleOverlappingGlyphs: true,
      cursorStyle: 'bar',
      cursorInactiveStyle: 'outline',
      rightClickSelectsWord: true,
    });
    const fa = new FitAddon();
    const sa = new SearchAddon();
    t.loadAddon(fa);
    t.loadAddon(sa);

    const container = document.createElement('div');
    container.style.cssText = 'width:100%;height:100%;';
    shellEl.appendChild(container);
    t.open(container);
    loadWebGLAddon(t);

    sa.onDidChangeResults(({ resultIndex, resultCount }) => {
      if (activeShellEntry?.container !== container) return;
      shellFindResultIndex = resultIndex;
      shellFindResultCount = resultCount;
    });

    t.attachCustomKeyEventHandler((e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'f' && e.type === 'keydown') {
        openShellFind();
        return false;
      }
      if (e.key === 'Escape' && e.type === 'keydown' && shellFindOpen) {
        closeShellFind();
        return false;
      }
      return true;
    });

    // Safety fallback — if first data never arrives, drop the loader after 3s
    setTimeout(() => {
      shellLoadingSessions = shellLoadingSessions.filter(id => id !== sessionId);
    }, AGENT_SHELL_LOADER_MS);

    const writeShellData = (data: string) => {
      if (!data) return;
      const sIds = get(agentShellIds);
      const shellId = sIds.get(sessionId);
      if (shellId) {
        agentWriteToTerminal(shellId, data).catch(() => {
          // Shell process died
          agentShellIds.update(m => { m.delete(sessionId); return new Map(m); });
          agentShellOpen.set(false);
          refitAll();
        });
      }
    };
    const shellImeState: ImeCoalesceState = { last: '', at: 0 };
    t.onData((rawData: string) => {
      const data = coalesceGrowingImeCommit(rawData, shellImeState);
      clearXtermTextarea(t);
      writeShellData(data);
    });

    let shellResizeTimer: ReturnType<typeof setTimeout> | null = null;
    new ResizeObserver(() => {
      if (!fa || container.offsetWidth <= 0) return;
      if (shellResizeTimer) clearTimeout(shellResizeTimer);
      shellResizeTimer = setTimeout(() => {
        shellResizeTimer = null;
        try {
          fa.fit();
          const sIds = get(agentShellIds);
          const shellId = sIds.get(sessionId);
          if (shellId) {
            const dims = fa.proposeDimensions();
            if (dims) agentResizeTerminal(shellId, dims.cols, dims.rows).catch(() => {});
          }
        } catch (_) {}
      }, RESIZE_DEBOUNCE_MS);
    }).observe(container);

    const sEntry = { term: t, fitAddon: fa, searchAddon: sa, container, terminalId: null as string | null };
    agentShellMap.update(m => { m.set(sessionId, sEntry); return new Map(m); });
    return sEntry;
  }

  function showShellEntry(sEntry: { term: Terminal; fitAddon: FitAddon; searchAddon: SearchAddon; container: HTMLDivElement; terminalId: string | null }) {
    if (activeShellEntry && activeShellEntry !== sEntry) {
      activeShellEntry.container.classList.add('agent-term-hidden');
      try { activeShellEntry.term.options.scrollback = 500; } catch (_) {}
      if (shellFindOpen) try { activeShellEntry.searchAddon.clearDecorations(); } catch { /* ignore */ }
    }
    sEntry.container.classList.remove('agent-term-hidden');
    try { sEntry.term.options.scrollback = 5000; } catch (_) {}
    activeShellEntry = sEntry;
    requestAnimationFrame(() => {
      try { sEntry.fitAddon.fit(); } catch (_) {}
      if (shellFindOpen) {
        shellFindInputEl?.focus();
        if (shellFindQuery) {
          shellFindResultIndex = -1;
          shellFindResultCount = 0;
          try {
            const found = sEntry.searchAddon.findNext(shellFindQuery, searchOpts(shellFindCaseSensitive, shellFindRegex, shellFindWholeWord));
            shellFindNoMatch = !found;
          } catch { shellFindNoMatch = true; }
        }
      }
    });
    scheduleFitRetries();
  }

  // Debounced fit triggered by container resize or store changes.
  // Belt-and-suspenders on top of the per-entry ResizeObservers — those
  // observe the inner xterm container; this one fires when the outer wrapper
  // (terminalEl / shellEl) changes size, e.g. when agentShellOpen closes.
  // True when the active session's terminal is being size-driven by the phone.
  // While so, the desktop must NOT re-fit the main xterm to the pane (that
  // would undo the adopted narrow size); the adopt $effect owns its size.
  function activeTermPhoneOwned(): boolean {
    const session = get(activeAgentSession);
    const termId = session ? get(agentTerminalIds).get(session.id) ?? null : null;
    return !!termId && get(phoneDrivenSizes).has(termId);
  }

  let _fitTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleFit() {
    if (_fitTimer !== null) clearTimeout(_fitTimer);
    _fitTimer = setTimeout(() => {
      _fitTimer = null;
      if (!activeTermPhoneOwned()) try { activeTermEntry?.fitAddon?.fit(); } catch (_) {}
      try { activeShellEntry?.fitAddon?.fit(); } catch (_) {}
    }, 60);
  }

  function scheduleFitRetries() {
    // Multiple delayed fits to catch slow-settling layouts (fresh launch,
    // mode-switch with animations, etc.).
    scheduleFit(); // 60ms default
    setTimeout(() => {
      if (!activeTermPhoneOwned()) try { activeTermEntry?.fitAddon.fit(); } catch {}
      try { activeShellEntry?.fitAddon.fit(); } catch {}
    }, 200);
    setTimeout(() => {
      if (!activeTermPhoneOwned()) try { activeTermEntry?.fitAddon.fit(); } catch {}
      try { activeShellEntry?.fitAddon.fit(); } catch {}
    }, 500);
  }

  function refitAll(sendPtyResize = false) {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        try {
          const tIds = get(agentTerminalIds);
          const session = get(activeAgentSession);
          const termId = session ? tIds.get(session.id) ?? null : null;
          // Phone-owned: the phone drives the size; don't refit or drive the
          // PTY (the adopt $effect keeps our xterm matching the phone).
          if (!(termId && get(phoneDrivenSizes).has(termId))) {
            activeTermEntry?.fitAddon?.fit();
            if (sendPtyResize && activeTermEntry?.fitAddon && termId) {
              const dims = activeTermEntry.fitAddon.proposeDimensions();
              if (dims) agentResizeTerminal(termId, dims.cols, dims.rows).catch(() => {});
            }
          }
        } catch (_) {}
        try {
          activeShellEntry?.fitAddon?.fit();
          if (sendPtyResize && activeShellEntry?.fitAddon) {
            const sIds = get(agentShellIds);
            const session = get(activeAgentSession);
            if (session) {
              const shellId = sIds.get(session.id);
              if (shellId) {
                const dims = activeShellEntry.fitAddon.proposeDimensions();
                if (dims) agentResizeTerminal(shellId, dims.cols, dims.rows).catch(() => {});
              }
            }
          }
        } catch (_) {}
      });
    });
  }

  async function spawnShellForSession(session: any) {
    if (!shellEl) return;
    const sMap = get(agentShellMap);
    let sEntry = sMap.get(session.id);
    const sIds = get(agentShellIds);

    if (sEntry && sIds.get(session.id)) {
      // Re-attach container if orphaned (shell toggle destroys/re-creates shellEl)
      if (sEntry.container.parentElement !== shellEl) {
        shellEl.appendChild(sEntry.container);
      }
      showShellEntry(sEntry);
      return;
    }
    // Fresh spawn — show loader until shell prints first prompt
    if (!shellLoadingSessions.includes(session.id)) {
      shellLoadingSessions = [...shellLoadingSessions, session.id];
    }
    if (!sEntry) {
      sEntry = createShellEntry(session.id);
    } else {
      sEntry.term.clear();
    }
    showShellEntry(sEntry);

    const projectPath = session.worktreePath || session.projectPath;
    const channel = new Channel();
    let shellFirstData = false;
    channel.onmessage = (msg: any) => {
      if (!msg.data) return;
      const bytes = base64ToBytes(msg.data);
      sEntry!.term.write(bytes);
      if (!shellFirstData) {
        shellFirstData = true;
        deferUntilFrame(() => {
          shellLoadingSessions = shellLoadingSessions.filter(id => id !== session.id);
        });
      }
    };
    try {
      const shellTermId = await agentSpawnShell(projectPath, channel);
      agentShellIds.update(m => { m.set(session.id, shellTermId); return new Map(m); });
    } catch (e) {
      sEntry.term.write(`\r\nFailed to spawn shell: ${e}\r\n`);
    }
  }

  function stopContextUsagePolling() {
    if (contextUsageInterval) {
      clearInterval(contextUsageInterval);
      contextUsageInterval = null;
    }
  }

  function startContextUsagePolling(session: any) {
    stopContextUsagePolling();
    const projectPath = session.worktreePath || session.projectPath;
    contextUsageInterval = setInterval(() => {
      const s = get(activeAgentSession);
      if (!s || s.id !== session.id) { stopContextUsagePolling(); return; }
      if (s.claudeSessionId) {
        refreshAgentContextUsage(s.id, projectPath, s.claudeSessionId, s.provider || 'claude');
      }
    }, AGENT_CONTEXT_USAGE_INTERVAL_MS);
  }

  let _spawnLock = false;
  // Per-session generation: invalidates stale Channel handlers when the SAME session respawns.
  // Global generation was wrong — it blocked writes from other sessions' tabs.
  const _spawnGenerations = new Map<string, number>();

  // Per-cwd capture lock. New-session spawns at the SAME spawnPath
  // serialize against an in-flight capture so the capture-poll for an
  // earlier sibling can't accidentally claim a later sibling's jsonl
  // (the rapid-double-launch race on shared cwds — non-git folders,
  // or git repos where worktree creation failed). Each entry resolves
  // when that spawn's capture-poll persists claudeSessionId (or after
  // a safety timeout if the spawn dies before capture). Per-session
  // worktrees give unique spawnPaths so this is a no-op for them.
  const _pendingCaptureByPath = new Map<string, Promise<void>>();

  async function selectSession(session: any) {
    console.log(`[TERM] selectSession called: id=${session?.id}, title=${session?.title}`);
    if (!session || !terminalEl) { console.log('[TERM] SKIP: no session or terminalEl'); return; }
    if (_spawnLock && currentSessionId === session.id) { console.log('[TERM] SKIP: spawnLock active for same session'); return; }
    currentSessionId = session.id;

    const tMap = get(agentTerminalMap);
    let entry = tMap.get(session.id);
    const tIds = get(agentTerminalIds);
    console.log(`[TERM] hasEntry=${!!entry}, hasTermId=${!!tIds.get(session.id)}, termId=${tIds.get(session.id) || 'none'}`);

    if (entry && tIds.get(session.id)) {
      // Re-attach container if orphaned (after mode switch destroys/re-creates DOM)
      if (entry.container.parentElement !== terminalEl) {
        terminalEl.appendChild(entry.container);
      }
      console.log('[TERM] EARLY RETURN: reusing existing running terminal');
      termReady = true;
      spawning = false;
      showTermEntry(entry);
      if (session.claudeSessionId) startContextUsagePolling(session);
      if (get(agentShellOpen)) {
        const sMap = get(agentShellMap);
        const sEntry = sMap.get(session.id);
        if (sEntry && shellEl && sEntry.container.parentElement !== shellEl) {
          shellEl.appendChild(sEntry.container);
        }
        spawnShellForSession(session);
      }
      refreshAgentGitStatus();
      return;
    }

    // Gate on the session's CLI being installed before creating any
    // terminal state. The provider lives on the session row; default
    // to 'claude' for legacy sessions written before the column existed.
    // Each provider has its own install guide modal — using the Claude
    // one for a Codex/Gemini/OpenCode session would tell the user to
    // install the wrong tool.
    const provider = (session.provider ?? 'claude') as 'claude' | 'codex' | 'gemini' | 'opencode';
    try {
      const installed = provider === 'claude'
        ? await agentCheckClaudeInstalled()
        : await agentCheckCliInstalled(provider);
      if (!installed) {
        notInstalledProvider = provider;
        showNotInstalled = true;
        spawning = false;
        return;
      }
    } catch (_) {
      // If the check itself fails (e.g. IPC not ready), let the spawn
      // proceed and surface any real error from the terminal itself.
    }

    const currentGen = (_spawnGenerations.get(session.id) || 0) + 1;
    _spawnGenerations.set(session.id, currentGen);
    console.log(`[TERM] SPAWNING NEW terminal for session ${session.id}, gen=${currentGen}`);
    // Kill old PTY process if still lingering
    const oldTermId = get(agentTerminalIds).get(session.id);
    if (oldTermId) {
      console.log(`[TERM] Killing old PTY: ${oldTermId}`);
      agentKillTerminal(oldTermId).catch(() => {});
      agentTerminalIds.update(m => { m.delete(session.id); return new Map(m); });
    }

    // Per-session generation already incremented above

    // Always dispose old xterm and create fresh — prevents stale data from old Channels
    if (entry) {
      entry.container.remove();
      entry.term.dispose();
      agentTerminalMap.update(m => { m.delete(session.id); return new Map(m); });
    }

    termReady = false;
    spawning = true;
    _spawnLock = true;
    entry = createTermEntry(session.id);
    showTermEntry(entry);

    // Hoisted outside the try so the catch can release the per-cwd
    // capture lock on spawn failure. Reassigned inside the try with
    // the real values once spawnPath / captureDone are known.
    let spawnPath: string = session.worktreePath || session.projectPath;
    let captureResolver: () => void = () => {};
    let captureDone: Promise<void> | null = null;

    try {
      await agentUpdateLastUsed(session.id);

      // (spawnPath is hoisted above so the catch block can use it)

      // Inject attached contexts into the file this CLI actually reads
      // — CLAUDE.md for Claude, AGENTS.md for Codex / OpenCode.
      try {
        const sessionContexts = await agentGetSessionContexts(session.id);
        if (sessionContexts.length > 0) {
          const contextIds = sessionContexts.map((c: any) => c.id);
          await agentInjectContexts(spawnPath, contextIds, session.provider || 'claude');
        }
      } catch (_) {}

      // Auto-create worktree for new sessions in git repos. Branch
      // name includes a short prefix of this session's UUID so two
      // sessions can never collide on the same worktree path
      // regardless of title — that was the root cause of the
      // "two Custom sessions, one loses context after restart" alpha
      // report: identical title slugs produced the same branch name,
      // `agent_create_worktree` reused the existing dir, both sessions
      // ended up sharing one Claude projects directory, the capture
      // loop then raced to claim the same `<id>.jsonl`. With UUID in
      // the branch, each session gets its own dedicated worktree → its
      // own encoded Claude dir → exactly one session file in there →
      // capture is unambiguous and resume after restart always finds
      // the right file. Title is kept as a readable suffix.
      if (!session.worktreePath && !session.claudeSessionId) {
        try {
          const isGit = await agentIsGitRepo(session.projectPath);
          if (isGit) {
            const uuidShort = session.id.replace(/-/g, '').slice(0, 8);
            const titleSlug = session.title.toLowerCase().replace(/\s+/g, '-');
            const purposeSlug = session.purpose.toLowerCase().replace(/\s+/g, '-');
            const rawBranch = `clauge/${purposeSlug}-${titleSlug}-${uuidShort}`;
            const branchName = rawBranch.replace(/[^a-zA-Z0-9/_\-.]/g, '').replace(/\.{2,}/g, '.').replace(/\.lock/g, '');
            const worktreePath = await agentCreateWorktree(session.projectPath, branchName);
            spawnPath = worktreePath;
            await agentUpdateWorktree(session.id, worktreePath, branchName);
            session.worktreePath = worktreePath;
            session.worktreeBranch = branchName;
            await loadAgentSessions();
          }
        } catch (e) {
          console.warn('Worktree creation failed, using original path:', e);
        }
      }

      // Per-cwd capture lock — register first (synchronously, so a
      // concurrent third spawn sees us as its predecessor rather than
      // chaining behind whoever WE waited on), then await any prior
      // pending capture. Only runs for brand-new sessions (no stored
      // claudeSessionId); resume paths don't compete for a new jsonl.
      // captureResolver / captureDone are declared in the outer scope
      // so the catch block can release on spawn failure.
      let priorCapture: Promise<void> | undefined;
      if (!session.claudeSessionId) {
        priorCapture = _pendingCaptureByPath.get(spawnPath);
        captureDone = new Promise<void>((r) => { captureResolver = r; });
        _pendingCaptureByPath.set(spawnPath, captureDone);
        // Safety release — if our own capture never fires (broken PTY,
        // killed early, capture-poll exhausts all attempts) siblings
        // shouldn't wait forever. AGENT_SESSION_CAPTURE_INTERVAL_MS *
        // 10 + slack = ~35s upper bound for a legit capture; round up.
        setTimeout(() => {
          if (_pendingCaptureByPath.get(spawnPath) === captureDone) {
            _pendingCaptureByPath.delete(spawnPath);
          }
          captureResolver();
        }, 40_000);
      }
      if (priorCapture) {
        // Cap the wait at 10s — if the prior spawn is genuinely slow we
        // accept a small re-introduction of the race rather than block
        // the user's click for the full safety-timeout. In practice
        // capture-poll fires within 3-6s on a healthy machine.
        await Promise.race([
          priorCapture.catch(() => {}),
          new Promise<void>((r) => setTimeout(r, 10_000)),
        ]);
      }

      // Get existing session IDs BEFORE spawning. Snapshot is taken
      // AFTER the per-cwd lock so it includes any jsonl the previous
      // sibling's capture-poll wrote during our wait. The
      // sibling-claim union still adds defense for the case where
      // capture-poll fired after disk write but before we resnapshot.
      let existingSessionIds: string[] = [];
      if (!session.claudeSessionId) {
        try {
          const existing = await agentDiscoverSessions(spawnPath, session.provider || 'claude');
          existingSessionIds = existing.map((s: any) => s.sessionId);
        } catch (_) {}
        const claimedBySiblings = get(agentSessions)
          .filter((o) => o.id !== session.id && !!o.claudeSessionId)
          .map((o) => o.claudeSessionId as string);
        existingSessionIds = Array.from(new Set([...existingSessionIds, ...claimedBySiblings]));
      }

      let outputReceived = false;
      let activityTimer: ReturnType<typeof setTimeout> | null = null;
      let activityBytes = 0;       // Rolling byte counter for activity detection
      let activityWindow: ReturnType<typeof setTimeout> | null = null;
      let lastUserInput = 0;       // Timestamp of last user keystroke
      const spawnTime = Date.now(); // Used to ignore exit detection during initial banner/history output
      const sessionId = session.id;
      const myGeneration = currentGen; // Capture per-session generation
      console.log(`[TERM] Creating Channel, gen=${myGeneration}, about to call agentSpawnTerminal`);
      const onOutput = new Channel();

      let firstDataSeen = false;
      onOutput.onmessage = (payload: any) => {
        // Ignore data from old/stale Channel if a newer spawn happened FOR THIS SESSION
        const latestGen = _spawnGenerations.get(sessionId) || 0;
        if (myGeneration !== latestGen) { console.log(`[TERM] STALE Channel write blocked: myGen=${myGeneration}, currentGen=${latestGen}`); return; }
        // First PTY data: defer the loader hide by two animation frames so xterm
        // has time to actually paint the bytes — flipping immediately leaves a
        // visible blank gap because xterm batches writes into the next frame.
        if (!firstDataSeen && payload.data && payload.exit !== true) {
          firstDataSeen = true;
          deferUntilFrame(() => {
            if (spawning) { spawning = false; termReady = true; }
          });
        }

        // PTY-close signal from Rust: reader thread sends { exit: true } when the
        // child process dies. This is the authoritative exit signal — no text matching
        // needed. Triggered when user types "exit" + Enter and Claude Code exits.
        if (payload.exit === true) {
          // Suppression is keyed per-terminal-id. If Clauge initiated this
          // kill (reset/relaunch/close-tab/delete), the termId was added
          // to the set before the kill. .delete returns true iff it was
          // present — single-shot consume. No timer, no global flag, no
          // interference with concurrent kills of other sessions.
          const thisTermId = entry?.terminalId ?? null;
          const wasSuppressed = thisTermId ? _suppressedTerminalIds.delete(thisTermId) : false;
          console.log(`[TERM] EXIT signaled by PTY close for session ${sessionId}, gen=${myGeneration}, suppressed=${wasSuppressed}`);
          agentTerminalIds.update(m => { m.delete(sessionId); return new Map(m); });
          const tMapNow = get(agentTerminalMap);
          const exitedEntry = tMapNow.get(sessionId);
          // Capture the resume id printed in the exit banner so the
          // next spawn can target this exact session.
          //   Claude:      `claude --resume <uuid>`
          //   Antigravity: `agy --conversation=<uuid>` or `agy --conversation <uuid>`
          // Both flow into `claudeSessionId` (column-name legacy — it
          // holds whatever resume id the provider needs).
          if (entry && entry._exitBuffer && !session.claudeSessionId) {
            const resumeMatch =
              entry._exitBuffer.match(/claude --resume ([a-f0-9-]{36})/) ||
              entry._exitBuffer.match(/agy --conversation[ =]([a-f0-9-]{36})/);
            if (resumeMatch) {
              session.claudeSessionId = resumeMatch[1];
              agentUpdateSessionId(session.id, resumeMatch[1]).catch(() => {});
            }
          }
          if (entry) entry._exitBuffer = '';
          agentSessionActivity.update(m => { m.set(sessionId, 'done'); return new Map(m); });
          // Exited session can no longer be awaiting input (backend also emits
          // agent-attention-cleared on exit; this is local cleanup).
          clearAwaiting(sessionId);
          notifyBuffers.delete(sessionId);
          notifyLastTimes.delete(sessionId);
          // Always dispose the xterm entry on exit. Re-opening the session
          // from the sidebar will auto-spawn a fresh CLI (with --resume if
          // claudeSessionId is set), so preserving scrollback in a dead
          // xterm just to overlay a "Session ended" banner is dead weight.
          if (exitedEntry) {
            try { exitedEntry.container.remove(); } catch (_) {}
            try { exitedEntry.term.dispose(); } catch (_) {}
            agentTerminalMap.update(m => { m.delete(sessionId); return new Map(m); });
          }
          // Only clear activeTermEntry if THIS session's entry was the active one.
          // Otherwise we silently lose the reference to the actually-displayed
          // session and the next showTermEntry won't hide its container — leaving
          // two terminals visible (cross-session leak).
          if (activeTermEntry === exitedEntry) activeTermEntry = null;
          if (currentSessionId === sessionId) currentSessionId = null;
          const allTabs = get(tabsStore);
          const exitedTab = allTabs.find(t => t.mode === 'agent' && t.key === sessionId);
          const currentActive = get(activeAgentSession);
          if (exitedTab) closeTab(exitedTab.id);
          if (currentActive?.id === sessionId) activeAgentSession.set(null);
          return;
        }

        // Write data to terminal
        if (entry!.term) {
          try {
            entry!.term.write(base64ToBytes(payload.data));
          } catch (_) {}
        }

        // Check for action-required prompts and notify (per-session)
        handleTerminalOutput(sessionId, payload.data);

        // Accumulate raw output into a rolling 500-char buffer. Used at
        // PTY-close time to extract the "claude --resume <id>" token as
        // a fallback resume-id capture when the post-spawn poll didn't
        // get to discover the .jsonl file. The previous regex-based
        // "session ended" / farewell detection ran here too, but was
        // unreliable (matched on any in-response mention of the phrase)
        // and shipped a misleading banner — removed entirely. PTY-close
        // (payload.exit === true) and write-failure (line ~418) remain
        // as the only authoritative exit signals.
        if (!entry!._exitBuffer) entry!._exitBuffer = '';
        try {
          entry!._exitBuffer += atob(payload.data);
          if (entry!._exitBuffer.length > 500) entry!._exitBuffer = entry!._exitBuffer.slice(-500);
        } catch (_) {}

        // Track activity — only mark 'running' if sustained output (not just echo/redraw)
        // Count bytes in a rolling 500ms window. Claude Code generating output produces
        // hundreds of bytes/sec. Echo/redraws are < 50 bytes total.
        {
          const payloadSize = payload.data?.length || 0;
          activityBytes += payloadSize;

          if (!activityWindow) {
            activityWindow = setTimeout(() => {
              activityWindow = null;
              if (activityBytes > 200) {
                // Substantial output — Claude Code is actively working
                agentSessionActivity.update(m => { m.set(sessionId, 'running'); return new Map(m); });
              }
              activityBytes = 0;
            }, AGENT_ACTIVITY_WINDOW_MS);
          }

          if (activityTimer) clearTimeout(activityTimer);
          activityTimer = setTimeout(() => {
            const act = get(agentSessionActivity);
            if (act.get(sessionId) === 'running') {
              agentSessionActivity.update(m => { m.set(sessionId, 'done'); return new Map(m); });
            }
          }, AGENT_ACTIVITY_DONE_MS);
        }

        // Capture session ID — retry every 3s until found (up to 30s)
        if (!outputReceived && !session.claudeSessionId) {
          outputReceived = true;
          let attempts = 0;
          const captureInterval = setInterval(async () => {
            attempts++;
            if (attempts > 10 || session.claudeSessionId) { clearInterval(captureInterval); return; }
            try {
              const allSessions = await agentDiscoverSessions(spawnPath, session.provider || 'claude');
              // Re-check the "claimed by siblings" set on every poll
              // tick — a concurrently-spawning session might have
              // claimed an id since we took the snapshot at spawn
              // time. Without this re-check, two simultaneous spawns
              // in the same encoded Claude dir would both pick the
              // most-recent file and end up sharing claudeSessionId.
              const claimedNow = new Set<string>([
                ...existingSessionIds,
                ...get(agentSessions)
                  .filter((o) => o.id !== session.id && !!o.claudeSessionId)
                  .map((o) => o.claudeSessionId as string),
              ]);
              const newSession = allSessions.find((s: any) => !claimedNow.has(s.sessionId));
              if (newSession) {
                await agentUpdateSessionId(session.id, newSession.sessionId);
                session.claudeSessionId = newSession.sessionId;
                await loadAgentSessions();
                clearInterval(captureInterval);
                // Release the per-cwd capture lock so any pending
                // sibling spawn can proceed and re-snapshot the disk
                // (which now includes our jsonl).
                if (captureDone && _pendingCaptureByPath.get(spawnPath) === captureDone) {
                  _pendingCaptureByPath.delete(spawnPath);
                }
                captureResolver();

                // Start context usage polling now that we have a session ID
                if (contextUsageInterval) clearInterval(contextUsageInterval);
                contextUsageInterval = setInterval(() => {
                  const s = get(activeAgentSession);
                  if (s?.claudeSessionId) {
                    const path = s.worktreePath || s.projectPath;
                    refreshAgentContextUsage(s.id, path, s.claudeSessionId, s.provider || 'claude');
                  }
                }, AGENT_CONTEXT_USAGE_INTERVAL_MS);
              }
            } catch (_) {}
          }, AGENT_SESSION_CAPTURE_INTERVAL_MS);
        }
      };

      // Flatten prompt to single line for shell compatibility (matches original Clauge)
      // Use frontend purpose prompt for fixed purposes, fall back to stored prompt for Custom
      const rawPrompt = getPurposePrompt(session.purpose) || session.contextPrompt || '';
      const purposePrompt = rawPrompt.replace(/\n+/g, ' ').replace(/\s+/g, ' ').trim();

      // Gemini and OpenCode: write the purpose prompt into the
      // project's agent file (GEMINI.md / AGENTS.md) before spawn.
      // Claude and Codex have real system-prompt CLI flags
      // (`--append-system-prompt` / `-c instructions=`) so their
      // persona travels via the spawn command instead — no file
      // write needed. Without this, Gemini used to run the persona
      // as the user's first message every spawn (the "starts working
      // without waiting" bug), and OpenCode silently ignored Custom-
      // purpose prompts entirely. The Rust side is no-op for any
      // other provider, so the call is safe to make unconditionally
      // and we keep the branch explicit only to skip the IPC round
      // trip on Claude/Codex.
      if (session.provider === 'gemini' || session.provider === 'opencode') {
        try {
          await agentInjectPurpose(spawnPath, session.provider, purposePrompt);
        } catch (e) {
          console.warn(`[TERM] ${session.provider} purpose-prompt injection failed:`, e);
        }
      }

      // Resume contract: the row's own claude_session_id IS the source
      // of truth. Set → resume that conversation. Null → spawn fresh,
      // let the capture-poll below claim the new jsonl and persist its
      // id. We deliberately do NOT reach to disk to "rehydrate" a
      // missing id by guessing from orphan jsonls at this cwd —
      // sibling sessions on non-git folders, freshly-cloud-restored
      // rows, and leftover jsonls from previously-deleted sessions
      // all share that cwd and were getting silently adopted, surfacing
      // a deleted (or sibling) conversation under a brand-new session.
      // Genuine crash-recovery (row lost its id, jsonl still on disk)
      // is now driven explicitly by the user via the Custom-purpose
      // "Resume existing session" picker in NewSessionModal.
      let resumeId = session.claudeSessionId || undefined;

      spawning = true;
      const termId = await agentSpawnTerminal({
        sessionId: resumeId,
        rowId: session.id,
        projectPath: spawnPath,
        contextPrompt: purposePrompt || undefined,
        skipPermissions: session.skipPermissions === 1 || undefined,
        gitName: session.gitName || undefined,
        gitEmail: session.gitEmail || undefined,
        // Pull the provider off the session row so Codex / OpenCode
        // sessions launch their own CLI instead of falling back to
        // Claude. Legacy rows pre-migration-13 have provider='claude'.
        provider: session.provider || 'claude',
        // Per-session custom binary path (Advanced > Custom Binary
        // Path in New/Edit Session). When set, the runner uses this
        // shell-quoted path in place of the bare binary name. NULL /
        // empty = standard $PATH lookup.
        binaryPath: session.binaryPath || undefined,
        // Codex workspace MCP token injection is backend-owned so the
        // token doesn't have to flow through frontend IPC.
        onOutput,
      });
      console.log(`[TERM] Spawn complete: termId=${termId}, gen=${myGeneration}`);
      agentTerminalIds.update(m => { m.set(session.id, termId); return new Map(m); });

      // Start context usage polling (Feature 2)
      startContextUsagePolling(session);

      // Visual fit only — ResizeObserver handles debounced PTY resize
      requestAnimationFrame(() => {
        try { entry!.fitAddon.fit(); } catch (_) {}
      });

      if (get(agentShellOpen)) spawnShellForSession(session);
      refreshAgentGitStatus();
    } catch (e) {
      entry.term.write(`\r\nFailed to spawn terminal: ${e}\r\n`);
      // Release the per-cwd lock on failure so siblings aren't blocked
      // by a spawn that died before its capture-poll could fire.
      if (captureDone && _pendingCaptureByPath.get(spawnPath) === captureDone) {
        _pendingCaptureByPath.delete(spawnPath);
      }
      captureResolver();
    } finally {
      _spawnLock = false;
    }
  }

  // Draggable divider between main terminal and shell (same pattern as AI panel)
  function handleDividerMousedown(e: MouseEvent) {
    e.preventDefault();
    dragging = true;

    const startX = e.clientX;
    const startW = mainWidth;
    const rect = wrapperEl.getBoundingClientRect();

    function onMove(e: MouseEvent) {
      const pct = ((e.clientX - rect.left) / rect.width) * 100;
      mainWidth = Math.max(25, Math.min(80, pct));
    }

    function onUp() {
      dragging = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      refitAll(true);
    }

    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }

  // React to theme changes — update all terminal instances
  const unsubAppearance = appearance.subscribe((app) => {
    if (!app) return;
    const termTheme = getTerminalTheme(app.theme, app.accentColor);
    termBg = termTheme.background || '#0d0d18';
    const tMap = get(agentTerminalMap);
    for (const [, entry] of tMap) {
      if (entry?.term) entry.term.options.theme = termTheme;
    }
    const sMap = get(agentShellMap);
    for (const [, entry] of sMap) {
      if (entry?.term) entry.term.options.theme = termTheme;
    }
  });

  // React to session changes
  const unsubSession = activeAgentSession.subscribe((session) => {
    console.log(`[TERM] SUBSCRIBER: session=${session?.id || 'null'}, currentSessionId=${currentSessionId}, match=${session?.id === currentSessionId}`);
    if (session && session.id !== currentSessionId) {
      console.log(`[TERM] SUBSCRIBER: triggering selectSession via rAF`);
      // Sync tab activation
      import('$lib/shared/stores/tabs').then(({ tabs: tabsStore, activateTab: activateTabFn }) => {
        const allTabs = get(tabsStore);
        const matchingTab = allTabs.find((t: any) => t.mode === 'agent' && t.key === session.id);
        if (matchingTab) activateTabFn(matchingTab.id);
      });
      requestAnimationFrame(() => selectSession(session));
    } else if (!session) {
      currentSessionId = null;
      if (activeTermEntry) {
        activeTermEntry.container.classList.add('agent-term-hidden');
        activeTermEntry = null;
      }
      if (activeShellEntry) {
        activeShellEntry.container.classList.add('agent-term-hidden');
        activeShellEntry = null;
      }
      // Clear git state and usage polling when no session
      agentGitBranchName.set('');
      agentGitFiles.set([]);
      agentGitAhead.set(0);
      agentGitBehind.set(0);
      if (contextUsageInterval) {
        clearInterval(contextUsageInterval);
        contextUsageInterval = null;
      }
    }
  });

  // React to shell toggle
  const unsubShell = agentShellOpen.subscribe((open) => {
    const session = get(activeAgentSession);
    if (open && session) {
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          spawnShellForSession(session);
          refitAll();
        });
      });
    } else {
      if (activeShellEntry) {
        activeShellEntry.container.classList.add('agent-term-hidden');
        activeShellEntry = null;
      }
      refitAll();
      // scheduleFit fires 60 ms after layout settles, catching any residual
      // height change that refitAll's double-rAF may still miss.
      scheduleFit();
    }
  });

  // Event handler for reset-session (Feature 4)
  function handleResetSession(e: Event) {
    const detail = (e as CustomEvent).detail;
    if (!detail?.session) return;
    const session = detail.session;

    // Kill terminal
    const tIds = get(agentTerminalIds);
    const termId = tIds.get(session.id);
    if (termId) {
      _suppressedTerminalIds.add(termId);
      agentKillTerminal(termId).catch(() => {});
    }

    // Kill shell
    const sIds = get(agentShellIds);
    const shellId = sIds.get(session.id);
    if (shellId) agentKillTerminal(shellId).catch(() => {});

    // Clear session ID. With the disk-rehydrate path removed, the next
    // spawn sees `claudeSessionId=null` and goes straight to a fresh
    // CLI session — no skip-flag dance needed.
    agentUpdateSessionId(session.id, '').catch(() => {});
    session.claudeSessionId = null;

    // Remove from maps
    agentTerminalIds.update(m => { m.delete(session.id); return new Map(m); });
    agentShellIds.update(m => { m.delete(session.id); return new Map(m); });

    // Remove terminal entry so selectSession creates fresh one
    const tMap = get(agentTerminalMap);
    const entry = tMap.get(session.id);
    if (entry) {
      entry.container.remove();
      entry.term.dispose();
      agentTerminalMap.update(m => { m.delete(session.id); return new Map(m); });
    }

    // Remove shell entry
    const sMap = get(agentShellMap);
    const sEntry = sMap.get(session.id);
    if (sEntry) {
      sEntry.container.remove();
      sEntry.term.dispose();
      agentShellMap.update(m => { m.delete(session.id); return new Map(m); });
    }

    // Reset current session ID so selectSession processes it
    currentSessionId = null;
    activeTermEntry = null;
    activeShellEntry = null;

    loadAgentSessions().then(() => {
      selectSession(session);
    });
  }

  // Event handler for relaunch — kills terminal but KEEPS session ID for --resume with updated prompt
  function handleRelaunchSession(e: Event) {
    const detail = (e as CustomEvent).detail;
    if (!detail?.session) return;
    const session = detail.session;

    // Kill terminal
    const tIds = get(agentTerminalIds);
    const termId = tIds.get(session.id);
    if (termId) {
      _suppressedTerminalIds.add(termId);
      agentKillTerminal(termId).catch(() => {});
    }

    // Kill shell
    const sIds = get(agentShellIds);
    const shellId = sIds.get(session.id);
    if (shellId) agentKillTerminal(shellId).catch(() => {});

    // Remove from maps (keep claudeSessionId for --resume)
    agentTerminalIds.update(m => { m.delete(session.id); return new Map(m); });
    agentShellIds.update(m => { m.delete(session.id); return new Map(m); });

    const tMap = get(agentTerminalMap);
    const entry = tMap.get(session.id);
    if (entry) {
      entry.container.remove();
      entry.term.dispose();
      agentTerminalMap.update(m => { m.delete(session.id); return new Map(m); });
    }

    const sMap = get(agentShellMap);
    const sEntry = sMap.get(session.id);
    if (sEntry) {
      sEntry.container.remove();
      sEntry.term.dispose();
      agentShellMap.update(m => { m.delete(session.id); return new Map(m); });
    }

    currentSessionId = null;
    activeTermEntry = null;
    activeShellEntry = null;

    // Relaunch with updated session data (prompt changes picked up from activeAgentSession)
    requestAnimationFrame(() => selectSession(session));
  }

  // Event handler for tab close — disposes UI state but does NOT kill the PTY
  // (the session stays alive in the background; user can reopen from sidebar).
  function handleCloseTabSession(e: Event) {
    const detail = (e as CustomEvent).detail;
    const sessionId = detail?.sessionId;
    if (!sessionId) return;

    // Defensive: in case the PTY happens to exit while the tab is closed,
    // add its termId so the exit handler routes through the suppressed path
    // instead of trying to close a tab that no longer exists.
    const tIdsClose = get(agentTerminalIds);
    const closingTermId = tIdsClose.get(sessionId);
    if (closingTermId) {
      _suppressedTerminalIds.add(closingTermId);
    }

    // Cleanup terminal entry
    const tMap = get(agentTerminalMap);
    const entry = tMap.get(sessionId);
    if (entry) {
      entry.container.remove();
      entry.term.dispose();
      agentTerminalMap.update(m => { m.delete(sessionId); return new Map(m); });
    }

    // Cleanup shell entry
    const sMap = get(agentShellMap);
    const sEntry = sMap.get(sessionId);
    if (sEntry) {
      sEntry.container.remove();
      sEntry.term.dispose();
      agentShellMap.update(m => { m.delete(sessionId); return new Map(m); });
    }

    agentTerminalIds.update(m => { m.delete(sessionId); return new Map(m); });
    agentShellIds.update(m => { m.delete(sessionId); return new Map(m); });

    if (activeTermEntry === entry) activeTermEntry = null;
    if (activeShellEntry === sEntry) activeShellEntry = null;
    currentSessionId = null;
  }

  // Event handler for delete-session (Feature 5)
  async function handleDeleteSession(e: Event) {
    const detail = (e as CustomEvent).detail;
    if (!detail?.session) return;
    const session = detail.session;

    // Kill terminal
    const tIds = get(agentTerminalIds);
    const termId = tIds.get(session.id);
    if (termId) {
      _suppressedTerminalIds.add(termId);
      await agentKillTerminal(termId).catch(() => {});
    }

    // Kill shell
    const sIds = get(agentShellIds);
    const shellId = sIds.get(session.id);
    if (shellId) await agentKillTerminal(shellId).catch(() => {});

    // Cleanup terminal entry
    const tMap = get(agentTerminalMap);
    const entry = tMap.get(session.id);
    if (entry) {
      entry.container.remove();
      entry.term.dispose();
      agentTerminalMap.update(m => { m.delete(session.id); return new Map(m); });
    }

    // Cleanup shell entry
    const sMap = get(agentShellMap);
    const sEntry = sMap.get(session.id);
    if (sEntry) {
      sEntry.container.remove();
      sEntry.term.dispose();
      agentShellMap.update(m => { m.delete(session.id); return new Map(m); });
    }

    agentTerminalIds.update(m => { m.delete(session.id); return new Map(m); });
    agentShellIds.update(m => { m.delete(session.id); return new Map(m); });

    // Remove worktree if exists. Surface failures via toast — silently
    // swallowing leaves an orphan directory on disk after the DB row is
    // gone, with no signal to the user.
    if (session.worktreePath) {
      try {
        await agentRemoveWorktree(session.projectPath, session.worktreePath);
      } catch (e) {
        showToast(`Worktree cleanup failed: ${friendlyError(e)}. The directory may remain at ${session.worktreePath}.`, 'error');
      }
    }

    // Delete from DB. If this fails the session ghost stays in the list
    // — better to know than to silently re-appear on next load.
    try {
      await agentDeleteSession(session.id);
    } catch (e) {
      errorToast('Failed to delete session', e);
      return;
    }

    // If this was the active session, clear it
    const currentActive = get(activeAgentSession);
    if (currentActive?.id === session.id) {
      currentSessionId = null;
      activeTermEntry = null;
      activeShellEntry = null;
      agentShellOpen.set(false);
      // Close the tab for deleted session
      import('$lib/shared/stores/tabs').then(({ tabs: tabsStore, closeTab: closeTabFn }) => {
        const allTabs = get(tabsStore);
        const agentTab = allTabs.find((t: any) => t.mode === 'agent' && t.key === session.id);
        if (agentTab) closeTabFn(agentTab.id);
      });
      activeAgentSession.set(null);
    }

    await loadAgentSessions();
  }

  onMount(async () => {
    // Load notification preferences from settings
    try {
      const [sound, dock] = await Promise.all([
        getSetting('agent_sound_enabled'),
        getSetting('agent_dock_bounce_enabled'),
      ]);
      agentSoundEnabled.set(sound !== 'false');
      agentDockBounceEnabled.set(dock !== 'false');
    } catch (_) {}

    // Authoritative attention clear from the backend: emitted whenever input
    // is sent to a session from ANY source (desktop or mobile) or it exits.
    // Reverse-look the terminalId to our sessionId via agentTerminalIds
    // (Map<sessionId, terminalId>) and clear that session's awaiting state.
    try {
      unlistenAttentionCleared = await listen<{ terminalId: string }>(
        'agent-attention-cleared',
        (event) => {
          const terminalId = event.payload?.terminalId;
          if (!terminalId) return;
          const tIds = get(agentTerminalIds);
          let matchedSessionId: string | null = null;
          for (const [sid, tid] of tIds) {
            if (tid === terminalId) { matchedSessionId = sid; break; }
          }
          if (matchedSessionId) clearAwaiting(matchedSessionId);
        },
      );
    } catch (_) {}

    // Listen for reset-session, delete-session, and close-tab-session events
    window.addEventListener(AGENT_EVENT.RESET_SESSION, handleResetSession);
    window.addEventListener(AGENT_EVENT.DELETE_SESSION, handleDeleteSession);
    window.addEventListener(AGENT_EVENT.CLOSE_TAB_SESSION, handleCloseTabSession);
    window.addEventListener(AGENT_EVENT.RELAUNCH_SESSION, handleRelaunchSession);

    // File drag-and-drop: write dropped file paths into the active terminal
    try {
      const win = getCurrentWindow();
      win.onDragDropEvent((event) => {
        if (event.payload.type === 'drop') {
          const paths = event.payload.paths;
          if (!paths?.length) return;
          const session = get(activeAgentSession);
          if (!session) return;
          const tIds = get(agentTerminalIds);
          const termId = tIds.get(session.id);
          if (termId) {
            const pathStr = paths.map((p: string) => p.includes(' ') ? `"${p}"` : p).join(' ');
            agentWriteToTerminal(termId, pathStr + ' ');
          }
        }
      }).then((unlisten) => {
        unlistenFileDrop = unlisten;
      });
    } catch (_) {}

    // Re-attach terminal after mode switch (component re-mount)
    const currentSession = get(activeAgentSession);
    console.log(`[TERM] onMount: currentSession=${currentSession?.id || 'null'}`);
    if (currentSession) {
      console.log(`[TERM] onMount: re-attaching session ${currentSession.id}`);
      currentSessionId = null; // Force selectSession to process
      requestAnimationFrame(() => selectSession(currentSession));
    }

    // Watch outer wrapper containers for size changes — fires when agentShellOpen
    // toggles (display:none removes the panel, terminalEl expands) and on any
    // other host-driven resize not covered by the per-entry ResizeObservers.
    _containerResizeObserver = new ResizeObserver(() => scheduleFit());
    if (terminalEl) _containerResizeObserver.observe(terminalEl);
    if (shellEl) _containerResizeObserver.observe(shellEl);

    // Fresh launch gets retry chain too
    scheduleFitRetries();
  });

  onDestroy(() => {
    _containerResizeObserver?.disconnect();
    _containerResizeObserver = null;
    if (_fitTimer !== null) { clearTimeout(_fitTimer); _fitTimer = null; }
    unsubSession();
    unsubShell();
    unsubAppearance();
    stopContextUsagePolling();
    // Stop the chime repeat and cancel any in-flight dock bounce so a bounce
    // doesn't linger after the panel unmounts.
    stopAlert();
    window.removeEventListener(AGENT_EVENT.RESET_SESSION, handleResetSession);
    window.removeEventListener(AGENT_EVENT.DELETE_SESSION, handleDeleteSession);
    window.removeEventListener(AGENT_EVENT.CLOSE_TAB_SESSION, handleCloseTabSession);
    window.removeEventListener(AGENT_EVENT.RELAUNCH_SESSION, handleRelaunchSession);
    if (unlistenFileDrop) unlistenFileDrop();
    if (unlistenAttentionCleared) unlistenAttentionCleared();
  });
</script>

<ProviderNotInstalledModal bind:show={showNotInstalled} provider={notInstalledProvider} />

{#if $activeAgentSession}
  <div class="agent-panel" bind:this={wrapperEl}>
    <div class="agent-terminal-main" style="width:{$agentShellOpen ? mainWidth + '%' : '100%'}">
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
        {@const _prov = $activeAgentSession?.provider ?? 'claude'}
        {@const _src = _prov === 'codex' ? '/codex.svg'
                     : _prov === 'gemini' ? '/gemini.svg'
                     : _prov === 'opencode' ? '/opencode-dark.svg'
                     : '/code-in-action.svg'}
        {@const _name = _prov === 'codex' ? 'Codex'
                      : _prov === 'gemini' ? 'Antigravity'
                      : _prov === 'opencode' ? 'OpenCode'
                      : 'Claude Code'}
        <div class="agent-loading">
          <img src={_src} alt="" class="loading-mascot" />
          <div class="loading-text">
            <span class="loading-title">Starting {_name}</span>
            <span class="loading-sub">Setting up terminal session<span class="loading-dots"></span></span>
          </div>
        </div>
      {/if}
      {#if termFindOpen}
        <div class="agent-find-bar">
          <!-- svelte-ignore a11y_autofocus -->
          <input
            bind:this={termFindInputEl}
            value={termFindQuery}
            oninput={onTermFindInput}
            onkeydown={onTermFindKeydown}
            onblur={() => { try { activeTermEntry?.searchAddon.clearActiveDecoration(); } catch { /* ignore */ } }}
            class="agent-find-input"
            class:no-match={termFindNoMatch}
            placeholder="Find in terminal…"
            spellcheck={false}
            autocomplete="off"
          />
          <div class="agent-find-sep"></div>
          <button class="agent-find-toggle" class:active={termFindCaseSensitive} onclick={toggleTermCase} title="Case sensitive (Aa)">Aa</button>
          <button class="agent-find-toggle" class:active={termFindRegex}         onclick={toggleTermRegex} title="Use regular expression (.*)">.*</button>
          <button class="agent-find-toggle" class:active={termFindWholeWord}     onclick={toggleTermWord}  title="Match whole word">W</button>
          <div class="agent-find-sep"></div>
          {#if termFindQuery}
            <span class="agent-find-count" class:no-results={termFindNoMatch}>
              {#if termFindNoMatch}No results
              {:else if termFindResultCount > 0}{termFindResultIndex === -1 ? `${termFindResultCount}+` : `${termFindResultIndex + 1} / ${termFindResultCount}`}
              {/if}
            </span>
          {/if}
          <button class="agent-find-btn" onclick={doTermFindPrev} title="Previous (Shift+Enter)">
            <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="18 15 12 9 6 15"/></svg>
          </button>
          <button class="agent-find-btn" onclick={doTermFindNext} title="Next (Enter)">
            <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="6 9 12 15 18 9"/></svg>
          </button>
          <div class="agent-find-sep"></div>
          <button class="agent-find-close" onclick={closeTermFind} title="Close (Esc)">
            <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>
      {/if}
      <div class="agent-terminal-container" class:term-hidden={!termReady || activePhoneOwned} bind:this={terminalEl} style="background:{termBg}"></div>
    </div>

    <div class="agent-shell-panel" class:dragging={dragging} style="display:{$agentShellOpen ? 'flex' : 'none'};width:{100 - mainWidth}%;flex:none;">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="agent-drag-handle" onmousedown={handleDividerMousedown}></div>
      <div class="agent-shell-wrap">
        {#if activeShellLoading}
          <div class="agent-loading shell-loading">
            <img src="/code-in-action.svg" alt="" class="loading-mascot" />
            <div class="loading-text">
              <span class="loading-title">Starting Shell</span>
              <span class="loading-sub">Loading terminal session<span class="loading-dots"></span></span>
            </div>
          </div>
        {/if}
        {#if shellFindOpen}
          <div class="agent-find-bar agent-shell-find-bar">
            <!-- svelte-ignore a11y_autofocus -->
            <input
              bind:this={shellFindInputEl}
              value={shellFindQuery}
              oninput={onShellFindInput}
              onkeydown={onShellFindKeydown}
              onblur={() => { try { activeShellEntry?.searchAddon.clearActiveDecoration(); } catch { /* ignore */ } }}
              class="agent-find-input"
              class:no-match={shellFindNoMatch}
              placeholder="Find in terminal…"
              spellcheck={false}
              autocomplete="off"
            />
            <div class="agent-find-sep"></div>
            <button class="agent-find-toggle" class:active={shellFindCaseSensitive} onclick={toggleShellCase} title="Case sensitive (Aa)">Aa</button>
            <button class="agent-find-toggle" class:active={shellFindRegex}         onclick={toggleShellRegex} title="Use regular expression (.*)">.*</button>
            <button class="agent-find-toggle" class:active={shellFindWholeWord}     onclick={toggleShellWord}  title="Match whole word">W</button>
            <div class="agent-find-sep"></div>
            {#if shellFindQuery}
              <span class="agent-find-count" class:no-results={shellFindNoMatch}>
                {#if shellFindNoMatch}No results
                {:else if shellFindResultCount > 0}{shellFindResultIndex === -1 ? `${shellFindResultCount}+` : `${shellFindResultIndex + 1} / ${shellFindResultCount}`}
                {/if}
              </span>
            {/if}
            <button class="agent-find-btn" onclick={doShellFindPrev} title="Previous (Shift+Enter)">
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="18 15 12 9 6 15"/></svg>
            </button>
            <button class="agent-find-btn" onclick={doShellFindNext} title="Next (Enter)">
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="6 9 12 15 18 9"/></svg>
            </button>
            <div class="agent-find-sep"></div>
            <button class="agent-find-close" onclick={closeShellFind} title="Close (Esc)">
              <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
          </div>
        {/if}
        <div class="agent-shell-container" class:term-hidden={activeShellLoading} bind:this={shellEl} style="background:{termBg}"></div>
      </div>
    </div>
  </div>
{:else}
  <div class="agent-empty">
    <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="var(--t4)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <polyline points="4 17 10 11 4 5"></polyline>
      <line x1="12" y1="19" x2="20" y2="19"></line>
    </svg>
    <p class="empty-title">No active session</p>
    <p class="empty-sub">Create a new session or select one from the sidebar</p>
  </div>
{/if}

<style>
  .agent-panel {
    flex: 1;
    display: flex;
    flex-direction: row;
    min-height: 0;
    overflow: hidden;
  }

  .agent-terminal-main {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 200px;
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

  .agent-terminal-container {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    opacity: 1;
    transition: opacity 0.35s ease;
  }
  .agent-terminal-container :global(.xterm) {
    height: 100% !important;
    padding: 0 !important;
  }
  .agent-terminal-container :global(.xterm-viewport) {
    height: 100% !important;
    /* Scrollbar overlays content instead of taking space */
    scrollbar-gutter: auto;
    /* xterm.css ships `background-color: #000` on the viewport (a 10+ year
       old macOS scrollbar workaround). Without this override, every theme
       except pure-black ones leaks black bars at the right/bottom where
       the canvas doesn't reach. Transparent lets the wrapper's themed bg
       (style="background:{termBg}") show through. */
    background-color: transparent !important;
  }
  .agent-terminal-container :global(.xterm-screen) {
    height: 100% !important;
  }
  .agent-shell-container :global(.xterm) {
    height: 100% !important;
    padding: 0 !important;
  }
  .agent-shell-container :global(.xterm-viewport) {
    height: 100% !important;
    scrollbar-gutter: auto;
    background-color: transparent !important;
  }
  .agent-shell-container :global(.xterm-screen) {
    height: 100% !important;
  }
  .agent-terminal-container {
    transition: opacity 0.15s ease;
  }
  .agent-terminal-container.term-hidden {
    opacity: 0;
  }
  /* Thin scrollbar for xterm */
  .agent-terminal-container :global(.xterm-viewport::-webkit-scrollbar) { width: 3px; }
  .agent-terminal-container :global(.xterm-viewport::-webkit-scrollbar-track) { background: transparent; }
  .agent-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb) { background: var(--surface-hover); border-radius: 3px; }
  .agent-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb:hover) { background: var(--surface-hover); }
  .agent-shell-container :global(.xterm-viewport::-webkit-scrollbar) { width: 3px; }
  .agent-shell-container :global(.xterm-viewport::-webkit-scrollbar-track) { background: transparent; }
  .agent-shell-container :global(.xterm-viewport::-webkit-scrollbar-thumb) { background: var(--surface-hover); border-radius: 3px; }
  .agent-shell-container :global(.xterm-viewport::-webkit-scrollbar-thumb:hover) { background: var(--surface-hover); }

  .agent-loading {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 16px;
    z-index: 2;
    animation: loadFadeIn 0.3s ease;
  }
  .loading-mascot {
    width: 48px;
    height: 48px;
    opacity: 0.7;
  }
  .loading-text {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
  .loading-title {
    font-size: 14px;
    font-weight: 500;
    color: var(--t2);
    font-family: var(--ui);
  }
  .loading-sub {
    font-size: 11px;
    color: var(--t4);
    font-family: var(--ui);
  }
  .loading-dots::after {
    content: '';
    animation: dots 1.5s steps(4, end) infinite;
  }
  @keyframes dots {
    0% { content: ''; }
    25% { content: '.'; }
    50% { content: '..'; }
    75% { content: '...'; }
  }
  @keyframes loadFadeIn {
    from { opacity: 0; transform: scale(0.97); }
    to { opacity: 1; transform: scale(1); }
  }

  .agent-shell-panel {
    display: flex;
    flex-direction: row;
    min-width: 200px;
  }
  .agent-shell-panel.dragging {
    transition: none;
    user-select: none;
  }
  .agent-drag-handle {
    width: 4px;
    flex-shrink: 0;
    cursor: col-resize;
    background: var(--b1);
    transition: background 0.15s;
  }
  .agent-drag-handle:hover,
  .agent-shell-panel.dragging .agent-drag-handle {
    background: var(--acc);
  }

  .agent-shell-wrap {
    flex: 1;
    min-height: 0;
    min-width: 0;
    position: relative;
    display: flex;
  }
  .agent-shell-container {
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
    transition: opacity 0.15s ease;
  }
  .agent-shell-container.term-hidden {
    opacity: 0;
  }

  :global(.agent-term-hidden) {
    display: none !important;
  }

  .shell-loading {
    /* same .agent-loading positioning, just scoped here */
  }

  .agent-find-bar {
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
  .agent-shell-find-bar {
    right: 8px;
  }
  @keyframes findSlideIn {
    from { opacity: 0; transform: translateY(-4px); }
    to   { opacity: 1; transform: translateY(0); }
  }
  .agent-find-input {
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
  .agent-find-input.no-match { color: var(--err, #f55); }
  .agent-find-input::placeholder { color: var(--t4); }
  .agent-find-sep {
    width: 1px;
    height: 16px;
    background: var(--b1);
    flex-shrink: 0;
    margin: 0 3px;
  }
  .agent-find-toggle {
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
  .agent-find-toggle:hover { background: var(--surface-hover); color: var(--t2); }
  .agent-find-toggle.active {
    background: color-mix(in srgb, var(--acc) 20%, transparent);
    color: var(--acc);
  }
  .agent-find-count {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--t3);
    white-space: nowrap;
    padding: 0 4px;
    min-width: 52px;
    text-align: center;
    flex-shrink: 0;
  }
  .agent-find-count.no-results { color: var(--err, #f55); }
  .agent-find-btn {
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
  .agent-find-btn:hover { background: var(--surface-hover); color: var(--t1); }
  .agent-find-close {
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
  .agent-find-close:hover { background: var(--surface-hover); color: var(--t2); }

  .agent-empty {
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

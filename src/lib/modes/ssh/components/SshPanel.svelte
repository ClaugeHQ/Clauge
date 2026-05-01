<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { Channel } from '@tauri-apps/api/core';
  import {
    activeSshProfile,
    sshProfiles,
    sshTerminalIds,
    sshConnStates,
    loadSshProfiles,
  } from '../stores';
  import {
    sshSpawnTerminal,
    sshWriteToTerminal,
    sshResizeTerminal,
    sshKillTerminal,
    sshTouchProfile,
  } from '../commands';
  import { tabs as tabsStore, activeTabId, addTab, activateTab, closeTab } from '$lib/shared/stores/tabs';
  import { newSshTabKey, profileIdFromTabKey } from '../tabkey';
  import { getTerminalTheme } from '$lib/utils/theme';
  import { appearance } from '$lib/stores/settings';
  import { showToast } from '$lib/shared/primitives/toast';
  import { base64ToBytes, deferUntilFrame, loadWebGLAddon } from '$lib/shared/primitives/terminal-utils';
  import { resolveSshCapture, rejectAllSshCaptures, type SshCaptureRequest } from '../ai/execute';
  import type { SshProfile, SshTerminalPayload } from '../types';
  import { SSH_EVENT } from '$lib/shared/constants/events';
  import { RESIZE_DEBOUNCE_MS, SSH_CAPTURE_TIMEOUT_MS } from '$lib/shared/constants/timings';

  let terminalEl: HTMLDivElement;

  // Per-tab xterm entry. Keyed by tab.key (== profile.id-based key).
  type TermEntry = {
    term: Terminal;
    fitAddon: FitAddon;
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

  // Map keyed by tabKey. Each spawn gets a unique key (`profileId#timestamp-N`)
  // so a single profile can have multiple independent tabs (Duplicate Session).
  const termEntries = new Map<string, TermEntry>();
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
      scrollback: 10000,
      lineHeight: 1.35,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    const container = document.createElement('div');
    container.style.cssText = 'width:100%;height:100%;display:none;';
    terminalEl.appendChild(container);
    term.open(container);
    loadWebGLAddon(term);

    const entry: TermEntry = {
      term,
      fitAddon,
      container,
      terminalId: null,
      profileId: profile.id,
      tabKey,
      generation: 0,
      capture: null,
    };

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

    termEntries.set(tabKey, entry);
    return entry;
  }

  function showEntry(entry: TermEntry) {
    if (activeEntry && activeEntry !== entry) {
      activeEntry.container.style.display = 'none';
      try { activeEntry.term.options.scrollback = 1000; } catch { /* ignore */ }
    }
    entry.container.style.display = 'block';
    try { entry.term.options.scrollback = 10000; } catch { /* ignore */ }
    activeEntry = entry;
    requestAnimationFrame(() => {
      try { entry.fitAddon.fit(); } catch { /* ignore */ }
      try { entry.term.focus(); } catch { /* ignore */ }
    });
  }

  function markExited(tabKey: string) {
    const entry = termEntries.get(tabKey);
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
      sshConnStates.update((m) => {
        m.set(entry.tabKey, 'connected');
        return new Map(m);
      });
      // Rust just bumped last_used_at as part of spawn — refresh the store so
      // the SshNav list reflects the new "Xs ago" instead of "never".
      loadSshProfiles().catch(() => {});

      // Send initial fit
      requestAnimationFrame(() => {
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
      termEntries.delete(entry.tabKey);
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
    let entry = termEntries.get(tabKey);
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
      termEntries.delete(tabKey);
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
  // `currentTabKey` to match what termEntries / sshTerminalIds / sshConnStates
  // are actually keyed by, otherwise this whole function operates on a key
  // nobody else has and the in-flight spawn happily completes and writes
  // 'connected' to the real key.
  async function cancelConnect() {
    if (!currentTabKey) return;
    const tabKey = currentTabKey;
    const entry = termEntries.get(tabKey);

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
      termEntries.delete(tabKey);
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
    const entry = termEntries.get(tabKey);
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

    const entry = termEntries.get(tabKey);
    if (entry) {
      if (entry.terminalId) sshKillTerminal(entry.terminalId).catch(() => {});
      try { entry.container.remove(); } catch { /* ignore */ }
      try { entry.term.dispose(); } catch { /* ignore */ }
      termEntries.delete(tabKey);
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
    for (const entry of termEntries.values()) {
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
      for (const entry of termEntries.values()) {
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
</script>

{#if $activeSshProfile}
  <div class="ssh-panel">
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

    <div class="ssh-terminal-container" class:term-hidden={!termReady && !activeIsExited} bind:this={terminalEl} style="background:{termBg}"></div>
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
  .ssh-terminal-container {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    transition: opacity 0.15s ease;
  }
  .ssh-terminal-container.term-hidden { opacity: 0; }
  .ssh-terminal-container :global(.xterm) { height: 100% !important; padding: 0 !important; }
  .ssh-terminal-container :global(.xterm-viewport) { height: 100% !important; scrollbar-gutter: auto; }
  .ssh-terminal-container :global(.xterm-screen) { height: 100% !important; }
  .ssh-terminal-container :global(.xterm-viewport::-webkit-scrollbar) { width: 3px; }
  .ssh-terminal-container :global(.xterm-viewport::-webkit-scrollbar-track) { background: transparent; }
  .ssh-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb) { background: rgba(255,255,255,0.10); border-radius: 3px; }
  .ssh-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb:hover) { background: rgba(255,255,255,0.20); }

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
    background: rgba(255,255,255,0.04);
    border-color: var(--b2);
    color: var(--t1);
    cursor: pointer;
  }

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

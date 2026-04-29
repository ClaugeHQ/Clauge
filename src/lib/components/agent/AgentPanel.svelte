<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { Channel } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import {
    activeAgentSession,
    agentTerminalMap,
    agentShellMap,
    agentTerminalIds,
    agentShellIds,
    agentShellOpen,
    agentSessionActivity,
    agentSessions,
    agentSoundEnabled,
    agentDockBounceEnabled,
  } from '$lib/stores/agent';
  import { getSetting } from '$lib/commands/settings';
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
    agentKillTerminal,
    agentRemoveWorktree,
    agentDeleteSession,
  } from '$lib/commands/agent';
  import { refreshAgentGitStatus, refreshAgentContextUsage, loadAgentSessions, agentGitBranchName, agentGitFiles, agentGitAhead, agentGitBehind } from '$lib/stores/agent';
  import { getTerminalTheme } from '$lib/utils/theme';
  import { appearance } from '$lib/stores/settings';
  import { getPurposePrompt } from '$lib/prompts/agent';
  import { AGENT_EVENT } from '$lib/shared/constants/events';
  import {
    AGENT_NOTIFY_DEBOUNCE_MS,
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

  // Active terminal entry refs
  let activeTermEntry: { term: Terminal; fitAddon: FitAddon; container: HTMLDivElement; terminalId: string | null; _exitBuffer?: string } | null = null;
  let activeShellEntry: { term: Terminal; fitAddon: FitAddon; container: HTMLDivElement; terminalId: string | null } | null = null;

  // Divider drag state
  let dragging = $state(false);
  let mainWidth = $state(55); // percentage (left terminal width)

  // Terminal background color (synced with theme to fill gaps)
  let termBg = $state('#0d0d18');

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

  // Context usage polling interval
  let contextUsageInterval: ReturnType<typeof setInterval> | null = null;

  // Suppress auto-switch on exit (set by reset/close actions)
  let _suppressExit = false;

  // --- Notification system for action-required prompts ---
  let notifyOutputBuffer = '';
  let notifyLastTime = 0;
  let notifyBufferTimer: ReturnType<typeof setTimeout> | null = null;
  let notifySoundInterval: ReturnType<typeof setInterval> | null = null;
  let unlistenFileDrop: (() => void) | null = null;

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

  function checkNotifyBuffer() {
    const buf = notifyOutputBuffer;
    notifyOutputBuffer = '';
    if (!buf) return;
    if (Date.now() - notifyLastTime < 10000) return;
    if (document.hasFocus()) return;

    if (actionPatterns.some(p => p.test(buf))) {
      notifyLastTime = Date.now();

      // Dock bounce — Critical = persistent bounce until focus
      if (get(agentDockBounceEnabled)) {
        import('@tauri-apps/api/window').then(({ getCurrentWindow, UserAttentionType }) => {
          getCurrentWindow().requestUserAttention(UserAttentionType.Critical);
        }).catch(() => {});
      }

      // Sound chime + repeat
      if (get(agentSoundEnabled)) {
        playChime();
        if (notifySoundInterval) clearInterval(notifySoundInterval);
        notifySoundInterval = setInterval(() => {
          if (document.hasFocus()) {
            clearInterval(notifySoundInterval!);
            notifySoundInterval = null;
            return;
          }
          playChime();
        }, AGENT_NOTIFY_REPEAT_MS);
      }
    }
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

  function handleTerminalOutput(base64Data: string) {
    try {
      const raw = atob(base64Data);
      const text = raw.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '').replace(/\x1b\][^\x07]*\x07/g, '');
      notifyOutputBuffer += text;

      // Check on every chunk if unfocused (timers may be throttled in background)
      if (!document.hasFocus()) checkNotifyBuffer();
      // Also debounce for when data arrives in small chunks
      if (notifyBufferTimer) clearTimeout(notifyBufferTimer);
      notifyBufferTimer = setTimeout(() => checkNotifyBuffer(), AGENT_NOTIFY_DEBOUNCE_MS);
    } catch (_) {}
  }

  async function loadWebGLAddon(term: Terminal) {
    try {
      const { WebglAddon } = await import('@xterm/addon-webgl');
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => { webgl.dispose(); });
      term.loadAddon(webgl);
    } catch (_) {} // Falls back to canvas renderer silently
  }

  function getCurrentTermTheme(): Record<string, string> {
    const app = get(appearance);
    return getTerminalTheme(app.theme, app.accentColor);
  }

  function createTermEntry(sessionId: string): { term: Terminal; fitAddon: FitAddon; container: HTMLDivElement; terminalId: string | null; _exitBuffer?: string } {
    const t = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", "SF Mono", "Menlo", monospace',
      theme: getCurrentTermTheme(),
      scrollback: 10000,
      lineHeight: 1.35,
    });
    const fa = new FitAddon();
    t.loadAddon(fa);

    const container = document.createElement('div');
    container.style.cssText = 'width:100%;height:100%;display:none;';
    terminalEl.appendChild(container);
    t.open(container);
    loadWebGLAddon(t);

    t.onData((data) => {
      // Only count REAL user typing as exit intent. xterm fires onData for protocol
      // replies too — focus reports (\x1b[I/\x1b[O), Device Attributes responses
      // (\x1b[?1;2c), Cmd+L (\x0c), etc. — none of which represent the user typing
      // an exit command. Restrict to printable chars or Enter, with no escape seq.
      const isUserTyping = !/\x1b/.test(data) && /[\x20-\x7e\r\n]/.test(data);
      if (isUserTyping) {
        const e = get(agentTerminalMap).get(sessionId);
        if (e) (e as any)._lastExitIntent = Date.now();
      }

      const tIds = get(agentTerminalIds);
      const termId = tIds.get(sessionId);
      if (termId) {
        agentWriteToTerminal(termId, data).catch(() => {
          // PTY dead (I/O error) — treat as session exit
          agentTerminalIds.update(m => { m.delete(sessionId); return new Map(m); });
          agentTerminalMap.update(m => { m.delete(sessionId); return new Map(m); });
          agentSessionActivity.update(m => { m.set(sessionId, 'done'); return new Map(m); });
          const currentActive = get(activeAgentSession);
          if (currentActive?.id === sessionId) {
            const activity = get(agentSessionActivity);
            const sessions = get(agentSessions);
            const nextRunning = sessions.find(s => s.id !== sessionId && activity.get(s.id) === 'running');
            if (nextRunning) { activeAgentSession.set(nextRunning); }
            else { currentSessionId = null; activeAgentSession.set(null); }
          }
        });
      }
    });

    let resizeTimer: ReturnType<typeof setTimeout> | null = null;
    new ResizeObserver(() => {
      if (!fa || container.offsetWidth <= 0) return;
      if (resizeTimer) clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        resizeTimer = null;
        try {
          fa.fit();
          // Skip PTY resize during drag — only resize on mouseup via refitAll
          if (dragging) return;
          const tIds = get(agentTerminalIds);
          const termId = tIds.get(sessionId);
          if (termId) {
            const dims = fa.proposeDimensions();
            if (dims) agentResizeTerminal(termId, dims.cols, dims.rows).catch(() => {});
          }
        } catch (_) {}
      }, RESIZE_DEBOUNCE_MS);
    }).observe(container);

    const entry = { term: t, fitAddon: fa, container, terminalId: null as string | null, _exitBuffer: '' };

    agentTerminalMap.update(m => { m.set(sessionId, entry); return new Map(m); });
    return entry;
  }

  function showTermEntry(entry: { term: Terminal; fitAddon: FitAddon; container: HTMLDivElement; terminalId: string | null }) {
    if (activeTermEntry && activeTermEntry !== entry) {
      activeTermEntry.container.style.display = 'none';
      try { activeTermEntry.term.options.scrollback = 1000; } catch (_) {}
    }
    entry.container.style.display = 'block';
    try { entry.term.options.scrollback = 10000; } catch (_) {}
    activeTermEntry = entry;
    requestAnimationFrame(() => {
      try { entry.fitAddon.fit(); } catch (_) {}
      try { entry.term.focus(); } catch (_) {}
    });
  }

  function createShellEntry(sessionId: string): { term: Terminal; fitAddon: FitAddon; container: HTMLDivElement; terminalId: string | null } {
    const t = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", "SF Mono", "Menlo", monospace',
      theme: getCurrentTermTheme(),
      scrollback: 5000,
      lineHeight: 1.35,
    });
    const fa = new FitAddon();
    t.loadAddon(fa);

    const container = document.createElement('div');
    container.style.cssText = 'width:100%;height:100%;display:none;';
    shellEl.appendChild(container);
    t.open(container);
    loadWebGLAddon(t);

    // Safety fallback — if first data never arrives, drop the loader after 3s
    setTimeout(() => {
      shellLoadingSessions = shellLoadingSessions.filter(id => id !== sessionId);
    }, AGENT_SHELL_LOADER_MS);

    t.onData((data) => {
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

    const sEntry = { term: t, fitAddon: fa, container, terminalId: null as string | null };
    agentShellMap.update(m => { m.set(sessionId, sEntry); return new Map(m); });
    return sEntry;
  }

  function showShellEntry(sEntry: { term: Terminal; fitAddon: FitAddon; container: HTMLDivElement; terminalId: string | null }) {
    if (activeShellEntry && activeShellEntry !== sEntry) {
      activeShellEntry.container.style.display = 'none';
      try { activeShellEntry.term.options.scrollback = 500; } catch (_) {}
    }
    sEntry.container.style.display = 'block';
    try { sEntry.term.options.scrollback = 5000; } catch (_) {}
    activeShellEntry = sEntry;
    requestAnimationFrame(() => { try { sEntry.fitAddon.fit(); } catch (_) {} });
  }

  function refitAll(sendPtyResize = false) {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        try {
          activeTermEntry?.fitAddon?.fit();
          if (sendPtyResize && activeTermEntry?.fitAddon) {
            const tIds = get(agentTerminalIds);
            const session = get(activeAgentSession);
            if (session) {
              const termId = tIds.get(session.id);
              if (termId) {
                const dims = activeTermEntry.fitAddon.proposeDimensions();
                if (dims) agentResizeTerminal(termId, dims.cols, dims.rows).catch(() => {});
              }
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
      const bytes = Uint8Array.from(atob(msg.data), (c: string) => c.charCodeAt(0));
      sEntry!.term.write(bytes);
      if (!shellFirstData) {
        shellFirstData = true;
        requestAnimationFrame(() => requestAnimationFrame(() => {
          shellLoadingSessions = shellLoadingSessions.filter(id => id !== session.id);
        }));
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
        refreshAgentContextUsage(s.id, projectPath, s.claudeSessionId);
      }
    }, AGENT_CONTEXT_USAGE_INTERVAL_MS);
  }

  let _spawnLock = false;
  // Per-session generation: invalidates stale Channel handlers when the SAME session respawns.
  // Global generation was wrong — it blocked writes from other sessions' tabs.
  const _spawnGenerations = new Map<string, number>();

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

    try {
      await agentUpdateLastUsed(session.id);

      let spawnPath = session.worktreePath || session.projectPath;

      // Inject attached contexts into CLAUDE.md before spawning
      try {
        const sessionContexts = await agentGetSessionContexts(session.id);
        if (sessionContexts.length > 0) {
          const contextIds = sessionContexts.map((c: any) => c.id);
          await agentInjectContexts(spawnPath, contextIds);
        }
      } catch (_) {}

      // Auto-create worktree for new sessions in git repos
      if (!session.worktreePath && !session.claudeSessionId) {
        try {
          const isGit = await agentIsGitRepo(session.projectPath);
          if (isGit) {
            const rawBranch = `clauge/${session.purpose.toLowerCase().replace(/\s+/g, '-')}-${session.title.toLowerCase().replace(/\s+/g, '-')}`;
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

      // Get existing session IDs BEFORE spawning
      let existingSessionIds: string[] = [];
      if (!session.claudeSessionId) {
        try {
          const existing = await agentDiscoverSessions(spawnPath);
          existingSessionIds = existing.map((s: any) => s.sessionId);
        } catch (_) {}
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
          requestAnimationFrame(() => requestAnimationFrame(() => {
            if (spawning) { spawning = false; termReady = true; }
          }));
        }

        // PTY-close signal from Rust: reader thread sends { exit: true } when the
        // child process dies. This is the authoritative exit signal — no text matching
        // needed. Triggered when user types "exit" + Enter and Claude Code exits.
        if (payload.exit === true) {
          console.log(`[TERM] EXIT signaled by PTY close for session ${sessionId}, gen=${myGeneration}, suppress=${_suppressExit}`);
          agentTerminalIds.update(m => { m.delete(sessionId); return new Map(m); });
          const tMapNow = get(agentTerminalMap);
          const exitedEntry = tMapNow.get(sessionId);
          if (exitedEntry) {
            try { exitedEntry.container.remove(); } catch (_) {}
            try { exitedEntry.term.dispose(); } catch (_) {}
          }
          agentTerminalMap.update(m => { m.delete(sessionId); return new Map(m); });
          // Capture session ID from buffered "claude --resume <id>" if available
          if (entry && entry._exitBuffer && !session.claudeSessionId) {
            const resumeMatch = entry._exitBuffer.match(/claude --resume ([a-f0-9-]+)/);
            if (resumeMatch) {
              session.claudeSessionId = resumeMatch[1];
              agentUpdateSessionId(session.id, resumeMatch[1]).catch(() => {});
            }
          }
          if (entry) entry._exitBuffer = '';
          agentSessionActivity.update(m => { m.set(sessionId, 'done'); return new Map(m); });
          activeTermEntry = null;
          if (currentSessionId === sessionId) currentSessionId = null;
          if (!_suppressExit) {
            const allTabs = get(tabsStore);
            const exitedTab = allTabs.find(t => t.mode === 'agent' && t.key === sessionId);
            if (exitedTab) {
              const remainingAgentTabs = allTabs.filter(t => t.mode === 'agent' && t.id !== exitedTab.id);
              closeTab(exitedTab.id);
              if (remainingAgentTabs.length > 0) {
                const nextTab = remainingAgentTabs[remainingAgentTabs.length - 1];
                activateTab(nextTab.id);
                if (nextTab.key) {
                  const sessions = get(agentSessions);
                  const nextSession = sessions.find(s => s.id === nextTab.key);
                  if (nextSession) activeAgentSession.set(nextSession);
                }
              } else {
                activeAgentSession.set(null);
              }
            } else {
              const currentActive = get(activeAgentSession);
              if (currentActive?.id === sessionId) activeAgentSession.set(null);
            }
          }
          _suppressExit = false;
          return;
        }

        // Write data to terminal
        if (entry!.term) {
          try {
            const binary = atob(payload.data);
            const bytes = new Uint8Array(binary.length);
            for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
            entry!.term.write(bytes);
          } catch (_) {}
        }

        // Check for action-required prompts and notify
        handleTerminalOutput(payload.data);

        // Session exit detection — buffer last 500 chars
        if (!entry!._exitBuffer) entry!._exitBuffer = '';
        try {
          const text = atob(payload.data);
          entry!._exitBuffer += text;
          if (entry!._exitBuffer.length > 500) entry!._exitBuffer = entry!._exitBuffer.slice(-500);
          // Strip ANSI codes
          const clean = entry!._exitBuffer
            .replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '')
            .replace(/\x1b\][^\x07]*\x07/g, '');

          // Only trigger exit detection if user RECENTLY typed /exit (within 10s).
          // Window resizes / PTY redraws can flood the buffer with conversation history
          // that contains farewell messages or "Resume this session with:" text from
          // prior interactions — those would falsely trigger exit on resize otherwise.
          const lastExitIntent = (entry as any)?._lastExitIntent || 0;
          const recentExitIntent = lastExitIntent && (Date.now() - lastExitIntent) < 10000;
          // Note: "Resume this session with:" and "claude --resume <id>" are NOT exit
          // patterns anymore — Claude Code v2 prints them in the startup banner too.
          // Real exits are caught via PTY-close (I/O error in agentWriteToTerminal)
          // and these explicit farewell patterns.
          const matched_ended = /session has ended|Exiting Claude/i.test(clean);
          const matched_farewell = /(Goodbye!|Bye!|See ya!|Catch you later!|Take care!|Until next time!)\s*$/i.test(clean);
          const exitMatched = recentExitIntent && (matched_ended || matched_farewell);
          if (matched_ended || matched_farewell) {
            console.log(`[TERM-DBG] regex-match session=${sessionId.slice(0,8)} ended=${matched_ended} farewell=${matched_farewell} recentIntent=${recentExitIntent} intentAge=${lastExitIntent ? Date.now() - lastExitIntent : 'never'}ms tail=${JSON.stringify(clean.slice(-200))}`);
          }
          if (exitMatched) {
            console.log(`[TERM] EXIT DETECTED for session ${sessionId}, gen=${myGeneration}`);
            agentTerminalIds.update(m => { m.delete(sessionId); return new Map(m); });
            // Remove terminal entry + dispose xterm + remove container from DOM
            const tMapNow = get(agentTerminalMap);
            const exitedEntry = tMapNow.get(sessionId);
            if (exitedEntry) {
              try { exitedEntry.container.remove(); } catch (_) {}
              try { exitedEntry.term.dispose(); } catch (_) {}
            }
            agentTerminalMap.update(m => { m.delete(sessionId); return new Map(m); });
            if (entry) entry._exitBuffer = '';
            // Capture session ID if not already set (for future --resume)
            const resumeMatch = clean.match(/claude --resume ([a-f0-9-]+)/);
            if (resumeMatch && !session.claudeSessionId) {
              const extractedSessionId = resumeMatch[1];
              agentUpdateSessionId(session.id, extractedSessionId).catch(() => {});
              session.claudeSessionId = extractedSessionId;
            }
            agentSessionActivity.update(m => { m.set(sessionId, 'done'); return new Map(m); });

            // Clear stale active reference — entry was just disposed/removed from DOM
            activeTermEntry = null;
            if (currentSessionId === sessionId) currentSessionId = null;

            // Synchronous tab-aware switching
            if (!_suppressExit) {
              const allTabs = get(tabsStore);
              const exitedTab = allTabs.find(t => t.mode === 'agent' && t.key === sessionId);
              if (exitedTab) {
                const remainingAgentTabs = allTabs.filter(t => t.mode === 'agent' && t.id !== exitedTab.id);
                closeTab(exitedTab.id);
                if (remainingAgentTabs.length > 0) {
                  const nextTab = remainingAgentTabs[remainingAgentTabs.length - 1];
                  activateTab(nextTab.id);
                  if (nextTab.key) {
                    const sessions = get(agentSessions);
                    const nextSession = sessions.find(s => s.id === nextTab.key);
                    if (nextSession) activeAgentSession.set(nextSession);
                  }
                } else {
                  activeAgentSession.set(null);
                }
              } else {
                const currentActive = get(activeAgentSession);
                if (currentActive?.id === sessionId) activeAgentSession.set(null);
              }
            }
            _suppressExit = false;
            return; // Stop processing — terminal is dead
          }
        } catch (_) {}

        // Track activity — only mark 'running' if sustained output (not just echo/redraw)
        // Count bytes in a rolling 500ms window. Claude Code generating output produces
        // hundreds of bytes/sec. Echo/redraws are < 50 bytes total.
        if (!_suppressExit) {
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
              const allSessions = await agentDiscoverSessions(spawnPath);
              const newSession = allSessions.find((s: any) => !existingSessionIds.includes(s.sessionId));
              if (newSession) {
                await agentUpdateSessionId(session.id, newSession.sessionId);
                session.claudeSessionId = newSession.sessionId;
                await loadAgentSessions();
                clearInterval(captureInterval);

                // Start context usage polling now that we have a session ID
                if (contextUsageInterval) clearInterval(contextUsageInterval);
                contextUsageInterval = setInterval(() => {
                  const s = get(activeAgentSession);
                  if (s?.claudeSessionId) {
                    const path = s.worktreePath || s.projectPath;
                    refreshAgentContextUsage(s.id, path, s.claudeSessionId);
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

      spawning = true;
      const termId = await agentSpawnTerminal({
        sessionId: session.claudeSessionId || undefined,
        projectPath: spawnPath,
        contextPrompt: purposePrompt || undefined,
        skipPermissions: session.skipPermissions === 1 || undefined,
        gitName: session.gitName || undefined,
        gitEmail: session.gitEmail || undefined,
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
        activeTermEntry.container.style.display = 'none';
        activeTermEntry = null;
      }
      if (activeShellEntry) {
        activeShellEntry.container.style.display = 'none';
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
        activeShellEntry.container.style.display = 'none';
        activeShellEntry = null;
      }
      refitAll();
    }
  });

  // Event handler for reset-session (Feature 4)
  function handleResetSession(e: Event) {
    const detail = (e as CustomEvent).detail;
    if (!detail?.session) return;
    const session = detail.session;
    _suppressExit = true;

    // Kill terminal
    const tIds = get(agentTerminalIds);
    const termId = tIds.get(session.id);
    if (termId) agentKillTerminal(termId).catch(() => {});

    // Kill shell
    const sIds = get(agentShellIds);
    const shellId = sIds.get(session.id);
    if (shellId) agentKillTerminal(shellId).catch(() => {});

    // Clear session ID
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
    _suppressExit = false; // Reset — killed PTY won't trigger exit detection to clear it

    loadAgentSessions().then(() => {
      selectSession(session);
    });
  }

  // Event handler for relaunch — kills terminal but KEEPS session ID for --resume with updated prompt
  function handleRelaunchSession(e: Event) {
    const detail = (e as CustomEvent).detail;
    if (!detail?.session) return;
    const session = detail.session;
    _suppressExit = true;

    // Kill terminal
    const tIds = get(agentTerminalIds);
    const termId = tIds.get(session.id);
    if (termId) agentKillTerminal(termId).catch(() => {});

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
    _suppressExit = false; // Reset — killed PTY won't trigger exit detection to clear it

    // Relaunch with updated session data (prompt changes picked up from activeAgentSession)
    requestAnimationFrame(() => selectSession(session));
  }

  // Event handler for tab close — kills terminal without deleting session from DB
  function handleCloseTabSession(e: Event) {
    const detail = (e as CustomEvent).detail;
    const sessionId = detail?.sessionId;
    if (!sessionId) return;

    _suppressExit = true;

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

    _suppressExit = true;

    // Kill terminal
    const tIds = get(agentTerminalIds);
    const termId = tIds.get(session.id);
    if (termId) await agentKillTerminal(termId).catch(() => {});

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

    // Remove worktree if exists
    if (session.worktreePath) {
      await agentRemoveWorktree(session.projectPath, session.worktreePath).catch(() => {});
    }

    // Delete from DB
    await agentDeleteSession(session.id).catch(() => {});

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
  });

  onDestroy(() => {
    unsubSession();
    unsubShell();
    unsubAppearance();
    stopContextUsagePolling();
    if (notifyBufferTimer) clearTimeout(notifyBufferTimer);
    if (notifySoundInterval) clearInterval(notifySoundInterval);
    window.removeEventListener(AGENT_EVENT.RESET_SESSION, handleResetSession);
    window.removeEventListener(AGENT_EVENT.DELETE_SESSION, handleDeleteSession);
    window.removeEventListener(AGENT_EVENT.CLOSE_TAB_SESSION, handleCloseTabSession);
    window.removeEventListener(AGENT_EVENT.RELAUNCH_SESSION, handleRelaunchSession);
    if (unlistenFileDrop) unlistenFileDrop();
  });
</script>

{#if $activeAgentSession}
  <div class="agent-panel" bind:this={wrapperEl}>
    <div class="agent-terminal-main" style="width:{$agentShellOpen ? mainWidth + '%' : '100%'}">
      {#if spawning}
        <div class="agent-loading">
          <img src="/code-in-action.svg" alt="" class="loading-mascot" />
          <div class="loading-text">
            <span class="loading-title">Starting Claude Code</span>
            <span class="loading-sub">Setting up terminal session<span class="loading-dots"></span></span>
          </div>
        </div>
      {/if}
      <div class="agent-terminal-container" class:term-hidden={!termReady} bind:this={terminalEl} style="background:{termBg}"></div>
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
  .agent-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb) { background: rgba(255,255,255,0.10); border-radius: 3px; }
  .agent-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb:hover) { background: rgba(255,255,255,0.20); }
  .agent-shell-container :global(.xterm-viewport::-webkit-scrollbar) { width: 3px; }
  .agent-shell-container :global(.xterm-viewport::-webkit-scrollbar-track) { background: transparent; }
  .agent-shell-container :global(.xterm-viewport::-webkit-scrollbar-thumb) { background: rgba(255,255,255,0.10); border-radius: 3px; }
  .agent-shell-container :global(.xterm-viewport::-webkit-scrollbar-thumb:hover) { background: rgba(255,255,255,0.20); }

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
  .shell-loading {
    /* same .agent-loading positioning, just scoped here */
  }

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

<script>
  import { onMount } from "svelte";
  import { invoke, Channel } from "@tauri-apps/api/core";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import "@xterm/xterm/css/xterm.css";
  import { theme } from "$lib/stores/theme.svelte";
  import { notifications } from "$lib/stores/notifications.svelte";
  import { updater } from "$lib/stores/updater.svelte";
  import { pluginsStore } from "$lib/stores/plugins.svelte";
  import { contextsStore } from "$lib/stores/contexts.svelte";
  import { usageStore } from "$lib/stores/usage.svelte";
  import { gitStore } from "$lib/stores/git.svelte";
  import { terminalStore } from "$lib/stores/terminal.svelte";
  import { shellStore } from "$lib/stores/shell.svelte";
  import DeleteConfirmModal from "$lib/components/DeleteConfirmModal.svelte";
  import UpdateToast from "$lib/components/UpdateToast.svelte";
  import BottomBar from "$lib/components/BottomBar.svelte";
  import SettingsModal from "$lib/components/SettingsModal.svelte";
  import UsageDashboard from "$lib/components/UsageDashboard.svelte";
  import NewSessionModal from "$lib/components/NewSessionModal.svelte";
  import { getPurposePrompt } from "$lib/prompts";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import "$lib/styles/app.css";

  let profiles = $state([]);
  let activeProfile = $state(null);
  let showModal = $state(false);
  let showSettings = $state(false);
  let settingsTab = $state('settings');
  // Context manager — state lives in contextsStore
  let terminalEl;
  let statusMsg = $state("Ready");
  let tokenUsage = $state(null);
  let tokenInterval = null;
  let appVersion = $state('');
  let claudePlan = $state('');
  let sidebarRef;

  // Delete confirmation
  let deleteConfirm = $state(null); // profile to confirm delete
  let profileMenuOpen = $state(false);
  // Git state lives in gitStore

  // Terminal management — one xterm per profile, switch between them (state in terminalStore)
  let shellEl;
  let wrapperEl;
  // Shell terminal management — one shell per profile (state in shellStore)
  let shellOpen = $derived(activeProfile ? (shellStore.shellOpenMap[activeProfile.id] ?? false) : false);

  const purposes = [
    { label: "Brainstorming", color: "#d2a8ff" },
    { label: "Development", color: "#3fb950" },
    { label: "Code Review", color: "#58a6ff" },
    { label: "PR Review", color: "#d29922" },
    { label: "Debugging", color: "#f85149" },
    { label: "Custom", color: "#8b949e" },
  ];
  const purposeColors = Object.fromEntries(purposes.map(p => [p.label, p.color]));

  // Theme definitions
  function applyTheme(themeName) {
    const { termTheme, cursor } = theme.applyTheme(themeName);
    // Update all existing terminals
    for (const [, entry] of terminalStore.terminalMap) {
      if (entry.term) entry.term.options.theme = { ...termTheme, cursor };
    }
  }

  function applyAccent(color) {
    const { termTheme, cursor } = theme.applyAccent(color);
    for (const [, entry] of terminalStore.terminalMap) { if (entry.term) entry.term.options.theme = { ...termTheme, cursor }; }
    for (const [, entry] of shellStore.shellMap) { if (entry.term) entry.term.options.theme = { ...termTheme, cursor }; }
  }

  async function loadProfiles() {
    try {
      profiles = await invoke("refresh_session_ids");
    } catch (e) {
      try { profiles = await invoke("get_profiles"); } catch (e2) { statusMsg = "Load failed: " + e2; }
    }
  }

  async function loadWebGLAddon(term) {
    try {
      const { WebglAddon } = await import("@xterm/addon-webgl");
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => { webgl.dispose(); });
      term.loadAddon(webgl);
    } catch(_) {} // Falls back to canvas renderer silently
  }

  function createTermEntry(profileId) {
    const t = new Terminal(terminalStore.getTermConfig());
    const fa = new FitAddon();
    t.loadAddon(fa);

    const container = document.createElement("div");
    container.style.cssText = "width:100%;height:100%;display:none;";
    terminalEl.appendChild(container);
    t.open(container);
    loadWebGLAddon(t);

    t.onData((data) => {
      const entry = terminalStore.terminalMap.get(profileId);
      if (entry?.terminalId) invoke("write_to_terminal", { terminalId: entry.terminalId, data });
    });

    new ResizeObserver(() => {
      if (fa && container.offsetWidth > 0) requestAnimationFrame(() => { try { fa.fit(); } catch(_) {} });
    }).observe(container);

    const entry = { term: t, fitAddon: fa, container, terminalId: null, channel: null };
    terminalStore.terminalMap.set(profileId, entry);
    return entry;
  }

  function showTermEntry(entry) {
    if (terminalStore.activeTermEntry && terminalStore.activeTermEntry !== entry) {
      terminalStore.activeTermEntry.container.style.display = "none";
      // Reduce scrollback on inactive terminal to save memory
      try { terminalStore.activeTermEntry.term.options.scrollback = 1000; } catch(_) {}
    }
    entry.container.style.display = "block";
    // Restore full scrollback on active terminal
    try { entry.term.options.scrollback = 10000; } catch(_) {}
    terminalStore.activeTermEntry = entry;
    terminalStore.currentTerminalId = entry.terminalId;
    requestAnimationFrame(() => { try { entry.fitAddon.fit(); } catch(_) {} });
  }

  function createShellEntry(profileId) {
    const t = new Terminal({ ...terminalStore.getTermConfig(), scrollback: 5000 });
    const fa = new FitAddon();
    t.loadAddon(fa);

    const container = document.createElement("div");
    container.style.cssText = "width:100%;height:100%;display:none;";
    shellEl.appendChild(container);
    t.open(container);
    loadWebGLAddon(t);

    t.onData((data) => {
      const sEntry = shellStore.shellMap.get(profileId);
      if (sEntry?.terminalId) {
        invoke("write_to_terminal", { terminalId: sEntry.terminalId, data }).catch(() => {
          // Shell process died — close shell panel
          sEntry.terminalId = null;
          if (activeProfile) {
            shellStore.shellOpenMap[activeProfile.id] = false;
            shellStore.shellOpenMap = {...shellStore.shellOpenMap};
            // Refit Claude terminal to full width with PTY resize
            requestAnimationFrame(() => {
              requestAnimationFrame(() => {
                handleWindowResize();
              });
            });
          }
        });
      }
    });

    new ResizeObserver(() => {
      if (fa && container.offsetWidth > 0) requestAnimationFrame(() => { try { fa.fit(); } catch(_) {} });
    }).observe(container);

    const sEntry = { term: t, fitAddon: fa, container, terminalId: null };
    shellStore.shellMap.set(profileId, sEntry);
    return sEntry;
  }

  function showShellEntry(sEntry) {
    if (shellStore.activeShellEntry && shellStore.activeShellEntry !== sEntry) {
      shellStore.activeShellEntry.container.style.display = "none";
      try { shellStore.activeShellEntry.term.options.scrollback = 500; } catch(_) {}
    }
    sEntry.container.style.display = "block";
    try { sEntry.term.options.scrollback = 5000; } catch(_) {}
    shellStore.activeShellEntry = sEntry;
    requestAnimationFrame(() => { try { sEntry.fitAddon.fit(); } catch(_) {} });
  }

  async function spawnShellForProfile(profile) {
    if (!shellEl) return;
    let sEntry = shellStore.shellMap.get(profile.id);
    if (sEntry && sEntry.terminalId) {
      showShellEntry(sEntry);
      return;
    }
    if (!sEntry) {
      sEntry = createShellEntry(profile.id);
    } else {
      // Respawning after exit — clear old content
      sEntry.term.clear();
    }
    showShellEntry(sEntry);

    const projectPath = profile.worktreePath || profile.projectPath;
    const channel = new Channel();
    channel.onmessage = (msg) => {
      if (!msg.data) return;
      const bytes = Uint8Array.from(atob(msg.data), c => c.charCodeAt(0));
      sEntry.term.write(bytes);
    };

    try {
      sEntry.terminalId = await invoke("spawn_shell", { projectPath, onOutput: channel });
    } catch(e) {
      sEntry.term.write(`\r\nFailed to spawn shell: ${e}\r\n`);
    }
  }

  function startDividerDrag(e) {
    e.preventDefault();
    const wrapper = wrapperEl;
    if (!wrapper) return;

    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    shellStore.isDraggingDivider = true;

    let fitTimer = null;
    function onMove(ev) {
      const rect = wrapper.getBoundingClientRect();
      const x = ev.clientX - rect.left;
      const pct = (x / rect.width) * 100;
      if (activeProfile) {
        shellStore.shellWidthMap[activeProfile.id] = Math.max(20, Math.min(80, 100 - pct));
        shellStore.shellWidthMap = {...shellStore.shellWidthMap};
      }
      // Throttle fit calls to every 100ms during drag — prevents xterm jank
      if (!fitTimer) {
        fitTimer = setTimeout(() => {
          fitTimer = null;
          try { terminalStore.activeTermEntry?.fitAddon?.fit(); } catch(_) {}
          try { shellStore.activeShellEntry?.fitAddon?.fit(); } catch(_) {}
        }, 100);
      }
    }

    function onUp() {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
      shellStore.isDraggingDivider = false;
      if (fitTimer) { clearTimeout(fitTimer); fitTimer = null; }
      // Final fit + PTY resize after drag ends
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          handleWindowResize();
        });
      });
    }

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }

  async function toggleShell() {
    if (!activeProfile && !shellOpen) return;
    const newState = !shellOpen;
    if (activeProfile) {
      shellStore.shellOpenMap[activeProfile.id] = newState;
      shellStore.shellOpenMap = {...shellStore.shellOpenMap};
    }
    if (newState && activeProfile) {
      // Wait for DOM to render the shell panel
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          spawnShellForProfile(activeProfile);
          // Refit Claude terminal since width changed
          if (terminalStore.activeTermEntry?.fitAddon) {
            try {
              terminalStore.activeTermEntry.fitAddon.fit();
              if (terminalStore.activeTermEntry.terminalId) {
                const dims = terminalStore.activeTermEntry.fitAddon.proposeDimensions();
                if (dims) invoke("resize_terminal", { terminalId: terminalStore.activeTermEntry.terminalId, cols: dims.cols, rows: dims.rows }).catch(() => {});
              }
            } catch(_) {}
          }
        });
      });
    } else {
      // Refit Claude terminal to take full width — double rAF to wait for layout
      if (terminalStore.activeTermEntry?.fitAddon) {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            try {
              terminalStore.activeTermEntry.fitAddon.fit();
              if (terminalStore.activeTermEntry.terminalId) {
                const dims = terminalStore.activeTermEntry.fitAddon.proposeDimensions();
                if (dims) invoke("resize_terminal", { terminalId: terminalStore.activeTermEntry.terminalId, cols: dims.cols, rows: dims.rows }).catch(() => {});
              }
            } catch(_) {}
          });
        });
      }
    }
  }

  async function selectProfile(profile) {
    activeProfile = profile;
    // Clear activity indicator when switching to this session
    if (terminalStore.sessionActivity[profile.id]) {
      delete terminalStore.sessionActivity[profile.id];
      terminalStore.sessionActivity = { ...terminalStore.sessionActivity };
    }
    let entry = terminalStore.terminalMap.get(profile.id);

    if (entry && entry.terminalId) {
      showTermEntry(entry);
      if (shellOpen) spawnShellForProfile(profile);
      statusMsg = profile.title;
      refreshGitStatus();
    } else {
      statusMsg = "Spawning...";
      if (!entry) {
        entry = createTermEntry(profile.id);
      } else {
        // Terminal exited (terminalId is null) — clear old content for respawn
        entry.term.clear();
        entry.term.write('\r\n\x1b[2mResuming session...\x1b[0m\r\n\r\n');
      }

      try {
        await invoke("update_last_used", { id: profile.id });

        // Every session gets its own worktree — full isolation
        let spawnPath = profile.worktreePath || profile.projectPath;

        if (!profile.worktreePath && !profile.claudeSessionId) {
          try {
            const isGit = await invoke("is_git_repo", { path: profile.projectPath });
            if (isGit) {
              const rawBranch = `clauge/${profile.purpose.toLowerCase().replace(/\s+/g, '-')}-${profile.title.toLowerCase().replace(/\s+/g, '-')}`;
              const branchName = rawBranch.replace(/[^a-zA-Z0-9/_\-.]/g, '').replace(/\.{2,}/g, '.').replace(/\.lock/g, '');
              const worktreePath = await invoke("create_worktree", { projectPath: profile.projectPath, branchName });
              spawnPath = worktreePath;
              await invoke("update_profile_worktree", { id: profile.id, worktreePath, worktreeBranch: branchName });
              profile.worktreePath = worktreePath;
              profile.worktreeBranch = branchName;
              await loadProfiles();
            }
          } catch(e) {
            console.warn("Worktree creation failed, using original path:", e);
          }
        }

        // Get existing session IDs BEFORE spawning
        let existingSessionIds = [];
        if (!profile.claudeSessionId) {
          try {
            const existing = await invoke("discover_sessions", { projectPath: spawnPath });
            existingSessionIds = existing.map(s => s.sessionId);
          } catch(e) {}
        }

        // Flatten prompt to single line for shell compatibility
        // Use frontend purpose prompt for fixed purposes, fall back to profile.contextPrompt for Custom
        const rawPrompt = getPurposePrompt(profile.purpose) || profile.contextPrompt || '';
        const purposePrompt = rawPrompt.replace(/\n+/g, ' ').replace(/\s+/g, ' ').trim();

        let outputReceived = false;
        let activityTimer = null;
        const profileId = profile.id;
        const onOutput = new Channel();
        onOutput.onmessage = (payload) => {
          if (entry.term) {
            try {
              const binary = atob(payload.data);
              const bytes = new Uint8Array(binary.length);
              for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
              entry.term.write(bytes);
            } catch(e) {}
          }
          // Detect action-required prompts and notify if window not focused
          notifications.checkForActionPrompt(payload.data, profile.title);
          // Detect Claude session exit (Ctrl+C, exit, /exit)
          // Buffer recent output to catch multi-chunk patterns
          if (!entry._exitBuffer) entry._exitBuffer = '';
          try {
            const text = atob(payload.data);
            entry._exitBuffer += text;
            // Keep buffer small — only last 500 chars
            if (entry._exitBuffer.length > 500) entry._exitBuffer = entry._exitBuffer.slice(-500);
            // Strip ANSI codes for matching
            const clean = entry._exitBuffer.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '').replace(/\x1b\][^\x07]*\x07/g, '');

            if (/Resume this session with:/.test(clean) || /claude --resume [a-f0-9-]+/.test(clean)) {
              entry.terminalId = null;
              entry._exitBuffer = '';
              const resumeMatch = clean.match(/claude --resume ([a-f0-9-]+)/);
              if (resumeMatch && !profile.claudeSessionId) {
                const extractedSessionId = resumeMatch[1];
                invoke("update_session_id", { id: profile.id, claudeSessionId: extractedSessionId }).then(() => {
                  profile.claudeSessionId = extractedSessionId;
                  loadProfiles();
                }).catch(() => {});
              }
              terminalStore.sessionActivity[profileId] = 'done';
              terminalStore.sessionActivity = { ...terminalStore.sessionActivity };
              // Auto-close: hide terminal, switch to another session or show empty state
              entry.container.style.display = "none";
              if (activeProfile?.id === profileId) {
                const otherProfile = profiles.find(p => p.id !== profileId && terminalStore.terminalMap.get(p.id)?.terminalId);
                if (otherProfile) {
                  selectProfile(otherProfile);
                } else {
                  activeProfile = null;
                  terminalStore.activeTermEntry = null;
                  terminalStore.currentTerminalId = null;
                }
              }
            }
          } catch(_) {}
          // Track activity for background sessions
          if (activeProfile?.id !== profileId) {
            terminalStore.sessionActivity[profileId] = 'active';
            terminalStore.sessionActivity = { ...terminalStore.sessionActivity };
            // After 2s of no new output, mark as done (Claude finished responding)
            if (activityTimer) clearTimeout(activityTimer);
            activityTimer = setTimeout(() => {
              if (terminalStore.sessionActivity[profileId] === 'active') {
                terminalStore.sessionActivity[profileId] = 'done';
                terminalStore.sessionActivity = { ...terminalStore.sessionActivity };
              }
            }, 2000);
          }
          // Capture session ID — retry every 3s until found (up to 30s)
          if (!outputReceived && !profile.claudeSessionId) {
            outputReceived = true;
            let attempts = 0;
            const captureInterval = setInterval(async () => {
              attempts++;
              if (attempts > 10 || profile.claudeSessionId) { clearInterval(captureInterval); return; }
              try {
                const allSessions = await invoke("discover_sessions", { projectPath: spawnPath });
                const newSession = allSessions.find(s => !existingSessionIds.includes(s.sessionId));
                if (newSession) {
                  await invoke("update_session_id", { id: profile.id, claudeSessionId: newSession.sessionId });
                  profile.claudeSessionId = newSession.sessionId;
                  await loadProfiles();
                  console.log("[Clauge] Session ID saved:", newSession.sessionId);
                  clearInterval(captureInterval);
                }
              } catch(e) {}
            }, 3000);
          }
        };
        entry.channel = onOutput;

        const tid = await invoke("spawn_terminal", {
          sessionId: profile.claudeSessionId || null,
          projectPath: spawnPath,
          contextPrompt: purposePrompt || null,
          skipPermissions: profile.skipPermissions || false,
          gitName: profile.gitName || null,
          gitEmail: profile.gitEmail || null,
          onOutput: onOutput,
        });
        entry.terminalId = tid;
        terminalStore.currentTerminalId = tid;
        statusMsg = profile.title;
        showTermEntry(entry);
        if (shellOpen) spawnShellForProfile(profile);

        refreshGitStatus();

        // Inject attached contexts into CLAUDE.md
        if (profile.contexts && profile.contexts.length > 0) {
          invoke("inject_session_context", { projectPath: spawnPath, contextNames: profile.contexts }).catch(() => {});
        }

        entry.fitAddon.fit();
        const dims = entry.fitAddon.proposeDimensions();
        if (dims) await invoke("resize_terminal", { terminalId: tid, cols: dims.cols, rows: dims.rows });
      } catch (e) {
        statusMsg = "ERROR: " + String(e);
        entry.term.writeln("\r\n\x1b[31mError: " + String(e) + "\x1b[0m");
        showTermEntry(entry);
      }
    }

    async function refreshTokens() {
      try {
        tokenUsage = await invoke("get_session_tokens", { projectPath: profile.projectPath, sessionId: profile.claudeSessionId || null });
      } catch(e) { tokenUsage = null; }
    }
    await refreshTokens();
    if (tokenInterval) clearInterval(tokenInterval);
    tokenInterval = setInterval(refreshTokens, 10000);
  }

  async function createSession({ path, title, purpose, skipPermissions, customPrompt, gitName, gitEmail, selectedSession, contexts }) {
    try {
      const profile = await invoke("create_profile", {
        title,
        purpose,
        projectPath: path,
        skipPermissions,
        customPrompt,
        gitName,
        gitEmail,
        contexts,
      });
      // Link existing session if selected (Custom purpose only)
      if (selectedSession) {
        await invoke("update_session_id", { id: profile.id, claudeSessionId: selectedSession });
        profile.claudeSessionId = selectedSession;
      }
      await loadProfiles();
      await selectProfile(profile);
    } catch (e) { statusMsg = "Create failed: " + e; }
  }

  function deleteProfile(e, profile) {
    e?.preventDefault();
    e?.stopPropagation();
    deleteConfirm = profile;
  }

  async function confirmDelete() {
    if (!deleteConfirm) return;
    const deletedProfile = { ...deleteConfirm };
    const deletedId = deletedProfile.id;

    // Clean up worktree
    if (deletedProfile.worktreePath && deletedProfile.projectPath) {
      try { await invoke("remove_worktree", { projectPath: deletedProfile.projectPath, worktreePath: deletedProfile.worktreePath, branchName: deletedProfile.worktreeBranch || null }); } catch(e) {}
    }

    await invoke("delete_profile", { id: deletedId });

    // Clean up terminal (backend PTY + child process)
    const entry = terminalStore.terminalMap.get(deletedId);
    if (entry) {
      if (entry.terminalId) {
        try { await invoke("kill_terminal", { terminalId: entry.terminalId }); } catch(e) {}
      }
      entry.container.style.display = "none";
      if (entry.term) entry.term.dispose();
      terminalStore.terminalMap.delete(deletedId);
    }

    // Clean up shell (backend PTY + child process)
    const sEntry = shellStore.shellMap.get(deletedId);
    if (sEntry) {
      if (sEntry.terminalId) {
        try { await invoke("kill_terminal", { terminalId: sEntry.terminalId }); } catch(e) {}
      }
      sEntry.container.style.display = "none";
      if (sEntry.term) sEntry.term.dispose();
      shellStore.shellMap.delete(deletedId);
    }

    if (activeProfile?.id === deletedId) {
      activeProfile = null;
      terminalStore.activeTermEntry = null;
      shellStore.activeShellEntry = null;
      terminalStore.currentTerminalId = null;
    }
    delete shellStore.shellOpenMap[deletedId];
    delete shellStore.shellWidthMap[deletedId];
    shellStore.shellOpenMap = {...shellStore.shellOpenMap};
    shellStore.shellWidthMap = {...shellStore.shellWidthMap};

    deleteConfirm = null;
    await loadProfiles();
  }

  // Git helpers — delegates to gitStore, passing resolved projectPath
  function gitPath() { return activeProfile?.worktreePath || activeProfile?.projectPath || ''; }
  function gitProfileId() { return activeProfile?.id; }
  function refreshGitStatus() { if (activeProfile) gitStore.refreshGitStatus(gitPath(), gitProfileId()); }
  function doGitCommit() { if (activeProfile) gitStore.doGitCommit(gitPath()); }
  function doGitCommitStaged() { if (activeProfile) gitStore.doGitCommitStaged(gitPath()); }
  function doGitPush() { if (activeProfile) gitStore.doGitPush(gitPath()); }
  function doGitPull() { if (activeProfile) gitStore.doGitPull(gitPath()); }
  function viewDiff(file) { if (activeProfile) gitStore.viewDiff(gitPath(), file); }
  function toggleStageFile(file) { if (activeProfile) gitStore.toggleStageFile(gitPath(), gitProfileId(), file); }
  function loadGitHistory() { if (activeProfile) gitStore.loadGitHistory(gitPath()); }
  function loadGitBranches() { if (activeProfile) gitStore.loadGitBranches(gitPath()); }
  function switchBranch(branchName) { if (activeProfile) gitStore.switchBranch(gitPath(), gitProfileId(), branchName); }
  function doGitStash() { if (activeProfile) gitStore.doGitStash(gitPath(), gitProfileId()); }
  function doGitStashPop() { if (activeProfile) gitStore.doGitStashPop(gitPath(), gitProfileId()); }

  function handleFileDrop(e) {
    if (!terminalStore.activeTermEntry?.terminalId) return;
    // Try web File API path (Tauri exposes file.path)
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      const paths = Array.from(files).map(f => f.path || f.name).filter(Boolean);
      if (paths.length > 0) {
        const text = paths.map(p => p.includes(' ') ? `"${p}"` : p).join(' ');
        invoke("write_to_terminal", { terminalId: terminalStore.activeTermEntry.terminalId, data: text }).catch(() => {});
        return;
      }
    }
    // Fallback: try text/uri-list
    const uriList = e.dataTransfer?.getData('text/uri-list');
    if (uriList) {
      const paths = uriList.split('\n').filter(l => l && !l.startsWith('#')).map(u => {
        try { return decodeURIComponent(new URL(u).pathname); } catch(_) { return u; }
      });
      if (paths.length > 0) {
        const text = paths.map(p => p.includes(' ') ? `"${p}"` : p).join(' ');
        invoke("write_to_terminal", { terminalId: terminalStore.activeTermEntry.terminalId, data: text }).catch(() => {});
      }
    }
  }

  function handleGlobalKeydown(e) {
    if (e.metaKey && e.key === 'n') { e.preventDefault(); showModal = true; }
    if (e.metaKey && e.key >= '1' && e.key <= '9') {
      e.preventDefault();
      const idx = parseInt(e.key) - 1;
      if (profiles[idx]) selectProfile(profiles[idx]);
    }
    if (e.metaKey && e.key === 'b') { e.preventDefault(); sidebarRef?.toggleSidebar(); }
    if (e.metaKey && e.key === 'l') { e.preventDefault(); toggleShell(); }
    if (e.key === 'Escape') { showModal = false; showSettings = false; }
  }

  function handleWindowResize() {
    requestAnimationFrame(() => {
      if (terminalStore.activeTermEntry?.fitAddon && terminalStore.activeTermEntry.container.offsetWidth > 0) {
        try {
          terminalStore.activeTermEntry.fitAddon.fit();
          if (terminalStore.activeTermEntry.terminalId) {
            const dims = terminalStore.activeTermEntry.fitAddon.proposeDimensions();
            if (dims) invoke("resize_terminal", { terminalId: terminalStore.activeTermEntry.terminalId, cols: dims.cols, rows: dims.rows }).catch(() => {});
          }
        } catch(_) {}
      }
      if (shellStore.activeShellEntry?.fitAddon && shellStore.activeShellEntry.container.offsetWidth > 0) {
        try {
          shellStore.activeShellEntry.fitAddon.fit();
          if (shellStore.activeShellEntry.terminalId) {
            const dims = shellStore.activeShellEntry.fitAddon.proposeDimensions();
            if (dims) invoke("resize_terminal", { terminalId: shellStore.activeShellEntry.terminalId, cols: dims.cols, rows: dims.rows }).catch(() => {});
          }
        } catch(_) {}
      }
    });
  }

  function openExternal(url) {
    import("@tauri-apps/plugin-opener").then(m => m.openUrl(url)).catch(() => window.open(url, "_blank"));
  }


  async function handleDragStart(e) {
    if (e.buttons === 1) {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      if (e.detail === 2) {
        getCurrentWindow().toggleMaximize();
      } else {
        getCurrentWindow().startDragging();
      }
    }
  }



  onMount(() => {
    applyTheme(theme.currentTheme);
    invoke("get_app_version").then(v => {
      appVersion = v;
      updater.checkWhatsNew(v);
      updater.checkAndDownloadUpdate();
    }).catch(() => {});
    invoke("get_claude_plan").then(p => { if (p) claudePlan = p; }).catch(() => {});


    // Listen for Tauri native file drop events
    import("@tauri-apps/api/webviewWindow").then(({ getCurrentWebviewWindow }) => {
      getCurrentWebviewWindow().onDragDropEvent((event) => {
        if (event.payload.type === 'drop' && terminalStore.activeTermEntry?.terminalId) {
          const paths = event.payload.paths || [];
          if (paths.length > 0) {
            const text = paths.map(p => p.includes(' ') ? `"${p}"` : p).join(' ');
            invoke("write_to_terminal", { terminalId: terminalStore.activeTermEntry.terminalId, data: text }).catch(() => {});
          }
        }
      });
    }).catch(() => {});

    // Priority 1: Load profiles (fast, <10ms)
    loadProfiles();

    // Priority 2: Load session key + usage limits (fast key read, then ~1.5s API call)
    invoke("load_session_key").then(savedKey => {
      if (savedKey) {
        usageStore.sessionKeyInput = savedKey;
        usageStore.sessionKeyConfigured = true;
        usageStore.loadUsageLimits();
        usageStore.startRefreshInterval();
      }
    }).catch(() => {});

    // Poll git status every 5 seconds when a session is active
    setInterval(() => { if (activeProfile) refreshGitStatus(); }, 5000);

  });
</script>

<svelte:window onkeydown={handleGlobalKeydown} onresize={handleWindowResize} onclick={() => { profileMenuOpen = false; gitStore.gitPanelOpen = false; contextsStore.showContextDropdown = false; }} oncontextmenu={(e) => { if (!import.meta.env.DEV) e.preventDefault(); }} />

<div class="app-wrapper">
<div class="app">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="drag-bar" onmousedown={handleDragStart}></div>
  <Sidebar
    bind:this={sidebarRef}
    {profiles}
    {activeProfile}
    {claudePlan}
    onSelect={selectProfile}
    onNewSession={() => showModal = true}
    onDeleteProfile={deleteProfile}
    onRefitTerminals={handleWindowResize}
  />

  <div class="terminal-wrapper" bind:this={wrapperEl}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="terminal-area" class:panel-focused={shellStore.focusedPanel === 'claude'} onclick={() => shellStore.focusedPanel = 'claude'}
      ondragover={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; }}
      ondrop={(e) => { e.preventDefault(); handleFileDrop(e); }}>
      {#if !activeProfile}
        <div class="empty-state">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="var(--border)" stroke-width="1.5">
            <polyline points="4 17 10 11 4 5"></polyline>
            <line x1="12" y1="19" x2="20" y2="19"></line>
          </svg>
          <p class="empty-title">No active session</p>
          <p class="empty-sub">Select a session from the sidebar or create a new one</p>
        </div>
      {/if}
      <div class="terminal-panel" bind:this={terminalEl}></div>
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="shell-divider" style="display:{shellOpen ? 'block' : 'none'}" onmousedown={startDividerDrag}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="shell-area" class:no-transition={shellStore.isDraggingDivider} class:panel-focused={shellStore.focusedPanel === 'shell'} onclick={() => shellStore.focusedPanel = 'shell'} style="display:{shellOpen ? 'flex' : 'none'};width:{shellStore.getShellWidth(activeProfile?.id)}%;flex:none;">
      <div class="shell-panel" bind:this={shellEl}></div>
    </div>
  </div>
</div>
<BottomBar {activeProfile} {appVersion} {claudePlan} {shellOpen} bind:profileMenuOpen bind:showSettings bind:settingsTab onToggleShell={toggleShell} onOpenExternal={openExternal} />

<UpdateToast {appVersion} onOpenExternal={openExternal} />
</div>

<NewSessionModal
  bind:show={showModal}
  {profiles}
  {activeProfile}
  onCreate={createSession}
/>

<SettingsModal bind:show={showSettings} bind:settingsTab {appVersion} onOpenExternal={openExternal} onApplyTheme={applyTheme} onApplyAccent={applyAccent} />

<!-- UpdateToast handles both the toast and the what's new modal -->

<UsageDashboard {claudePlan} />

{#if contextsStore.showContextPicker}
<!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
<div class="modal-backdrop">
  <div class="modal" style="max-width:400px;">
    <h2 style="font-size:14px;">Manage Contexts — {contextsStore.showContextPicker.title}</h2>
    {#if contextsStore.contextSnippets.length === 0}
      <p style="font-size:12px;color:var(--text-secondary);">No contexts created yet. Go to Settings → Contexts to create one.</p>
    {:else}
      <div class="ctx-picker-list">
        {#each contextsStore.contextSnippets as ctx}
          <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
          <div class="ctx-picker-item" onclick={() => {
            const p = contextsStore.showContextPicker;
            const attached = p.contexts || [];
            const projectPath = p.worktreePath || p.projectPath;
            if (attached.includes(ctx.name)) {
              const updated = attached.filter(c => c !== ctx.name);
              contextsStore.attachContextsToSession(p.id, projectPath, updated);
              p.contexts = updated;
            } else {
              const updated = [...attached, ctx.name];
              contextsStore.attachContextsToSession(p.id, projectPath, updated);
              p.contexts = updated;
            }
            contextsStore.showContextPicker = {...contextsStore.showContextPicker};
          }}>
            <span class="ctx-picker-check">
              {#if (contextsStore.showContextPicker.contexts || []).includes(ctx.name)}
                <svg width="12" height="12" viewBox="0 0 16 16" fill="var(--accent)"><path d="M13.78 4.22a.75.75 0 010 1.06l-7.25 7.25a.75.75 0 01-1.06 0L2.22 9.28a.75.75 0 011.06-1.06L6 10.94l6.72-6.72a.75.75 0 011.06 0z"/></svg>
              {:else}
                <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="var(--text-secondary)" stroke-width="1.5"><rect x="2" y="2" width="12" height="12" rx="2"/></svg>
              {/if}
            </span>
            <div class="ctx-picker-info">
              <span class="ctx-picker-name">{ctx.name}</span>
              <span class="ctx-picker-preview">{ctx.preview}</span>
            </div>
          </div>
        {/each}
      </div>
    {/if}
    <div class="modal-actions">
      <button onclick={() => contextsStore.showContextPicker = false}>Done</button>
    </div>
  </div>
</div>
{/if}

<DeleteConfirmModal profile={deleteConfirm} onConfirm={confirmDelete} onCancel={() => deleteConfirm = null} />

{#if pluginsStore.pluginUninstallConfirm}
<div class="modal-backdrop" style="z-index:1100;">
  <div class="modal" style="max-width:360px;animation:slideIn 0.15s ease-out;">
    <div style="text-align:center;padding:20px 20px 0;">
      <svg width="32" height="32" viewBox="0 0 16 16" fill="#f85149" style="margin-bottom:12px;"><path d="M6.5 1.75a.25.25 0 01.25-.25h2.5a.25.25 0 01.25.25V3h-3V1.75zM11 3V1.75A1.75 1.75 0 009.25 0h-2.5A1.75 1.75 0 005 1.75V3H2.75a.75.75 0 000 1.5h.928l.747 10.218A1.75 1.75 0 006.172 16h3.656a1.75 1.75 0 001.747-1.282L12.322 4.5h.928a.75.75 0 000-1.5H11z"/></svg>
      <h2 style="font-size:15px;margin-bottom:8px;">Uninstall plugin?</h2>
      <p style="font-size:13px;color:var(--text-secondary);line-height:1.5;">
        Are you sure you want to uninstall <strong style="color:var(--text-primary);">{pluginsStore.pluginUninstallConfirm.name}</strong>?
      </p>
    </div>
    <div class="modal-actions" style="padding:16px 20px;">
      <button onclick={() => pluginsStore.pluginUninstallConfirm = null}>Cancel</button>
      <button style="background:#f85149 !important;border-color:transparent !important;color:#fff !important;" onclick={() => pluginsStore.uninstallPlugin(pluginsStore.pluginUninstallConfirm)}>Uninstall</button>
    </div>
  </div>
</div>
{/if}

<style>
  /* Page layout — app shell, terminal area, shell panel, empty state */
  .drag-bar { position: fixed; top: 0; left: 0; right: 0; height: 38px; z-index: 9999; cursor: default; }
  .app-wrapper { display: flex; flex-direction: column; height: 100vh; width: 100vw; overflow: hidden; }
  .app { display: flex; flex: 1; min-height: 0; overflow: hidden; background: transparent; }

  .terminal-wrapper { flex: 1; min-width: 0; display: flex; height: 100%; overflow: hidden; }
  .terminal-area { flex: 1; min-width: 0; height: 100%; background: var(--term-bg); position: relative; overflow: hidden; -webkit-app-region: no-drag; }
  .terminal-panel { width: 100%; height: 100%; padding: 4px; -webkit-app-region: no-drag; }
  .terminal-panel :global(.xterm) { height: 100%; }
  .terminal-panel :global(.xterm-viewport) { overflow-y: auto !important; }
  .terminal-panel :global(.xterm-viewport::-webkit-scrollbar) { width: 8px; }
  .terminal-panel :global(.xterm-viewport::-webkit-scrollbar-thumb) { background: var(--border); border-radius: 4px; }

  .shell-divider { width: 4px; background: transparent; flex-shrink: 0; cursor: col-resize; position: relative; }
  .shell-divider::after { content: ''; position: absolute; left: 1px; top: 0; bottom: 0; width: 1px; background: var(--border); }
  .shell-divider:hover::after { background: var(--accent); width: 2px; left: 1px; }
  .shell-area { min-width: 0; height: 100%; display: flex; flex-direction: column; background: var(--term-bg); overflow: hidden; transition: width 0.15s ease; }
  .shell-area.no-transition { transition: none; }
  .shell-panel { flex: 1; padding: 4px; min-width: 0; overflow: hidden; }
  .shell-panel :global(.xterm) { height: 100%; }
  .shell-panel :global(.xterm-viewport) { overflow-y: auto !important; }
  .shell-panel :global(.xterm-viewport::-webkit-scrollbar) { width: 8px; }
  .shell-panel :global(.xterm-viewport::-webkit-scrollbar-thumb) { background: var(--border); border-radius: 4px; }

  .empty-state { display: flex; flex-direction: column; align-items: center; justify-content: center; height: 100%; gap: 12px; position: absolute; inset: 0; }
  .empty-title { font-size: 16px; font-weight: 500; color: var(--text-primary); }
  .empty-sub { font-size: 13px; color: var(--text-secondary); }


  /* Context picker modal — uses shared .modal-backdrop and .modal from app.css */
  .ctx-picker-list { display: flex; flex-direction: column; gap: 2px; max-height: 300px; overflow-y: auto; }
  .ctx-picker-item { display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-radius: 5px; cursor: pointer; transition: background 0.1s; }
  .ctx-picker-item:hover { background: rgba(255,255,255,0.04); }
  .ctx-picker-check { flex-shrink: 0; display: flex; }
  .ctx-picker-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .ctx-picker-name { font-size: 12px; font-weight: 500; color: var(--text-primary); }
  .ctx-picker-preview { font-size: 10px; color: var(--text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>

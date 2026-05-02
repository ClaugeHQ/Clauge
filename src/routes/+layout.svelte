<script lang="ts">
  import '../app.css';
  import Sidebar from '$lib/components/sidebar/Sidebar.svelte';
  import NavPanel from '$lib/components/nav/NavPanel.svelte';
  import Topbar from '$lib/components/topbar/Topbar.svelte';
  import StatusBar from '$lib/components/statusbar/StatusBar.svelte';
  import Toast from '$lib/shared/primitives/Toast.svelte';
  import ContextMenu from '$lib/shared/primitives/ContextMenu.svelte';
  import EnvManagerModal from '$lib/components/env/EnvManagerModal.svelte';
  import SettingsModal from '$lib/components/settings/SettingsModal.svelte';
  import GitHubConnect from '$lib/components/github/GitHubConnect.svelte';
  import { loadAgentSessions, loadAgentContexts } from '$lib/modes/agent/stores';
  import { getPurposeColor } from '$lib/modes/agent/ai/prompt';
  import NewSessionModal from '$lib/modes/agent/components/NewSessionModal.svelte';
  import EditSessionModal from '$lib/modes/agent/components/EditSessionModal.svelte';
  import UsageDashboard from '$lib/modes/agent/components/UsageDashboard.svelte';
  import favicon from '$lib/assets/favicon.svg';

  import { onMount, onDestroy } from 'svelte';
  import { loadCollections, clearActiveRequest, collections } from '$lib/modes/rest/stores';
  import { loadEnvironments } from '$lib/modes/rest/stores';
  import { loadConnections as loadSqlConnections, loadSqlScripts, showSqlConnectionDialog, editingSqlConnection, handleSqlConnectionSave, showSqlDisconnectConfirm, sqlDisconnectTarget, disconnectFromDb, connections as sqlConnections } from '$lib/modes/sql/stores';
  import { showToast } from '$lib/shared/primitives/toast';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import { loadNoSqlConnections, showNoSqlConnectionDialog, editingNoSqlConnection, handleNoSqlConnectionSave, nosqlConnections } from '$lib/modes/nosql/stores';
  import SqlConnectionDialog from '$lib/modes/sql/components/ConnectionDialog.svelte';
  import NoSqlConnectionDialog from '$lib/modes/nosql/components/ConnectionDialog.svelte';
  import { loadSettings, loadAppearance, appearance } from '$lib/stores/settings';
  import { setConnected, setLastSynced, hasSyncedOnce, markSynced, showSyncRestorePrompt } from '$lib/stores/github';
  import { githubGetStatus, gistCheckExists, gistSyncPush, gistSyncPull } from '$lib/commands/github';
  import { activeModal, aiPanelOpen, mode } from '$lib/stores/app';
  import { agentSessionKey, loadAgentUsageLimits, loadAgentClaudePlan, agentSessions, activeAgentSession } from '$lib/modes/agent/stores';
  import { sshProfiles, activeSshProfile, loadSshProfiles } from '$lib/modes/ssh/stores';
  import type { SshProfile } from '$lib/modes/ssh/types';
  import { explorerConnections, loadExplorerConnections, activeExplorerConnection } from '$lib/modes/explorer/stores';
  import type { ExplorerConnection } from '$lib/modes/explorer/types';
  import { getSetting } from '$lib/commands/settings';
  import AIPanel from '$lib/components/ai/AIPanel.svelte';
  import { tabs, addTab, activeTabId, activateTab } from '$lib/shared/stores/tabs';
  import type { AgentSession } from '$lib/modes/agent/types';
  import { setupGlobalShortcuts, teardownGlobalShortcuts } from '$lib/utils/shortcuts';
  import { applyTheme } from '$lib/utils/theme';
  import ShortcutsOverlay from '$lib/shared/primitives/ShortcutsOverlay.svelte';
  import SaveRequestDialog from '$lib/shared/primitives/SaveRequestDialog.svelte';
  import Onboarding from '$lib/components/onboarding/Onboarding.svelte';
  import WhatsNewModal from '$lib/shared/primitives/WhatsNewModal.svelte';
  import UpdateNotification from '$lib/shared/primitives/UpdateNotification.svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { get } from 'svelte/store';
  import { SSH_EVENT, AGENT_EVENT, APP_EVENT, EXPLORER_EVENT } from '$lib/shared/constants/events';
  import { PERIODIC_SYNC_INTERVAL_MS, USAGE_LIMITS_POLL_INTERVAL_MS, SPLASH_FADE_OUT_MS } from '$lib/shared/constants/timings';
  import { DEFAULT_ACCENT_COLOR } from '$lib/shared/constants/colors';

  let { children } = $props();

  let showSaveDialog = $state(false);
  let saveDialogTabId = $state(-1);
  let syncInterval: ReturnType<typeof setInterval> | null = null;
  let usageLimitsInterval: ReturnType<typeof setInterval> | null = null;

  let showNewSessionModal = $state(false);
  let showEditSessionModal = $state(false);
  let showUsageDashboard = $state(false);
  let editSessionTarget = $state<AgentSession | null>(null);
  let showSessionPicker = $state(false);
  let showSshPicker = $state(false);
  let sshPickerX = $state(290);
  let sshPickerY = $state(48);
  let showExplorerPicker = $state(false);
  let explorerPickerX = $state(290);
  let explorerPickerY = $state(48);

  function handleAgentNewSession() {
    showNewSessionModal = true;
  }

  function handleAgentShowUsageDashboard() {
    showUsageDashboard = true;
  }

  function handleAgentEditSession(e: Event) {
    const detail = (e as CustomEvent).detail;
    if (detail?.session) {
      editSessionTarget = detail.session;
      showEditSessionModal = true;
    }
  }

  let pickerX = $state(290);
  let pickerY = $state(48);

  function handleAgentAddTab(e: Event) {
    const detail = (e as CustomEvent).detail;
    if (detail?.x) pickerX = detail.x;
    if (detail?.y) pickerY = detail.y;
    const sessions = get(agentSessions);
    if (sessions.length === 0) {
      window.dispatchEvent(new CustomEvent(AGENT_EVENT.NEW_SESSION));
    } else {
      showSessionPicker = true;
    }
  }

  function openSessionTab(session: AgentSession) {
    const currentTabs = get(tabs);
    const existing = currentTabs.find(t => t.mode === 'agent' && t.key === session.id);
    if (existing) {
      activateTab(existing.id);
    } else {
      addTab(session.title, 'agent', session.id, getPurposeColor(session.purpose));
    }
    activeAgentSession.set(session);
    showSessionPicker = false;
  }

  function pickerNewSession() {
    showSessionPicker = false;
    window.dispatchEvent(new CustomEvent(AGENT_EVENT.NEW_SESSION));
  }

  // SSH "+ tab" handler — mirrors agent. No profiles → open create modal,
  // otherwise show a small picker to choose which profile to open.
  async function handleSshAddTab(e: Event) {
    const detail = (e as CustomEvent).detail;
    if (detail?.x) sshPickerX = detail.x;
    if (detail?.y) sshPickerY = detail.y;
    // Make sure the in-memory profiles list is fresh before deciding.
    if (get(sshProfiles).length === 0) {
      try { await loadSshProfiles(); } catch { /* ignore */ }
    }
    if (get(sshProfiles).length === 0) {
      window.dispatchEvent(new CustomEvent(SSH_EVENT.NEW_PROFILE));
    } else {
      showSshPicker = true;
    }
  }

  function openSshTabFromPicker(profile: SshProfile) {
    activeSshProfile.set(profile);
    window.dispatchEvent(new CustomEvent(SSH_EVENT.OPEN_TAB, { detail: profile }));
    showSshPicker = false;
  }

  function pickerNewSshProfile() {
    showSshPicker = false;
    window.dispatchEvent(new CustomEvent(SSH_EVENT.NEW_PROFILE));
  }

  // Explorer "+ tab" handler — mirrors SSH. No connections → open the
  // kind picker (NavPanel hosts it via 'explorer:add-connection'),
  // otherwise show a picker to choose which connection to open.
  async function handleExplorerAddTab(e: Event) {
    const detail = (e as CustomEvent).detail;
    if (detail?.x) explorerPickerX = detail.x;
    if (detail?.y) explorerPickerY = detail.y;
    if (get(explorerConnections).length === 0) {
      try { await loadExplorerConnections(); } catch { /* ignore */ }
    }
    if (get(explorerConnections).length === 0) {
      window.dispatchEvent(new CustomEvent(EXPLORER_EVENT.ADD_CONNECTION));
    } else {
      showExplorerPicker = true;
    }
  }

  function openExplorerTabFromPicker(conn: ExplorerConnection) {
    activeExplorerConnection.set(conn);
    window.dispatchEvent(new CustomEvent(EXPLORER_EVENT.OPEN_TAB, { detail: conn }));
    showExplorerPicker = false;
  }

  function pickerNewExplorerConnection() {
    showExplorerPicker = false;
    window.dispatchEvent(new CustomEvent(EXPLORER_EVENT.ADD_CONNECTION));
  }

  function explorerKindBadge(kind: string): string {
    switch (kind) {
      case 'sftp': return 'SFTP';
      case 'ftp': return 'FTP';
      case 's3': return 'S3';
      case 'azure_blob': return 'Azure';
      default: return kind.toUpperCase();
    }
  }

  function explorerSubLine(conn: ExplorerConnection): string {
    if (conn.kind === 'sftp' || conn.kind === 'ftp') {
      const u = conn.username ?? '';
      const h = conn.host ?? '';
      const portSuffix = conn.port && ((conn.kind === 'sftp' && conn.port !== 22) || (conn.kind === 'ftp' && conn.port !== 21))
        ? `:${conn.port}` : '';
      return h ? `${u}${u ? '@' : ''}${h}${portSuffix}` : '';
    }
    if (conn.kind === 's3') return conn.s3Bucket ?? '';
    if (conn.kind === 'azure_blob') return `${conn.azureAccount ?? ''}/${conn.azureContainer ?? ''}`;
    return '';
  }

  async function handleDragStart(e: MouseEvent) {
    if (e.buttons !== 1) return;
    const win = getCurrentWindow();
    if (e.detail === 2) {
      win.toggleMaximize();
    } else {
      win.startDragging();
    }
  }

  // Global drag handler: any element with data-drag-region attr
  function handleGlobalMousedown(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (target.closest('[data-drag-region]') && !target.closest('button, input, select, textarea, a, [role="button"]')) {
      handleDragStart(e);
    }
  }

  async function handleSqlDisconnectConfirm() {
    const target = get(sqlDisconnectTarget);
    if (!target) return;
    try {
      await disconnectFromDb(target.id);
      showToast(`Disconnected from ${target.name}`, 'success');
    } catch (err: any) {
      showToast(err.toString(), 'error');
    }
    sqlDisconnectTarget.set(null);
  }

  function handleSaveNewRequest(e: Event) {
    const detail = (e as CustomEvent).detail;
    saveDialogTabId = detail?.tabId ?? get(activeTabId);
    showSaveDialog = true;
  }

  function handleTabClosePrompt(e: Event) {
    // This event is handled by Topbar component via its own listener
  }

  onDestroy(() => {
    teardownGlobalShortcuts();
    window.removeEventListener(APP_EVENT.SAVE_NEW_REQUEST, handleSaveNewRequest);
    window.removeEventListener(AGENT_EVENT.NEW_SESSION, handleAgentNewSession);
    window.removeEventListener(AGENT_EVENT.EDIT_SESSION, handleAgentEditSession);
    window.removeEventListener(AGENT_EVENT.SHOW_USAGE_DASHBOARD, handleAgentShowUsageDashboard);
    window.removeEventListener(AGENT_EVENT.ADD_TAB, handleAgentAddTab);
    window.removeEventListener(SSH_EVENT.ADD_TAB, handleSshAddTab);
    window.removeEventListener(EXPLORER_EVENT.ADD_TAB, handleExplorerAddTab);
    if (syncInterval) clearInterval(syncInterval);
    if (usageLimitsInterval) clearInterval(usageLimitsInterval);
  });

  function applyAppearanceOnStartup() {
    const config = get(appearance);
    applyTheme(config.theme || 'dark-glass', config.accentColor || DEFAULT_ACCENT_COLOR);
  }

  // Disable macOS autocorrect/autocapitalize on all inputs
  function disableAutocorrect(el: Element) {
    el.setAttribute('autocorrect', 'off');
    el.setAttribute('autocapitalize', 'off');
    if (!el.hasAttribute('spellcheck')) el.setAttribute('spellcheck', 'false');
  }

  onMount(async () => {
    // Fade out splash screen now that the layout is mounted
    requestAnimationFrame(() => {
      const splash = document.getElementById('clauge-splash');
      if (splash) {
        splash.classList.add('fade-out');
        setTimeout(() => splash.remove(), SPLASH_FADE_OUT_MS);
      }
    });
    setupGlobalShortcuts();
    window.addEventListener(APP_EVENT.SAVE_NEW_REQUEST, handleSaveNewRequest);
    window.addEventListener(AGENT_EVENT.NEW_SESSION, handleAgentNewSession);
    window.addEventListener(AGENT_EVENT.EDIT_SESSION, handleAgentEditSession);
    window.addEventListener(AGENT_EVENT.SHOW_USAGE_DASHBOARD, handleAgentShowUsageDashboard);
    window.addEventListener(AGENT_EVENT.ADD_TAB, handleAgentAddTab);
    window.addEventListener(SSH_EVENT.ADD_TAB, handleSshAddTab);
    window.addEventListener(EXPLORER_EVENT.ADD_TAB, handleExplorerAddTab);

    // Apply to existing and future inputs/textareas
    document.querySelectorAll('input, textarea').forEach(disableAutocorrect);
    const observer = new MutationObserver(mutations => {
      for (const m of mutations) {
        for (const node of m.addedNodes) {
          if (node instanceof HTMLElement) {
            if (node.matches('input, textarea')) disableAutocorrect(node);
            node.querySelectorAll('input, textarea').forEach(disableAutocorrect);
          }
        }
      }
    });
    observer.observe(document.body, { childList: true, subtree: true });

    // Block right-click and reload shortcuts in production
    if (!import.meta.env.DEV) {
      document.addEventListener('contextmenu', (e) => e.preventDefault());
      document.addEventListener('keydown', (e) => {
        // Block Cmd+R, Cmd+Shift+R, F5
        if ((e.metaKey || e.ctrlKey) && e.key === 'r') e.preventDefault();
        if (e.key === 'F5') e.preventDefault();
        // Block Cmd+Shift+I (dev tools)
        if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === 'I') e.preventDefault();
      });
    }

    // Window focus/blur for custom traffic light dimming
    window.addEventListener('blur', () => document.body.classList.add('window-blurred'));
    window.addEventListener('focus', () => document.body.classList.remove('window-blurred'));
    await Promise.all([
      loadCollections(),
      loadEnvironments(),
      loadSettings(),
      loadAppearance(),
      loadSqlConnections(),
      loadSqlScripts(),
      loadNoSqlConnections(),
      loadAgentSessions(),
      loadAgentContexts(),
      loadExplorerConnections(),
    ]);

    applyAppearanceOnStartup();

    // No default tab — user creates tabs by clicking "+" or opening a request

    // Check GitHub connection status on startup
    try {
      const userInfo = await githubGetStatus();
      if (userInfo) {
        setConnected(userInfo.username, userInfo.avatarUrl);

        // Check if local has no user-created data (stores are loaded by this point)
        const localEmpty = get(collections).length === 0
          && get(sqlConnections).length === 0
          && get(nosqlConnections).length === 0;

        if (localEmpty && !get(hasSyncedOnce)) {
          // First time with no user data — check if cloud has data to restore
          try {
            const gistExists = await gistCheckExists();
            if (gistExists) {
              showSyncRestorePrompt.set(true);
            } else {
              // No cloud data either — mark synced so future pushes work
              markSynced();
            }
          } catch (e) {
            console.warn('[Clauge Sync] Cloud check failed:', e);
            markSynced();
          }
        } else if (get(hasSyncedOnce)) {
          // Returning user with local data — auto-push
          gistSyncPush().then((msg) => {
            console.info('[Clauge Sync]', msg);
            if (!msg.includes('Skipped')) setLastSynced(new Date().toISOString());
          }).catch((e) => {
            console.error('[Clauge Sync] Auto-push failed:', e);
          });
        } else {
          // First time with data locally — mark synced and push
          markSynced();
          gistSyncPush().then((msg) => {
            console.info('[Clauge Sync]', msg);
            if (!msg.includes('Skipped')) setLastSynced(new Date().toISOString());
          }).catch((e) => {
            console.error('[Clauge Sync] Auto-push failed:', e);
          });
        }

        // Periodic sync every 5 minutes (only if synced before and data exists)
        syncInterval = setInterval(async () => {
          if (!get(hasSyncedOnce)) return;
          try {
            const msg = await gistSyncPush();
            if (!msg.includes('Skipped')) setLastSynced(new Date().toISOString());
          } catch (e) {
            console.error('[Clauge Sync] Periodic push failed:', e);
          }
        }, PERIODIC_SYNC_INTERVAL_MS);
      }
    } catch (e) {
      console.warn('GitHub status check failed:', e);
    }

    // Check for updates silently on startup and show What's New if version changed
    try {
      const { checkAndDownloadUpdate, checkWhatsNew } = await import('$lib/utils/updater');
      const { getVersion } = await import('@tauri-apps/api/app');
      getVersion().then((v: string) => {
        checkWhatsNew(v);
      }).catch(() => {});
      checkAndDownloadUpdate();
    } catch {
      // Updater not available in dev mode
    }

    // Load Claude plan from keychain
    loadAgentClaudePlan();

    // Load agent session key and start usage limits polling
    try {
      const key = await getSetting('agent_session_key');
      if (key) {
        agentSessionKey.set(key);
        loadAgentUsageLimits();
        usageLimitsInterval = setInterval(() => {
          if (get(mode) === 'agent') loadAgentUsageLimits();
        }, USAGE_LIMITS_POLL_INTERVAL_MS);
      }
    } catch { /* ignore */ }
  });
</script>

<svelte:head>
  <link rel="icon" href={favicon} />
</svelte:head>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="app-shell" onmousedown={handleGlobalMousedown}>
  <Sidebar />
  <NavPanel />
  <div class="app-content">
    <Topbar />
    <div class="app-workspace">
      {@render children()}
    </div>
    <StatusBar />
  </div>
  <AIPanel />
</div>

{#if showSessionPicker}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="session-picker-overlay" onmousedown={() => (showSessionPicker = false)}></div>
  <div class="session-picker" style="top:{pickerY}px;left:{pickerX}px;">
    <div class="session-picker-header">Open Agent Session</div>
    <div class="session-picker-list">
      {#each $agentSessions as session (session.id)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="session-picker-item"
          onmousedown={(e) => { e.stopPropagation(); openSessionTab(session); }}
        >
          <span class="session-picker-title">{session.title}</span>
          <span
            class="session-picker-badge"
            style="color: {getPurposeColor(session.purpose)}; border-color: {getPurposeColor(session.purpose)};"
          >{session.purpose}</span>
        </div>
      {/each}
    </div>
    <div class="session-picker-footer">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <button class="session-picker-new" onmousedown={(e) => { e.stopPropagation(); pickerNewSession(); }}>
        + New Session
      </button>
    </div>
  </div>
{/if}

{#if showSshPicker}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="session-picker-overlay" onmousedown={() => (showSshPicker = false)}></div>
  <div class="session-picker" style="top:{sshPickerY}px;left:{sshPickerX}px;">
    <div class="session-picker-header">Open SSH Connection</div>
    <div class="session-picker-list">
      {#each $sshProfiles as profile (profile.id)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="session-picker-item ssh-picker-item"
          onmousedown={(e) => { e.stopPropagation(); openSshTabFromPicker(profile); }}
        >
          <div class="ssh-picker-text">
            <span class="session-picker-title">{profile.name}</span>
            <span class="ssh-picker-sub">{profile.username}@{profile.host}{profile.port !== 22 ? `:${profile.port}` : ''}</span>
          </div>
        </div>
      {/each}
    </div>
    <div class="session-picker-footer">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <button class="session-picker-new" onmousedown={(e) => { e.stopPropagation(); pickerNewSshProfile(); }}>
        + New SSH Profile
      </button>
    </div>
  </div>
{/if}

{#if showExplorerPicker}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="session-picker-overlay" onmousedown={() => (showExplorerPicker = false)}></div>
  <div class="session-picker" style="top:{explorerPickerY}px;left:{explorerPickerX}px;">
    <div class="session-picker-header">Open Explorer Connection</div>
    <div class="session-picker-list">
      {#each $explorerConnections as conn (conn.id)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="session-picker-item ssh-picker-item"
          onmousedown={(e) => { e.stopPropagation(); openExplorerTabFromPicker(conn); }}
        >
          <div class="ssh-picker-text">
            <span class="session-picker-title">{conn.name}</span>
            <span class="ssh-picker-sub">{explorerSubLine(conn)}</span>
          </div>
          <span class="session-picker-badge explorer-badge">{explorerKindBadge(conn.kind)}</span>
        </div>
      {/each}
    </div>
    <div class="session-picker-footer">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <button class="session-picker-new explorer-new" onmousedown={(e) => { e.stopPropagation(); pickerNewExplorerConnection(); }}>
        + New Connection
      </button>
    </div>
  </div>
{/if}

<Toast />
<ContextMenu />
<EnvManagerModal />
<SettingsModal />
<GitHubConnect />
<ShortcutsOverlay show={$activeModal === 'shortcuts'} onclose={() => activeModal.set(null)} />
<SaveRequestDialog bind:show={showSaveDialog} tabId={saveDialogTabId} />
<Onboarding />
<WhatsNewModal />
<UpdateNotification />
<ConfirmDialog
  bind:show={$showSqlDisconnectConfirm}
  title="Disconnect"
  message={`Are you sure you want to disconnect from "${$sqlDisconnectTarget?.name ?? ''}"?`}
  confirmText="Disconnect"
  onconfirm={handleSqlDisconnectConfirm}
/>
<SqlConnectionDialog
  bind:show={$showSqlConnectionDialog}
  editConnection={$editingSqlConnection}
  onsave={handleSqlConnectionSave}
  onclose={() => editingSqlConnection.set(null)}
/>
<NoSqlConnectionDialog
  bind:show={$showNoSqlConnectionDialog}
  connection={$editingNoSqlConnection}
  onsave={handleNoSqlConnectionSave}
  onclose={() => editingNoSqlConnection.set(null)}
/>
<NewSessionModal bind:show={showNewSessionModal} />
<EditSessionModal bind:show={showEditSessionModal} bind:session={editSessionTarget} />
<UsageDashboard bind:show={showUsageDashboard} />

<style>
  .app-shell {
    display: flex;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
  }
  .app-content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--c);
  }
  .app-workspace {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .session-picker-overlay {
    position: fixed;
    inset: 0;
    z-index: 999;
  }

  .session-picker {
    position: fixed;
    z-index: 1000;
    background: var(--n);
    border: 1px solid var(--b1);
    border-radius: 10px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
    min-width: 280px;
    max-width: 360px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .session-picker-header {
    padding: 10px 14px 8px;
    font-size: 11px;
    font-family: var(--ui);
    color: var(--t3);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    border-bottom: 1px solid var(--b1);
  }

  .session-picker-list {
    max-height: 320px;
    overflow-y: auto;
    padding: 4px 0;
  }

  .session-picker-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 14px;
    cursor: pointer;
    transition: background 0.12s;
  }

  .session-picker-item:hover {
    background: var(--b1);
  }

  .session-picker-title {
    font-size: 13px;
    font-family: var(--ui);
    color: var(--t1);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* SSH picker: stacked name + user@host */
  .ssh-picker-item .ssh-picker-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ssh-picker-item .session-picker-title {
    flex: 0 0 auto;
  }
  .ssh-picker-sub {
    font-size: 11px;
    font-family: var(--mono);
    color: var(--t3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .session-picker-badge {
    font-size: 10px;
    font-family: var(--ui);
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid;
    opacity: 0.85;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .session-picker-footer {
    border-top: 1px solid var(--b1);
    padding: 6px 8px;
  }

  .session-picker-new {
    width: 100%;
    background: none;
    border: none;
    cursor: pointer;
    padding: 7px 10px;
    border-radius: var(--radius-md, 6px);
    font-size: 13px;
    font-family: var(--ui);
    color: var(--agent, #d2a8ff);
    text-align: left;
    transition: background 0.12s;
  }

  .session-picker-new:hover {
    background: var(--b1);
  }

  /* Explorer picker uses the explorer accent so kind/badges and the
     "new connection" CTA match the rest of the mode. */
  .explorer-badge {
    color: var(--explorer);
    border-color: var(--explorer);
  }
  .explorer-new {
    color: var(--explorer);
  }
</style>

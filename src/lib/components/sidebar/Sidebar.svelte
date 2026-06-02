<script lang="ts">
  import { onMount } from 'svelte';
  import { mode, navOpen, aiPanelOpen, activeModal, setMode as appSetMode } from '$lib/stores/app';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { isMac, isLinux } from '$lib/utils/platform';
  import { cloudConnected, cloudConflicts, syncing, setSyncing, setDisconnected, showSyncRestorePrompt, markSynced } from '$lib/stores/cloud';
  import ConflictResolverModal from '$lib/components/cloud/ConflictResolverModal.svelte';

  let conflictResolverOpen = $state(false);

  function openConflictResolver() {
    profileMenuOpen = false;
    conflictResolverOpen = true;
  }
  import { cloudSyncPushNow, cloudSyncRestore, cloudLogout } from '$lib/commands/cloud';
  import { loadCollections } from '$lib/modes/rest/stores';
  import { loadEnvironments } from '$lib/modes/rest/stores';
  import { loadConnections as loadSqlConnections, loadSqlScripts } from '$lib/modes/sql/stores';
  import { loadNoSqlConnections } from '$lib/modes/nosql/stores';
  import { friendlyError } from '$lib/utils/errors';
  import SidebarButton from './SidebarButton.svelte';
  import Avatar from './Avatar.svelte';
  import type { AppMode } from '$lib/stores/app';
  import { checkAndDownloadUpdate, showWhatsNewModal, whatsNewContent, updateAvailable } from '$lib/utils/updater';
  import { showToast } from '$lib/shared/primitives/toast';
  import { FULLSCREEN_POLL_INTERVAL_MS } from '$lib/shared/constants/timings';
  import { tabs, activeTabId, openSettingsTab, lastActiveTabPerMode } from '$lib/shared/stores/tabs';
  import { get } from 'svelte/store';
  import { activateTabAcrossMode } from '$lib/utils/tabActivation';

  let profileMenuOpen = $state(false);
  let previousMode: AppMode = 'rest';
  let isFullscreen = $state(false);

  // Custom traffic-light controls render on macOS + Linux — both run with
  // decorations:false (mac via tauri.macos.conf.json for vibrancy; Linux via
  // tauri.linux.conf.json to escape GTK's thick title bar). Windows keeps
  // native chrome since we don't ship a Win-specific custom titlebar.
  const showCustomChrome = isMac() || isLinux();


  onMount(() => {
    // Check fullscreen state periodically
    const checkFs = setInterval(async () => {
      try { isFullscreen = await getCurrentWindow().isFullscreen(); } catch {}
    }, FULLSCREEN_POLL_INTERVAL_MS);
    return () => clearInterval(checkFs);
  });

  async function wcClose() { await getCurrentWindow().close(); }
  async function wcMinimize() { await getCurrentWindow().minimize(); }
  async function wcFullscreen() {
    const win = getCurrentWindow();
    await win.setFullscreen(!(await win.isFullscreen()));
  }

  function toggleNav() {
    navOpen.update(v => !v);
  }

  function setMode(m: AppMode) {
    // Click same mode again with panel already open → close it (cursor is
    // on the sidebar so this is the only way to actively dismiss without
    // moving across to content).
    if ($mode === m && $navOpen) {
      navOpen.set(false);
      return;
    }
    if (m !== 'history') previousMode = m;
    void appSetMode(m);
    if (m === 'canvas') {
      navOpen.set(false);
    } else {
      navOpen.set(true);
    }
    realignActiveTabToMode(m);
  }

  // After a mode switch, if the currently active tab belongs to a
  // different mode, restore the user's last-active tab IN THE NEW MODE
  // so flipping Agent → REST → Agent returns to the exact tab they
  // were on, not just any tab of that mode. Falls back to creation
  // order for the very first switch to a mode (no memory yet) or when
  // the memorized tab has since been closed.
  function realignActiveTabToMode(m: AppMode) {
    const currentActiveId = get(activeTabId);
    const allTabs = get(tabs);
    const currentTab = allTabs.find((t) => t.id === currentActiveId);
    if (currentTab && currentTab.mode === m) return;

    // First preference: the memorized tab for this mode.
    const memorized = get(lastActiveTabPerMode).get(m);
    if (memorized !== undefined && allTabs.some((t) => t.id === memorized)) {
      activateTabAcrossMode(memorized);
      return;
    }

    // Fall back: most-recently-created tab of this mode.
    const newModeTabs = allTabs.filter((t) => t.mode === m);
    if (newModeTabs.length > 0) {
      activateTabAcrossMode(newModeTabs[newModeTabs.length - 1].id);
    }
  }

  function toggleHistory() {
    if ($mode === 'history' && $navOpen) {
      void appSetMode(previousMode);
      navOpen.set(false);
      realignActiveTabToMode(previousMode);
      return;
    }
    if ($mode !== 'history') {
      previousMode = $mode as any;
      void appSetMode('history');
    }
    navOpen.set(true);
  }


  function toggleProfileMenu() {
    profileMenuOpen = !profileMenuOpen;
  }

  let checkingForUpdates = $state(false);

  async function handleCheckForUpdates() {
    profileMenuOpen = false;
    if (checkingForUpdates) return;
    checkingForUpdates = true;
    try {
      const result = await checkAndDownloadUpdate();
      if (result) {
        if (result.infoOnly) {
          showToast(`v${result.version} is available to download`, 'success');
        } else {
          showToast(`Update v${result.version} downloaded — ready to install`, 'success');
        }
      } else {
        showToast("You're on the latest version", 'success');
      }
    } catch {
      showToast('Failed to check for updates', 'error');
    } finally {
      checkingForUpdates = false;
    }
  }

  function formatSyncTime(iso: string | null): string {
    if (!iso) return '';
    try {
      const d = new Date(iso);
      const diff = Date.now() - d.getTime();
      if (diff < 60000) return 'just now';
      if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`;
      if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`;
      return d.toLocaleDateString();
    } catch { return ''; }
  }

  async function handleSyncNow() {
    if ($syncing) return;
    setSyncing(true);
    try {
      const pushed = await cloudSyncPushNow();
      if (pushed.length) {
        markSynced();
        showToast('Synced', 'success');
      } else {
        showToast('Already up to date', 'info');
      }
    } catch (e) {
      showToast(friendlyError(e), 'error');
    } finally {
      setSyncing(false);
    }
  }

  // Handle the "cloud data found on first connect" prompt — pulls all
  // domains, marks synced, then fires the post-restore "re-enter your
  // secrets" banner (which subsumes the success toast when relevant).
  async function handleFirstConnectRestore() {
    showSyncRestorePrompt.set(false);
    setSyncing(true);
    try {
      await cloudSyncRestore();
      await Promise.all([
        loadCollections(),
        loadEnvironments(),
        loadSqlConnections(),
        loadNoSqlConnections(),
        loadSqlScripts(),
      ]);
      markSynced();
      const { announceRestoreCompletion } = await import('$lib/stores/missingCredentials');
      const shown = await announceRestoreCompletion();
      if (!shown) showToast('Restored from cloud', 'success');
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      setSyncing(false);
    }
  }

  function handleFirstConnectSkip() {
    showSyncRestorePrompt.set(false);
    markSynced(); // Don't ask again
  }

  async function handleLogout() {
    profileMenuOpen = false;
    try {
      await cloudLogout();
      setDisconnected();
      showToast('Signed out', 'info');
    } catch (e) {
      showToast(friendlyError(e), 'error');
    }
  }

  async function openExternal(url: string) {
    try {
      const { openUrl } = await import('@tauri-apps/plugin-opener');
      await openUrl(url);
    } catch {
      window.open(url, '_blank');
    }
  }

  function handleProfileAction(action: string) {
    profileMenuOpen = false;
    switch (action) {
      case 'sync': openSettingsTab('account'); break;
      case 'settings': openSettingsTab('account'); break;
      case 'check-updates': handleCheckForUpdates(); break;
      case 'whats-new': openExternal('https://clauge.in/changelog.html'); break;
      case 'report': openExternal('https://github.com/ansxuman/Clauge/issues/new/choose'); break;
      case 'about': openSettingsTab('about'); break;
    }
  }

</script>

<svelte:window onclick={() => profileMenuOpen = false} />

<aside class="sidebar glass-surface">
  <div class="sidebar-drag" data-drag-region></div>
  {#if showCustomChrome}
    <div class="wc-area" data-drag-region>
      <div class="wc-dots">
        <button class="wc-dot wc-close" onclick={wcClose}></button>
        <button class="wc-dot wc-min" onclick={wcMinimize}></button>
        <button class="wc-dot wc-max" onclick={wcFullscreen}></button>
      </div>
    </div>
  {/if}

  <!-- Mode buttons -->
  <SidebarButton label="Agent" tip="Agent" active={$mode === 'agent'} dotColor="var(--agent, #d2a8ff)" id="sbi-agent" onclick={() => setMode('agent')}>
    <!-- Sparkle — universal AI/agent icon (Claude, Cursor, Notion AI) -->
    <svg viewBox="0 0 24 24"><path d="M12 3l1.6 4.8L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.2L12 3z"/><path d="M18.5 14l.9 2.6 2.6.9-2.6.9-.9 2.6-.9-2.6-2.6-.9 2.6-.9.9-2.6z"/></svg>
  </SidebarButton>
  <!-- Canvas sits with Agent + Workspace as the "meta" group (top); data/protocol modes follow. -->
  <SidebarButton label="Atlas" tip="Atlas" active={$mode === 'canvas'} id="sbi-canvas" onclick={() => setMode('canvas')}>
    <!-- Frame corners — spatial canvas / freeform workspace -->
    <svg viewBox="0 0 24 24"><path d="M2 7V3h4"/><path d="M22 7V3h-4"/><path d="M2 17v4h4"/><path d="M22 17v4h-4"/><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
  </SidebarButton>
  <SidebarButton label="Workspace" tip="Workspaces (Notes & Boards)" active={$mode === 'workspace'} id="sbi-workspace" onclick={() => setMode('workspace')}>
    <!-- 2×2 grid — workspace = a dashboard of mixed items. Distinct
         from Explorer's folder, REST's globe, etc. -->
    <svg viewBox="0 0 24 24"><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg>
  </SidebarButton>
  <SidebarButton label="REST" tip="REST API" active={$mode === 'rest'} dotColor="var(--rest)" id="sbi-rest" onclick={() => setMode('rest')}>
    <!-- Globe — clearly "web/HTTP" -->
    <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z"/></svg>
  </SidebarButton>
  <SidebarButton label="SQL" tip="SQL" active={$mode === 'sql'} dotColor="var(--sql)" id="sbi-sql" onclick={() => setMode('sql')}>
    <!-- Database cylinder — kept, slightly tighter -->
    <svg viewBox="0 0 24 24"><ellipse cx="12" cy="5" rx="8" ry="2.5"/><path d="M4 5v14c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5V5"/><path d="M4 12c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5"/></svg>
  </SidebarButton>
  <SidebarButton label="NoSQL" tip="NoSQL" active={$mode === 'nosql'} dotColor="var(--nosql)" id="sbi-nosql" onclick={() => setMode('nosql')}>
    <!-- Curly braces — JSON / NoSQL document style -->
    <svg viewBox="0 0 24 24"><path d="M8 3a2 2 0 00-2 2v4a2 2 0 01-2 2H3a1 1 0 000 2h1a2 2 0 012 2v4a2 2 0 002 2"/><path d="M16 3a2 2 0 012 2v4a2 2 0 002 2h1a1 1 0 010 2h-1a2 2 0 00-2 2v4a2 2 0 01-2 2"/></svg>
  </SidebarButton>
  <SidebarButton label="SSH" tip="SSH" active={$mode === 'ssh'} dotColor="var(--ssh)" id="sbi-ssh" onclick={() => setMode('ssh')}>
    <!-- Terminal prompt >_  — universal "shell" icon -->
    <svg viewBox="0 0 24 24"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>
  </SidebarButton>
  <SidebarButton label="Explorer" tip="Explorer (Files)" active={$mode === 'explorer'} dotColor="var(--explorer)" id="sbi-explorer" onclick={() => setMode('explorer')}>
    <!-- Folder + cloud — "files / storage" -->
    <svg viewBox="0 0 24 24"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>
  </SidebarButton>

  <div class="sb-sep"></div>

  <!-- History -->
  <SidebarButton label="History" tip="History" active={$mode === 'history'} id="sbi-history" onclick={toggleHistory}>
    <!-- Clock with rewind arrow — distinct from a generic clock -->
    <svg viewBox="0 0 24 24"><path d="M3 12a9 9 0 109-9 9.5 9.5 0 00-6.4 2.5L3 8"/><polyline points="3 3 3 8 8 8"/><polyline points="12 8 12 13 15 15"/></svg>
  </SidebarButton>

  <!-- Bottom section -->
  <div class="sb-bot">
    <div class="profile-wrap">
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="avatar-slot" onclick={(e: MouseEvent) => { e.stopPropagation(); toggleProfileMenu(); }}>
        <Avatar />
      </div>
      {#if profileMenuOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div class="profile-menu" onclick={(e: MouseEvent) => e.stopPropagation()}>
          {#if $cloudConnected}
            <div class="pm-sync-status">
              <svg viewBox="0 0 24 24"><path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10 10-4.5 10-10S17.5 2 12 2z"/><path d="M7.5 12l3 3 6-6"/></svg>
              <span class="pm-sync-label">All Data Synced</span>
            </div>
            {#if $cloudConflicts.length > 0}
              <!-- Conflict-mode replacement for the Sync Now row. Same
                   position, different label + accent treatment so the
                   user reaches resolution from the same place. -->
              <button class="pm-item pm-action-required" onclick={openConflictResolver}>
                <svg viewBox="0 0 24 24"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9"  x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>
                Action Required ({$cloudConflicts.length})
              </button>
            {:else}
              <button class="pm-item" onclick={() => { handleSyncNow(); }}>
                <svg class:pm-spinning={$syncing} viewBox="0 0 24 24"><path d="M23 4v6h-6"/><path d="M1 20v-6h6"/><path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/></svg>
                {$syncing ? 'Syncing...' : 'Sync Now'}
              </button>
            {/if}
            <button class="pm-item" onclick={() => handleProfileAction('sync')}>
              <svg viewBox="0 0 24 24"><path d="M18 8a6 6 0 11-12 0 6 6 0 0112 0z"/><path d="M3 21v-2a4 4 0 014-4h10a4 4 0 014 4v2"/></svg>
              Account
            </button>
          {:else}
            <button class="pm-item" onclick={() => handleProfileAction('sync')}>
              <svg class="gh-icon" viewBox="0 0 24 24"><path d="M18 8a6 6 0 11-12 0 6 6 0 0112 0z"/><path d="M3 21v-2a4 4 0 014-4h10a4 4 0 014 4v2"/></svg>
              Sign in to sync
            </button>
          {/if}
          <div class="pm-sep"></div>

          <button class="pm-item" onclick={() => handleProfileAction('settings')}>
            <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z"/></svg>
            Settings
          </button>
          <button class="pm-item" onclick={() => handleProfileAction('check-updates')}>
            <svg viewBox="0 0 24 24"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
            {checkingForUpdates ? 'Checking...' : 'Check for Updates'}
          </button>
          <div class="pm-sep"></div>
          <button class="pm-item" onclick={() => handleProfileAction('whats-new')}>
            <svg viewBox="0 0 24 24"><path d="M12 2L15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26z"/></svg>
            What's New
            <svg class="pm-external" viewBox="0 0 24 24"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
          </button>
          <button class="pm-item" onclick={() => handleProfileAction('report')}>
            <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M12 8v4M12 16h.01"/></svg>
            Report an Issue
            <svg class="pm-external" viewBox="0 0 24 24"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
          </button>
          {#if $cloudConnected}
            <div class="pm-sep"></div>
            <button class="pm-item pm-logout" onclick={handleLogout}>
              <svg viewBox="0 0 24 24"><path d="M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" y1="12" x2="9" y2="12"/></svg>
              Sign out
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</aside>

<ConflictResolverModal bind:show={conflictResolverOpen} />

<!-- First-connect restore prompt: when existing cloud data found.
     Overlay click is NOT bound to skip — a misclick outside the modal
     used to call markSynced() which permanently dismissed the prompt
     (`hasSyncedOnce` is persisted to localStorage). The user has to
     click Skip explicitly. -->
{#if $showSyncRestorePrompt}
  <div class="sync-confirm-overlay">
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="sync-confirm" onclick={(e) => e.stopPropagation()}>
      <div class="sync-confirm-icon">
        <svg viewBox="0 0 24 24" width="28" height="28"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
      </div>
      <div class="sync-confirm-title">Cloud backup found</div>
      <div class="sync-confirm-desc">Your account has data saved in the cloud. Restoring replaces this device's REST collections, SQL/NoSQL connections, agents, SSH profiles, explorer paths, and workspace coworkers with the cloud copy.</div>
      <div class="sync-confirm-actions">
        <button class="sync-confirm-btn" onclick={handleFirstConnectSkip}>Skip</button>
        <button class="sync-confirm-btn primary" onclick={handleFirstConnectRestore}>Restore</button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ── Sync Confirm Dialog ── */
  .sync-confirm-overlay {
    position: fixed;
    top: 0; left: 0; right: 0; bottom: 0;
    background: var(--scrim-strong);
    z-index: var(--z-drawer);
    display: flex;
    align-items: center;
    justify-content: center;
    animation: scFadeIn 0.12s ease;
  }
  @keyframes scFadeIn { from { opacity: 0; } to { opacity: 1; } }
  .sync-confirm {
    background: var(--modal-bg, var(--n));
    border: 1px solid var(--b2);
    border-radius: var(--radius-xl);
    box-shadow: 0 24px 48px rgba(0,0,0,0.5);
    padding: 28px 32px;
    width: 360px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    animation: scSlideIn 0.15s ease;
  }
  @keyframes scSlideIn { from { opacity: 0; transform: translateY(-8px); } to { opacity: 1; transform: translateY(0); } }
  .sync-confirm-icon svg {
    stroke: var(--acc);
    fill: none;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .sync-confirm-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }
  .sync-confirm-desc {
    font-size: 13px;
    color: var(--t2);
    font-family: var(--ui);
    text-align: center;
    line-height: 1.5;
  }
  .sync-confirm-actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
    width: 100%;
  }
  .sync-confirm-btn {
    flex: 1;
    height: 32px;
    border-radius: 8px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: border-color 0.12s, color 0.12s;
  }
  .sync-confirm-btn:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .sync-confirm-btn.primary {
    border: none;
    background: var(--acc);
    color: #fff;
    font-weight: 600;
  }
  .sync-confirm-btn.primary:hover {
    opacity: 0.9;
  }
  .sidebar {
    width: 72px;
    background: var(--s);
    border-right: 1px solid var(--b1);
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 0 0 10px;
    gap: 2px;
    flex-shrink: 0;
    z-index: 150;
  }
  .sidebar-drag {
    height: 0px;
    flex-shrink: 0;
    width: 100%;
  }
  .wc-area {
    width: 72px;
    height: 46px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .wc-dots {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .wc-dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: none;
    cursor: default;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: filter 0.1s;
  }
  .wc-dot:hover { filter: brightness(0.85); }
  .wc-close { background: #ff5f57; }
  .wc-min { background: #febc2e; }
  .wc-max { background: #28c840; }
  :global(body.window-blurred) .wc-dot {
    background: var(--t4) !important;
  }
  .sb-sep {
    width: 24px;
    height: 1px;
    background: var(--b1);
    margin: 6px 0;
  }
  .sb-bot {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  .profile-wrap {
    position: relative;
  }
  .avatar-slot {
    position: relative;
    display: inline-flex;
    cursor: pointer;
  }
  .profile-menu {
    position: absolute;
    bottom: 100%;
    left: calc(100% + 8px);
    margin-bottom: 4px;
    background: var(--modal-bg, #101016);
    border: 1px solid var(--b1);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0,0,0,0.5);
    z-index: 200;
    min-width: 180px;
    padding: 4px;
    animation: pmIn 0.12s ease;
  }
  @keyframes pmIn {
    from { opacity: 0; transform: translateY(4px); }
    to { opacity: 1; transform: none; }
  }
  .pm-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 12px;
    border: none;
    background: transparent;
    color: var(--t1);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    border-radius: 5px;
    transition: background 0.08s;
    white-space: nowrap;
  }
  .pm-item:hover {
    background: var(--surface-hover);
  }
  .pm-item svg {
    width: 14px;
    height: 14px;
    stroke: var(--t2);
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex-shrink: 0;
    min-width: 14px;
  }
  .pm-sep {
    height: 1px;
    background: var(--b1);
    margin: 4px 8px;
  }
  .pm-item :global(.gh-icon) {
    fill: var(--t2);
    stroke: none !important;
    width: 14px !important;
    height: 14px !important;
    min-width: 14px !important;
  }
  .pm-sync-status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    font-size: 11px;
    color: var(--state-saved);
    font-family: var(--ui);
  }
  .pm-sync-status svg {
    width: 14px; height: 14px; min-width: 14px;
    stroke: var(--state-saved); fill: none;
    stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round;
    flex-shrink: 0;
  }
  .pm-sync-label { font-weight: 500; }
  .pm-sync-time { color: var(--t3); font-size: 10px; }
  .pm-external {
    width: 10px !important; height: 10px !important;
    stroke: var(--t3) !important;
    margin-left: auto;
    flex-shrink: 0;
  }
  :global(.pm-spinning) {
    animation: pmSpin 0.8s linear infinite;
  }
  @keyframes pmSpin {
    to { transform: rotate(360deg); }
  }
  /* "Action Required" replaces the Sync Now row when there are unresolved
     conflicts — accent tint signals the user has a decision to make. */
  .pm-action-required {
    color: var(--acc) !important;
    font-weight: 600;
  }
  .pm-action-required svg { stroke: var(--acc) !important; }
  .pm-action-required:hover {
    background: color-mix(in srgb, var(--acc) 10%, transparent) !important;
  }
  .pm-logout { color: var(--err) !important; }
  .pm-logout svg { stroke: var(--err) !important; }
  .pm-logout:hover { background: rgba(240,68,68,0.08) !important; }
  :global(.spin) {
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>

<script lang="ts">
  import { onMount } from 'svelte';
  import { mode, navOpen, aiPanelOpen, activeModal } from '$lib/stores/app';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { activeHistoryEntry } from '$lib/stores/history';
  import { githubConnected, githubUsername, githubAvatarUrl, syncing, lastSyncedAt, setSyncing, setLastSynced, setDisconnected, showSyncRestorePrompt, markSynced } from '$lib/stores/github';
  import { gistSyncPush, gistSyncPull, githubDisconnect } from '$lib/commands/github';
  import { loadCollections } from '$lib/stores/collections';
  import { loadEnvironments } from '$lib/stores/environments';
  import { loadConnections as loadSqlConnections, loadSqlScripts } from '$lib/stores/sql';
  import { loadNoSqlConnections } from '$lib/stores/nosql';
  import { friendlyError } from '$lib/utils/errors';
  import SidebarButton from './SidebarButton.svelte';
  import Avatar from './Avatar.svelte';
  import type { AppMode } from '$lib/stores/app';
  import { checkAndDownloadUpdate, showWhatsNewModal, whatsNewContent, updateAvailable } from '$lib/utils/updater';
  import { showToast } from '$lib/components/shared/toast';

  let profileMenuOpen = $state(false);
  let previousMode: AppMode = 'rest';
  let isFullscreen = $state(false);


  onMount(() => {
    // Check fullscreen state periodically
    const checkFs = setInterval(async () => {
      try { isFullscreen = await getCurrentWindow().isFullscreen(); } catch {}
    }, 1000);
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
    if (m !== 'history') previousMode = m;
    mode.set(m);
    activeHistoryEntry.set(null);
    if (!$navOpen) navOpen.set(true);
  }

  function toggleHistory() {
    if ($mode === 'history') {
      mode.set(previousMode);
      activeHistoryEntry.set(null);
    } else {
      previousMode = $mode as any;
      mode.set('history');
    }
    if (!$navOpen) navOpen.set(true);
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
        showToast(`Update v${result.version} downloaded — ready to install`, 'success');
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

  let showRestoreConfirm = $state(false);

  async function handleSyncNow() {
    if ($syncing) return;
    setSyncing(true);
    try {
      const msg = await gistSyncPush();
      console.info('[Clauge Sync]', msg);
      if (!msg.includes('Skipped')) {
        markSynced();
        setLastSynced(new Date().toISOString());
        showToast('Synced to cloud', 'success');
      } else {
        // Push was skipped (empty data) — offer restore
        showRestoreConfirm = true;
      }
    } catch (e) {
      showToast(friendlyError(e), 'error');
    } finally {
      setSyncing(false);
    }
  }

  async function handleRestoreConfirmed() {
    showRestoreConfirm = false;
    setSyncing(true);
    try {
      const msg = await gistSyncPull();
      console.info('[Clauge Sync]', msg);
      await Promise.all([
        loadCollections(),
        loadEnvironments(),
        loadSqlConnections(),
        loadNoSqlConnections(),
        loadSqlScripts(),
      ]);
      markSynced();
      setLastSynced(new Date().toISOString());
      showToast('Restored from cloud', 'success');
    } catch (e: any) {
      if (String(e).includes('empty') || String(e).includes('No Clauge sync gist')) {
        showToast('Cloud backup is empty — nothing to restore', 'info');
      } else {
        showToast(friendlyError(e), 'error');
      }
    } finally {
      setSyncing(false);
    }
  }

  // Handle the "cloud data found on first connect" prompt
  async function handleFirstConnectRestore() {
    showSyncRestorePrompt.set(false);
    await handleRestoreConfirmed();
  }

  function handleFirstConnectSkip() {
    showSyncRestorePrompt.set(false);
    markSynced(); // Don't ask again
  }

  async function handleLogout() {
    profileMenuOpen = false;
    try {
      await githubDisconnect();
      setDisconnected();
      showToast('Logged out', 'info');
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
      case 'sync': activeModal.set('github'); break;
      case 'settings': activeModal.set('settings'); break;
      case 'check-updates': handleCheckForUpdates(); break;
      case 'whats-new': openExternal('https://clauge.dev/changelog.html'); break;
      case 'report': openExternal('https://github.com/ansxuman/Clauge/issues/new'); break;
      case 'coffee': openExternal('https://buymeacoffee.com/ansxuman'); break;
      case 'about': activeModal.set('settings'); break;
    }
  }

</script>

<svelte:window onclick={() => profileMenuOpen = false} />

<aside class="sidebar glass-surface">
  <div class="sidebar-drag" data-drag-region></div>
  <div class="wc-area" data-drag-region>
    <div class="wc-dots">
      <button class="wc-dot wc-close" onclick={wcClose}></button>
      <button class="wc-dot wc-min" onclick={wcMinimize}></button>
      <button class="wc-dot wc-max" onclick={wcFullscreen}></button>
    </div>
  </div>

  <!-- Mode buttons -->
  <SidebarButton tip="Agent" active={$mode === 'agent'} dotColor="var(--agent, #d2a8ff)" id="sbi-agent" onclick={() => setMode('agent')}>
    <svg viewBox="0 0 24 24"><rect x="4" y="2" width="16" height="20" rx="2" ry="2"/><line x1="8" y1="6" x2="16" y2="6"/><line x1="8" y1="10" x2="16" y2="10"/><line x1="8" y1="14" x2="12" y2="14"/></svg>
  </SidebarButton>
  <SidebarButton tip="REST API" active={$mode === 'rest'} dotColor="var(--rest)" id="sbi-rest" onclick={() => setMode('rest')}>
    <svg viewBox="0 0 24 24"><path d="M10 13a5 5 0 007.54.54l3-3a5 5 0 00-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 00-7.54-.54l-3 3a5 5 0 007.07 7.07l1.71-1.71"/></svg>
  </SidebarButton>
  <SidebarButton tip="SQL" active={$mode === 'sql'} dotColor="var(--sql)" id="sbi-sql" onclick={() => setMode('sql')}>
    <svg viewBox="0 0 24 24"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>
  </SidebarButton>
  <SidebarButton tip="NoSQL" active={$mode === 'nosql'} dotColor="var(--nosql)" id="sbi-nosql" onclick={() => setMode('nosql')}>
    <svg viewBox="0 0 24 24" fill="none"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="13" y2="17"/></svg>
  </SidebarButton>
  <SidebarButton tip="SSH" active={$mode === 'ssh'} dotColor="var(--ssh)" id="sbi-ssh" onclick={() => setMode('ssh')}>
    <svg viewBox="0 0 24 24" fill="none"><rect x="2" y="4" width="20" height="6" rx="1"/><rect x="2" y="14" width="20" height="6" rx="1"/><line x1="6" y1="7" x2="6.01" y2="7"/><line x1="6" y1="17" x2="6.01" y2="17"/></svg>
  </SidebarButton>

  <div class="sb-sep"></div>

  <!-- History -->
  <SidebarButton tip="History" active={$mode === 'history'} id="sbi-history" onclick={toggleHistory}>
    <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/></svg>
  </SidebarButton>

  <!-- Bottom section -->
  <div class="sb-bot">
    <div class="profile-wrap">
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div onclick={(e: MouseEvent) => { e.stopPropagation(); toggleProfileMenu(); }}>
        <Avatar />
      </div>
      {#if profileMenuOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div class="profile-menu" onclick={(e: MouseEvent) => e.stopPropagation()}>
          {#if $githubConnected}
            <!-- Sync status -->
            <div class="pm-sync-status">
              <svg viewBox="0 0 24 24"><path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10 10-4.5 10-10S17.5 2 12 2z"/><path d="M7.5 12l3 3 6-6"/></svg>
              <span class="pm-sync-label">All Data Synced</span>
              {#if $lastSyncedAt}
                <span class="pm-sync-time">· {formatSyncTime($lastSyncedAt)}</span>
              {/if}
            </div>
            <!-- Sync Now -->
            <button class="pm-item" onclick={() => { handleSyncNow(); }}>
              <svg class:pm-spinning={$syncing} viewBox="0 0 24 24"><path d="M23 4v6h-6"/><path d="M1 20v-6h6"/><path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/></svg>
              {$syncing ? 'Syncing...' : 'Sync Now'}
            </button>
            <!-- GitHub user -->
            <button class="pm-item" onclick={() => openExternal(`https://github.com/${$githubUsername}`)}>
              <svg class="gh-icon" viewBox="0 0 16 16"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>
              <span class="pm-gh-user">@{$githubUsername}</span>
              <svg class="pm-external" viewBox="0 0 24 24"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
            </button>
          {:else}
            <button class="pm-item" onclick={() => handleProfileAction('sync')}>
              <svg class="gh-icon" viewBox="0 0 16 16"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>
              Connect GitHub
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
          <button class="pm-item pm-coffee" onclick={() => handleProfileAction('coffee')}>
            <svg viewBox="0 0 24 24"><path d="M17 8h1a4 4 0 110 8h-1"/><path d="M3 8h14v9a4 4 0 01-4 4H7a4 4 0 01-4-4V8z"/><line x1="6" y1="2" x2="6" y2="4"/><line x1="10" y1="2" x2="10" y2="4"/><line x1="14" y1="2" x2="14" y2="4"/></svg>
            Buy Me a Coffee
            <svg class="pm-external" viewBox="0 0 24 24"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
          </button>
          {#if $githubConnected}
            <div class="pm-sep"></div>
            <button class="pm-item pm-logout" onclick={handleLogout}>
              <svg viewBox="0 0 24 24"><path d="M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" y1="12" x2="9" y2="12"/></svg>
              Logout
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</aside>

<!-- Restore confirm: when Sync Now is clicked but local is empty -->
{#if showRestoreConfirm}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="sync-confirm-overlay" onclick={() => showRestoreConfirm = false}>
    <div class="sync-confirm" onclick={(e) => e.stopPropagation()}>
      <div class="sync-confirm-icon">
        <svg viewBox="0 0 24 24" width="28" height="28"><path d="M23 4v6h-6"/><path d="M1 20v-6h6"/><path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/></svg>
      </div>
      <div class="sync-confirm-title">Local data is empty</div>
      <div class="sync-confirm-desc">Would you like to restore your data from cloud?</div>
      <div class="sync-confirm-actions">
        <button class="sync-confirm-btn" onclick={() => showRestoreConfirm = false}>Cancel</button>
        <button class="sync-confirm-btn primary" onclick={handleRestoreConfirmed}>Restore from Cloud</button>
      </div>
    </div>
  </div>
{/if}

<!-- First-connect restore prompt: when existing cloud data found -->
{#if $showSyncRestorePrompt}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="sync-confirm-overlay" onclick={handleFirstConnectSkip}>
    <div class="sync-confirm" onclick={(e) => e.stopPropagation()}>
      <div class="sync-confirm-icon">
        <svg viewBox="0 0 24 24" width="28" height="28"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
      </div>
      <div class="sync-confirm-title">Cloud backup found</div>
      <div class="sync-confirm-desc">Your GitHub account has existing data. Would you like to restore it?</div>
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
    background: rgba(0,0,0,0.5);
    backdrop-filter: blur(8px);
    z-index: 9999;
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
    background: rgba(255,255,255,0.06);
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
    color: var(--ok);
    font-family: var(--ui);
  }
  .pm-sync-status svg {
    width: 14px; height: 14px; min-width: 14px;
    stroke: var(--ok); fill: none;
    stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round;
    flex-shrink: 0;
  }
  .pm-sync-label { font-weight: 500; }
  .pm-sync-time { color: var(--t3); font-size: 10px; }
  .pm-gh-user { }
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
  .pm-coffee { color: #f5a623 !important; }
  .pm-coffee svg { stroke: #f5a623 !important; }
  .pm-coffee:hover { background: rgba(245,166,35,0.08) !important; }
  .pm-logout { color: var(--err) !important; }
  .pm-logout svg { stroke: var(--err) !important; }
  .pm-logout:hover { background: rgba(240,68,68,0.08) !important; }
  /* Active mode left-border accent pills */
  :global(#sbi-agent.on::before) {
    content: '';
    position: absolute;
    left: -1px;
    top: 7px;
    bottom: 7px;
    width: 2px;
    border-radius: 0 2px 2px 0;
    background: var(--agent);
  }
  :global(#sbi-rest.on::before) {
    content: '';
    position: absolute;
    left: -1px;
    top: 7px;
    bottom: 7px;
    width: 2px;
    border-radius: 0 2px 2px 0;
    background: var(--rest);
  }
  :global(#sbi-sql.on::before) {
    content: '';
    position: absolute;
    left: -1px;
    top: 7px;
    bottom: 7px;
    width: 2px;
    border-radius: 0 2px 2px 0;
    background: var(--sql);
  }
  :global(#sbi-nosql.on::before) {
    content: '';
    position: absolute;
    left: -1px;
    top: 7px;
    bottom: 7px;
    width: 2px;
    border-radius: 0 2px 2px 0;
    background: var(--nosql);
  }
  :global(#sbi-ssh.on::before) {
    content: '';
    position: absolute;
    left: -1px;
    top: 7px;
    bottom: 7px;
    width: 2px;
    border-radius: 0 2px 2px 0;
    background: var(--ssh);
  }
  :global(#sbi-history.on::before) {
    content: '';
    position: absolute;
    left: -1px;
    top: 7px;
    bottom: 7px;
    width: 2px;
    border-radius: 0 2px 2px 0;
    background: var(--acc);
  }
  :global(.spin) {
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>

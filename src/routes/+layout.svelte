<script lang="ts">
  import '../app.css';
  import Sidebar from '$lib/components/sidebar/Sidebar.svelte';
  import NavPanel from '$lib/components/nav/NavPanel.svelte';
  import Topbar from '$lib/components/topbar/Topbar.svelte';
  import StatusBar from '$lib/components/statusbar/StatusBar.svelte';
  import Toast from '$lib/components/shared/Toast.svelte';
  import ContextMenu from '$lib/components/shared/ContextMenu.svelte';
  import EnvManagerModal from '$lib/components/env/EnvManagerModal.svelte';
  import SettingsModal from '$lib/components/settings/SettingsModal.svelte';
  import GitHubConnect from '$lib/components/github/GitHubConnect.svelte';
  import NewSessionModal from '$lib/components/agent/NewSessionModal.svelte';
  import EditSessionModal from '$lib/components/agent/EditSessionModal.svelte';
  import favicon from '$lib/assets/favicon.svg';

  import { onMount, onDestroy } from 'svelte';
  import { loadCollections, clearActiveRequest, collections } from '$lib/stores/collections';
  import { loadEnvironments } from '$lib/stores/environments';
  import { loadConnections as loadSqlConnections, loadSqlScripts, showSqlConnectionDialog, editingSqlConnection, handleSqlConnectionSave, showSqlDisconnectConfirm, sqlDisconnectTarget, disconnectFromDb, connections as sqlConnections } from '$lib/stores/sql';
  import { showToast } from '$lib/components/shared/toast';
  import ConfirmDialog from '$lib/components/shared/ConfirmDialog.svelte';
  import { loadNoSqlConnections, showNoSqlConnectionDialog, editingNoSqlConnection, handleNoSqlConnectionSave, nosqlConnections } from '$lib/stores/nosql';
  import SqlConnectionDialog from '$lib/components/sql/ConnectionDialog.svelte';
  import NoSqlConnectionDialog from '$lib/components/nosql/ConnectionDialog.svelte';
  import { loadSettings, loadAppearance, appearance } from '$lib/stores/settings';
  import { setConnected, setLastSynced, hasSyncedOnce, markSynced, showSyncRestorePrompt } from '$lib/stores/github';
  import { githubGetStatus, gistCheckExists, gistSyncPush, gistSyncPull } from '$lib/commands/github';
  import { activeModal, aiPanelOpen } from '$lib/stores/app';
  import AIPanel from '$lib/components/ai/AIPanel.svelte';
  import { tabs, addTab, activeTabId } from '$lib/stores/tabs';
  import type { AgentSession } from '$lib/types/agent';
  import { setupGlobalShortcuts, teardownGlobalShortcuts } from '$lib/utils/shortcuts';
  import { applyTheme } from '$lib/utils/theme';
  import ShortcutsOverlay from '$lib/components/shared/ShortcutsOverlay.svelte';
  import SaveRequestDialog from '$lib/components/shared/SaveRequestDialog.svelte';
  import Onboarding from '$lib/components/onboarding/Onboarding.svelte';
  import WhatsNewModal from '$lib/components/shared/WhatsNewModal.svelte';
  import UpdateNotification from '$lib/components/shared/UpdateNotification.svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { get } from 'svelte/store';

  let { children } = $props();

  let showSaveDialog = $state(false);
  let saveDialogTabId = $state(-1);
  let syncInterval: ReturnType<typeof setInterval> | null = null;

  let showNewSessionModal = $state(false);
  let showEditSessionModal = $state(false);
  let editSessionTarget = $state<AgentSession | null>(null);

  function handleAgentNewSession() {
    showNewSessionModal = true;
  }

  function handleAgentEditSession(e: Event) {
    const detail = (e as CustomEvent<AgentSession>).detail;
    if (detail) {
      editSessionTarget = detail;
      showEditSessionModal = true;
    }
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
    window.removeEventListener('clauge:save-new-request', handleSaveNewRequest);
    window.removeEventListener('agent:new-session', handleAgentNewSession);
    window.removeEventListener('agent:edit-session', handleAgentEditSession);
    if (syncInterval) clearInterval(syncInterval);
  });

  function applyAppearanceOnStartup() {
    const config = get(appearance);
    applyTheme(config.theme || 'dark-glass', config.accentColor || '#6366f1');
  }

  // Disable macOS autocorrect/autocapitalize on all inputs
  function disableAutocorrect(el: Element) {
    el.setAttribute('autocorrect', 'off');
    el.setAttribute('autocapitalize', 'off');
    if (!el.hasAttribute('spellcheck')) el.setAttribute('spellcheck', 'false');
  }

  onMount(async () => {
    setupGlobalShortcuts();
    window.addEventListener('clauge:save-new-request', handleSaveNewRequest);
    window.addEventListener('agent:new-session', handleAgentNewSession);
    window.addEventListener('agent:edit-session', handleAgentEditSession);

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
        }, 5 * 60 * 1000);
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
</style>

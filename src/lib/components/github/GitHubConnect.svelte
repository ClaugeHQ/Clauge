<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import Modal from '$lib/components/shared/Modal.svelte';
  import { activeModal } from '$lib/stores/app';
  import { githubConnected, githubUsername, setConnected, setDisconnected, syncing, setSyncing, setLastSynced, markSynced, showSyncRestorePrompt } from '$lib/stores/github';
  import { githubDisconnect, githubGetOauthUrl, githubConnectWithToken, gistSyncPull, gistCheckExists } from '$lib/commands/github';
  import { collections } from '$lib/modes/rest/stores';
  import { connections as sqlConnections } from '$lib/modes/sql/stores';
  import { nosqlConnections } from '$lib/modes/nosql/stores';
  import { get } from 'svelte/store';
  import { showToast } from '$lib/components/shared/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { loadCollections } from '$lib/modes/rest/stores';
  import { loadEnvironments } from '$lib/modes/rest/stores';
  import { loadConnections as loadSqlConnections, loadSqlScripts } from '$lib/modes/sql/stores';
  import { loadNoSqlConnections } from '$lib/modes/nosql/stores';

  let show = $state(false);
  let connecting = $state(false);
  let disconnecting = $state(false);

  $effect(() => {
    show = $activeModal === 'github';
  });

  $effect(() => {
    if (!show && $activeModal === 'github') {
      activeModal.set(null);
    }
  });

  // Listen for deep link callback with OAuth token
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    try {
      const { onOpenUrl } = await import('@tauri-apps/plugin-deep-link');
      unlisten = await onOpenUrl(async (urls) => {
        for (const url of urls) {
          if (url.includes('oauth-callback')) {
            const params = new URL(url).searchParams;
            const token = params.get('token');
            if (token) {
              connecting = true;
              try {
                const username = await githubConnectWithToken(token);
                setConnected(username);
                showToast(`Connected as ${username}`, 'success');
                activeModal.set(null);
                // Check if local is empty and cloud has data to restore
                const localEmpty = get(collections).length === 0
                  && get(sqlConnections).length === 0
                  && get(nosqlConnections).length === 0;
                if (localEmpty) {
                  try {
                    const gistExists = await gistCheckExists();
                    if (gistExists) showSyncRestorePrompt.set(true);
                    else markSynced();
                  } catch { markSynced(); }
                } else {
                  markSynced();
                }
              } catch (e) {
                showToast(friendlyError(e), 'error');
              } finally {
                connecting = false;
              }
            }
          }
        }
      });
    } catch {
      // Deep link not available in dev mode
    }
  });

  onDestroy(() => {
    unlisten?.();
  });

  async function handleConnect() {
    connecting = true;
    try {
      const url = await githubGetOauthUrl();
      try {
        const { openUrl } = await import('@tauri-apps/plugin-opener');
        await openUrl(url);
      } catch {
        window.open(url, '_blank');
      }
      // Wait for deep link callback — connecting stays true
    } catch (e) {
      showToast(friendlyError(e), 'error');
      connecting = false;
    }
  }

  async function handleDisconnect() {
    disconnecting = true;
    try {
      await githubDisconnect();
      setDisconnected();
      showToast('Disconnected from GitHub', 'info');
    } catch (e) {
      showToast(friendlyError(e), 'error');
    } finally {
      disconnecting = false;
    }
  }

  async function handleRestore() {
    if ($syncing) return;
    setSyncing(true);
    try {
      await gistSyncPull();
      await Promise.all([
        loadCollections(),
        loadEnvironments(),
        loadSqlConnections(),
        loadNoSqlConnections(),
        loadSqlScripts(),
      ]);
      markSynced();
      setLastSynced(new Date().toISOString());
      showToast('Restored all data from cloud', 'success');
    } catch (e) {
      showToast(friendlyError(e), 'error');
    } finally {
      setSyncing(false);
    }
  }
</script>

<Modal bind:show title="GitHub Sync" width="420px">
  {#if $githubConnected}
    <div class="gh-connected">
      <div class="gh-avatar-row">
        <div class="gh-avatar-circle">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 00-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0020 4.77 5.07 5.07 0 0019.91 1S18.73.65 16 2.48a13.38 13.38 0 00-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 005 4.77a5.44 5.44 0 00-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 009 18.13V22"/>
          </svg>
        </div>
        <div class="gh-info">
          <span class="gh-username">{$githubUsername}</span>
          <span class="gh-status-text">Connected</span>
        </div>
      </div>
      <div class="gh-actions">
        <button class="gh-restore-btn" onclick={handleRestore} disabled={$syncing}>
          {$syncing ? 'Restoring...' : 'Restore from Cloud'}
        </button>
        <button class="gh-disconnect-btn" onclick={handleDisconnect} disabled={disconnecting}>
          {disconnecting ? 'Disconnecting...' : 'Disconnect'}
        </button>
      </div>
    </div>
  {:else if connecting}
    <div class="gh-waiting">
      <span class="gh-spinner"></span>
      <p class="gh-desc">Waiting for GitHub authorization...</p>
      <p class="gh-hint">Complete the authorization in your browser, then return here.</p>
      <button class="gh-cancel-btn" onclick={() => connecting = false}>Cancel</button>
    </div>
  {:else}
    <div class="gh-login">
      <p class="gh-desc">
        Connect your GitHub account to sync your collections, requests, and environments via GitHub Gist.
      </p>
      <button class="gh-connect-btn" onclick={handleConnect}>
        <svg viewBox="0 0 16 16" fill="currentColor"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>
        Connect with GitHub
      </button>
    </div>
  {/if}
</Modal>

<style>
  .gh-login { display: flex; flex-direction: column; gap: 16px; align-items: center; padding: 8px 0; }
  .gh-desc { font-size: 13px; color: var(--t2); font-family: var(--ui); line-height: 1.5; margin: 0; text-align: center; }
  .gh-hint { font-size: 11px; color: var(--t3); font-family: var(--ui); margin: 0; text-align: center; }
  .gh-connect-btn {
    display: flex; align-items: center; gap: 8px;
    padding: 10px 24px; border-radius: 8px; border: none;
    background: var(--acc); color: #fff;
    font-family: var(--ui); font-size: 12px; font-weight: 600;
    cursor: default; transition: opacity 0.15s;
  }
  .gh-connect-btn:hover { opacity: 0.9; }
  .gh-connect-btn svg { width: 16px; height: 16px; }

  /* Waiting state */
  .gh-waiting { display: flex; flex-direction: column; align-items: center; gap: 12px; padding: 16px 0; }
  .gh-spinner {
    width: 24px; height: 24px; border: 3px solid var(--b1); border-top-color: var(--acc);
    border-radius: 50%; animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .gh-cancel-btn {
    height: 28px; padding: 0 14px; border-radius: 8px;
    border: 1px solid var(--b1); background: transparent;
    color: var(--t3); font-size: 12px; font-family: var(--ui); cursor: default;
  }
  .gh-cancel-btn:hover { color: var(--t1); border-color: var(--b2); }

  /* Connected */
  .gh-connected { display: flex; flex-direction: column; align-items: center; gap: 20px; padding: 16px 0; }
  .gh-avatar-row { display: flex; align-items: center; gap: 14px; }
  .gh-avatar-circle {
    width: 44px; height: 44px; border-radius: 50%;
    background: linear-gradient(135deg, var(--acc), #1dc880);
    display: flex; align-items: center; justify-content: center; color: #fff;
  }
  .gh-avatar-circle svg { width: 22px; height: 22px; }
  .gh-info { display: flex; flex-direction: column; gap: 2px; }
  .gh-username { font-size: 15px; font-weight: 600; color: var(--t1); font-family: var(--ui); }
  .gh-status-text { font-size: 12px; color: #1dc880; font-family: var(--ui); }
  .gh-actions { display: flex; flex-direction: column; gap: 8px; align-items: center; width: 100%; }
  .gh-restore-btn {
    padding: 8px 24px; border-radius: 8px; border: none;
    background: var(--acc); color: #fff; font-family: var(--ui); font-size: 12px; font-weight: 600;
    cursor: default; transition: opacity 0.15s; width: 100%; max-width: 200px;
  }
  .gh-restore-btn:hover:not(:disabled) { opacity: 0.9; }
  .gh-restore-btn:disabled { opacity: 0.5; }
  .gh-disconnect-btn {
    padding: 7px 24px; border-radius: 8px; border: 1px solid var(--b1);
    background: transparent; color: var(--t2); font-family: var(--ui); font-size: 12px;
    cursor: default; transition: border-color 0.15s, color 0.15s, background 0.15s;
  }
  .gh-disconnect-btn:hover:not(:disabled) { border-color: #f04444; color: #f04444; background: rgba(240,68,68,0.08); }
  .gh-disconnect-btn:disabled { opacity: 0.5; }
</style>

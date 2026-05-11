<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { activeModal } from '$lib/stores/app';
  import {
    cloudConnected, cloudUser, cloudProviders, activeProvider,
    cloudPlan, syncing, setSyncing, setConnected, setDisconnected,
    showSyncRestorePrompt, markSynced, lastSyncedByKind, setLastSyncedForKinds,
    type Provider,
  } from '$lib/stores/cloud';
  import {
    cloudGithubLoginUrl, cloudGoogleLoginUrl, cloudExchangeCode,
    cloudLogout, cloudWipeRemote, cloudDeleteAccount, cloudSyncRestore,
    cloudSyncPushNow, cloudLinkProvider, cloudUnlinkProvider,
    cloudGetStatus, cloudCheckRemoteExists,
  } from '$lib/commands/cloud';
  import { APP_EVENT } from '$lib/shared/constants/events';
  import { settings } from '$lib/stores/settings';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';

  let show = $state(false);
  let connecting = $state<Provider | null>(null);
  let linking = $state<Provider | null>(null);
  let disconnecting = $state(false);
  let confirmingDelete = $state(false);
  let confirmingWipe = $state(false);
  let deleteSlugInput = $state('');

  $effect(() => { show = $activeModal === 'cloud'; });

  async function handleOAuthCallback(e: Event) {
    if (!get(settings)['onboarding_complete']) return;
    const detail = (e as CustomEvent<{ provider: Provider; code: string }>).detail;
    if (!detail?.code || !detail?.provider) return;

    // If we're already connected, treat this as a link.
    if (get(cloudConnected) && linking) {
      try {
        const status = await cloudLinkProvider(detail.provider, detail.code);
        if (status.user) {
          setConnected(status.user, status.providers, status.activeProvider, status.plan);
          showToast(`Linked ${detail.provider}`, 'success');
        }
      } catch (err) {
        showToast(friendlyError(err), 'error');
      } finally {
        linking = null;
      }
      return;
    }

    connecting = detail.provider;
    try {
      const status = await cloudExchangeCode(detail.provider, detail.code);
      if (status.user) {
        setConnected(status.user, status.providers, status.activeProvider, status.plan);
        setLastSyncedForKinds(status.lastSynced);
        showToast(`Connected as ${status.user.displayName || status.user.slug}`, 'success');
      }
      activeModal.set(null);

      // First-sign-in: prompt for restore if local has data AND remote has data.
      try {
        const remoteHas = await cloudCheckRemoteExists();
        if (remoteHas) showSyncRestorePrompt.set(true);
        else markSynced();
      } catch {
        markSynced();
      }
    } catch (err) {
      showToast(friendlyError(err), 'error');
    } finally {
      connecting = null;
    }
  }

  onMount(() => {
    window.addEventListener(APP_EVENT.OAUTH_CALLBACK, handleOAuthCallback);
    // Pull fresh status from server when modal opens. If we're already
    // connected per the store, refresh user/providers/last-synced.
    if (get(cloudConnected)) {
      cloudGetStatus()
        .then((s) => {
          if (s.user) setConnected(s.user, s.providers, s.activeProvider, s.plan);
          setLastSyncedForKinds(s.lastSynced);
        })
        .catch(() => {});
    }
  });
  onDestroy(() => {
    window.removeEventListener(APP_EVENT.OAUTH_CALLBACK, handleOAuthCallback);
  });

  async function openOAuthUrl(url: string) {
    try {
      const { openUrl } = await import('@tauri-apps/plugin-opener');
      await openUrl(url);
    } catch {
      window.open(url, '_blank');
    }
  }

  async function connect(provider: Provider) {
    connecting = provider;
    try {
      const url = provider === 'github'
        ? await cloudGithubLoginUrl()
        : await cloudGoogleLoginUrl();
      await openOAuthUrl(url);
      // Wait for deep-link callback. `connecting` stays set so UI shows spinner.
    } catch (err) {
      showToast(friendlyError(err), 'error');
      connecting = null;
    }
  }

  async function link(provider: Provider) {
    linking = provider;
    try {
      const url = provider === 'github'
        ? await cloudGithubLoginUrl()
        : await cloudGoogleLoginUrl();
      await openOAuthUrl(url);
    } catch (err) {
      showToast(friendlyError(err), 'error');
      linking = null;
    }
  }

  async function unlink(provider: Provider) {
    try {
      const status = await cloudUnlinkProvider(provider);
      if (status.user) {
        setConnected(status.user, status.providers, status.activeProvider, status.plan);
      }
      showToast(`Unlinked ${provider}`, 'info');
    } catch (err) {
      showToast(friendlyError(err), 'error');
    }
  }

  async function handleDisconnect() {
    disconnecting = true;
    try {
      await cloudLogout();
      setDisconnected();
      showToast('Signed out of Clauge cloud', 'info');
    } catch (err) {
      showToast(friendlyError(err), 'error');
    } finally {
      disconnecting = false;
    }
  }

  async function handleRestore() {
    if (get(syncing)) return;
    setSyncing(true);
    try {
      await cloudSyncRestore();
      markSynced();
      // Reload all mode stores so the UI reflects the freshly-imported rows.
      const [{ loadCollections, loadEnvironments }, { loadConnections: loadSql, loadSqlScripts }, { loadNoSqlConnections }] = await Promise.all([
        import('$lib/modes/rest/stores'),
        import('$lib/modes/sql/stores'),
        import('$lib/modes/nosql/stores'),
      ]);
      await Promise.all([loadCollections(), loadEnvironments(), loadSql(), loadSqlScripts(), loadNoSqlConnections()]);
      showToast('Restored from cloud', 'success');
    } catch (err) {
      showToast(friendlyError(err), 'error');
    } finally {
      setSyncing(false);
    }
  }

  async function handlePushNow() {
    if (get(syncing)) return;
    setSyncing(true);
    try {
      const pushed = await cloudSyncPushNow();
      showToast(pushed.length ? `Pushed: ${pushed.join(', ')}` : 'Nothing to push — everything is up to date', 'success');
    } catch (err) {
      showToast(friendlyError(err), 'error');
    } finally {
      setSyncing(false);
    }
  }

  async function handleWipeRemote() {
    try {
      await cloudWipeRemote();
      setDisconnected();
      showToast('Cloud data wiped — your local data is intact', 'success');
      confirmingWipe = false;
    } catch (err) {
      showToast(friendlyError(err), 'error');
    }
  }

  async function handleDeleteAccount() {
    const expected = $cloudUser?.slug || '';
    if (deleteSlugInput.trim() !== expected) {
      showToast('Type your handle exactly to confirm', 'error');
      return;
    }
    try {
      await cloudDeleteAccount(deleteSlugInput.trim());
      setDisconnected();
      showToast('Account deleted — your local data is intact', 'success');
      confirmingDelete = false;
      deleteSlugInput = '';
    } catch (err) {
      showToast(friendlyError(err), 'error');
    }
  }

  function providerLinked(p: Provider): boolean {
    return $cloudProviders.some((x) => x.provider === p);
  }

  function fmtSyncTime(iso: string | undefined): string {
    if (!iso) return 'Never';
    const d = new Date(iso);
    const diff = Date.now() - d.getTime();
    if (diff < 60_000) return 'just now';
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
    if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
    return d.toLocaleDateString();
  }
</script>

<Modal bind:show title="Cloud Sync" width="480px">
  {#if $cloudConnected && $cloudUser}
    <!-- Connected: show user + linked providers + sync controls + danger zone -->
    <div class="cc-connected">
      <div class="cc-user">
        {#if $cloudUser.avatarUrl}
          <img class="cc-avatar" src={$cloudUser.avatarUrl} alt="" />
        {:else}
          <div class="cc-avatar cc-avatar-fallback">{($cloudUser.displayName || $cloudUser.slug || '?').charAt(0).toUpperCase()}</div>
        {/if}
        <div class="cc-user-text">
          <div class="cc-name">{$cloudUser.displayName || $cloudUser.slug}</div>
          <div class="cc-email">{$cloudUser.email || `@${$cloudUser.slug}`}</div>
        </div>
      </div>

      <div class="cc-section">
        <h4>Linked accounts</h4>
        {#each ['github', 'google'] as p}
          {@const linked = providerLinked(p as Provider)}
          <div class="cc-prov">
            <span class="cc-prov-name">{p === 'github' ? 'GitHub' : 'Google'}</span>
            {#if linked}
              {@const meta = $cloudProviders.find((x) => x.provider === p)}
              <span class="cc-prov-handle">{meta?.providerLogin || meta?.email || ''}</span>
              {#if $cloudProviders.length > 1}
                <button class="cc-mini-btn" onclick={() => unlink(p as Provider)}>Unlink</button>
              {:else}
                <span class="cc-prov-note">primary</span>
              {/if}
            {:else}
              <button class="cc-mini-btn cc-mini-link" onclick={() => link(p as Provider)} disabled={linking === p}>
                {linking === p ? 'Opening browser…' : 'Link'}
              </button>
            {/if}
          </div>
        {/each}
      </div>

      <div class="cc-section">
        <h4>Sync status</h4>
        <div class="cc-sync-grid">
          {#each ['rest', 'sql', 'nosql', 'agent', 'ssh', 'explorer'] as k}
            <div class="cc-sync-row">
              <span class="cc-kind">{k}</span>
              <span class="cc-when">{fmtSyncTime($lastSyncedByKind[k])}</span>
            </div>
          {/each}
        </div>
      </div>

      <div class="cc-actions">
        <button class="cc-btn cc-btn-primary" onclick={handlePushNow} disabled={$syncing}>
          {$syncing ? 'Working…' : 'Push now'}
        </button>
        <button class="cc-btn" onclick={handleRestore} disabled={$syncing}>
          Restore from cloud
        </button>
        <button class="cc-btn cc-btn-ghost" onclick={handleDisconnect} disabled={disconnecting}>
          {disconnecting ? 'Signing out…' : 'Sign out'}
        </button>
      </div>

      <details class="cc-danger">
        <summary>Danger zone</summary>
        {#if !confirmingWipe && !confirmingDelete}
          <button class="cc-danger-btn" onclick={() => (confirmingWipe = true)}>Wipe my cloud data</button>
          <button class="cc-danger-btn cc-danger-strong" onclick={() => (confirmingDelete = true)}>Delete my account</button>
        {/if}
        {#if confirmingWipe}
          <div class="cc-confirm">
            <p>This removes your cloud sync data. Your account stays, your local data stays, and you can re-push at any time.</p>
            <div class="cc-confirm-row">
              <button class="cc-btn-secondary" onclick={() => (confirmingWipe = false)}>Cancel</button>
              <button class="cc-danger-btn" onclick={handleWipeRemote}>Wipe cloud data</button>
            </div>
          </div>
        {/if}
        {#if confirmingDelete}
          <div class="cc-confirm">
            <p>This permanently removes your Clauge account and all linked providers and cloud data. Local data on this device is NOT affected. This cannot be undone.</p>
            <label class="cc-confirm-label">
              Type your handle <code>{$cloudUser.slug}</code> to confirm
              <input bind:value={deleteSlugInput} type="text" placeholder={$cloudUser.slug} />
            </label>
            <div class="cc-confirm-row">
              <button class="cc-btn-secondary" onclick={() => { confirmingDelete = false; deleteSlugInput = ''; }}>Cancel</button>
              <button class="cc-danger-btn cc-danger-strong" onclick={handleDeleteAccount} disabled={deleteSlugInput.trim() !== $cloudUser.slug}>Delete account</button>
            </div>
          </div>
        {/if}
      </details>
    </div>
  {:else if connecting}
    <div class="cc-waiting">
      <span class="cc-spinner"></span>
      <p>Waiting for {connecting === 'github' ? 'GitHub' : 'Google'} authorization…</p>
      <p class="cc-hint">Complete the authorization in your browser, then return here.</p>
      <button class="cc-btn-secondary" onclick={() => (connecting = null)}>Cancel</button>
    </div>
  {:else}
    <!-- Signed out: show provider buttons -->
    <div class="cc-login">
      <p class="cc-intro">Sign in to sync your collections, connections, and saved queries across devices.</p>
      <button class="cc-btn cc-btn-github" onclick={() => connect('github')}>
        <svg viewBox="0 0 16 16" fill="currentColor" width="16" height="16"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.22 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>
        Continue with GitHub
      </button>
      <button class="cc-btn cc-btn-google" onclick={() => connect('google')}>
        <svg viewBox="0 0 24 24" width="16" height="16"><path d="M22.5 12.27c0-.79-.07-1.54-.2-2.27H12v4.51h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.32z" fill="#4285F4"/><path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34A853"/><path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09 0-.73.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" fill="#FBBC05"/><path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#EA4335"/></svg>
        Continue with Google
      </button>
      <p class="cc-fine">We only request your basic profile — no access to your repos, files, or email content.</p>
    </div>
  {/if}
</Modal>

<style>
  .cc-login { display: flex; flex-direction: column; align-items: center; padding: 12px 0; gap: 12px; }
  .cc-intro { font-size: 13.5px; color: var(--t2); text-align: center; margin: 0 0 8px; max-width: 360px; line-height: 1.5; }
  .cc-btn {
    display: flex; align-items: center; gap: 10px; padding: 11px 22px;
    border-radius: 8px; border: 1px solid var(--b1); background: transparent;
    color: var(--t1); font-size: 13px; font-weight: 600; cursor: default;
    transition: background .15s, border-color .15s; min-width: 240px;
    justify-content: center;
  }
  .cc-btn:hover:not(:disabled) { background: rgba(255,255,255,0.04); border-color: var(--b2); }
  .cc-btn:disabled { opacity: 0.5; }
  .cc-btn-primary { background: var(--acc); color: #fff; border-color: transparent; }
  .cc-btn-primary:hover:not(:disabled) { opacity: 0.9; background: var(--acc); }
  .cc-btn-github { color: var(--t1); }
  .cc-btn-google { color: var(--t1); }
  .cc-btn-ghost { color: var(--t3); border-color: transparent; }
  .cc-btn-ghost:hover:not(:disabled) { color: #f04444; background: rgba(240,68,68,0.06); }
  .cc-fine { font-size: 11.5px; color: var(--t3); margin: 4px 0 0; text-align: center; max-width: 360px; line-height: 1.5; }

  .cc-connected { display: flex; flex-direction: column; gap: 18px; padding: 6px 0 4px; }
  .cc-user { display: flex; align-items: center; gap: 12px; }
  .cc-avatar { width: 44px; height: 44px; border-radius: 50%; }
  .cc-avatar-fallback {
    background: linear-gradient(135deg, var(--acc), #1dc880);
    display: flex; align-items: center; justify-content: center;
    color: #fff; font-weight: 600; font-size: 18px;
  }
  .cc-user-text { display: flex; flex-direction: column; gap: 2px; }
  .cc-name { font-size: 14px; font-weight: 600; color: var(--t1); }
  .cc-email { font-size: 12px; color: var(--t3); }
  .cc-section h4 {
    font-size: 11px; text-transform: uppercase; letter-spacing: 0.1em;
    color: var(--t3); margin: 0 0 8px; font-weight: 600;
  }
  .cc-prov {
    display: flex; align-items: center; gap: 10px;
    padding: 8px 10px; border-radius: 6px;
    border: 1px solid var(--b1); margin-bottom: 6px;
    font-size: 12.5px;
  }
  .cc-prov-name { font-weight: 600; color: var(--t1); min-width: 60px; }
  .cc-prov-handle { color: var(--t2); flex: 1; }
  .cc-prov-note { color: var(--t3); font-size: 11px; font-style: italic; }
  .cc-mini-btn {
    padding: 4px 10px; font-size: 11px; border-radius: 4px;
    border: 1px solid var(--b1); background: transparent; color: var(--t2); cursor: default;
  }
  .cc-mini-btn:hover:not(:disabled) { background: rgba(255,255,255,0.04); color: var(--t1); }
  .cc-mini-link { color: var(--acc); border-color: var(--acc); }
  .cc-sync-grid { display: flex; flex-direction: column; gap: 4px; }
  .cc-sync-row {
    display: flex; justify-content: space-between; font-size: 12px;
    padding: 4px 8px; border-radius: 4px;
  }
  .cc-sync-row:nth-child(odd) { background: rgba(255,255,255,0.02); }
  .cc-kind { color: var(--t2); font-family: var(--font-mono, ui-monospace); }
  .cc-when { color: var(--t3); }

  .cc-actions { display: flex; flex-direction: column; gap: 6px; align-items: stretch; }

  .cc-danger {
    margin-top: 8px; padding-top: 12px;
    border-top: 1px solid var(--b1);
  }
  .cc-danger summary {
    cursor: default; color: var(--t3); font-size: 12px;
    font-weight: 600; padding: 4px 0;
  }
  .cc-danger-btn {
    display: block; width: 100%; padding: 8px 12px; margin-top: 8px;
    border-radius: 6px; border: 1px solid rgba(240,68,68,0.25);
    background: transparent; color: #f04444; font-size: 12px; cursor: default;
  }
  .cc-danger-btn:hover:not(:disabled) { background: rgba(240,68,68,0.08); }
  .cc-danger-strong { border-color: #f04444; }
  .cc-confirm {
    margin-top: 10px; padding: 12px; border-radius: 6px;
    border: 1px solid rgba(240,68,68,0.3); background: rgba(240,68,68,0.04);
    font-size: 12.5px; color: var(--t2);
  }
  .cc-confirm p { margin: 0 0 10px; line-height: 1.5; }
  .cc-confirm-label { display: flex; flex-direction: column; gap: 6px; font-size: 11.5px; color: var(--t3); }
  .cc-confirm-label code { font-family: var(--font-mono, ui-monospace); color: var(--t1); }
  .cc-confirm-label input {
    padding: 7px 10px; border-radius: 4px; border: 1px solid var(--b1);
    background: rgba(0,0,0,0.25); color: var(--t1); font-size: 12.5px;
  }
  .cc-confirm-row { display: flex; gap: 8px; margin-top: 10px; }
  .cc-btn-secondary {
    flex: 1; padding: 7px 12px; font-size: 12px; border-radius: 4px;
    border: 1px solid var(--b1); background: transparent; color: var(--t2); cursor: default;
  }

  .cc-waiting { display: flex; flex-direction: column; align-items: center; gap: 12px; padding: 20px 0; }
  .cc-spinner {
    width: 22px; height: 22px;
    border: 3px solid rgba(255,255,255,0.12);
    border-top-color: var(--acc);
    border-radius: 50%; animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .cc-hint { font-size: 12px; color: var(--t3); text-align: center; }
</style>

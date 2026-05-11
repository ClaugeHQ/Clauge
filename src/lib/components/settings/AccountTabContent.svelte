<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import {
    cloudConnected, cloudUser, cloudProviders, cloudPlan, activeProvider,
    syncing, setSyncing, setConnected, setDisconnected,
    lastSyncedByKind, setLastSyncedForKinds,
    type Provider,
  } from '$lib/stores/cloud';
  import {
    cloudGetStatus, cloudGithubLoginUrl, cloudGoogleLoginUrl,
    cloudLinkProvider, cloudUnlinkProvider, cloudLogout,
    cloudWipeRemote, cloudDeleteAccount,
    cloudSyncPushNow, cloudSyncRestore,
    cloudUpdateProfile,
  } from '$lib/commands/cloud';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';

  let displayNameInput = $state('');
  let firstNameInput = $state('');
  let lastNameInput = $state('');
  let savingProfile = $state(false);
  let refreshing = $state(false);
  let linking = $state<Provider | null>(null);
  let signingIn = $state<Provider | null>(null);

  let confirmingWipe = $state(false);
  let confirmingDelete = $state(false);
  let deleteSlugInput = $state('');

  // Sync local input state from store when user data loads / changes.
  $effect(() => {
    const u = $cloudUser;
    if (u) {
      displayNameInput = u.displayName ?? '';
      firstNameInput   = u.firstName ?? '';
      lastNameInput    = u.lastName ?? '';
    }
  });

  onMount(() => {
    if (get(cloudConnected)) refreshStatus().catch(() => {});
  });

  async function refreshStatus() {
    refreshing = true;
    try {
      const s = await cloudGetStatus();
      if (s.user) {
        setConnected(s.user, s.providers, s.activeProvider, s.plan);
        setLastSyncedForKinds(s.lastSynced);
      }
    } catch (e) {
      showToast(friendlyError(e), 'error');
    } finally {
      refreshing = false;
    }
  }

  function profileChanged(): boolean {
    const u = $cloudUser;
    if (!u) return false;
    return (
      displayNameInput.trim() !== (u.displayName ?? '') ||
      firstNameInput.trim()   !== (u.firstName ?? '')   ||
      lastNameInput.trim()    !== (u.lastName ?? '')
    );
  }

  async function saveProfile() {
    if (savingProfile) return;
    savingProfile = true;
    try {
      const s = await cloudUpdateProfile({
        displayName: displayNameInput.trim(),
        firstName:   firstNameInput.trim(),
        lastName:    lastNameInput.trim(),
      });
      if (s.user) setConnected(s.user, s.providers, s.activeProvider, s.plan);
      showToast('Profile updated', 'success');
    } catch (e) {
      showToast(friendlyError(e), 'error');
    } finally {
      savingProfile = false;
    }
  }

  async function openOAuth(provider: Provider) {
    try {
      const url = provider === 'github' ? await cloudGithubLoginUrl() : await cloudGoogleLoginUrl();
      const { openUrl } = await import('@tauri-apps/plugin-opener');
      await openUrl(url);
    } catch {
      // fallthrough — onboarding's handler will catch the deep link
    }
  }

  async function signIn(provider: Provider) {
    signingIn = provider;
    try { await openOAuth(provider); } finally { /* spinner clears on deep-link */ }
  }

  async function linkAdditional(provider: Provider) {
    linking = provider;
    try { await openOAuth(provider); } finally { /* spinner clears on deep-link */ }
  }

  async function unlink(provider: Provider) {
    try {
      const s = await cloudUnlinkProvider(provider);
      if (s.user) setConnected(s.user, s.providers, s.activeProvider, s.plan);
      showToast(`Unlinked ${provider}`, 'info');
    } catch (e) {
      showToast(friendlyError(e), 'error');
    }
  }

  async function signOut() {
    try {
      await cloudLogout();
      setDisconnected();
      showToast('Signed out', 'info');
    } catch (e) {
      showToast(friendlyError(e), 'error');
    }
  }

  async function pushNow() {
    if ($syncing) return;
    setSyncing(true);
    try {
      const pushed = await cloudSyncPushNow();
      showToast(pushed.length ? `Pushed: ${pushed.join(', ')}` : 'Up to date', 'success');
    } catch (e) { showToast(friendlyError(e), 'error'); }
    finally { setSyncing(false); }
  }

  async function restoreFromCloud() {
    if ($syncing) return;
    setSyncing(true);
    try {
      await cloudSyncRestore();
      const [r, s, n] = await Promise.all([
        import('$lib/modes/rest/stores'),
        import('$lib/modes/sql/stores'),
        import('$lib/modes/nosql/stores'),
      ]);
      await Promise.all([
        r.loadCollections(), r.loadEnvironments(),
        s.loadConnections(), s.loadSqlScripts(),
        n.loadNoSqlConnections(),
      ]);
      showToast('Restored from cloud', 'success');
    } catch (e) { showToast(friendlyError(e), 'error'); }
    finally { setSyncing(false); }
  }

  async function wipeRemote() {
    try {
      await cloudWipeRemote();
      setDisconnected();
      showToast('Cloud data wiped — local data intact', 'success');
      confirmingWipe = false;
    } catch (e) { showToast(friendlyError(e), 'error'); }
  }

  async function deleteAccount() {
    const expected = $cloudUser?.slug ?? '';
    if (deleteSlugInput.trim() !== expected) {
      showToast('Type your handle exactly to confirm', 'error');
      return;
    }
    try {
      await cloudDeleteAccount(deleteSlugInput.trim());
      setDisconnected();
      showToast('Account deleted — local data intact', 'success');
      confirmingDelete = false;
      deleteSlugInput = '';
    } catch (e) { showToast(friendlyError(e), 'error'); }
  }

  function providerLinked(p: Provider): boolean {
    return $cloudProviders.some((x) => x.provider === p);
  }

  function fmtTime(iso: string | undefined): string {
    if (!iso) return 'Never';
    const d = new Date(iso);
    const diff = Date.now() - d.getTime();
    if (diff < 60_000) return 'just now';
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
    if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
    return d.toLocaleDateString();
  }

  async function openUpgrade() {
    // Pro flow lands in Phase D — for now, link to pricing page.
    try {
      const { openUrl } = await import('@tauri-apps/plugin-opener');
      await openUrl('https://clauge.in/pricing');
    } catch {
      window.open('https://clauge.in/pricing', '_blank');
    }
  }
</script>

<div class="acc-pane">
  {#if !$cloudConnected}
    <!-- Signed out: invite to sign in -->
    <section class="acc-empty">
      <div class="acc-empty-icon">
        <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.5">
          <path d="M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2" />
          <circle cx="12" cy="7" r="4" />
        </svg>
      </div>
      <h2>Sign in to Clauge</h2>
      <p>Connect your GitHub or Google account to sync your collections, connections, and saved queries across devices.</p>
      <div class="acc-empty-buttons">
        <button class="acc-btn acc-btn-primary" onclick={() => signIn('github')} disabled={!!signingIn}>
          {#if signingIn === 'github'}Opening browser…{:else}Continue with GitHub{/if}
        </button>
        <button class="acc-btn" onclick={() => signIn('google')} disabled={!!signingIn}>
          {#if signingIn === 'google'}Opening browser…{:else}Continue with Google{/if}
        </button>
      </div>
      <p class="acc-fine">We only request basic profile info — no access to your repos, files, or email content.</p>
    </section>
  {:else if $cloudUser}
    <!-- Signed in: full account view -->
    <div class="acc-stack">

      <!-- Profile card -->
      <section class="acc-card">
        <div class="acc-profile-header">
          {#if $cloudUser.avatarUrl}
            <img class="acc-avatar" src={$cloudUser.avatarUrl} alt="" />
          {:else}
            <div class="acc-avatar acc-avatar-fallback">{($cloudUser.displayName ?? $cloudUser.slug).charAt(0).toUpperCase()}</div>
          {/if}
          <div class="acc-profile-text">
            <div class="acc-profile-name">{$cloudUser.displayName ?? $cloudUser.slug}</div>
            <div class="acc-profile-email">{$cloudUser.email ?? `@${$cloudUser.slug}`}</div>
            <div class="acc-profile-meta">
              <span class="acc-plan-pill" class:is-pro={$cloudPlan === 'pro'}>
                {$cloudPlan === 'pro' ? 'Pro' : 'Free'}
              </span>
              <span class="acc-slug">@{$cloudUser.slug}</span>
            </div>
          </div>
          <button class="acc-refresh-btn" onclick={refreshStatus} title="Refresh status" disabled={refreshing}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" class:is-spinning={refreshing}>
              <path d="M23 4v6h-6"/><path d="M1 20v-6h6"/>
              <path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/>
            </svg>
          </button>
        </div>

        <!-- Editable fields -->
        <div class="acc-fields">
          <label class="acc-field">
            <span class="acc-field-label">Display name</span>
            <input type="text" bind:value={displayNameInput} maxlength="120" />
          </label>
          <div class="acc-field-row">
            <label class="acc-field">
              <span class="acc-field-label">First name</span>
              <input type="text" bind:value={firstNameInput} maxlength="80" />
            </label>
            <label class="acc-field">
              <span class="acc-field-label">Last name</span>
              <input type="text" bind:value={lastNameInput} maxlength="80" />
            </label>
          </div>
          <div class="acc-field-actions">
            <button class="acc-btn acc-btn-primary" onclick={saveProfile} disabled={!profileChanged() || savingProfile}>
              {savingProfile ? 'Saving…' : 'Save changes'}
            </button>
          </div>
          <p class="acc-fine acc-fine-inline">Email and handle are sourced from your sign-in provider. Email read-only; handle (slug) editable in a future release.</p>
        </div>
      </section>

      <!-- Linked accounts -->
      <section class="acc-card">
        <h3 class="acc-card-title">Linked accounts</h3>
        {#each ['github', 'google'] as p}
          {@const linked = providerLinked(p as Provider)}
          {@const meta = linked ? $cloudProviders.find((x) => x.provider === p) : null}
          <div class="acc-prov-row">
            <span class="acc-prov-name">{p === 'github' ? 'GitHub' : 'Google'}</span>
            {#if linked}
              <span class="acc-prov-handle">{meta?.providerLogin ?? meta?.email ?? ''}</span>
              {#if $cloudProviders.length > 1}
                <button class="acc-mini-btn" onclick={() => unlink(p as Provider)}>Unlink</button>
              {:else}
                <span class="acc-prov-note">primary</span>
              {/if}
            {:else}
              <span class="acc-prov-handle acc-prov-empty">Not linked</span>
              <button class="acc-mini-btn acc-mini-link" onclick={() => linkAdditional(p as Provider)} disabled={linking === p}>
                {linking === p ? 'Opening…' : 'Link'}
              </button>
            {/if}
          </div>
        {/each}
      </section>

      <!-- Subscription -->
      <section class="acc-card">
        <h3 class="acc-card-title">Subscription</h3>
        {#if $cloudPlan === 'pro'}
          <p class="acc-sub-status">Clauge <strong>Pro</strong> · active</p>
          <button class="acc-btn" onclick={openUpgrade}>Manage in Polar portal</button>
        {:else}
          <p class="acc-sub-status">You're on the <strong>Free</strong> plan. Upgrade to unlock premium themes and managed AI.</p>
          <button class="acc-btn acc-btn-primary" onclick={openUpgrade}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" style="margin-right:6px"><polyline points="9 18 15 12 9 6"/></svg>
            See Pro pricing
          </button>
          <p class="acc-fine acc-fine-inline">Pro is coming soon. Pricing will be announced on clauge.in/pricing.</p>
        {/if}
      </section>

      <!-- Cloud sync -->
      <section class="acc-card">
        <h3 class="acc-card-title">Cloud sync</h3>
        <div class="acc-sync-grid">
          {#each ['rest', 'sql', 'nosql', 'agent', 'ssh', 'explorer'] as k}
            <div class="acc-sync-row">
              <span class="acc-sync-kind">{k}</span>
              <span class="acc-sync-when">{fmtTime($lastSyncedByKind[k])}</span>
            </div>
          {/each}
        </div>
        <div class="acc-sync-actions">
          <button class="acc-btn" onclick={pushNow} disabled={$syncing}>{$syncing ? 'Working…' : 'Push now'}</button>
          <button class="acc-btn" onclick={restoreFromCloud} disabled={$syncing}>Restore from cloud</button>
          <button class="acc-btn acc-btn-ghost" onclick={signOut}>Sign out</button>
        </div>
      </section>

      <!-- Danger zone -->
      <section class="acc-card acc-card-danger">
        <h3 class="acc-card-title">Danger zone</h3>
        {#if !confirmingWipe && !confirmingDelete}
          <div class="acc-danger-row">
            <div>
              <strong>Wipe my cloud data</strong>
              <p>Removes synced data from our servers. Your account stays, your local data stays, you can re-push anytime.</p>
            </div>
            <button class="acc-danger-btn" onclick={() => (confirmingWipe = true)}>Wipe cloud data</button>
          </div>
          <div class="acc-danger-row">
            <div>
              <strong>Delete my account</strong>
              <p>Permanently removes your Clauge account, all linked providers, and all cloud data. Local data on this device is NOT affected. Cannot be undone.</p>
            </div>
            <button class="acc-danger-btn acc-danger-strong" onclick={() => (confirmingDelete = true)}>Delete account</button>
          </div>
        {/if}
        {#if confirmingWipe}
          <div class="acc-confirm">
            <p>This deletes synced data from our servers. Your local data stays. Confirm?</p>
            <div class="acc-confirm-row">
              <button class="acc-btn acc-btn-ghost" onclick={() => (confirmingWipe = false)}>Cancel</button>
              <button class="acc-danger-btn" onclick={wipeRemote}>Yes, wipe cloud data</button>
            </div>
          </div>
        {/if}
        {#if confirmingDelete}
          <div class="acc-confirm">
            <p>This permanently removes your Clauge account. Type your handle <code>{$cloudUser.slug}</code> to confirm:</p>
            <input class="acc-confirm-input" bind:value={deleteSlugInput} placeholder={$cloudUser.slug} />
            <div class="acc-confirm-row">
              <button class="acc-btn acc-btn-ghost" onclick={() => { confirmingDelete = false; deleteSlugInput = ''; }}>Cancel</button>
              <button class="acc-danger-btn acc-danger-strong" onclick={deleteAccount} disabled={deleteSlugInput.trim() !== $cloudUser.slug}>Delete account</button>
            </div>
          </div>
        {/if}
      </section>

    </div>
  {/if}
</div>

<style>
  .acc-pane {
    padding: 4px 2px 12px;
    font-family: var(--ui);
  }
  .acc-empty {
    max-width: 440px; margin: 32px auto;
    text-align: center;
    padding: 32px 24px;
    border: 1px solid var(--b1);
    border-radius: 12px;
    background: rgba(255,255,255,0.02);
  }
  .acc-empty-icon {
    width: 72px; height: 72px; margin: 0 auto 18px;
    display: flex; align-items: center; justify-content: center;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--acc), #1dc880);
    color: #fff;
  }
  .acc-empty h2 { font-size: 17px; font-weight: 600; margin: 0 0 8px; color: var(--t1); }
  .acc-empty p  { font-size: 13.5px; color: var(--t2); line-height: 1.5; margin: 0 0 22px; }
  .acc-empty-buttons { display: flex; flex-direction: column; gap: 8px; max-width: 280px; margin: 0 auto 16px; }

  .acc-stack { display: flex; flex-direction: column; gap: 14px; }
  .acc-card {
    padding: 16px 18px;
    border: 1px solid var(--b1);
    border-radius: 10px;
    background: rgba(255,255,255,0.02);
  }
  .acc-card-title {
    font-size: 11px; text-transform: uppercase; letter-spacing: 0.12em;
    font-weight: 600; color: var(--t3); margin: 0 0 12px;
  }

  .acc-profile-header { display: flex; gap: 14px; align-items: center; margin-bottom: 18px; }
  .acc-avatar { width: 56px; height: 56px; border-radius: 50%; flex-shrink: 0; }
  .acc-avatar-fallback {
    background: linear-gradient(135deg, var(--acc), #1dc880);
    display: flex; align-items: center; justify-content: center;
    color: #fff; font-size: 22px; font-weight: 600;
  }
  .acc-profile-text { display: flex; flex-direction: column; gap: 2px; flex: 1; }
  .acc-profile-name  { font-size: 16px; font-weight: 600; color: var(--t1); }
  .acc-profile-email { font-size: 12.5px; color: var(--t3); }
  .acc-profile-meta { display: flex; align-items: center; gap: 8px; margin-top: 4px; }
  .acc-plan-pill {
    font-size: 10px; padding: 2px 8px; border-radius: 999px;
    background: rgba(255,255,255,0.08); color: var(--t2);
    font-weight: 600; text-transform: uppercase; letter-spacing: 0.08em;
  }
  .acc-plan-pill.is-pro {
    background: linear-gradient(120deg, var(--acc), #1dc880);
    color: #fff;
  }
  .acc-slug { font-size: 11.5px; color: var(--t3); font-family: var(--mono, ui-monospace); }
  .acc-refresh-btn {
    background: transparent; border: 1px solid var(--b1); border-radius: 6px;
    padding: 6px; cursor: default; color: var(--t3);
  }
  .acc-refresh-btn:hover { color: var(--t1); border-color: var(--b2); }
  .acc-refresh-btn .is-spinning { animation: acc-spin 1s linear infinite; }
  @keyframes acc-spin { to { transform: rotate(360deg); } }

  .acc-fields { display: flex; flex-direction: column; gap: 12px; }
  .acc-field { display: flex; flex-direction: column; gap: 5px; flex: 1; }
  .acc-field-label {
    font-size: 11px; color: var(--t3);
    font-weight: 500; letter-spacing: 0.04em;
  }
  .acc-field input {
    padding: 8px 10px; font-size: 13px;
    border: 1px solid var(--b1); border-radius: 6px;
    background: rgba(0,0,0,0.25); color: var(--t1);
    font-family: var(--ui);
  }
  .acc-field input:focus { outline: none; border-color: var(--acc); }
  .acc-field-row { display: flex; gap: 12px; }
  .acc-field-actions { display: flex; justify-content: flex-end; margin-top: 4px; }

  .acc-btn {
    padding: 8px 16px; font-size: 12.5px; font-weight: 500;
    border-radius: 6px; border: 1px solid var(--b1);
    background: transparent; color: var(--t1);
    cursor: default;
    transition: background .15s, border-color .15s, opacity .15s;
    font-family: var(--ui);
    display: inline-flex; align-items: center; gap: 6px;
  }
  .acc-btn:hover:not(:disabled) { background: rgba(255,255,255,0.05); border-color: var(--b2); }
  .acc-btn:disabled { opacity: 0.5; }
  .acc-btn-primary {
    background: var(--acc); border-color: transparent; color: #fff;
  }
  .acc-btn-primary:hover:not(:disabled) { opacity: 0.92; background: var(--acc); }
  .acc-btn-ghost { border-color: transparent; color: var(--t3); }
  .acc-btn-ghost:hover:not(:disabled) { color: #f04444; background: rgba(240,68,68,0.06); }

  .acc-fine { font-size: 11.5px; color: var(--t3); line-height: 1.5; margin: 6px 0 0; }
  .acc-fine-inline { margin-top: 8px; }

  .acc-prov-row {
    display: flex; align-items: center; gap: 12px;
    padding: 8px 10px; border-radius: 6px;
    border: 1px solid var(--b1); margin-bottom: 6px;
    font-size: 12.5px;
  }
  .acc-prov-row:last-child { margin-bottom: 0; }
  .acc-prov-name { font-weight: 600; color: var(--t1); min-width: 60px; }
  .acc-prov-handle { color: var(--t2); flex: 1; }
  .acc-prov-empty { font-style: italic; color: var(--t3); }
  .acc-prov-note { color: var(--t3); font-size: 11px; font-style: italic; }
  .acc-mini-btn {
    padding: 4px 10px; font-size: 11px; border-radius: 4px;
    border: 1px solid var(--b1); background: transparent;
    color: var(--t2); cursor: default;
  }
  .acc-mini-btn:hover:not(:disabled) { background: rgba(255,255,255,0.04); color: var(--t1); }
  .acc-mini-link { color: var(--acc); border-color: var(--acc); }

  .acc-sub-status { font-size: 13px; color: var(--t2); line-height: 1.5; margin: 0 0 12px; }
  .acc-sub-status strong { color: var(--t1); }

  .acc-sync-grid { display: flex; flex-direction: column; gap: 4px; margin-bottom: 12px; }
  .acc-sync-row {
    display: flex; justify-content: space-between;
    font-size: 12px; padding: 5px 8px; border-radius: 4px;
  }
  .acc-sync-row:nth-child(odd) { background: rgba(255,255,255,0.02); }
  .acc-sync-kind { color: var(--t2); font-family: var(--mono, ui-monospace); }
  .acc-sync-when { color: var(--t3); }
  .acc-sync-actions { display: flex; gap: 8px; flex-wrap: wrap; }

  .acc-card-danger { border-color: rgba(240, 68, 68, 0.18); }
  .acc-danger-row {
    display: flex; gap: 16px; align-items: center;
    padding: 10px 0;
    border-top: 1px solid var(--b1);
  }
  .acc-danger-row:first-of-type { border-top: none; padding-top: 4px; }
  .acc-danger-row > div { flex: 1; }
  .acc-danger-row strong { font-size: 13px; color: var(--t1); }
  .acc-danger-row p { font-size: 12px; color: var(--t3); line-height: 1.5; margin: 4px 0 0; }
  .acc-danger-btn {
    padding: 7px 12px; font-size: 12px; border-radius: 6px;
    border: 1px solid rgba(240,68,68,0.3);
    background: transparent; color: #f04444; cursor: default; white-space: nowrap;
  }
  .acc-danger-btn:hover:not(:disabled) { background: rgba(240,68,68,0.08); }
  .acc-danger-strong { border-color: #f04444; }
  .acc-confirm {
    margin-top: 10px; padding: 14px; border-radius: 8px;
    border: 1px solid rgba(240,68,68,0.3); background: rgba(240,68,68,0.04);
    font-size: 12.5px; color: var(--t2);
  }
  .acc-confirm p { margin: 0 0 10px; line-height: 1.5; }
  .acc-confirm p code { font-family: var(--mono, ui-monospace); color: var(--t1); }
  .acc-confirm-input {
    width: 100%; box-sizing: border-box;
    padding: 8px 10px; border-radius: 4px;
    border: 1px solid var(--b1); background: rgba(0,0,0,0.25);
    color: var(--t1); font-family: var(--ui); font-size: 12.5px;
    margin-bottom: 10px;
  }
  .acc-confirm-row { display: flex; gap: 8px; justify-content: flex-end; }
</style>

<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { sshProfiles, activeSshProfile, loadSshProfiles, sshTerminalIds, sshConnStates } from '../stores';
  import { sshTouchProfile, sshDeleteProfile, sshKillTerminal, sshReadConfigHosts } from '../commands';
  import { profileIdFromTabKey } from '../tabkey';
  import { tabs as tabsStore, closeTab } from '$lib/shared/stores/tabs';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import { showToast } from '$lib/shared/primitives/toast';
  import NewSshProfileModal from './NewSshProfileModal.svelte';
  import EditSshProfileModal from './EditSshProfileModal.svelte';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import type { SshProfile } from '../types';
  import { SSH_EVENT } from '$lib/shared/constants/events';
  import { mode } from '$lib/stores/app';
  import { tabs as tabsStore2, addTab, activateTab } from '$lib/shared/stores/tabs';
  import { explorerConnections, explorerConnStates, loadExplorerConnections } from '$lib/modes/explorer/stores';
  import { newExplorerTabKey } from '$lib/modes/explorer/tabkey';
  import { createConnection, openSession } from '$lib/modes/explorer/commands';
  import type { ExplorerConnection } from '$lib/modes/explorer/types';


  interface Props {
    searchQuery?: string;
  }
  let { searchQuery = '' }: Props = $props();

  let showAdd = $state(false);
  let showEdit = $state(false);
  let editTarget = $state<SshProfile | null>(null);
  // Which tab to open the unified add/import modal on. Set to 'import'
  // by the empty-state callout, otherwise 'manual'.
  let addInitialView = $state<'manual' | 'import'>('manual');
  // Number of importable hosts found in ~/.ssh/config (excludes already-imported).
  // Only used for the empty-state callout — non-zero means "user has hosts in
  // their ssh_config but no Clauge profiles yet, surface the import path".
  let importableCount = $state(0);

  // Confirm dialog — single instance drives all SSH confirm prompts
  // (Disconnect, Delete). Switched from bespoke portal markup to the
  // shared ConfirmDialog primitive so SSH/Explorer/SQL/NoSQL all read
  // identically.
  let confirmShow = $state(false);
  let confirmTitle = $state('');
  let confirmMessage = $state('');
  let confirmDanger = $state(false);
  let confirmAction: (() => Promise<void>) | null = $state(null);

  function showConfirm(title: string, message: string, danger: boolean, action: () => Promise<void>) {
    confirmTitle = title;
    confirmMessage = message;
    confirmDanger = danger;
    confirmAction = action;
    confirmShow = true;
  }

  async function refreshImportableCount() {
    try {
      const list = await sshReadConfigHosts();
      importableCount = list.filter((h) => !h.alreadyExists).length;
    } catch {
      importableCount = 0;
    }
  }

  // After the add/import modal closes, re-probe the importable count so
  // the empty-state callout reflects what was just imported (typically
  // drops to 0 and the callout disappears).
  let addWasOpen = false;
  $effect(() => {
    if (showAdd) {
      addWasOpen = true;
    } else if (addWasOpen) {
      addWasOpen = false;
      refreshImportableCount();
    }
  });

  onMount(() => {
    loadSshProfiles();
    // Probe ~/.ssh/config quietly — only used to enable the empty-state
    // import callout. No prompt or banner unless the user has zero
    // profiles AND there are importable hosts.
    refreshImportableCount();
    // The "+" near tabs in Topbar and the picker's "+ New SSH Profile" both
    // dispatch SSH_EVENT.NEW_PROFILE. Listen here since the modal lives in
    // this component.
    const onNewProfile = () => {
      addInitialView = 'manual';
      showAdd = true;
    };
    window.addEventListener(SSH_EVENT.NEW_PROFILE, onNewProfile);
    return () => window.removeEventListener(SSH_EVENT.NEW_PROFILE, onNewProfile);
  });

  export function showAddProfile() {
    addInitialView = 'manual';
    showAdd = true;
  }

  function openAddImport() {
    addInitialView = 'import';
    showAdd = true;
  }

  /** Close every tab + kill every session belonging to a given profile.
   * Used by the "Disconnect" context-menu entry. */
  function disconnectAllForProfile(profile: SshProfile) {
    const allTabs = get(tabsStore);
    const tids = get(sshTerminalIds);
    const matching = allTabs.filter(
      (t) => t.mode === 'ssh' && t.key && profileIdFromTabKey(t.key) === profile.id
    );
    for (const tab of matching) {
      if (tab.key) {
        const termId = tids.get(tab.key);
        if (termId) sshKillTerminal(termId).catch(() => {});
        window.dispatchEvent(new CustomEvent(SSH_EVENT.CLOSE_TAB, { detail: { tabKey: tab.key } }));
      }
      closeTab(tab.id);
    }
    if (get(activeSshProfile)?.id === profile.id) activeSshProfile.set(null);
  }

  /** Open this SSH profile as an Explorer SFTP tab. Reuses the existing
   *  ssh_profiles row — finds-or-creates an explorer_connections row that
   *  references it, then switches to Explorer mode and opens a tab. */
  async function openSftpForProfile(profile: SshProfile) {
    try {
      // Make sure we have the latest connection list.
      if (get(explorerConnections).length === 0) {
        try { await loadExplorerConnections(); } catch { /* ignore */ }
      }
      let conn = get(explorerConnections).find(
        (c) => c.kind === 'sftp' && c.sshProfileId === profile.id,
      );
      if (!conn) {
        conn = await createConnection({
          id: '',
          name: profile.name,
          kind: 'sftp',
          accentColor: null,
          lastUsedAt: null,
          createdAt: '',
          sshProfileId: profile.id,
          sftpWorkingDir: null,
          host: null, port: null, username: null, authType: null, keyPath: null,
          ftpPassive: 1, ftpTls: null,
          s3Preset: null, s3Endpoint: null, s3Region: null, s3Bucket: null, s3PathStyle: 0,
          azureAccount: null, azureContainer: null, azureAuthKind: null,
        } as ExplorerConnection);
        await loadExplorerConnections();
      }
      // Switch to Explorer mode + open a tab on this connection.
      mode.set('explorer');
      const allTabs = get(tabsStore2);
      const existing = allTabs.find(
        (t) => t.mode === 'explorer' && t.key && t.key.startsWith(`${conn!.id}#`),
      );
      if (existing) {
        activateTab(existing.id);
        return;
      }
      const tabKey = newExplorerTabKey(conn.id);
      explorerConnStates.update((m) => {
        const next = new Map(m);
        next.set(tabKey, 'connecting');
        return next;
      });
      addTab(profile.name, 'explorer', tabKey, `var(--explorer)`);
      try {
        await openSession(conn.id, tabKey);
        explorerConnStates.update((m) => {
          const next = new Map(m);
          next.set(tabKey, 'connected');
          return next;
        });
      } catch (err: any) {
        explorerConnStates.update((m) => {
          const next = new Map(m);
          next.set(tabKey, 'error');
          return next;
        });
        showToast(`SFTP connection failed: ${err}`, 'error');
      }
    } catch (e: any) {
      showToast(`Open files failed: ${e}`, 'error');
    }
  }

  const filteredProfiles = $derived(
    searchQuery
      ? $sshProfiles.filter(
          (p) =>
            p.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
            p.host.toLowerCase().includes(searchQuery.toLowerCase()) ||
            p.username.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : $sshProfiles
  );

  function handleSelect(profile: SshProfile) {
    activeSshProfile.set(profile);
    // Best-effort touch + refresh so the "last used" stamp in the list updates
    // immediately without waiting for the next nav re-render.
    sshTouchProfile(profile.id)
      .then(() => loadSshProfiles())
      .catch(() => {});
    window.dispatchEvent(new CustomEvent(SSH_EVENT.OPEN_TAB, { detail: profile }));
  }

  function showProfileMenu(e: MouseEvent, profile: SshProfile) {
    e.preventDefault();
    e.stopPropagation();

    // A profile is "connected" if any of its sessions (tabs) are live.
    const tids = $sshTerminalIds;
    const states = $sshConnStates;
    const hasConnected = Array.from(tids.keys()).some(
      (k) => profileIdFromTabKey(k) === profile.id && states.get(k) === 'connected'
    );

    const items: any[] = [];

    if (hasConnected) {
      items.push({
        label: 'Disconnect',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18.36 6.64a9 9 0 11-12.73 0"/><line x1="12" y1="2" x2="12" y2="12"/></svg>',
        action: () => showConfirm(
          'Disconnect SSH session',
          `Disconnect from "${profile.name}"? Any open terminal tabs for this connection will be closed.`,
          false,
          async () => { disconnectAllForProfile(profile); },
        ),
      });
      items.push({
        label: 'Duplicate Session',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>',
        action: () => {
          window.dispatchEvent(new CustomEvent(SSH_EVENT.DUPLICATE_SESSION, { detail: profile }));
        },
      });
    } else {
      items.push({
        label: 'Connect',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/><path d="M12 5l7 7-7 7"/></svg>',
        action: () => handleSelect(profile),
      });
    }

    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: 'Open files (SFTP)',
      icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>',
      action: () => openSftpForProfile(profile),
    });
    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: 'Edit',
      icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
      action: () => {
        editTarget = profile;
        showEdit = true;
      },
    });

    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: 'Delete',
      icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>',
      danger: true,
      action: () => showConfirm(
        'Delete SSH Profile',
        `Delete "${profile.name}"? This cannot be undone.`,
        true,
        async () => {
          // Close any open tabs / kill any live sessions for this profile
          // FIRST, then delete the DB row. The other order would leave
          // tabs referencing a phantom profile id with no way to render.
          disconnectAllForProfile(profile);
          try {
            await sshDeleteProfile(profile.id);
            await loadSshProfiles();
            showToast('Profile deleted', 'success');
          } catch (e: any) {
            showToast(String(e), 'error');
          }
        },
      ),
    });

    showContextMenu(e.clientX, e.clientY, items);
  }

  async function handleConfirmOk() {
    confirmShow = false;
    if (confirmAction) await confirmAction();
    confirmAction = null;
  }

  function relativeTime(iso: string | null): string {
    if (!iso) return 'never';
    let normalized = iso;
    // SQLite "YYYY-MM-DD HH:MM:SS" (UTC) → ISO with T and Z.
    if (/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(normalized)) {
      normalized = normalized.replace(' ', 'T') + 'Z';
    }
    // chrono::Utc::now().to_rfc3339() emits microsecond precision (.713742+00:00).
    // WKWebView's Date constructor only honors 3-digit millis — strip extra digits.
    normalized = normalized.replace(/(\.\d{3})\d+/, '$1');
    // RFC3339 "+00:00" is valid but some parsers prefer "Z" — normalize.
    normalized = normalized.replace(/\+00:00$/, 'Z');
    const t = new Date(normalized).getTime();
    if (Number.isNaN(t)) {
      // Last-resort hand parse: YYYY-MM-DD[T ]HH:MM:SS with optional .frac and TZ.
      const m = iso.match(/^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2}):(\d{2})/);
      if (m) {
        const ms = Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]);
        if (!Number.isNaN(ms)) return formatDiff(Date.now() - ms);
      }
      return 'never';
    }
    return formatDiff(Date.now() - t);
  }

  function formatDiff(diff: number): string {
    if (diff < 0) return 'just now';
    if (diff < 60_000) return 'just now';
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
    if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
    if (diff < 30 * 86_400_000) return `${Math.floor(diff / 86_400_000)}d ago`;
    if (diff < 365 * 86_400_000) return `${Math.floor(diff / (30 * 86_400_000))}mo ago`;
    return `${Math.floor(diff / (365 * 86_400_000))}y ago`;
  }
</script>

<div class="ssh-nav">
  {#if filteredProfiles.length === 0}
    <div class="nav-empty">
      {#if searchQuery}
        <span>No results for "{searchQuery}"</span>
      {:else}
        <span>No SSH profiles yet</span>
        <button class="nav-empty-btn" onclick={() => (showAdd = true)}>+ New Connection</button>
        {#if importableCount > 0}
          <button class="nav-empty-btn import" onclick={openAddImport}>
            Import {importableCount} {importableCount === 1 ? 'host' : 'hosts'} from SSH config
          </button>
        {/if}
      {/if}
    </div>
  {:else}
    {#each filteredProfiles as profile (profile.id)}
      {@const connected = Array.from($sshTerminalIds.keys()).some((k) => profileIdFromTabKey(k) === profile.id && $sshConnStates.get(k) === 'connected')}
      <button
        class="profile-item"
        class:active={$activeSshProfile?.id === profile.id}
        class:connected
        onclick={() => handleSelect(profile)}
        oncontextmenu={(e) => showProfileMenu(e, profile)}
      >
        <span class="profile-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
            <rect x="2" y="4" width="20" height="6" rx="1"/>
            <rect x="2" y="14" width="20" height="6" rx="1"/>
            <line x1="6" y1="7" x2="6.01" y2="7"/>
            <line x1="6" y1="17" x2="6.01" y2="17"/>
          </svg>
          {#if connected}<span class="profile-status-dot" aria-label="Connected" title="Connected"></span>{/if}
        </span>
        <div class="profile-body">
          <div class="profile-row-top">
            <span class="profile-name">{profile.name}</span>
          </div>
          <div class="profile-row-bot">
            <span class="profile-host">{profile.username}@{profile.host}{profile.port !== 22 ? `:${profile.port}` : ''}</span>
            <span class="profile-time-spacer"></span>
            <span class="profile-time">{relativeTime(profile.lastUsedAt)}</span>
          </div>
        </div>
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <span
          class="profile-ellipsis"
          role="button"
          tabindex="-1"
          title="More"
          onclick={(e) => { e.stopPropagation(); showProfileMenu(e, profile); }}
        >
          <svg viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="5" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="12" cy="19" r="1.5"/></svg>
        </span>
      </button>
    {/each}
  {/if}
</div>

<NewSshProfileModal bind:show={showAdd} initialView={addInitialView} />
<EditSshProfileModal bind:show={showEdit} bind:profile={editTarget} />

<ConfirmDialog
  bind:show={confirmShow}
  title={confirmTitle}
  message={confirmMessage}
  confirmText={confirmDanger ? 'Delete' : 'Disconnect'}
  confirmColor={confirmDanger ? 'var(--err)' : 'var(--acc)'}
  onconfirm={handleConfirmOk}
/>

<style>
  .ssh-nav {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .ssh-nav::-webkit-scrollbar { width: 3px; }
  .ssh-nav::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .nav-empty {
    padding: 24px 12px;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--ui);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  .nav-empty-btn {
    padding: 5px 12px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11px;
    font-family: var(--ui);
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .nav-empty-btn:hover { background: var(--c); border-color: var(--b2); color: var(--t1); }
  .nav-empty-btn.import {
    border-style: dashed;
    color: var(--t3);
  }
  .nav-empty-btn.import:hover {
    border-style: solid;
    border-color: var(--ssh, var(--acc));
    color: var(--ssh, var(--acc));
  }

  .profile-item {
    width: 100%;
    min-height: 46px;
    border: none;
    background: transparent;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    cursor: pointer;
    transition: background 0.08s;
    text-align: left;
    position: relative;
  }
  .profile-item:hover { background: var(--c); }
  .profile-item.active {
    background: color-mix(in srgb, var(--ssh, var(--acc)) 12%, transparent);
  }

  .profile-icon {
    position: relative;
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--acc);
    transition: color 0.15s, transform 0.15s;
  }
  .profile-icon svg {
    width: 16px;
    height: 16px;
    position: relative;
    z-index: 1;
  }
  /* Connected state — softer than the original. Border-only ring (no bg
     fill), tighter pulse spread, slower cadence. Goal: peripherally
     visible "this is live" without drawing attention away from the row
     content. Reverted from the unified left-stripe attempt (didn't read
     well alongside the icon). */
  .profile-item.connected .profile-icon { color: var(--ok, #1dc880); }
  .profile-item.connected .profile-icon::before {
    content: '';
    position: absolute;
    inset: -2px;
    border-radius: 8px;
    border: 1px solid color-mix(in srgb, var(--ok, #1dc880) 28%, transparent);
    z-index: 0;
  }
  .profile-status-dot {
    position: absolute;
    top: 1px;
    right: 1px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok, #1dc880);
    box-shadow: 0 0 0 1.5px var(--n);
    z-index: 2;
    animation: profileStatusPulse 3s ease-in-out infinite;
  }
  @keyframes profileStatusPulse {
    0%, 100% { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 2px color-mix(in srgb, var(--ok, #1dc880) 30%, transparent); }
    50%      { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 5px color-mix(in srgb, var(--ok, #1dc880) 0%, transparent); }
  }

  .profile-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .profile-row-top { display: flex; align-items: center; gap: 6px; }
  .profile-name {
    font-family: var(--ui);
    font-size: 12px;
    color: var(--t2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .profile-item.active .profile-name { color: var(--t1); }
  .profile-row-bot { display: flex; align-items: center; gap: 5px; }
  .profile-host {
    font-size: 10.5px;
    color: var(--t3);
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .profile-time-spacer { flex: 1; }
  .profile-time {
    font-family: var(--ui);
    font-size: 9px;
    color: var(--t4);
    white-space: nowrap;
  }

  .profile-ellipsis {
    width: 18px; height: 18px;
    display: none; align-items: center; justify-content: center;
    border-radius: 3px; flex-shrink: 0; cursor: default;
    color: var(--t3); transition: background 0.1s, color 0.1s;
  }
  .profile-ellipsis svg { width: 14px; height: 14px; }
  .profile-item:hover .profile-ellipsis { display: flex; }
  .profile-ellipsis:hover { background: rgba(255,255,255,0.08); color: var(--t1); }

</style>

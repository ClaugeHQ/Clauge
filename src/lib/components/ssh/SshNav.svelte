<script lang="ts">
  import { onMount } from 'svelte';
  import { sshProfiles, activeSshProfile, loadSshProfiles, sshTerminalIds, sshConnStates } from '$lib/stores/ssh';
  import { sshTouchProfile, sshDeleteProfile, sshCreateProfile } from '$lib/commands/ssh';
  import { showContextMenu } from '$lib/components/shared/contextmenu';
  import { showToast } from '$lib/components/shared/toast';
  import NewSshProfileModal from './NewSshProfileModal.svelte';
  import EditSshProfileModal from './EditSshProfileModal.svelte';
  import type { SshProfile } from '$lib/types/ssh';

  // Teleport: lift modals/confirm dialog to body, escapes nav stacking context.
  function teleport(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentElement === document.body) node.remove();
      },
    };
  }

  interface Props {
    searchQuery?: string;
  }
  let { searchQuery = '' }: Props = $props();

  let showAdd = $state(false);
  let showEdit = $state(false);
  let editTarget = $state<SshProfile | null>(null);

  // Confirm dialog
  let confirmShow = $state(false);
  let confirmTitle = $state('');
  let confirmMessage = $state('');
  let confirmAction: (() => Promise<void>) | null = $state(null);

  onMount(() => {
    loadSshProfiles();
  });

  export function showAddProfile() {
    showAdd = true;
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
    window.dispatchEvent(new CustomEvent('ssh:open-tab', { detail: profile }));
  }

  function showProfileMenu(e: MouseEvent, profile: SshProfile) {
    e.preventDefault();
    e.stopPropagation();

    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Connect',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/><path d="M12 5l7 7-7 7"/></svg>',
        action: () => handleSelect(profile),
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Edit',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
        action: () => {
          editTarget = profile;
          showEdit = true;
        },
      },
      {
        label: 'Duplicate',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>',
        action: async () => {
          try {
            await sshCreateProfile({
              name: profile.name + ' (copy)',
              host: profile.host,
              port: profile.port,
              username: profile.username,
              authType: profile.authType,
              keyPath: profile.keyPath,
              accentColor: profile.accentColor,
              // Secrets do not duplicate — user must re-enter for the copy
              secret: null,
              passphrase: null,
            });
            await loadSshProfiles();
            showToast('Profile duplicated — re-enter secret if needed', 'info');
          } catch (e: any) {
            showToast(String(e), 'error');
          }
        },
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Delete',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>',
        danger: true,
        action: () => {
          confirmTitle = 'Delete SSH Profile';
          confirmMessage = `Delete "${profile.name}"? This cannot be undone.`;
          confirmAction = async () => {
            try {
              await sshDeleteProfile(profile.id);
              await loadSshProfiles();
              showToast('Profile deleted', 'success');
            } catch (e: any) {
              showToast(String(e), 'error');
            }
          };
          confirmShow = true;
        },
      },
    ]);
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
      {/if}
    </div>
  {:else}
    {#each filteredProfiles as profile (profile.id)}
      {@const connected = $sshTerminalIds.has(profile.id) && $sshConnStates.get(profile.id) === 'connected'}
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

<NewSshProfileModal bind:show={showAdd} />
<EditSshProfileModal bind:show={showEdit} bind:profile={editTarget} />

{#if confirmShow}
  <div class="confirm-portal" use:teleport>
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="confirm-overlay" onclick={() => (confirmShow = false)}>
      <div class="confirm-dialog" onclick={(e) => e.stopPropagation()}>
        <div class="confirm-title">{confirmTitle}</div>
        <div class="confirm-msg">{confirmMessage}</div>
        <div class="confirm-actions">
          <button class="confirm-btn" onclick={() => (confirmShow = false)}>Cancel</button>
          <button class="confirm-btn danger" onclick={handleConfirmOk}>Delete</button>
        </div>
      </div>
    </div>
  </div>
{/if}

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
    color: var(--ssh, var(--acc));
    transition: color 0.15s, transform 0.15s;
  }
  .profile-icon svg {
    width: 16px;
    height: 16px;
  }
  /* Connected state: subtle background ring + pulsing green status dot.
     Distinguishes "this profile has a live SSH session" from idle profiles
     when many are listed. */
  .profile-item.connected .profile-icon {
    color: var(--ok, #1dc880);
  }
  .profile-item.connected .profile-icon::before {
    content: '';
    position: absolute;
    inset: -2px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--ok, #1dc880) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--ok, #1dc880) 35%, transparent);
    z-index: 0;
  }
  .profile-icon svg { position: relative; z-index: 1; }
  .profile-status-dot {
    position: absolute;
    top: 0px;
    right: 0px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ok, #1dc880);
    box-shadow: 0 0 0 2px var(--n);
    z-index: 2;
    animation: profileStatusPulse 2s ease-in-out infinite;
  }
  @keyframes profileStatusPulse {
    0%, 100% { box-shadow: 0 0 0 2px var(--n), 0 0 0 4px color-mix(in srgb, var(--ok, #1dc880) 50%, transparent); }
    50%      { box-shadow: 0 0 0 2px var(--n), 0 0 0 8px color-mix(in srgb, var(--ok, #1dc880) 0%, transparent); }
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

  /* Confirm dialog */
  .confirm-overlay {
    position: fixed; top: 0; left: 0; width: 100vw; height: 100vh;
    background: rgba(0,0,0,0.4); z-index: 9999;
    display: flex; align-items: center; justify-content: center;
  }
  .confirm-dialog {
    background: var(--modal-bg, var(--n)); border: 1px solid var(--b1);
    border-radius: 12px; padding: 24px; min-width: 320px; max-width: 400px;
    box-shadow: 0 16px 48px rgba(0,0,0,0.5);
  }
  .confirm-title {
    font-size: 15px; font-weight: 600; color: var(--t1); font-family: var(--ui);
    margin-bottom: 8px;
  }
  .confirm-msg {
    font-size: 13px; color: var(--t2); font-family: var(--ui); line-height: 1.5;
    margin-bottom: 20px;
  }
  .confirm-actions { display: flex; justify-content: flex-end; gap: 8px; }
  .confirm-btn {
    padding: 7px 16px; border-radius: 8px; font-size: 12px; font-weight: 600;
    font-family: var(--ui); cursor: default; border: 1px solid var(--b1);
    background: transparent; color: var(--t2); transition: all 0.12s;
  }
  .confirm-btn:hover { background: var(--c); color: var(--t1); }
  .confirm-btn.danger { background: var(--err); color: #fff; border-color: transparent; }
  .confirm-btn.danger:hover { opacity: 0.9; }
</style>

<script lang="ts">
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { sshCreateProfile, sshReadConfigHosts, sshImportConfigHosts } from '../commands';
  import { loadSshProfiles, sshProfiles } from '../stores';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import type { SshAuthType, SshProfile, SshConfigHost } from '../types';
  import { get } from 'svelte/store';
  import { SSH_EVENT } from '$lib/shared/constants/events';

  type InitialView = 'manual' | 'import';
  type Tab = 'general' | 'advanced' | 'import';
  type ConnectionMode = 'direct' | 'jump' | 'command';

  interface Props {
    show?: boolean;
    /** Which tab to show when the modal opens. Defaults to 'manual'
     *  (which maps to the General form tab). 'import' opens straight on
     *  the import-from-SSH-config view. The 'manual'/'import' API is
     *  preserved from the pre-tabs design — internal `Tab` adds a third
     *  state ('advanced') that callers don't need to know about. */
    initialView?: InitialView;
    /** Called after a profile is successfully created. Lets callers
     *  (e.g. the SQL/NoSQL ConnectionDialog) auto-select the new
     *  profile in their picker. Optional — existing usage that just
     *  binds `show` keeps working untouched. */
    onCreated?: (profile: SshProfile) => void;
  }

  let { show = $bindable(false), initialView = 'manual', onCreated }: Props = $props();

  let activeTab = $state<Tab>('general');

  $effect(() => {
    if (show) activeTab = initialView === 'import' ? 'import' : 'general';
  });

  // Empty by default — user types a name explicitly. Avoids the bad
  // pattern of many "New Connection" duplicates accumulating when users
  // don't notice a prefill.
  let name = $state('');
  let host = $state('');
  let port = $state(22);
  let username = $state('');
  let authType = $state<SshAuthType>('key');
  let keyPath = $state('');
  let passphrase = $state('');
  let password = $state('');
  let revealSecret = $state(false);
  let loading = $state(false);

  // 'direct' → straight TCP; 'jump' → tunnel through another saved
  // profile (OpenSSH ProxyJump); 'command' → spawn a subprocess as the
  // transport (OpenSSH ProxyCommand). Mutually exclusive at the UI level.
  // If both are set on a row (e.g. from import), the connect path uses
  // ProxyCommand (matches OpenSSH precedence).
  let connectionMode = $state<ConnectionMode>('direct');
  let jumpProfileId = $state<string>('');
  let proxyCommand = $state('');

  async function pickKeyFile() {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({
        multiple: false,
        title: 'Select SSH Private Key',
        filters: [
          { name: 'SSH Keys', extensions: ['pem', 'key', 'ppk'] },
          { name: 'All Files', extensions: ['*'] },
        ],
      });
      if (typeof selected === 'string') keyPath = selected;
    } catch {
      /* user cancelled or dialog plugin unavailable — silently ignore */
    }
  }

  async function handleCreate() {
    if (!host.trim() || !username.trim() || !name.trim()) return;
    if (authType === 'key' && !keyPath.trim()) return;

    loading = true;
    try {
      const profile = await sshCreateProfile({
        name: name.trim(),
        host: host.trim(),
        port: Number(port) || 22,
        username: username.trim(),
        authType,
        keyPath: authType === 'key' ? keyPath.trim() : null,
        accentColor: null,
        // 'password' allows empty secret — modal prompts at connect time.
        // ssh-agent profiles store nothing locally.
        secret: authType === 'password' && password ? password : null,
        passphrase: authType === 'key' && passphrase ? passphrase : null,
        jumpProfileId: connectionMode === 'jump' && jumpProfileId ? jumpProfileId : null,
        proxyCommand:
          connectionMode === 'command' && proxyCommand.trim() ? proxyCommand.trim() : null,
      });
      await loadSshProfiles();
      window.dispatchEvent(new CustomEvent(SSH_EVENT.PROFILE_CREATED, { detail: profile }));
      // Caller (SQL/NoSQL ConnectionDialog) wants to auto-select the new
      // profile — skip OPEN_TAB so we don't open an SSH terminal they
      // didn't ask for.
      if (onCreated) {
        onCreated(profile);
      } else {
        window.dispatchEvent(new CustomEvent(SSH_EVENT.OPEN_TAB, { detail: profile }));
      }
      show = false;
      resetForm();
      showToast('SSH profile saved', 'success');
    } catch (e: any) {
      showToast(String(e), 'error');
    } finally {
      loading = false;
    }
  }

  function resetForm() {
    name = '';
    host = '';
    port = 22;
    username = '';
    authType = 'key';
    keyPath = '';
    passphrase = '';
    password = '';
    revealSecret = false;
    connectionMode = 'direct';
    jumpProfileId = '';
    proxyCommand = '';
  }

  // Case-insensitive duplicate-name check against the existing list.
  let nameDuplicate = $derived.by(() => {
    const trimmed = name.trim().toLowerCase();
    if (!trimmed) return false;
    return get(sshProfiles).some((p) => p.name.trim().toLowerCase() === trimmed);
  });

  // ── Import view state ────────────────────────────────────────────────
  let importHosts = $state<SshConfigHost[]>([]);
  let importLoading = $state(false);
  let importing = $state(false);
  let importSelected = $state<Set<string>>(new Set());
  let importError = $state<string | null>(null);

  async function loadImportHosts() {
    importLoading = true;
    importError = null;
    try {
      const list = await sshReadConfigHosts();
      importHosts = list;
      importSelected = new Set(list.filter((h) => !h.alreadyExists).map((h) => h.alias));
    } catch (e: any) {
      importError = String(e);
      importHosts = [];
      importSelected = new Set();
    } finally {
      importLoading = false;
    }
  }

  $effect(() => {
    if (show && activeTab === 'import') loadImportHosts();
  });

  let importableCount = $derived(importHosts.filter((h) => !h.alreadyExists).length);
  let importSkippedCount = $derived(importHosts.filter((h) => h.alreadyExists).length);
  let importAllSelected = $derived(
    importableCount > 0 && importSelected.size === importableCount,
  );

  // Reverse lookup so we can show "required by X" — only counts SELECTED
  // hosts; an unimported host's ProxyJump declaration shouldn't visually
  // flag its target.
  const requiredBy = $derived.by(() => {
    const map = new Map<string, string[]>();
    for (const sel of importSelected) {
      const host = importHosts.find((h) => h.alias === sel);
      if (!host) continue;
      for (const dep of host.proxyJumpAliases) {
        const existing = map.get(dep) ?? [];
        if (!existing.includes(sel)) existing.push(sel);
        map.set(dep, existing);
      }
    }
    return map;
  });

  function importToggle(alias: string, alreadyExists: boolean) {
    if (alreadyExists) return;
    const next = new Set(importSelected);
    if (next.has(alias)) {
      // Don't auto-uncheck dependents — let the user explicitly drop
      // them. Orphaned dependents import without a jump pointer; the
      // import code handles unresolved aliases by skipping.
      next.delete(alias);
    } else {
      next.add(alias);
      // Auto-check ProxyJump dependencies that are also in the list and
      // not already imported. Without this, importing `thetastrike`
      // alone (with `ProxyJump nx.thetasecure.com`) produces a profile
      // with NULL jump_profile_id — connect would then try direct TCP
      // to a private bastion-only host and fail.
      const host = importHosts.find((h) => h.alias === alias);
      if (host?.proxyJumpAliases.length) {
        for (const depAlias of host.proxyJumpAliases) {
          const dep = importHosts.find((h) => h.alias === depAlias);
          if (dep && !dep.alreadyExists) next.add(depAlias);
        }
      }
    }
    importSelected = next;
  }

  function importToggleAll() {
    if (importAllSelected) {
      importSelected = new Set();
    } else {
      importSelected = new Set(importHosts.filter((h) => !h.alreadyExists).map((h) => h.alias));
    }
  }

  async function doImport() {
    if (importSelected.size === 0) return;
    importing = true;
    try {
      const count = await sshImportConfigHosts(Array.from(importSelected));
      await loadSshProfiles();
      showToast(`Imported ${count} ${count === 1 ? 'host' : 'hosts'}`, 'success');
      show = false;
    } catch (e: any) {
      errorToast('Import failed', e);
    } finally {
      importing = false;
    }
  }

  let canCreate = $derived(
    name.trim() !== '' &&
      !nameDuplicate &&
      host.trim() !== '' &&
      username.trim() !== '' &&
      ((authType === 'key' && keyPath.trim() !== '') ||
        authType === 'password' ||
        authType === 'agent')
  );

  // Dot indicator on Advanced when routing is non-direct, so users
  // notice there's something custom set without clicking the tab.
  let advancedDirty = $derived(connectionMode !== 'direct');
</script>

<Modal bind:show title="New SSH connection" width="560px">
  <div class="sshd-root">
    <!-- Single tab row: General / Advanced / Import (Import pushed right) -->
    <div class="sshd-tabs" role="tablist">
      <button type="button" role="tab" class="sshd-tab" class:active={activeTab === 'general'} aria-selected={activeTab === 'general'} onclick={() => (activeTab = 'general')}>General</button>
      <button type="button" role="tab" class="sshd-tab" class:active={activeTab === 'advanced'} aria-selected={activeTab === 'advanced'} onclick={() => (activeTab = 'advanced')}>
        Advanced
        {#if advancedDirty}<span class="sshd-tab-dot" aria-hidden="true"></span>{/if}
      </button>
      <button type="button" role="tab" class="sshd-tab sshd-tab-end" class:active={activeTab === 'import'} aria-selected={activeTab === 'import'} onclick={() => (activeTab = 'import')}>
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" aria-hidden="true"><path d="M12 4v12M7 11l5 5 5-5M5 20h14" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>
        Import
      </button>
    </div>

    {#if activeTab === 'general'}
      <div class="sshd-block">
        <span class="sshd-label">Connection name</span>
        <input class="sshd-input" class:error={nameDuplicate} type="text" bind:value={name} placeholder="prod-bastion, my-droplet…" />
        {#if nameDuplicate}<span class="sshd-error-text">A profile named "{name.trim()}" already exists.</span>{/if}
      </div>

      <div class="sshd-row">
        <div class="sshd-block grow">
          <span class="sshd-label">Host</span>
          <input class="sshd-input mono" type="text" bind:value={host} placeholder="example.com or 10.0.0.5" />
        </div>
        <div class="sshd-block narrow">
          <span class="sshd-label">Port</span>
          <input class="sshd-input mono" type="number" min="1" max="65535" bind:value={port} />
        </div>
      </div>

      <div class="sshd-block">
        <span class="sshd-label">Username</span>
        <input class="sshd-input mono" type="text" bind:value={username} placeholder="root, ubuntu, ec2-user…" />
      </div>

      <div class="sshd-block">
        <span class="sshd-label">Authentication</span>
        <div class="sshd-tile-row">
          <button type="button" class="sshd-tile" class:active={authType === 'key'} onclick={() => (authType = 'key')}>
            <span class="sshd-tile-icon"><svg viewBox="0 0 24 24" width="15" height="15" fill="none"><circle cx="8" cy="15" r="4" stroke="currentColor" stroke-width="1.6"/><path d="M11 12l8-8M15 8l3 3M17 6l3 3" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg></span>
            <span class="sshd-tile-label">Key</span>
          </button>
          <button type="button" class="sshd-tile" class:active={authType === 'password'} onclick={() => (authType = 'password')}>
            <span class="sshd-tile-icon"><svg viewBox="0 0 24 24" width="15" height="15" fill="none"><circle cx="6.5" cy="12" r="1.5" fill="currentColor"/><circle cx="12" cy="12" r="1.5" fill="currentColor"/><circle cx="17.5" cy="12" r="1.5" fill="currentColor"/><rect x="3" y="7" width="18" height="10" rx="2" stroke="currentColor" stroke-width="1.5"/></svg></span>
            <span class="sshd-tile-label">Password</span>
          </button>
          <button type="button" class="sshd-tile" class:active={authType === 'agent'} onclick={() => (authType = 'agent')}>
            <span class="sshd-tile-icon"><svg viewBox="0 0 24 24" width="15" height="15" fill="none"><path d="M4 7h12a3 3 0 010 6H8a3 3 0 000 6h12" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg></span>
            <span class="sshd-tile-label">Agent</span>
          </button>
        </div>
      </div>

      {#if authType === 'agent'}
        <div class="sshd-info">Uses keys from ssh-agent. Run <code>ssh-add</code> first. Required for hardware tokens (YubiKey, smartcard).</div>
      {:else if authType === 'key'}
        <div class="sshd-block">
          <span class="sshd-label">Private key file</span>
          <div class="sshd-with-suffix">
            <input class="sshd-input mono" type="text" bind:value={keyPath} placeholder="/home/me/.ssh/id_ed25519" />
            <button class="sshd-suffix-btn" onclick={pickKeyFile} type="button" title="Choose file" aria-label="Choose file">
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none"><path d="M4 7a2 2 0 012-2h3l2 2h7a2 2 0 012 2v8a2 2 0 01-2 2H6a2 2 0 01-2-2V7z" stroke="currentColor" stroke-width="1.6"/></svg>
            </button>
          </div>
        </div>
        <div class="sshd-block">
          <span class="sshd-label">Passphrase <span class="sshd-optional">(if encrypted)</span></span>
          <div class="sshd-with-suffix">
            <input class="sshd-input" type={revealSecret ? 'text' : 'password'} bind:value={passphrase} placeholder="Leave empty if none" autocomplete="off" />
            <button class="sshd-suffix-btn" type="button" onclick={() => (revealSecret = !revealSecret)} aria-label={revealSecret ? 'Hide passphrase' : 'Show passphrase'}>
              {#if revealSecret}<svg viewBox="0 0 24 24" width="14" height="14" fill="none"><path d="M4 4l16 16" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/><path d="M9.5 5.5A9 9 0 0121 12c-.6 1.2-1.4 2.3-2.4 3.2M14.5 18.5A9 9 0 013 12c.6-1.2 1.4-2.3 2.4-3.2" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/><circle cx="12" cy="12" r="3" stroke="currentColor" stroke-width="1.6"/></svg>{:else}<svg viewBox="0 0 24 24" width="14" height="14" fill="none"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z" stroke="currentColor" stroke-width="1.6"/><circle cx="12" cy="12" r="3" stroke="currentColor" stroke-width="1.6"/></svg>{/if}
            </button>
          </div>
        </div>
      {:else}
        <div class="sshd-block">
          <span class="sshd-label">Password <span class="sshd-optional">(optional)</span></span>
          <div class="sshd-with-suffix">
            <input class="sshd-input" type={revealSecret ? 'text' : 'password'} bind:value={password} placeholder="Blank = prompt at connect time" autocomplete="off" />
            <button class="sshd-suffix-btn" type="button" onclick={() => (revealSecret = !revealSecret)} aria-label={revealSecret ? 'Hide password' : 'Show password'}>
              {#if revealSecret}<svg viewBox="0 0 24 24" width="14" height="14" fill="none"><path d="M4 4l16 16" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/><path d="M9.5 5.5A9 9 0 0121 12c-.6 1.2-1.4 2.3-2.4 3.2M14.5 18.5A9 9 0 013 12c.6-1.2 1.4-2.3 2.4-3.2" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/><circle cx="12" cy="12" r="3" stroke="currentColor" stroke-width="1.6"/></svg>{:else}<svg viewBox="0 0 24 24" width="14" height="14" fill="none"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z" stroke="currentColor" stroke-width="1.6"/><circle cx="12" cy="12" r="3" stroke="currentColor" stroke-width="1.6"/></svg>{/if}
            </button>
          </div>
        </div>
      {/if}
    {:else if activeTab === 'advanced'}
      <div class="sshd-block">
        <span class="sshd-label">Routing</span>
        <div class="sshd-tile-row">
          <button type="button" class="sshd-tile" class:active={connectionMode === 'direct'} onclick={() => (connectionMode = 'direct')}>
            <span class="sshd-tile-icon"><svg viewBox="0 0 24 24" width="15" height="15" fill="none"><path d="M4 12h14M14 7l5 5-5 5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg></span>
            <span class="sshd-tile-label">Direct</span>
          </button>
          <button type="button" class="sshd-tile" class:active={connectionMode === 'jump'} onclick={() => (connectionMode = 'jump')}>
            <span class="sshd-tile-icon"><svg viewBox="0 0 24 24" width="15" height="15" fill="none"><circle cx="5" cy="17" r="2" stroke="currentColor" stroke-width="1.6"/><circle cx="12" cy="7" r="2" stroke="currentColor" stroke-width="1.6"/><circle cx="19" cy="17" r="2" stroke="currentColor" stroke-width="1.6"/><path d="M6.6 15.5L10.6 8.5M13.4 8.5L17.4 15.5" stroke="currentColor" stroke-width="1.6"/></svg></span>
            <span class="sshd-tile-label">Jump host</span>
          </button>
          <button type="button" class="sshd-tile" class:active={connectionMode === 'command'} onclick={() => (connectionMode = 'command')}>
            <span class="sshd-tile-icon"><svg viewBox="0 0 24 24" width="15" height="15" fill="none"><rect x="3" y="5" width="18" height="14" rx="2" stroke="currentColor" stroke-width="1.6"/><path d="M7 10l3 2-3 2M12 14h5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg></span>
            <span class="sshd-tile-label">Proxy cmd</span>
          </button>
        </div>
      </div>

      {#if connectionMode === 'direct'}
        <div class="sshd-info">Direct TCP to the host. No bastion or proxy.</div>
      {:else if connectionMode === 'jump'}
        <div class="sshd-block">
          <span class="sshd-label">Jump profile</span>
          <div class="sshd-select-wrap">
            <select class="sshd-input mono sshd-select" bind:value={jumpProfileId}>
              <option value="">— Select a jump profile —</option>
              {#each $sshProfiles as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
            </select>
            <svg class="sshd-select-chev" viewBox="0 0 24 24" width="14" height="14" fill="none" aria-hidden="true"><path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>
          </div>
          <span class="sshd-caption">Connect to this profile first, then open a tunneled channel to the destination. For multi-hop chains, set the jump profile's own jump host.</span>
        </div>
      {:else}
        <div class="sshd-block">
          <span class="sshd-label">Proxy command</span>
          <input class="sshd-input mono" type="text" bind:value={proxyCommand} placeholder="cloudflared access ssh --hostname %h" autocomplete="off" spellcheck="false" />
          <span class="sshd-caption"><code>%h</code> host · <code>%p</code> port · <code>%r</code> user · tokenized as argv, no shell.</span>
        </div>
      {/if}
    {:else}
      <!-- Import view -->
      {#if importLoading}
        <div class="sshd-status">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" class="sshd-spin" aria-hidden="true"><circle cx="12" cy="12" r="8" stroke="currentColor" stroke-width="2" stroke-opacity="0.3"/><path d="M12 4a8 8 0 018 8" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>
          Reading SSH config…
        </div>
      {:else if importError}
        <div class="sshd-status error">Couldn't read SSH config: {importError}</div>
      {:else if importHosts.length === 0}
        <div class="sshd-status">No host entries found in <code>~/.ssh/config</code>. Add some there and re-open.</div>
      {:else}
        <div class="sshd-import-bar">
          <div>
            <span class="sshd-import-count">{importHosts.length} host{importHosts.length === 1 ? '' : 's'}</span>
            {#if importSkippedCount > 0}<span class="sshd-import-skipped"> · {importSkippedCount} already imported</span>{/if}
          </div>
          {#if importableCount > 0}
            <label class="sshd-import-all">
              <input type="checkbox" checked={importAllSelected} onchange={importToggleAll} />
              <span>Select all</span>
            </label>
          {/if}
        </div>

        <div class="sshd-import-list">
          {#each importHosts as h (h.alias)}
            <label class="sshd-import-row" class:disabled={h.alreadyExists}>
              <input type="checkbox" checked={importSelected.has(h.alias)} disabled={h.alreadyExists} onchange={() => importToggle(h.alias, h.alreadyExists)} />
              <div class="sshd-import-body">
                <div class="sshd-import-head">
                  <span class="sshd-import-alias">{h.alias}</span>
                  {#if h.alreadyExists}<span class="sshd-badge muted">imported</span>{:else if h.proxyCommand}<span class="sshd-badge accent">via cmd</span>{:else if h.proxyJumpAliases.length > 0}<span class="sshd-badge accent">via {h.proxyJumpAliases.join(' → ')}</span>{/if}
                  {#if requiredBy.get(h.alias)?.length}<span class="sshd-badge warn">required by {requiredBy.get(h.alias)!.join(', ')}</span>{/if}
                </div>
                <div class="sshd-import-meta">{h.user ? `${h.user}@` : ''}{h.hostname}{h.port !== 22 ? `:${h.port}` : ''}{#if h.identityFile}<span class="sshd-import-meta-dim"> · {h.identityFile}</span>{/if}</div>
              </div>
            </label>
          {/each}
        </div>
      {/if}
    {/if}

    <div class="sshd-footer">
      {#if activeTab === 'import'}
        <span class="sshd-keychain-note" title="Passwords aren't in SSH config — password-auth hosts will need a password set in Edit after import.">
          <svg viewBox="0 0 24 24" width="11" height="11" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="1.6"/><path d="M12 7v6M12 16.5v.1" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
          set passwords after import
        </span>
      {:else}
        <span class="sshd-keychain-note" title="Secrets are stored in your system keychain — never in the Clauge database.">
          <svg viewBox="0 0 24 24" width="11" height="11" fill="none" aria-hidden="true"><rect x="5" y="11" width="14" height="9" rx="2" stroke="currentColor" stroke-width="1.6"/><path d="M8 11V8a4 4 0 018 0v3" stroke="currentColor" stroke-width="1.6"/></svg>
          stored in keychain
        </span>
      {/if}
      <div class="sshd-spacer"></div>
      <button type="button" class="sshd-btn outline" onclick={() => { show = false; resetForm(); }}>Cancel</button>
      {#if activeTab === 'import'}
        <button type="button" class="sshd-btn primary" disabled={importSelected.size === 0 || importing || importLoading} onclick={doImport}>
          {#if importing}<svg viewBox="0 0 24 24" width="13" height="13" fill="none" class="sshd-spin" aria-hidden="true"><circle cx="12" cy="12" r="8" stroke="currentColor" stroke-width="2" stroke-opacity="0.3"/><path d="M12 4a8 8 0 018 8" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>Importing…{:else}Import {importSelected.size} {importSelected.size === 1 ? 'host' : 'hosts'}{/if}
        </button>
      {:else}
        <button type="button" class="sshd-btn primary" onclick={handleCreate} disabled={!canCreate || loading}>
          {#if loading}<svg viewBox="0 0 24 24" width="13" height="13" fill="none" class="sshd-spin" aria-hidden="true"><circle cx="12" cy="12" r="8" stroke="currentColor" stroke-width="2" stroke-opacity="0.3"/><path d="M12 4a8 8 0 018 8" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>Saving…{:else}Save connection{/if}
        </button>
      {/if}
    </div>
  </div>
</Modal>

<style>
  .sshd-root {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin: -4px 0 -4px;
  }

  /* ── Tabs (underline style, matches Agent NewSessionModal) ──────── */
  .sshd-tabs {
    display: flex;
    gap: 4px;
    margin: -4px -4px 4px;
    border-bottom: 1px solid var(--b1);
  }
  .sshd-tab {
    position: relative;
    background: transparent;
    border: none;
    padding: 10px 16px;
    font-family: var(--ui);
    font-size: 13px;
    color: var(--t3);
    cursor: default;
    transition: color 0.12s;
    border-radius: 0;
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }
  .sshd-tab:hover { color: var(--t1); }
  .sshd-tab.active {
    color: var(--t1);
    font-weight: 600;
  }
  .sshd-tab.active::after {
    content: '';
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: -1px;
    height: 2px;
    background: var(--ssh, var(--acc));
    border-radius: 2px 2px 0 0;
  }
  .sshd-tab-dot {
    width: 6px;
    height: 6px;
    border-radius: 99px;
    background: var(--ssh, var(--acc));
  }
  /* Import tab sits at the far right — visually marks "alternate input
     mode" vs the General/Advanced configuration tabs. */
  .sshd-tab-end { margin-left: auto; }

  /* ── Labels + inputs ─────────────────────────────────────────────── */
  .sshd-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .sshd-block.grow { flex: 1 1 0; }
  .sshd-block.narrow { flex: 0 0 110px; }
  .sshd-row { display: flex; gap: 10px; }
  .sshd-label {
    font-family: var(--mono);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--t3);
  }
  .sshd-optional {
    text-transform: none;
    letter-spacing: 0;
    font-weight: normal;
    color: var(--t3);
    font-size: 10.5px;
  }
  .sshd-input {
    width: 100%;
    box-sizing: border-box;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 9px;
    padding: 9px 12px;
    color: var(--t1);
    font-size: 13.5px;
    font-family: var(--ui);
    outline: none;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .sshd-input.mono { font-family: var(--mono); }
  .sshd-input::placeholder { color: var(--t3); }
  .sshd-input:focus {
    border-color: var(--ssh, var(--acc));
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--ssh, var(--acc)) 16%, transparent);
  }
  .sshd-input.error, .sshd-input.error:focus {
    border-color: var(--err);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--err) 14%, transparent);
  }
  .sshd-error-text {
    color: var(--err);
    font-size: 11.5px;
    font-family: var(--ui);
    line-height: 1.4;
  }
  .sshd-caption {
    font-family: var(--ui);
    font-size: 11.5px;
    color: var(--t3);
    line-height: 1.5;
  }
  .sshd-caption code {
    font-family: var(--mono);
    color: var(--t2);
    padding: 0 4px;
    border-radius: 4px;
    background: var(--e);
    font-size: 10.5px;
  }

  /* ── Input with suffix button ────────────────────────────────────── */
  .sshd-with-suffix { position: relative; }
  .sshd-with-suffix .sshd-input { padding-right: 40px; }
  .sshd-suffix-btn {
    position: absolute;
    right: 5px;
    top: 50%;
    transform: translateY(-50%);
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    background: var(--c);
    border: 1px solid var(--b1);
    border-radius: 7px;
    color: var(--t2);
    cursor: default;
    transition: color 0.15s, border-color 0.15s;
  }
  .sshd-suffix-btn:hover { color: var(--t1); border-color: var(--b2); }

  /* ── Tile row ────────────────────────────────────────────────────── */
  .sshd-tile-row {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
  }
  .sshd-tile {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 9px 10px;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 9px;
    cursor: default;
    transition: border-color 0.15s, background 0.15s;
  }
  .sshd-tile:hover { border-color: var(--b2); }
  .sshd-tile.active {
    background: color-mix(in srgb, var(--ssh, var(--acc)) 12%, transparent);
    border-color: var(--ssh, var(--acc));
  }
  .sshd-tile-icon {
    line-height: 0;
    color: var(--t2);
    opacity: 0.85;
    transition: color 0.15s, opacity 0.15s;
  }
  .sshd-tile:hover .sshd-tile-icon { opacity: 1; }
  .sshd-tile.active .sshd-tile-icon { color: var(--ssh, var(--acc)); opacity: 1; }
  .sshd-tile-label {
    font-family: var(--ui);
    font-size: 12.5px;
    font-weight: 500;
    color: var(--t2);
  }
  .sshd-tile.active .sshd-tile-label { color: var(--t1); }

  /* ── Select ──────────────────────────────────────────────────────── */
  .sshd-select-wrap { position: relative; }
  .sshd-select {
    appearance: none;
    -webkit-appearance: none;
    padding-right: 34px;
    cursor: default;
  }
  .sshd-select-chev {
    position: absolute;
    right: 12px;
    top: 50%;
    transform: translateY(-50%);
    pointer-events: none;
    color: var(--t3);
  }

  /* ── Info card ───────────────────────────────────────────────────── */
  .sshd-info {
    padding: 9px 12px;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 8px;
    font-family: var(--ui);
    font-size: 11.5px;
    color: var(--t2);
    line-height: 1.5;
  }
  .sshd-info code {
    font-family: var(--mono);
    color: var(--t1);
    padding: 1px 4px;
    border-radius: 3px;
    background: var(--c);
    font-size: 10.5px;
  }

  /* ── Footer ──────────────────────────────────────────────────────── */
  .sshd-footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-top: 14px;
    margin-top: 2px;
    border-top: 1px solid var(--b1);
  }
  .sshd-spacer { flex: 1; }
  .sshd-keychain-note {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: var(--ui);
    font-size: 11px;
    color: var(--t3);
    padding: 3px 9px;
    background: color-mix(in srgb, var(--ssh, var(--acc)) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--ssh, var(--acc)) 18%, transparent);
    border-radius: 99px;
  }
  .sshd-keychain-note > svg { flex-shrink: 0; color: var(--ssh, var(--acc)); }
  .sshd-btn {
    height: 34px;
    padding: 0 16px;
    border-radius: 9px;
    font-family: var(--ui);
    font-size: 12.5px;
    font-weight: 500;
    cursor: default;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    transition: background 0.15s, border-color 0.15s, color 0.15s, opacity 0.15s;
  }
  .sshd-btn.outline {
    background: transparent;
    border: 1px solid var(--b1);
    color: var(--t2);
  }
  .sshd-btn.outline:hover { border-color: var(--b2); color: var(--t1); }
  .sshd-btn.primary {
    background: var(--ssh, var(--acc));
    border: 1px solid var(--ssh, var(--acc));
    color: #fff;
    font-weight: 600;
    padding: 0 20px;
    box-shadow: 0 6px 16px -8px color-mix(in srgb, var(--ssh, var(--acc)) 80%, transparent);
  }
  .sshd-btn.primary:hover:not(:disabled) { filter: brightness(1.05); }
  .sshd-btn.primary:disabled { opacity: 0.45; }
  .sshd-spin { animation: sshd-spin 0.7s linear infinite; }
  @keyframes sshd-spin { to { transform: rotate(360deg); } }

  /* ── Import view ─────────────────────────────────────────────────── */
  .sshd-status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 20px 4px;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 13px;
    line-height: 1.5;
  }
  .sshd-status.error { color: var(--err); }
  .sshd-status code {
    font-family: var(--mono);
    background: var(--e);
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 12px;
    color: var(--t2);
  }
  .sshd-import-bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 9px;
    font-family: var(--ui);
    font-size: 12px;
  }
  .sshd-import-count { color: var(--t1); font-weight: 500; }
  .sshd-import-skipped { color: var(--t3); }
  .sshd-import-all {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--t2);
    cursor: default;
  }
  .sshd-import-all input { accent-color: var(--ssh, var(--acc)); }
  .sshd-import-list {
    max-height: 320px;
    overflow-y: auto;
    overflow-x: hidden;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-right: 4px;
  }
  .sshd-import-list::-webkit-scrollbar { width: 6px; }
  .sshd-import-list::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 3px; }
  .sshd-import-list::-webkit-scrollbar-thumb:hover { background: var(--b2); }
  .sshd-import-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 9px 12px;
    border-radius: 9px;
    border: 1px solid var(--b1);
    background: var(--e);
    cursor: default;
    transition: border-color 0.12s;
  }
  .sshd-import-row:hover:not(.disabled) { border-color: var(--b2); }
  .sshd-import-row.disabled { opacity: 0.5; }
  .sshd-import-row input[type="checkbox"] { margin-top: 2px; flex-shrink: 0; accent-color: var(--ssh, var(--acc)); }
  .sshd-import-body { flex: 1; min-width: 0; }
  .sshd-import-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 7px;
    font-family: var(--ui);
    font-size: 13px;
    font-weight: 500;
    color: var(--t1);
  }
  .sshd-import-alias {
    border-left: 2px solid var(--ssh, var(--acc));
    padding-left: 8px;
    margin-left: -2px;
  }
  .sshd-import-meta {
    margin-top: 3px;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sshd-import-meta-dim { color: color-mix(in srgb, var(--t3) 70%, transparent); }
  .sshd-badge {
    display: inline-flex;
    align-items: center;
    font-family: var(--mono);
    font-size: 9.5px;
    font-weight: 500;
    letter-spacing: 0.04em;
    padding: 1px 7px;
    border-radius: 99px;
    text-transform: lowercase;
  }
  .sshd-badge.muted { background: var(--c); color: var(--t3); border: 1px solid var(--b1); }
  .sshd-badge.accent {
    background: color-mix(in srgb, var(--ssh, var(--acc)) 16%, transparent);
    color: var(--ssh, var(--acc));
    border: 1px solid color-mix(in srgb, var(--ssh, var(--acc)) 28%, transparent);
  }
  .sshd-badge.warn {
    background: color-mix(in srgb, var(--warn, #f5a623) 16%, transparent);
    color: var(--warn, #f5a623);
    border: 1px solid color-mix(in srgb, var(--warn, #f5a623) 30%, transparent);
  }

  @media (max-width: 520px) {
    .sshd-row { flex-direction: column; }
    .sshd-block.narrow { flex: 1 1 auto; }
  }
</style>

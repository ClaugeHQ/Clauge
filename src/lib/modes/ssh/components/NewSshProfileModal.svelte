<script lang="ts">
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { sshCreateProfile } from '../commands';
  import { loadSshProfiles, sshProfiles } from '../stores';
  import { showToast } from '$lib/shared/primitives/toast';
  import type { SshAuthType, SshProfile } from '../types';
  import { get } from 'svelte/store';
  import { SSH_EVENT } from '$lib/shared/constants/events';

  interface Props {
    show?: boolean;
    /** Called after a profile is successfully created. Lets callers
     * (e.g. the SQL/NoSQL ConnectionDialog) auto-select the new profile
     * in their picker. Optional — existing usage that just binds `show`
     * keeps working untouched. */
    onCreated?: (profile: SshProfile) => void;
  }

  let { show = $bindable(false), onCreated }: Props = $props();

  // ── Form state ─────────────────────────────────────────────────────────────
  // Empty by default — user types a name explicitly. Avoids the bad pattern of
  // many "New Connection" duplicates accumulating when users don't notice the prefill.
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
      if (typeof selected === 'string') {
        keyPath = selected;
      }
    } catch {
      // user cancelled or dialog plugin unavailable — silently ignore
    }
  }

  async function handleCreate() {
    if (!host.trim() || !username.trim() || !name.trim()) return;
    if (authType === 'key' && !keyPath.trim()) return;
    if (authType === 'password' && !password) return;
    // 'agent' has no profile-side requirements — keys live in ssh-agent.

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
        secret: authType === 'password' ? password : null,
        passphrase: authType === 'key' && passphrase ? passphrase : null,
        // ssh-agent profiles store nothing locally — agent holds the key.
      });
      await loadSshProfiles();
      window.dispatchEvent(new CustomEvent(SSH_EVENT.PROFILE_CREATED, { detail: profile }));
      // Notify caller (e.g. SQL/NoSQL ConnectionDialog) so it can auto-select
      // the new profile. Skip the OPEN_TAB broadcast in that case — the user
      // is configuring a DB connection, not opening an SSH terminal.
      if (onCreated) {
        onCreated(profile);
      } else {
        // Auto-connect: open the new profile in a tab immediately. If the
        // connection fails it surfaces via the reconnect banner — no
        // information is lost.
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
  }

  // Duplicate-name check (case-insensitive trim) against the existing list.
  let nameDuplicate = $derived.by(() => {
    const trimmed = name.trim().toLowerCase();
    if (!trimmed) return false;
    return get(sshProfiles).some((p) => p.name.trim().toLowerCase() === trimmed);
  });

  let canCreate = $derived(
    name.trim() !== '' &&
      !nameDuplicate &&
      host.trim() !== '' &&
      username.trim() !== '' &&
      ((authType === 'key' && keyPath.trim() !== '') ||
        (authType === 'password' && password !== '') ||
        authType === 'agent')
  );
</script>

<Modal bind:show title="New SSH Connection" width="460px">
  <div class="ns-form">
    <label class="ns-field">
      <span class="ns-label">Name</span>
      <input
        class="ns-input"
        class:ns-input-error={nameDuplicate}
        type="text"
        bind:value={name}
        placeholder="My Server"
      />
      {#if nameDuplicate}
        <span class="ns-field-error">A profile named "{name.trim()}" already exists. Pick a different name.</span>
      {/if}
    </label>

    <div class="ns-row">
      <label class="ns-field" style="flex:3">
        <span class="ns-label">Host</span>
        <input class="ns-input" type="text" bind:value={host} placeholder="example.com or 10.0.0.5" />
      </label>
      <label class="ns-field" style="flex:1">
        <span class="ns-label">Port</span>
        <input class="ns-input" type="number" min="1" max="65535" bind:value={port} />
      </label>
    </div>

    <label class="ns-field">
      <span class="ns-label">Username</span>
      <input class="ns-input" type="text" bind:value={username} placeholder="root, ubuntu, ec2-user…" />
    </label>

    <div class="ns-field">
      <span class="ns-label">Authentication</span>
      <div class="ns-radio-row">
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <span class="ns-chip" class:selected={authType === 'key'} onclick={() => (authType = 'key')}>SSH Key</span>
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <span class="ns-chip" class:selected={authType === 'password'} onclick={() => (authType = 'password')}>Password</span>
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <span class="ns-chip" class:selected={authType === 'agent'} onclick={() => (authType = 'agent')}>SSH Agent</span>
      </div>
    </div>

    {#if authType === 'agent'}
      <div class="ns-field">
        <span class="ns-optional">
          Uses keys loaded into the running ssh-agent (Unix socket on macOS/Linux, named pipe on Windows).
          Required for hardware tokens (YubiKey, smartcard). Run <code>ssh-add</code> to load keys first.
        </span>
      </div>
    {:else if authType === 'key'}
      <label class="ns-field">
        <span class="ns-label">Private Key File</span>
        <div class="ns-path-row">
          <input class="ns-input ns-path-input" type="text" bind:value={keyPath} placeholder="/home/me/.ssh/id_ed25519" />
          <button class="ns-btn-browse" onclick={pickKeyFile} type="button">Choose…</button>
        </div>
      </label>
      <label class="ns-field">
        <span class="ns-label">Passphrase <span class="ns-optional">(if encrypted)</span></span>
        <div class="ns-path-row">
          <input
            class="ns-input ns-path-input"
            type={revealSecret ? 'text' : 'password'}
            bind:value={passphrase}
            placeholder="leave empty if none"
            autocomplete="off"
          />
          <button class="ns-btn-browse" type="button" onclick={() => (revealSecret = !revealSecret)}>
            {revealSecret ? 'Hide' : 'Show'}
          </button>
        </div>
      </label>
    {:else}
      <label class="ns-field">
        <span class="ns-label">Password</span>
        <div class="ns-path-row">
          <input
            class="ns-input ns-path-input"
            type={revealSecret ? 'text' : 'password'}
            bind:value={password}
            placeholder="••••••••"
            autocomplete="off"
          />
          <button class="ns-btn-browse" type="button" onclick={() => (revealSecret = !revealSecret)}>
            {revealSecret ? 'Hide' : 'Show'}
          </button>
        </div>
      </label>
    {/if}

    <p class="ns-helper">
      Secrets are stored in your system keychain — not in the Clauge database.
    </p>

    <div class="ns-actions">
      <button class="ns-btn-cancel" onclick={() => { show = false; resetForm(); }}>Cancel</button>
      <button class="ns-btn-create" onclick={handleCreate} disabled={!canCreate || loading}>
        {loading ? 'Saving…' : 'Save'}
      </button>
    </div>
  </div>
</Modal>

<style>
  .ns-form { display: flex; flex-direction: column; gap: 12px; }
  .ns-field { display: flex; flex-direction: column; gap: 4px; }
  .ns-label { font-size: 12px; font-weight: 600; color: var(--t2); text-transform: uppercase; font-family: var(--ui); }
  .ns-optional { font-size: 10px; color: var(--t3); font-weight: normal; text-transform: none; }
  .ns-input {
    width: 100%; background: var(--e); border: 1px solid var(--b1); border-radius: 6px;
    padding: 8px 10px; font-size: 13px; color: var(--t1); outline: none; box-sizing: border-box;
    font-family: var(--mono); transition: border-color 0.15s;
  }
  .ns-input:focus { border-color: var(--ssh, var(--acc)); }
  .ns-input::placeholder { color: var(--t3); }
  .ns-input.ns-input-error,
  .ns-input.ns-input-error:focus {
    border-color: var(--err);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--err) 14%, transparent);
  }
  .ns-field-error {
    color: var(--err);
    font-size: 11.5px;
    font-family: var(--ui);
    margin-top: 2px;
    line-height: 1.4;
  }
  .ns-row { display: flex; gap: 8px; }
  .ns-path-row { display: flex; gap: 8px; }
  .ns-path-input { flex: 1; }
  .ns-btn-browse {
    background: var(--n); border: 1px solid var(--b1); border-radius: 6px;
    padding: 8px 12px; color: var(--t1); font-size: 12px; cursor: pointer;
    white-space: nowrap; font-family: var(--ui); transition: border-color 0.15s;
  }
  .ns-btn-browse:hover { border-color: var(--b2); }
  .ns-radio-row { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 4px; }
  .ns-chip {
    padding: 5px 12px; border-radius: 14px; border: 1px solid var(--b1);
    background: transparent; color: var(--t2); font-size: 12px; cursor: pointer;
    font-family: var(--ui); transition: background 0.15s, color 0.15s; user-select: none;
  }
  .ns-chip:hover:not(.selected) { background: rgba(255,255,255,0.06); }
  .ns-chip.selected {
    font-weight: 600;
    color: var(--ssh, var(--acc));
    border-color: var(--ssh, var(--acc));
    background: color-mix(in srgb, var(--ssh, var(--acc)) 15%, transparent);
  }
  .ns-helper {
    font-size: 11px; color: var(--t3); font-family: var(--ui);
    margin: 0; line-height: 1.5;
  }
  .ns-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; padding-top: 12px; border-top: 1px solid var(--b1); }
  .ns-btn-cancel {
    padding: 7px 16px; border-radius: 6px; font-size: 13px; cursor: pointer;
    border: 1px solid var(--b1); background: transparent; color: var(--t2); font-family: var(--ui);
  }
  .ns-btn-cancel:hover { background: rgba(255,255,255,0.04); }
  .ns-btn-create {
    padding: 7px 16px; border-radius: 6px; font-size: 13px; cursor: pointer;
    border: none; background: var(--ssh, var(--acc)); color: #fff; font-weight: 600; font-family: var(--ui);
  }
  .ns-btn-create:hover:not(:disabled) { filter: brightness(1.1); }
  .ns-btn-create:disabled { opacity: 0.4; cursor: not-allowed; }
</style>

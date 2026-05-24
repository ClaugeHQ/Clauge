<script lang="ts">
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { sshProfiles, loadSshProfiles } from '$lib/modes/ssh/stores';
  import { createConnection, updateConnection, setSecret, getSecret } from '$lib/modes/explorer/commands';
  import { loadExplorerConnections } from '$lib/modes/explorer/stores';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import { onMount } from 'svelte';
  import type { ExplorerConnection } from '$lib/modes/explorer/types';

  interface Props {
    show: boolean;
    /** When set, modal opens in EDIT mode pre-filled from this row. */
    editing?: ExplorerConnection | null;
    onclose?: () => void;
  }

  let { show = $bindable(), editing = null, onclose }: Props = $props();
  const isEdit = $derived(!!editing);

  type Source = 'direct' | 'profile';
  let source = $state<Source>('direct');

  // Shared
  let name = $state('');
  let workingDir = $state('');

  // Existing SSH profile
  let sshProfileId = $state<string>('');

  // Direct
  let host = $state('');
  let port = $state(22);
  let username = $state('');
  let authType = $state<'password' | 'key' | 'agent'>('password');
  let keyPath = $state('');
  let secret = $state('');

  let saving = $state(false);

  // Native file picker for the private key — same pattern SSH mode uses.
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

  onMount(async () => {
    if ($sshProfiles.length === 0) {
      try { await loadSshProfiles(); } catch { /* ignore */ }
    }
    if ($sshProfiles.length > 0) sshProfileId = $sshProfiles[0].id;
  });

  // Re-prefill whenever the modal is (re)opened in edit mode. Track the
  // last id we filled from so the same `editing` row doesn't overwrite
  // user edits mid-typing.
  let lastFilledId = $state<string | null>(null);
  $effect(() => {
    if (show && editing && editing.id !== lastFilledId) {
      lastFilledId = editing.id;
      name = editing.name ?? '';
      workingDir = editing.sftpWorkingDir ?? '';
      if (editing.sshProfileId) {
        source = 'profile';
        sshProfileId = editing.sshProfileId;
      } else {
        source = 'direct';
        host = editing.host ?? '';
        port = editing.port ?? 22;
        username = editing.username ?? '';
        authType = (editing.authType as typeof authType) ?? 'password';
        keyPath = editing.keyPath ?? '';
      }
      // Pre-fill the secret box (lets the user see/replace it). Empty
      // string is fine for unencrypted keys / agent auth.
      secret = '';
      const secretName = authType === 'password' ? 'password' : authType === 'key' ? 'passphrase' : null;
      if (secretName) {
        getSecret(editing.id, secretName).then((v) => { if (v != null) secret = v; }).catch(() => {});
      }
    }
    if (!show) {
      lastFilledId = null;
    }
  });

  function resetForm() {
    name = '';
    workingDir = '';
    source = 'direct';
    sshProfileId = $sshProfiles[0]?.id ?? '';
    host = '';
    port = 22;
    username = '';
    authType = 'password';
    keyPath = '';
    secret = '';
  }

  async function handleSave() {
    if (!name.trim()) { showToast('Name is required', 'error'); return; }
    if (source === 'profile' && !sshProfileId) {
      showToast('Pick an SSH profile or switch to "New connection details"', 'error');
      return;
    }
    if (source === 'direct' && !host.trim()) {
      showToast('Host is required', 'error'); return;
    }
    saving = true;
    try {
      const payload: ExplorerConnection = {
        id: editing?.id ?? '',
        name: name.trim(),
        kind: 'sftp',
        accentColor: editing?.accentColor ?? null,
        lastUsedAt: editing?.lastUsedAt ?? null,
        createdAt: editing?.createdAt ?? '',
        sshProfileId: source === 'profile' ? sshProfileId : null,
        sftpWorkingDir: workingDir.trim() || null,
        host: source === 'direct' ? host.trim() : null,
        port: source === 'direct' ? port : null,
        username: source === 'direct' ? username.trim() : null,
        authType: source === 'direct' ? authType : null,
        keyPath: source === 'direct' && authType === 'key' ? keyPath.trim() : null,
        ftpPassive: 1,
        ftpTls: null,
        s3Preset: null, s3Endpoint: null, s3Region: null, s3Bucket: null, s3PathStyle: 0,
        azureAccount: null, azureContainer: null, azureAuthKind: null,
      };
      let connId: string;
      if (isEdit && editing) {
        await updateConnection(payload);
        connId = editing.id;
      } else {
        const created = await createConnection(payload);
        connId = created.id;
      }
      if (source === 'direct' && (authType === 'password' || authType === 'key') && secret) {
        const secretName = authType === 'password' ? 'password' : 'passphrase';
        await setSecret(connId, secretName, secret);
      }
      await loadExplorerConnections();
      showToast(isEdit ? 'SFTP connection updated' : 'SFTP connection saved', 'success');
      resetForm();
      show = false;
      onclose?.();
    } catch (e: any) {
      errorToast('Save failed', e);
    } finally {
      saving = false;
    }
  }
</script>

<Modal bind:show title={isEdit ? 'Edit SFTP connection' : 'New SFTP connection'} width="640px" onclose={() => { onclose?.(); }}>
  <div class="form">
    <label class="row">
      <span>Name</span>
      <input class="inp" type="text" bind:value={name} placeholder="e.g. Production server" />
    </label>

    <div class="seg">
      <button class:on={source === 'direct'} onclick={() => source = 'direct'}>New connection details</button>
      <button class:on={source === 'profile'} onclick={() => source = 'profile'}>Use existing SSH profile</button>
    </div>

    {#if source === 'profile'}
      <label class="row">
        <span>SSH profile</span>
        {#if $sshProfiles.length === 0}
          <span class="hint">No SSH profiles yet — add one in SSH mode first.</span>
        {:else}
          <select class="inp" bind:value={sshProfileId}>
            {#each $sshProfiles as p (p.id)}
              <option value={p.id}>{p.name} — {p.username}@{p.host}:{p.port}</option>
            {/each}
          </select>
        {/if}
      </label>
    {:else}
      <label class="row">
        <span>Host</span>
        <input class="inp" type="text" bind:value={host} placeholder="server.example.com" />
      </label>
      <label class="row">
        <span>Port</span>
        <input class="inp" type="number" bind:value={port} />
      </label>
      <label class="row">
        <span>Username</span>
        <input class="inp" type="text" bind:value={username} />
      </label>
      <label class="row">
        <span>Auth</span>
        <select class="inp" bind:value={authType}>
          <option value="password">Password</option>
          <option value="key">Private key</option>
          <option value="agent">SSH agent</option>
        </select>
      </label>
      {#if authType === 'key'}
        <label class="row">
          <span>Key path</span>
          <div class="row-inline">
            <input class="inp" type="text" bind:value={keyPath} placeholder="~/.ssh/id_ed25519" />
            <button type="button" class="btn-browse" onclick={pickKeyFile}>Choose…</button>
          </div>
        </label>
        <label class="row">
          <span>Passphrase (if any)</span>
          <input class="inp" type="password" bind:value={secret} />
        </label>
      {:else if authType === 'password'}
        <label class="row">
          <span>Password</span>
          <input class="inp" type="password" bind:value={secret} />
        </label>
      {/if}
    {/if}

    <label class="row">
      <span>Working directory <span class="optional">(optional)</span></span>
      <input class="inp" type="text" bind:value={workingDir} placeholder="/home/user" />
    </label>

    <div class="actions">
      <button class="btn" onclick={() => { show = false; onclose?.(); }}>Cancel</button>
      <button class="btn primary" onclick={handleSave} disabled={saving}>
        {saving ? 'Saving…' : isEdit ? 'Save changes' : 'Save connection'}
      </button>
    </div>
  </div>
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    /* Make sure inputs/selects fill the modal body width and never push
       horizontal overflow. */
    min-width: 0;
  }
  .row { display: flex; flex-direction: column; gap: 4px; font-family: var(--ui); font-size: 12px; color: var(--t2); min-width: 0; }
  .row .optional { color: var(--t4); font-weight: 400; }
  .row .hint { font-size: 11px; color: var(--t3); padding: 4px 0; }
  .inp {
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 5px;
    color: var(--t1);
    /* Lock height instead of relying on padding alone — input vs select
       have different default content boxes, so equal padding produces
       different rendered heights. Explicit height = pixel-perfect parity. */
    height: 32px;
    padding: 0 10px;
    font-size: 13px;
    font-family: var(--mono);
    outline: none;
    transition: border-color 0.12s;
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
    text-overflow: ellipsis;
  }
  .inp:focus { border-color: var(--acc); }

  /* <select> default appearance pulls in OS-styled dropdown chrome that
     ignores our colours and shifts internal padding. Strip it and draw
     a small caret as a background image so the field reads like the
     other inputs. */
  select.inp {
    -webkit-appearance: none;
    -moz-appearance: none;
    appearance: none;
    padding-right: 28px;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23b0b0c8' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'><polyline points='3 5 6 8 9 5'/></svg>");
    background-repeat: no-repeat;
    background-position: right 10px center;
    background-size: 10px 10px;
  }

  /* Hide WebKit's native number-input spinner — it ignores our theme
     and shows as a black native control on dark backgrounds. Users can
     still bump values with keyboard up/down arrows when focused. */
  .inp[type="number"]::-webkit-inner-spin-button,
  .inp[type="number"]::-webkit-outer-spin-button {
    -webkit-appearance: none;
    appearance: none;
    margin: 0;
  }
  .inp[type="number"] {
    appearance: textfield;
    -moz-appearance: textfield;
  }
  .row-inline {
    display: flex;
    gap: 6px;
    align-items: center;
    min-width: 0;
  }
  .btn-browse {
    flex-shrink: 0;
    height: 32px;
    padding: 0 12px;
    border: 1px solid var(--b1);
    background: var(--surface-hover);
    color: var(--t2);
    border-radius: 5px;
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: border-color 0.12s, color 0.12s;
  }
  .btn-browse:hover { border-color: var(--acc); color: var(--t1); }
  .seg {
    display: flex;
    gap: 4px;
    background: var(--n2);
    padding: 3px;
    border-radius: 6px;
    border: 1px solid var(--b1);
  }
  .seg button {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--t3);
    padding: 6px 10px;
    border-radius: 4px;
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: background 0.1s, color 0.1s;
  }
  .seg button.on { background: var(--acc); color: #fff; }
  .actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 8px; }
  .btn {
    padding: 7px 14px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
  }
  .btn:hover:not(:disabled) { border-color: var(--b2); color: var(--t1); }
  .btn.primary { background: var(--acc); color: #fff; border-color: transparent; }
  .btn:disabled { opacity: 0.5; }
</style>

<script lang="ts">
  import Modal from '$lib/components/shared/Modal.svelte';
  import { sshUpdateProfile } from '$lib/commands/ssh';
  import { loadSshProfiles } from '$lib/stores/ssh';
  import { showToast } from '$lib/components/shared/toast';
  import type { SshAuthType, SshProfile } from '$lib/types/ssh';

  let { show = $bindable(false), profile = $bindable<SshProfile | null>(null) } = $props();

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

  $effect(() => {
    if (profile && show) {
      name = profile.name;
      host = profile.host;
      port = profile.port;
      username = profile.username;
      authType = profile.authType;
      keyPath = profile.keyPath ?? '';
      passphrase = '';
      password = '';
      revealSecret = false;
    }
  });

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
      /* ignore */
    }
  }

  async function handleSave() {
    if (!profile) return;
    if (!name.trim() || !host.trim() || !username.trim()) return;
    if (authType === 'key' && !keyPath.trim()) return;

    loading = true;
    try {
      const updated = await sshUpdateProfile({
        id: profile.id,
        name: name.trim(),
        host: host.trim(),
        port: Number(port) || 22,
        username: username.trim(),
        authType,
        keyPath: authType === 'key' ? keyPath.trim() : null,
        accentColor: profile.accentColor,
        // Only send `secret` if the user typed something — empty strings keep existing.
        secret: authType === 'password' && password ? password : undefined,
        passphrase: authType === 'key' && passphrase ? passphrase : undefined,
      });
      await loadSshProfiles();
      window.dispatchEvent(new CustomEvent('ssh:profile-updated', { detail: updated }));
      show = false;
      showToast('SSH profile updated', 'success');
    } catch (e: any) {
      showToast(String(e), 'error');
    } finally {
      loading = false;
    }
  }

  let canSave = $derived(
    name.trim() !== '' &&
      host.trim() !== '' &&
      username.trim() !== '' &&
      (authType !== 'key' || keyPath.trim() !== '')
  );
</script>

<Modal bind:show title="Edit SSH Connection" width="460px">
  {#if profile}
    <div class="ns-form">
      <label class="ns-field">
        <span class="ns-label">Name</span>
        <input class="ns-input" type="text" bind:value={name} />
      </label>

      <div class="ns-row">
        <label class="ns-field" style="flex:3">
          <span class="ns-label">Host</span>
          <input class="ns-input" type="text" bind:value={host} />
        </label>
        <label class="ns-field" style="flex:1">
          <span class="ns-label">Port</span>
          <input class="ns-input" type="number" min="1" max="65535" bind:value={port} />
        </label>
      </div>

      <label class="ns-field">
        <span class="ns-label">Username</span>
        <input class="ns-input" type="text" bind:value={username} />
      </label>

      <div class="ns-field">
        <span class="ns-label">Authentication</span>
        <div class="ns-radio-row">
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
          <span class="ns-chip" class:selected={authType === 'key'} onclick={() => (authType = 'key')}>SSH Key</span>
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
          <span class="ns-chip" class:selected={authType === 'password'} onclick={() => (authType = 'password')}>Password</span>
        </div>
      </div>

      {#if authType === 'key'}
        <label class="ns-field">
          <span class="ns-label">Private Key File</span>
          <div class="ns-path-row">
            <input class="ns-input ns-path-input" type="text" bind:value={keyPath} />
            <button class="ns-btn-browse" type="button" onclick={pickKeyFile}>Choose…</button>
          </div>
        </label>
        <label class="ns-field">
          <span class="ns-label">Passphrase</span>
          <div class="ns-path-row">
            <input
              class="ns-input ns-path-input"
              type={revealSecret ? 'text' : 'password'}
              bind:value={passphrase}
              placeholder="Leave blank to keep existing"
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
              placeholder="Leave blank to keep existing"
              autocomplete="off"
            />
            <button class="ns-btn-browse" type="button" onclick={() => (revealSecret = !revealSecret)}>
              {revealSecret ? 'Hide' : 'Show'}
            </button>
          </div>
        </label>
      {/if}

      <div class="ns-actions">
        <button class="ns-btn-cancel" onclick={() => (show = false)}>Cancel</button>
        <button class="ns-btn-create" onclick={handleSave} disabled={!canSave || loading}>
          {loading ? 'Saving…' : 'Save Changes'}
        </button>
      </div>
    </div>
  {/if}
</Modal>

<style>
  .ns-form { display: flex; flex-direction: column; gap: 12px; }
  .ns-field { display: flex; flex-direction: column; gap: 4px; }
  .ns-label { font-size: 12px; font-weight: 600; color: var(--t2); text-transform: uppercase; font-family: var(--ui); }
  .ns-input {
    width: 100%; background: var(--e); border: 1px solid var(--b1); border-radius: 6px;
    padding: 8px 10px; font-size: 13px; color: var(--t1); outline: none; box-sizing: border-box;
    font-family: var(--mono); transition: border-color 0.15s;
  }
  .ns-input:focus { border-color: var(--ssh, var(--acc)); }
  .ns-input::placeholder { color: var(--t3); }
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

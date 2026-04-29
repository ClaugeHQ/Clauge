<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { nosqlTestConnection } from '../commands';
  import type { NoSqlConnection, NoSqlConnectionConfig } from '../types';
  import { sshProfiles, loadSshProfiles } from '$lib/modes/ssh/stores';
  import type { SshProfile } from '$lib/modes/ssh/types';
  import NewSshProfileModal from '$lib/modes/ssh/components/NewSshProfileModal.svelte';

  interface Props {
    show: boolean;
    connection?: NoSqlConnection | null;
    onsave?: (config: NoSqlConnectionConfig) => void | Promise<void>;
    onclose?: () => void;
  }

  let { show = $bindable(false), connection = null, onsave, onclose }: Props = $props();

  let name = $state('');
  let driver = $state<'mongodb' | 'redis'>('mongodb');
  let connectionString = $state('');
  let host = $state('localhost');
  let port = $state(27017);
  let database = $state('');
  let username = $state('');
  let password = $state('');
  let ssl = $state(false);
  let directConnection = $state(false);
  let useConnectionString = $state(true);
  let testing = $state(false);
  let saving = $state(false);
  let testStatus = $state('');
  // Generation counter — bumped on every fresh test AND on dialog close.
  // Each await checks `myGen !== testGen` to bail silently when stale.
  // In-flight backend test will complete; we just discard the result.
  let testGen = $state(0);

  // SSH tunnel section state ─────────────────────────────────────────────
  let useSshTunnel = $state(false);
  let selectedSshProfileId = $state<string | null>(null);
  let showNewSshModal = $state(false);

  $effect(() => {
    if (show && connection) {
      name = connection.name;
      driver = connection.driver;
      connectionString = connection.connectionString;
      host = connection.host;
      port = connection.port;
      database = connection.databaseName ?? '';
      username = connection.username ?? '';
      password = connection.password ?? '';
      ssl = !!connection.ssl;
      directConnection = !!connection.directConnection;
      useConnectionString = !!connection.connectionString;
      const sid = connection.sshProfileId ?? null;
      useSshTunnel = !!sid;
      selectedSshProfileId = sid;
    } else if (show && !connection) {
      name = '';
      driver = 'mongodb';
      connectionString = '';
      host = 'localhost';
      port = 27017;
      database = '';
      username = '';
      password = '';
      ssl = false;
      directConnection = false;
      useConnectionString = true;
      useSshTunnel = false;
      selectedSshProfileId = null;
    }
  });

  $effect(() => {
    if (driver === 'redis' && port === 27017) {
      port = 6379;
    } else if (driver === 'mongodb' && port === 6379) {
      port = 27017;
    }
  });

  // Populate SSH profiles store on first dialog open.
  $effect(() => {
    if (show && $sshProfiles.length === 0) {
      loadSshProfiles();
    }
  });

  // Default-select first profile when toggle flips on without a prior choice.
  $effect(() => {
    if (useSshTunnel && !selectedSshProfileId && $sshProfiles.length > 0) {
      selectedSshProfileId = $sshProfiles[0].id;
    }
  });

  const selectedProfile = $derived(
    selectedSshProfileId ? $sshProfiles.find((p) => p.id === selectedSshProfileId) ?? null : null
  );

  function handleNewSshCreated(profile: SshProfile) {
    selectedSshProfileId = profile.id;
    useSshTunnel = true;
  }

  function buildConfig(): NoSqlConnectionConfig {
    return {
      name: name.trim() || `${driver} connection`,
      driver,
      connectionString: useConnectionString ? connectionString.trim() : '',
      host: useConnectionString ? '' : host.trim(),
      port: useConnectionString ? 0 : port,
      database: database.trim() || undefined,
      username: username.trim() || undefined,
      password: password.trim() || undefined,
      ssl,
      directConnection: driver === 'mongodb' ? directConnection : undefined,
      sshProfileId: useSshTunnel && selectedSshProfileId ? selectedSshProfileId : null,
    };
  }

  /** Best-effort host:port extraction from a `mongodb://` or `redis://` URL.
   * Returns `null` if the URL has no usable authority (e.g. mongodb+srv,
   * malformed strings) — caller should skip the pre-flight in that case. */
  function targetFromUri(uri: string): { host: string; port: number } | null {
    try {
      // mongodb+srv resolves via DNS — no single host/port to tunnel to.
      if (uri.startsWith('mongodb+srv://')) return null;
      const u = new URL(uri);
      if (!u.hostname) return null;
      const defaultPort = driver === 'redis' ? 6379 : 27017;
      const p = u.port ? Number(u.port) : defaultPort;
      return { host: u.hostname, port: p };
    } catch {
      return null;
    }
  }

  async function handleTest() {
    const myGen = ++testGen;
    testing = true;
    testStatus = '';
    try {
      if (useSshTunnel && selectedSshProfileId) {
        const target = useConnectionString
          ? targetFromUri(connectionString.trim())
          : { host: host.trim(), port };
        if (target) {
          testStatus = 'Testing tunnel…';
          try {
            await invoke('ssh_tunnel_test', {
              profileId: selectedSshProfileId,
              targetHost: target.host,
              targetPort: target.port,
            });
          } catch (e: any) {
            if (myGen !== testGen) return; // stale — dialog closed or new test started
            showToast(`Tunnel test failed: ${friendlyError(e)}`, 'error');
            return;
          }
          if (myGen !== testGen) return;
          testStatus = 'Testing database…';
        }
      }
      const msg = await nosqlTestConnection(buildConfig());
      if (myGen !== testGen) return;
      showToast(msg || 'Connection successful', 'success');
    } catch (e: any) {
      if (myGen !== testGen) return;
      showToast(friendlyError(e), 'error');
    } finally {
      if (myGen === testGen) {
        testing = false;
        testStatus = '';
      }
    }
  }

  // Reset state + invalidate in-flight tests when the dialog closes.
  // In-flight backend test will complete; we just discard the result.
  $effect(() => {
    if (!show) {
      testGen++;
      testing = false;
      testStatus = '';
    }
  });

  async function handleSave() {
    const config = buildConfig();
    if (!config.name) {
      showToast('Name is required', 'error');
      return;
    }
    if (useConnectionString && !config.connectionString) {
      showToast('Connection string is required', 'error');
      return;
    }
    if (!useConnectionString && !config.host) {
      showToast('Host is required', 'error');
      return;
    }
    if (useSshTunnel && !selectedSshProfileId) {
      showToast('Pick an SSH profile or turn off the tunnel', 'error');
      return;
    }
    saving = true;
    try {
      await onsave?.(config);
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      saving = false;
    }
  }
</script>

<Modal bind:show title={connection ? 'Edit Connection' : 'New Connection'} width="520px" {onclose}>
  <div class="conn-form">
    <label class="conn-field">
      <span class="conn-label">Name</span>
      <input class="conn-input" type="text" bind:value={name} placeholder="My connection" autocomplete="off" autocorrect="off" autocapitalize="off" spellcheck="false" />
    </label>

    <label class="conn-field">
      <span class="conn-label">Driver</span>
      <select class="conn-select" bind:value={driver}>
        <option value="mongodb">MongoDB</option>
        <option value="redis">Redis</option>
      </select>
    </label>

    <div class="conn-toggle-row">
      <button
        class="conn-toggle-btn"
        class:active={useConnectionString}
        onclick={() => useConnectionString = true}
      >Connection String</button>
      <button
        class="conn-toggle-btn"
        class:active={!useConnectionString}
        onclick={() => useConnectionString = false}
      >Host / Port</button>
    </div>

    {#if useConnectionString}
      <label class="conn-field">
        <span class="conn-label">Connection String</span>
        <input
          class="conn-input mono"
          type="text"
          bind:value={connectionString}
          placeholder={driver === 'mongodb' ? 'mongodb://localhost:27017' : 'redis://localhost:6379'}
          autocomplete="off" autocorrect="off" autocapitalize="off" spellcheck="false"
        />
      </label>
    {:else}
      <div class="conn-row">
        <label class="conn-field" style="flex:2">
          <span class="conn-label">Host</span>
          <input class="conn-input" type="text" bind:value={host} placeholder="localhost" />
        </label>
        <label class="conn-field" style="flex:1">
          <span class="conn-label">Port</span>
          <input class="conn-input" type="number" bind:value={port} />
        </label>
      </div>

      {#if driver === 'mongodb'}
        <label class="conn-field">
          <span class="conn-label">Database</span>
          <input class="conn-input" type="text" bind:value={database} placeholder="(optional)" />
        </label>
        <div class="conn-row">
          <label class="conn-field" style="flex:1">
            <span class="conn-label">Username</span>
            <input class="conn-input" type="text" bind:value={username} placeholder="(optional)" />
          </label>
          <label class="conn-field" style="flex:1">
            <span class="conn-label">Password</span>
            <input class="conn-input" type="password" bind:value={password} placeholder="(optional)" />
          </label>
        </div>
      {:else}
        <label class="conn-field">
          <span class="conn-label">Password</span>
          <input class="conn-input" type="password" bind:value={password} placeholder="(optional)" />
        </label>
      {/if}
    {/if}

    <label class="conn-check">
      <input type="checkbox" bind:checked={ssl} />
      <span>Use SSL / TLS</span>
    </label>

    {#if driver === 'mongodb'}
      <label class="conn-check">
        <input type="checkbox" bind:checked={directConnection} />
        <span>Direct Connection</span>
      </label>
    {/if}

    <!-- SSH Tunnel section -->
    <section class="ssh-section" class:expanded={useSshTunnel}>
      <header class="ssh-section-head">
        <span class="ssh-section-title">SSH Tunnel</span>
        <label class="ssh-toggle">
          <input type="checkbox" bind:checked={useSshTunnel} />
          <span>Connect via SSH tunnel</span>
        </label>
      </header>

      {#if useSshTunnel}
        <div class="ssh-section-body">
          {#if $sshProfiles.length === 0}
            <p class="ssh-empty">No SSH profiles yet.</p>
            <button class="ssh-new-btn primary" type="button" onclick={() => (showNewSshModal = true)}>
              + Create new SSH profile…
            </button>
          {:else}
            <label class="conn-field">
              <span class="conn-label">SSH Profile</span>
              <select class="conn-select" bind:value={selectedSshProfileId}>
                {#each $sshProfiles as p (p.id)}
                  <option value={p.id}>{p.name}</option>
                {/each}
              </select>
              {#if selectedProfile}
                <span class="ssh-caption">{selectedProfile.username}@{selectedProfile.host}:{selectedProfile.port}</span>
              {/if}
            </label>
            <button class="ssh-new-btn" type="button" onclick={() => (showNewSshModal = true)}>
              + Create new SSH profile…
            </button>
          {/if}
        </div>
      {/if}
    </section>

    <div class="conn-actions">
      <button class="conn-test-btn" onclick={handleTest} disabled={testing || saving}>
        {testing ? (testStatus || 'Testing…') : 'Test Connection'}
      </button>
      <div class="conn-spacer"></div>
      <button class="conn-cancel-btn" onclick={() => show = false} disabled={saving}>Cancel</button>
      <button class="conn-save-btn" onclick={handleSave} disabled={saving || testing}>
        {saving ? 'Saving...' : 'Save'}
      </button>
    </div>
  </div>
</Modal>

<NewSshProfileModal bind:show={showNewSshModal} onCreated={handleNewSshCreated} />

<style>
  .conn-form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .conn-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .conn-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--t2);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .conn-input {
    height: 32px;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 0 10px;
    font-size: 12.5px;
    color: var(--t1);
    font-family: var(--mono);
    outline: none;
    transition: border-color 0.15s;
  }
  .conn-input.mono {
    font-family: var(--mono);
  }
  .conn-input:focus {
    border-color: var(--acc);
  }
  .conn-input::placeholder {
    color: var(--t3);
  }
  .conn-select {
    height: 32px;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 0 10px;
    font-size: 12.5px;
    color: var(--t1);
    font-family: var(--ui);
    outline: none;
    cursor: default;
  }
  .conn-row {
    display: flex;
    gap: 10px;
  }
  .conn-toggle-row {
    display: flex;
    gap: 0;
    border: 1px solid var(--b1);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .conn-toggle-btn {
    flex: 1;
    height: 28px;
    border: none;
    background: transparent;
    color: var(--t3);
    font-size: 11px;
    font-family: var(--ui);
    cursor: default;
    transition: background 0.12s, color 0.12s;
  }
  .conn-toggle-btn.active {
    background: var(--acc);
    color: #fff;
  }
  .conn-check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--t2);
    font-family: var(--ui);
    cursor: default;
  }
  .conn-check input {
    accent-color: var(--acc);
  }
  /* SSH tunnel section — visually distinct group, indented body when expanded. */
  .ssh-section {
    border: 1px solid var(--b1);
    border-radius: 8px;
    background: color-mix(in srgb, var(--e) 60%, transparent);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .ssh-section.expanded {
    border-color: var(--b2);
  }
  .ssh-section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .ssh-section-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--t2);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .ssh-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--t2);
    font-family: var(--ui);
    cursor: default;
  }
  .ssh-toggle input {
    accent-color: var(--acc);
  }
  .ssh-section-body {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-left: 2px solid var(--b1);
    padding: 4px 0 4px 10px;
  }
  .ssh-caption {
    font-size: 11px;
    color: var(--t3);
    font-family: var(--mono);
    margin-top: 2px;
  }
  .ssh-empty {
    margin: 0;
    font-size: 12px;
    color: var(--t3);
    font-family: var(--ui);
  }
  .ssh-new-btn {
    align-self: flex-start;
    background: transparent;
    border: 1px dashed var(--b1);
    border-radius: 6px;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    padding: 6px 12px;
    cursor: default;
    transition: border-color 0.12s, color 0.12s;
  }
  .ssh-new-btn:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .ssh-new-btn.primary {
    border-style: solid;
    border-color: var(--acc);
    color: var(--acc);
  }
  .conn-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 6px;
    border-top: 1px solid var(--b1);
  }
  .conn-spacer {
    flex: 1;
  }
  .conn-test-btn {
    height: 34px;
    padding: 0 20px;
    border-radius: 8px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: border-color 0.12s, color 0.12s;
    /* min-width keeps the Test button stable when its label cycles
       through "Testing tunnel…" / "Testing database…" — otherwise the
       button reflows and visually shifts the adjacent Cancel/Save. */
    min-width: 160px;
    text-align: center;
  }
  .conn-test-btn:hover:not(:disabled) {
    border-color: var(--b2);
    color: var(--t1);
  }
  .conn-test-btn:disabled {
    opacity: 0.5;
  }
  .conn-cancel-btn {
    height: 34px;
    padding: 0 20px;
    border-radius: 8px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: border-color 0.12s, color 0.12s;
  }
  .conn-cancel-btn:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .conn-save-btn {
    height: 34px;
    padding: 0 20px;
    border-radius: 8px;
    border: none;
    background: var(--acc);
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    font-family: var(--ui);
    cursor: default;
    transition: opacity 0.12s;
  }
  .conn-save-btn:hover:not(:disabled) {
    opacity: 0.85;
  }
  .conn-save-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>

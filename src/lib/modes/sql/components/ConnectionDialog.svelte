<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import type { SqlConnectionConfig, SqlDriver, SqlConnection } from '../types';
  import { SQL_DIALECTS, defaultPortFor, descriptorFor } from '../dialects';
  import { sqlTestConnection } from '../commands';
  import { showToast } from '$lib/shared/primitives/toast';
  import { sshProfiles, loadSshProfiles } from '$lib/modes/ssh/stores';
  import type { SshProfile } from '$lib/modes/ssh/types';
  import NewSshProfileModal from '$lib/modes/ssh/components/NewSshProfileModal.svelte';

  interface Props {
    show: boolean;
    editConnection?: SqlConnection | null;
    onsave?: (config: SqlConnectionConfig) => void;
    onclose?: () => void;
  }

  let { show = $bindable(false), editConnection = null, onsave, onclose }: Props = $props();

  // Default new-connection driver — first registered dialect.
  const DEFAULT_DRIVER: SqlDriver = SQL_DIALECTS[0].key;
  const DEFAULT_PORT = SQL_DIALECTS[0].defaultPort;

  let name = $state('');
  let driver = $state<SqlDriver>(DEFAULT_DRIVER);
  let host = $state('localhost');
  let port = $state(DEFAULT_PORT);
  let database = $state('');
  let username = $state('');
  let password = $state('');
  let ssl = $state(false);
  let testing = $state(false);
  let testStatus = $state(''); // two-step status text during a tunneled test
  // Generation counter — bumped on every fresh test AND on dialog close.
  // Each await checks `myGen !== testGen` to bail silently when stale.
  // In-flight backend test will complete; we just discard the result.
  let testGen = $state(0);

  // SSH tunnel section state ─────────────────────────────────────────────
  let useSshTunnel = $state(false);
  let selectedSshProfileId = $state<string | null>(null);
  let showNewSshModal = $state(false);

  $effect(() => {
    if (show && editConnection) {
      name = editConnection.name;
      driver = editConnection.driver;
      host = editConnection.host;
      port = editConnection.port;
      database = editConnection.databaseName;
      username = editConnection.username;
      password = editConnection.password;
      ssl = !!editConnection.ssl;
      // SSH tunnel: round-trip the saved profile id.
      const sid = editConnection.sshProfileId ?? null;
      useSshTunnel = !!sid;
      selectedSshProfileId = sid;
    } else if (show && !editConnection) {
      name = '';
      driver = DEFAULT_DRIVER;
      host = 'localhost';
      port = DEFAULT_PORT;
      database = '';
      username = '';
      password = '';
      ssl = false;
      useSshTunnel = false;
      selectedSshProfileId = null;
    }
  });

  // Load SSH profiles on dialog open if the store is empty — keeps the
  // picker populated without forcing the user to visit SSH mode first.
  $effect(() => {
    if (show && $sshProfiles.length === 0) {
      loadSshProfiles();
    }
  });

  // Default selection: when toggling on with no prior selection, pick the
  // first available profile so the picker isn't empty.
  $effect(() => {
    if (useSshTunnel && !selectedSshProfileId && $sshProfiles.length > 0) {
      selectedSshProfileId = $sshProfiles[0].id;
    }
  });

  function handleDriverChange(e: Event) {
    const newDriver = (e.target as HTMLSelectElement).value as SqlDriver;
    driver = newDriver;
    port = defaultPortFor(newDriver);
  }

  const usesHostPort = $derived(descriptorFor(driver)?.usesHostPort ?? false);
  const selectedProfile = $derived(
    selectedSshProfileId ? $sshProfiles.find((p) => p.id === selectedSshProfileId) ?? null : null
  );

  function handleNewSshCreated(profile: SshProfile) {
    // Auto-select the freshly created profile so the user doesn't have to
    // hunt for it in the dropdown.
    selectedSshProfileId = profile.id;
    useSshTunnel = true;
  }

  function buildConfig(): SqlConnectionConfig {
    return {
      name: name.trim(),
      driver,
      host,
      port,
      database,
      username,
      password,
      ssl,
      sshProfileId: useSshTunnel && selectedSshProfileId ? selectedSshProfileId : null,
    };
  }

  async function handleTest() {
    const myGen = ++testGen;
    testing = true;
    testStatus = '';
    try {
      // Two-step flow when tunneling: prove the bastion works first so the
      // user sees a clear "tunnel failed" vs "DB failed" error.
      if (useSshTunnel && selectedSshProfileId && usesHostPort) {
        testStatus = 'Testing tunnel…';
        try {
          await invoke('ssh_tunnel_test', {
            profileId: selectedSshProfileId,
            targetHost: host,
            targetPort: port,
          });
        } catch (e: any) {
          if (myGen !== testGen) return; // stale — dialog closed or new test started
          showToast(`Tunnel test failed: ${e?.toString?.() ?? e}`, 'error');
          return;
        }
        if (myGen !== testGen) return;
        testStatus = 'Testing database…';
      }
      const result = await sqlTestConnection(buildConfig());
      if (myGen !== testGen) return;
      showToast(result || 'Connection successful', 'success');
    } catch (err: any) {
      if (myGen !== testGen) return;
      showToast(err.toString(), 'error');
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

  function handleSave() {
    if (!name.trim()) {
      showToast('Connection name is required', 'error');
      return;
    }
    if (useSshTunnel && !selectedSshProfileId) {
      showToast('Pick an SSH profile or turn off the tunnel', 'error');
      return;
    }
    onsave?.(buildConfig());
    show = false;
  }
</script>

<Modal bind:show title={editConnection ? 'Edit Connection' : 'New Connection'} width="460px" {onclose}>
  <div class="conn-form">
    <label class="conn-field">
      <span class="conn-label">Name</span>
      <input class="conn-input" type="text" bind:value={name} placeholder="My Database" />
    </label>

    <label class="conn-field">
      <span class="conn-label">Driver</span>
      <select class="conn-select" value={driver} onchange={handleDriverChange}>
        {#each SQL_DIALECTS as d (d.key)}
          <option value={d.key}>{d.displayName}</option>
        {/each}
      </select>
    </label>

    {#if usesHostPort}
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
    {/if}

    <label class="conn-field">
      <span class="conn-label">{usesHostPort ? 'Database' : 'File Path'}</span>
      <input class="conn-input" type="text" bind:value={database} placeholder={usesHostPort ? 'mydb' : '/path/to/db.sqlite'} />
    </label>

    {#if usesHostPort}
      <div class="conn-row">
        <label class="conn-field" style="flex:1">
          <span class="conn-label">Username</span>
          <input class="conn-input" type="text" bind:value={username} placeholder="user" />
        </label>
        <label class="conn-field" style="flex:1">
          <span class="conn-label">Password</span>
          <input class="conn-input" type="password" bind:value={password} placeholder="password" />
        </label>
      </div>

      <label class="conn-check">
        <input type="checkbox" bind:checked={ssl} />
        <span>Use SSL</span>
      </label>

      <!-- SSH Tunnel section — hidden for SQLite (no host/port). -->
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
    {/if}

    <div class="conn-actions">
      <button class="conn-btn outline" onclick={handleTest} disabled={testing}>
        {testing ? (testStatus || 'Testing…') : 'Test Connection'}
      </button>
      <div style="flex:1"></div>
      <button class="conn-btn outline" onclick={() => show = false}>Cancel</button>
      <button class="conn-btn primary" onclick={handleSave}>Save</button>
    </div>
  </div>
</Modal>

<NewSshProfileModal bind:show={showNewSshModal} onCreated={handleNewSshCreated} />

<style>
  .conn-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .conn-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .conn-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--t2);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .conn-input, .conn-select {
    height: 32px;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 0 10px;
    font-size: 12.5px;
    font-family: var(--mono);
    color: var(--t1);
    outline: none;
    transition: border-color 0.15s;
  }
  .conn-input:focus, .conn-select:focus {
    border-color: var(--acc);
  }
  .conn-input::placeholder {
    color: var(--t3);
  }
  .conn-select {
    cursor: default;
    font-family: var(--ui);
  }
  .conn-row {
    display: flex;
    gap: 10px;
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
    padding-left: 4px;
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
    padding-top: 8px;
    border-top: 1px solid var(--b1);
  }
  .conn-btn {
    height: 34px;
    padding: 0 20px;
    border-radius: 8px;
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: opacity 0.12s, border-color 0.12s, color 0.12s;
    /* min-width keeps the Test button stable when its label cycles
       through "Testing tunnel…" / "Testing database…" — otherwise the
       button reflows and visually shifts the adjacent Cancel/Save. */
    min-width: 160px;
    text-align: center;
  }
  .conn-btn.outline {
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
  }
  .conn-btn.outline:hover:not(:disabled) {
    border-color: var(--b2);
    color: var(--t1);
  }
  .conn-btn.outline:disabled {
    opacity: 0.5;
  }
  .conn-btn.primary {
    border: none;
    background: var(--acc);
    color: #fff;
    font-weight: 600;
  }
  .conn-btn.primary:hover {
    opacity: 0.85;
  }
</style>

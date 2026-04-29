<script lang="ts">
  import Modal from '$lib/components/shared/Modal.svelte';
  import { showToast } from '$lib/components/shared/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { nosqlTestConnection } from '../commands';
  import type { NoSqlConnection, NoSqlConnectionConfig } from '../types';

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
    }
  });

  $effect(() => {
    if (driver === 'redis' && port === 27017) {
      port = 6379;
    } else if (driver === 'mongodb' && port === 6379) {
      port = 27017;
    }
  });

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
    };
  }

  async function handleTest() {
    testing = true;
    try {
      const msg = await nosqlTestConnection(buildConfig());
      showToast(msg || 'Connection successful', 'success');
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      testing = false;
    }
  }

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

    <div class="conn-actions">
      <button class="conn-test-btn" onclick={handleTest} disabled={testing || saving}>
        {testing ? 'Testing...' : 'Test Connection'}
      </button>
      <div class="conn-spacer"></div>
      <button class="conn-cancel-btn" onclick={() => show = false} disabled={saving}>Cancel</button>
      <button class="conn-save-btn" onclick={handleSave} disabled={saving || testing}>
        {saving ? 'Saving...' : 'Save'}
      </button>
    </div>
  </div>
</Modal>

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

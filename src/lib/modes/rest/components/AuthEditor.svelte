<script lang="ts">
  import EnvInput from '$lib/components/shared/EnvInput.svelte';

  type AuthData = {
    token?: string;
    username?: string;
    password?: string;
    key_name?: string;
    key_value?: string;
    add_to?: 'header' | 'query';
  };

  let { authType = 'none', authData = '{}', onchange }: {
    authType: string;
    authData: string;
    onchange: (authType: string, authData: string) => void;
  } = $props();

  let localType = $state(authType);
  let data: AuthData = $state({});
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    localType = authType;
    try {
      data = JSON.parse(authData || '{}');
    } catch {
      data = {};
    }
  });

  function debounceSave() {
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      onchange(localType, JSON.stringify(data));
    }, 300);
  }

  function handleTypeChange(e: Event) {
    const target = e.target as HTMLSelectElement;
    localType = target.value;
    if (localType === 'bearer') {
      data = { token: '' };
    } else if (localType === 'basic') {
      data = { username: '', password: '' };
    } else if (localType === 'apikey') {
      data = { key_name: '', key_value: '', add_to: 'header' };
    } else {
      data = {};
    }
    onchange(localType, JSON.stringify(data));
  }

  function updateField(field: string, value: string) {
    (data as Record<string, string>)[field] = value;
    debounceSave();
  }
</script>

<div class="auth-panel">
  <select value={localType} onchange={handleTypeChange}>
    <option value="none">No Auth</option>
    <option value="bearer">Bearer Token</option>
    <option value="basic">Basic Auth</option>
    <option value="apikey">API Key</option>
  </select>

  {#if localType === 'none'}
    <span class="auth-msg">No authentication configured for this request.</span>
  {:else if localType === 'bearer'}
    <label>Token</label>
    <EnvInput
      value={data.token ?? ''}
      placeholder="Enter token or &#123;&#123;variable&#125;&#125;"
      onchange={(v) => updateField('token', v)}
    />
  {:else if localType === 'basic'}
    <label>Username</label>
    <EnvInput
      value={data.username ?? ''}
      placeholder="Username or &#123;&#123;variable&#125;&#125;"
      onchange={(v) => updateField('username', v)}
    />
    <label>Password</label>
    <EnvInput
      value={data.password ?? ''}
      placeholder="Password or &#123;&#123;variable&#125;&#125;"
      type="password"
      onchange={(v) => updateField('password', v)}
    />
  {:else if localType === 'apikey'}
    <label>Key name</label>
    <EnvInput
      value={data.key_name ?? ''}
      placeholder="X-API-Key"
      onchange={(v) => updateField('key_name', v)}
    />
    <label>Key value</label>
    <EnvInput
      value={data.key_value ?? ''}
      placeholder="Enter value or &#123;&#123;variable&#125;&#125;"
      onchange={(v) => updateField('key_value', v)}
    />
    <label>Add to</label>
    <select
      value={data.add_to ?? 'header'}
      onchange={(e) => updateField('add_to', (e.target as HTMLSelectElement).value)}
    >
      <option value="header">Header</option>
      <option value="query">Query Param</option>
    </select>
  {/if}
</div>

<style>
  .auth-panel {
    padding: 12px 14px;
    flex: 1;
    overflow-y: auto;
    background: transparent;
  }
  .auth-panel select {
    background: var(--n2);
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 5px 10px;
    color: var(--t1);
    font-family: var(--mono);
    font-size: 12.5px;
    outline: none;
    cursor: pointer;
    margin-bottom: 12px;
  }
  .auth-panel label {
    display: block;
    font-size: 11px;
    font-weight: 500;
    color: var(--t3);
    margin-bottom: 4px;
    margin-top: 8px;
    font-family: var(--mono);
  }
  .auth-panel label:first-of-type {
    margin-top: 0;
  }
  .auth-msg {
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    font-style: italic;
  }
</style>

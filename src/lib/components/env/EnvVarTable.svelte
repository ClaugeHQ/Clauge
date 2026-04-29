<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    loadEnvVariables,
    setEnvVariable,
    updateEnvVariable,
    deleteEnvVariable
  } from '$lib/modes/rest/stores';
  import type { EnvVariable } from '$lib/types';
  import { showToast } from '$lib/components/shared/toast';

  interface Props {
    environmentId: string;
  }

  let { environmentId }: Props = $props();

  let variables = $state<EnvVariable[]>([]);
  let revealedIds = $state<Set<string>>(new Set());
  let dirty = $state(false);
  let saving = $state(false);
  let loadVersion = 0;

  async function load(envId: string) {
    const version = ++loadVersion;
    try {
      const result = await loadEnvVariables(envId);
      if (version === loadVersion) {
        variables = result;
        dirty = false;
      }
    } catch {
      if (version === loadVersion) {
        variables = [];
      }
    }
  }

  $effect(() => {
    if (environmentId) {
      load(environmentId);
      revealedIds = new Set();
    }
  });

  function toggleReveal(id: string) {
    const next = new Set(revealedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    revealedIds = next;
  }

  function handleKeyChange(idx: number, val: string) {
    variables[idx].key = val;
    dirty = true;
  }

  function handleValueChange(idx: number, val: string) {
    variables[idx].value = val;
    dirty = true;
  }

  function handleSecretToggle(idx: number) {
    variables[idx].isSecret = variables[idx].isSecret ? 0 : 1;
    dirty = true;
  }

  function handleDelete(idx: number) {
    const v = variables[idx];
    if (v.id.startsWith('temp_')) {
      variables = variables.filter((_, i) => i !== idx);
    } else {
      deleteEnvVariable(v.id)
        .then(() => { variables = variables.filter((_, i) => i !== idx); })
        .catch(() => showToast('Failed to delete variable', 'error'));
    }
  }

  function addVariable() {
    const tempId = `temp_${Date.now()}`;
    variables = [...variables, {
      id: tempId,
      environmentId,
      key: '',
      value: '',
      isSecret: 0,
      sortOrder: variables.length,
    } as EnvVariable];
    dirty = true;
  }

  async function saveAll() {
    saving = true;
    let errorCount = 0;
    for (let i = 0; i < variables.length; i++) {
      const v = variables[i];
      if (!v.key.trim()) continue; // skip empty keys
      try {
        if (v.id.startsWith('temp_')) {
          const saved = await setEnvVariable(environmentId, v.key, v.value, v.isSecret);
          variables[i] = saved;
        } else {
          await updateEnvVariable(v.id, v.key, v.value, v.isSecret);
        }
      } catch {
        errorCount++;
      }
    }
    // Remove rows with empty keys (user left them blank)
    variables = variables.filter(v => v.key.trim() !== '');
    saving = false;
    if (errorCount > 0) {
      dirty = true; // Keep dirty so user can retry
      showToast(`${errorCount} variable(s) failed to save`, 'error');
    } else {
      dirty = false;
      showToast('Variables saved', 'success');
    }
  }
</script>

<div class="env-var-table">
  <div class="var-header">
    <span class="var-col-key">Key</span>
    <span class="var-col-val">Value</span>
    <span class="var-col-act"></span>
  </div>

  {#each variables as v, idx (idx)}
    <div class="var-row">
      <input
        class="var-input var-key"
        type="text"
        placeholder="KEY"
        bind:value={v.key}
        oninput={() => { dirty = true; }}
      />
      <div class="var-value-wrap">
        {#if v.isSecret && !revealedIds.has(v.id)}
          <input
            class="var-input var-val"
            type="password"
            bind:value={v.value}
            oninput={() => { dirty = true; }}
          />
        {:else}
          <input
            class="var-input var-val"
            type="text"
            placeholder="value"
            bind:value={v.value}
            oninput={() => { dirty = true; }}
          />
        {/if}
        <button
          class="var-eye"
          class:active={v.isSecret === 1}
          title={v.isSecret ? 'Secret (click to toggle)' : 'Not secret (click to toggle)'}
          onclick={() => handleSecretToggle(idx)}
        >
          {#if v.isSecret}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94"/>
              <path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19"/>
              <line x1="1" y1="1" x2="23" y2="23"/>
            </svg>
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/>
              <circle cx="12" cy="12" r="3"/>
            </svg>
          {/if}
        </button>
        {#if v.isSecret}
          <button
            class="var-reveal"
            title={revealedIds.has(v.id) ? 'Hide' : 'Reveal'}
            onclick={() => toggleReveal(v.id)}
          >
            {revealedIds.has(v.id) ? 'hide' : 'show'}
          </button>
        {/if}
      </div>
      <button class="var-del" title="Delete variable" onclick={() => handleDelete(idx)}>&times;</button>
    </div>
  {/each}

  <div class="var-actions">
    <button class="var-add" onclick={addVariable}>+ Add Variable</button>
    {#if dirty}
      <button class="var-save" onclick={saveAll} disabled={saving}>
        {saving ? 'Saving...' : 'Save'}
      </button>
    {/if}
  </div>
</div>

<style>
  .env-var-table {
    margin-top: 12px;
  }

  .var-header {
    display: flex;
    gap: 6px;
    padding: 0 0 6px 0;
    font-size: 11px;
    font-weight: 600;
    color: var(--t3);
    font-family: var(--mono);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .var-col-key { width: 140px; flex-shrink: 0; }
  .var-col-val { flex: 1; }
  .var-col-act { width: 26px; }

  .var-row {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 6px 0;
    margin-bottom: 0;
  }

  .var-input {
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 4px;
    padding: 5px 8px;
    font-family: var(--mono);
    font-size: 12.5px;
    color: var(--t1);
    outline: none;
    transition: border-color 0.15s;
  }

  .var-input:focus {
    border-color: var(--b2);
  }

  .var-input::placeholder {
    color: var(--t3);
  }

  .var-key {
    width: 140px;
    flex-shrink: 0;
  }

  .var-value-wrap {
    flex: 1;
    display: flex;
    gap: 4px;
    align-items: center;
  }

  .var-val {
    flex: 1;
    min-width: 0;
  }

  .var-eye {
    width: 26px;
    height: 26px;
    border: none;
    background: transparent;
    color: var(--t3);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    transition: color 0.12s;
    flex-shrink: 0;
  }

  .var-eye:hover { color: var(--t1); }
  .var-eye.active { color: var(--warn); }

  .var-reveal {
    border: none;
    background: transparent;
    color: var(--t3);
    cursor: pointer;
    font-size: 10px;
    font-family: var(--mono);
    padding: 2px 4px;
    flex-shrink: 0;
  }

  .var-reveal:hover { color: var(--t1); }

  .var-del {
    width: 22px;
    height: 22px;
    border: none;
    background: transparent;
    color: var(--t3);
    cursor: pointer;
    font-size: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 3px;
    transition: color 0.1s;
    flex-shrink: 0;
  }

  .var-del:hover { color: var(--err); }

  .var-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
  }

  .var-add {
    padding: 4px 12px;
    border-radius: 4px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t3);
    font-size: 11px;
    font-family: var(--mono);
    cursor: pointer;
    transition: border-color 0.1s, color 0.1s;
  }

  .var-add:hover {
    border-color: var(--b2);
    color: var(--t1);
  }

  .var-save {
    padding: 4px 14px;
    border-radius: 4px;
    border: 1px solid color-mix(in srgb, var(--acc) 40%, transparent);
    background: transparent;
    color: var(--acc);
    font-size: 11px;
    font-family: var(--mono);
    font-weight: 600;
    cursor: pointer;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }

  .var-save:hover {
    background: var(--acc);
    color: #fff;
    border-color: var(--acc);
  }

  .var-save:disabled {
    opacity: 0.5;
    pointer-events: none;
  }
</style>

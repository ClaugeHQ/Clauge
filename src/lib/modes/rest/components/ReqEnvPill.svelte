<script lang="ts">
  import { activeRequest, requestEnvOverrides, setRequestEnv } from '$lib/modes/rest/stores';
  import { activeTabId } from '$lib/shared/stores/tabs';
  import { environments, activeEnvId, getEffectiveEnvId } from '$lib/modes/rest/stores';

  let dropdownOpen = $state(false);

  // Use request ID for saved requests, tab ID for unsaved
  const overrideKey = $derived($activeRequest?.id ?? String($activeTabId));

  const effectiveEnvId = $derived(
    getEffectiveEnvId(overrideKey, $requestEnvOverrides, $activeEnvId)
  );

  const effectiveEnv = $derived(
    $environments.find(e => e.id === effectiveEnvId) ?? null
  );

  const isUsingGlobal = $derived(
    !$requestEnvOverrides[overrideKey]
  );

  const globalEnv = $derived(
    $environments.find(e => e.id === $activeEnvId) ?? null
  );

  let pillEl = $state<HTMLButtonElement | null>(null);
  let ddStyle = $state('');

  function toggleDropdown() {
    if (!dropdownOpen && pillEl) {
      const rect = pillEl.getBoundingClientRect();
      const spaceBelow = window.innerHeight - rect.bottom;
      const estimatedHeight = Math.min(($environments.length + 2) * 34 + 16, 300);
      if (spaceBelow < estimatedHeight) {
        // Open upward
        ddStyle = `bottom: ${window.innerHeight - rect.top + 4}px; top: auto; right: ${window.innerWidth - rect.right}px; position: fixed;`;
      } else {
        // Open downward
        ddStyle = `top: ${rect.bottom + 4}px; right: ${window.innerWidth - rect.right}px; position: fixed;`;
      }
    }
    dropdownOpen = !dropdownOpen;
  }

  function selectEnv(envId: string | null) {
    setRequestEnv(overrideKey, envId);
    dropdownOpen = false;
  }

  function handleClickOutside(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (!target.closest('.req-env-wrap')) {
      dropdownOpen = false;
    }
  }
</script>

<svelte:window onclick={handleClickOutside} />

<div class="req-env-wrap">
  <button bind:this={pillEl} class="req-env-pill" onclick={toggleDropdown} title="Environment for this request">
    {#if isUsingGlobal}
      <span class="req-env-global">Global &middot;</span>
    {/if}
    <span class="req-env-dot" style:background={effectiveEnv?.color ?? 'var(--t3)'}></span>
    <span class="req-env-label">{effectiveEnv?.name ?? 'None'}</span>
  </button>

  {#if dropdownOpen}
    <div class="coll-env-dd" style={ddStyle}>
      <!-- Use Global option -->
      <button
        class="coll-env-dd-item"
        class:active={isUsingGlobal}
        onclick={() => selectEnv(null)}
      >
        <span class="coll-env-dd-dot" style:background={globalEnv?.color ?? 'var(--t3)'}></span>
        <span>Use Global ({globalEnv?.name ?? 'None'})</span>
        {#if isUsingGlobal}
          <span class="coll-env-dd-check">&#10003;</span>
        {/if}
      </button>
      <div class="coll-env-dd-sep"></div>

      <!-- All environments -->
      {#each $environments as env (env.id)}
        <button
          class="coll-env-dd-item"
          class:active={!isUsingGlobal && effectiveEnvId === env.id}
          onclick={() => selectEnv(env.id)}
        >
          <span class="coll-env-dd-dot" style:background={env.color}></span>
          <span>{env.name}</span>
          {#if !isUsingGlobal && effectiveEnvId === env.id}
            <span class="coll-env-dd-check">&#10003;</span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .req-env-wrap {
    position: relative;
    flex-shrink: 0;
  }
  .req-env-pill {
    height: 32px;
    padding: 0 10px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: var(--e);
    color: var(--t2);
    font-size: 10.5px;
    font-family: var(--mono);
    display: flex;
    align-items: center;
    gap: 5px;
    cursor: pointer;
    user-select: none;
    transition: border-color 0.12s, color 0.12s;
    flex-shrink: 0;
  }
  .req-env-pill:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .req-env-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
  }
  .req-env-global {
    color: var(--t3);
    font-size: 9px;
    margin-right: 2px;
  }
  .req-env-label {
    white-space: nowrap;
  }
  .coll-env-dd {
    background: var(--modal-bg, #101016);
    max-height: 300px;
    overflow-y: auto;
    border: 1px solid var(--b1);
    border-radius: 6px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.5);
    z-index: 9999;
    min-width: 180px;
    padding: 3px;
  }
  .coll-env-dd-item {
    width: 100%;
    padding: 6px 10px;
    border-radius: 5px;
    border: none;
    background: transparent;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    font-family: var(--mono);
    color: var(--t2);
    transition: background 0.08s;
    text-align: left;
  }
  .coll-env-dd-item:hover {
    background: var(--c);
    color: var(--t1);
  }
  .coll-env-dd-item.active {
    color: var(--t1);
    font-weight: 600;
  }
  .coll-env-dd-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .coll-env-dd-check {
    margin-left: auto;
    font-size: 12px;
    color: var(--ok);
  }
  .coll-env-dd-sep {
    height: 1px;
    background: var(--b1);
    margin: 3px 6px;
  }
</style>

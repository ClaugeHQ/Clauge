<script lang="ts">
  import { environments, activeEnvId, setActiveEnv } from '$lib/modes/rest/stores';
  import { activeModal } from '$lib/stores/app';
  import { derived } from 'svelte/store';
  import Dropdown from '$lib/components/shared/Dropdown.svelte';

  const activeEnv = derived(
    [environments, activeEnvId],
    ([$envs, $id]) => $envs.find(e => e.id === $id) ?? null
  );

  let envName = $derived($activeEnv?.name ?? 'No env');
  let envColor = $derived($activeEnv?.color ?? 'var(--t3)');

  let showDropdown = $state(false);
  let pillEl = $state<HTMLDivElement | null>(null);

  let dropdownItems = $derived(() => {
    const items: Array<{ label: string; icon?: string; action: () => void; separator?: boolean }> = [];
    for (const env of $environments) {
      items.push({
        label: env.name,
        icon: env.id === $activeEnvId ? '\u2713' : ' ',
        action: () => setActiveEnv(env.id),
      });
    }
    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: 'Manage environments\u2026',
      action: () => activeModal.set('environments'),
    });
    return items;
  });

  function toggleDropdown() {
    showDropdown = !showDropdown;
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="env-pill" title="Switch environment" bind:this={pillEl} onclick={toggleDropdown}>
  <span class="env-dot" style="background:{envColor}"></span>
  <span>{envName}</span>
  <svg class="env-chevron" width="8" height="8" viewBox="0 0 8 8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
    <path d="M2 3L4 5L6 3"/>
  </svg>
</div>

<Dropdown items={dropdownItems()} bind:show={showDropdown} anchor={pillEl} />

<style>
  .env-pill {
    padding: 4px 10px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: var(--e);
    font-size: 11px;
    color: var(--t2);
    font-family: var(--mono);
    cursor: pointer;
    user-select: none;
    transition: border-color 0.15s;
    display: flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
  }
  .env-pill:hover {
    border-color: var(--b2);
  }
  .env-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .env-chevron {
    margin-left: 2px;
    opacity: 0.5;
  }
</style>

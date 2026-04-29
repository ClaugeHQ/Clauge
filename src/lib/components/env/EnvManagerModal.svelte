<script lang="ts">
  import { tick, onDestroy } from 'svelte';
  import Modal from '$lib/components/shared/Modal.svelte';
  import ConfirmDialog from '$lib/components/shared/ConfirmDialog.svelte';
  import EnvSidebar from './EnvSidebar.svelte';
  import EnvVarTable from './EnvVarTable.svelte';
  import {
    environments,
    activeEnvId,
    createEnvironment,
    updateEnvironment,
    deleteEnvironment,
    setDefaultEnv
  } from '$lib/modes/rest/stores';
  import { activeModal } from '$lib/stores/app';
  import { showToast } from '$lib/components/shared/toast';
  import { ENV_SAVE_DEBOUNCE_MS } from '$lib/shared/constants/timings';

  let show = $state(false);

  $effect(() => {
    show = $activeModal === 'environments';
  });

  $effect(() => {
    if (!show && $activeModal === 'environments') {
      activeModal.set(null);
    }
  });

  let selectedId = $state<string | null>(null);
  let nameInput = $state<HTMLInputElement | null>(null);
  let showDeleteConfirm = $state(false);

  const PRESET_COLORS = [
    '#4ade80', // green
    '#facc15', // yellow
    '#f87171', // red
    '#60a5fa', // blue
    '#a78bfa', // purple
    '#fb923c', // orange
    '#2dd4bf', // teal
    '#f472b6', // pink
  ];

  let envList = $derived($environments);
  let selectedEnv = $derived(envList.find(e => e.id === selectedId) ?? null);
  let isDefault = $derived(selectedEnv?.isDefault === 1);
  let nameValue = $state('');
  let colorValue = $state('');

  // Sync name/color when selection changes
  $effect(() => {
    if (selectedEnv) {
      nameValue = selectedEnv.name;
      colorValue = selectedEnv.color;
    }
  });

  // Auto-select the currently active env when modal opens
  $effect(() => {
    if (show && envList.length > 0) {
      const currentActive = $activeEnvId;
      const exists = currentActive && envList.some(e => e.id === currentActive);
      if (exists) {
        selectedId = currentActive;
      } else {
        selectedId = envList[0].id;
      }
    }
    if (!show) {
      selectedId = null;
    }
  });

  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  function debounceSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      if (selectedId && nameValue.trim()) {
        try {
          await updateEnvironment(selectedId, nameValue.trim(), colorValue);
        } catch (err) {
          showToast('Failed to update environment', 'error');
        }
      }
    }, ENV_SAVE_DEBOUNCE_MS);
  }

  function handleNameChange(val: string) {
    nameValue = val;
    debounceSave();
  }

  function handleColorSelect(color: string) {
    colorValue = color;
    debounceSave();
  }

  async function handleSetDefault() {
    if (!selectedId) return;
    try {
      await setDefaultEnv(selectedId);
      showToast('Default environment updated', 'success');
    } catch (err) {
      showToast('Failed to set default', 'error');
    }
  }

  async function handleCreate() {
    try {
      const env = await createEnvironment('New Environment', PRESET_COLORS[Math.floor(Math.random() * PRESET_COLORS.length)]);
      selectedId = env.id;
      await tick();
      nameInput?.focus();
      nameInput?.select();
    } catch (err) {
      showToast('Failed to create environment', 'error');
    }
  }

  function handleSelect(id: string) {
    selectedId = id;
  }

  function confirmDelete() {
    showDeleteConfirm = true;
  }

  async function handleDelete() {
    if (!selectedId) return;
    const delId = selectedId;
    try {
      await deleteEnvironment(delId);
      // After delete, select next available env or null
      const remaining = envList.filter(e => e.id !== delId);
      selectedId = remaining.length > 0 ? remaining[0].id : null;
      showToast('Environment deleted', 'success');
    } catch (err) {
      showToast('Failed to delete environment', 'error');
    }
  }

  function handleClose() {
    if (saveTimer) clearTimeout(saveTimer);
    selectedId = null;
  }

  onDestroy(() => {
    if (saveTimer) clearTimeout(saveTimer);
  });
</script>

<Modal bind:show title="Environments" width="680px" onclose={handleClose}>
  <div class="env-manager">
    <EnvSidebar
      environments={envList}
      {selectedId}
      onselect={handleSelect}
      oncreate={handleCreate}
    />

    <div class="env-detail">
      {#if selectedEnv}
        <div class="env-detail-section">
          <label class="env-label">Name</label>
          <input
            bind:this={nameInput}
            class="env-name-input"
            type="text"
            value={nameValue}
            oninput={(e) => handleNameChange(e.currentTarget.value)}
          />
        </div>

        <div class="env-detail-section">
          <label class="env-label">Color</label>
          <div class="color-swatches">
            {#each PRESET_COLORS as color}
              <button
                class="color-swatch"
                class:active={colorValue === color}
                style="background: {color}"
                title={color}
                onclick={() => handleColorSelect(color)}
              ></button>
            {/each}
          </div>
        </div>

        {#if isDefault}
          <div class="env-detail-section">
            <span class="env-is-default">Default environment</span>
          </div>
        {:else if envList.length > 1}
          <div class="env-detail-section">
            <button class="env-set-default" onclick={handleSetDefault}>
              Set as Default
            </button>
          </div>
        {/if}

        <div class="env-detail-section env-vars-section">
          <label class="env-label">Variables</label>
          <EnvVarTable environmentId={selectedEnv.id} />
        </div>

        <div class="env-detail-footer">
          <button class="env-delete-btn" onclick={confirmDelete}>
            Delete Environment
          </button>
        </div>
      {:else}
        <div class="env-empty">
          <span>Select or create an environment</span>
        </div>
      {/if}
    </div>
  </div>
</Modal>

<ConfirmDialog
  bind:show={showDeleteConfirm}
  title="Delete Environment"
  message="Are you sure you want to delete this environment? All variables will be lost."
  confirmText="Delete"
  onconfirm={handleDelete}
/>

<style>
  .env-manager {
    display: flex;
    margin: -18px;
    min-height: 420px;
  }

  .env-detail {
    flex: 1;
    padding: 16px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .env-detail-section {
    margin-bottom: 16px;
  }

  .env-label {
    display: block;
    font-size: 11px;
    font-weight: 500;
    color: var(--t3);
    font-family: var(--mono);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 6px;
  }

  .env-name-input {
    width: 100%;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 7px 10px;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 13px;
    outline: none;
    transition: border-color 0.15s;
    box-sizing: border-box;
  }

  .env-name-input:focus {
    border-color: var(--acc);
  }

  .color-swatches {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .color-swatch {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    transition: transform 0.1s, border-color 0.1s;
    outline: none;
  }

  .color-swatch:hover {
    transform: scale(1.15);
  }

  .color-swatch.active {
    border-color: var(--t1);
    box-shadow: 0 0 0 2px var(--n);
  }

  .env-set-default {
    padding: 5px 12px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11px;
    font-family: var(--ui);
    cursor: pointer;
    transition: border-color 0.1s, color 0.1s;
  }

  .env-set-default:hover {
    border-color: var(--b2);
    color: var(--t1);
  }

  .env-is-default {
    font-size: 11px;
    color: var(--t3);
    font-family: var(--ui);
    font-style: italic;
  }

  .env-vars-section {
    flex: 1;
  }

  .env-detail-footer {
    margin-top: auto;
    padding-top: 12px;
    border-top: 1px solid var(--b1);
  }

  .env-delete-btn {
    background: transparent;
    border: none;
    color: var(--err);
    font-size: 12px;
    font-family: var(--ui);
    cursor: pointer;
    padding: 4px 0;
    transition: opacity 0.1s;
  }

  .env-delete-btn:hover {
    opacity: 0.8;
    text-decoration: underline;
  }

  .env-empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t3);
    font-size: 13px;
    font-family: var(--ui);
  }
</style>

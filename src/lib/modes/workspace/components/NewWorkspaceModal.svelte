<script lang="ts">
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { createWorkspace, updateWorkspace } from '../stores';
  import { agentSessions } from '$lib/modes/agent/stores';
  import { get } from 'svelte/store';
  import type { Workspace } from '../types';

  interface Props {
    show: boolean;
    /** When set, modal runs in edit mode — pre-fills name + project,
     *  saves via workspaceUpdate instead of create. Used by the
     *  workspace ellipsis menu's "Change project" entry. */
    editing?: Workspace | null;
    onclose?: () => void;
  }

  let { show = $bindable(), editing = null, onclose }: Props = $props();

  let name = $state('');
  let projectPath = $state<string | null>(null);
  let loading = $state(false);

  const isEdit = $derived(!!editing);

  // Recent project paths from existing agent sessions — biggest user
  // win on this modal. Most workspaces will be tied to a project the
  // user has already touched in Agent mode.
  const knownProjects = $derived.by(() => {
    const seen = new Map<string, string>(); // path -> project_name
    for (const s of get(agentSessions) || []) {
      if (s.projectPath && !seen.has(s.projectPath)) {
        seen.set(s.projectPath, s.projectName || s.projectPath);
      }
    }
    return Array.from(seen.entries()).map(([path, name]) => ({ path, name }));
  });

  $effect(() => {
    if (show) {
      // Pre-fill from `editing` when in edit mode, else reset.
      if (editing) {
        name = editing.name;
        projectPath = editing.projectPath;
      } else {
        name = '';
        projectPath = null;
      }
      loading = false;
    }
  });

  async function pickFolder() {
    try {
      const selected = await openDialog({ directory: true, multiple: false });
      if (typeof selected === 'string') {
        projectPath = selected;
        // Auto-fill the name from the folder if name is empty.
        if (!name.trim()) {
          const parts = selected.split('/').filter(Boolean);
          name = parts[parts.length - 1] || '';
        }
      }
    } catch (e) {
      errorToast('Folder pick failed', e);
    }
  }

  async function submit() {
    const trimmed = name.trim();
    if (!trimmed) return;
    loading = true;
    try {
      if (editing) {
        await updateWorkspace({ id: editing.id, name: trimmed, projectPath });
        showToast(`Updated "${trimmed}"`, 'success');
      } else {
        await createWorkspace({ name: trimmed, projectPath });
        showToast(`Created "${trimmed}"`, 'success');
      }
      show = false;
      onclose?.();
    } catch (e) {
      errorToast('Failed', e);
    } finally {
      loading = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) submit();
  }
</script>

<Modal bind:show title={isEdit ? 'Edit Workspace' : 'New Workspace'} width="440px" {onclose}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="nw-form" onkeydown={handleKeydown}>
    <label class="nw-field">
      <span class="nw-label">Name</span>
      <input
        class="nw-input"
        type="text"
        placeholder="e.g. Project X"
        bind:value={name}
      />
    </label>

    <div class="nw-field">
      <span class="nw-label">Project</span>

      <div class="nw-path-row">
        <input
          class="nw-input nw-input-flex"
          type="text"
          placeholder="Path to a project directory (optional)"
          bind:value={projectPath}
        />
        <button class="nw-browse" onclick={pickFolder}>Browse</button>
        {#if projectPath}
          <button class="nw-clear" onclick={() => projectPath = null} title="Clear">×</button>
        {/if}
      </div>

      {#if knownProjects.length > 0}
        <div class="nw-recent">
          <span class="nw-recent-label">Recent projects</span>
          <div class="nw-chips">
            {#each knownProjects as p (p.path)}
              <button
                class="nw-chip"
                class:active={projectPath === p.path}
                onclick={() => projectPath = p.path}
                title={p.path}
              >
                <span class="nw-chip-dot"></span>
                {p.name}
              </button>
            {/each}
          </div>
        </div>
      {/if}
    </div>

    <div class="nw-actions">
      <button class="nw-btn-cancel" onclick={() => { show = false; onclose?.(); }}>Cancel</button>
      <button class="nw-btn-create" onclick={submit} disabled={!name.trim() || loading}>
        {loading ? (isEdit ? 'Saving…' : 'Creating…') : (isEdit ? 'Save' : 'Create')}
      </button>
    </div>
  </div>
</Modal>

<style>
  .nw-form {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 18px 20px;
  }
  .nw-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .nw-label {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--t2);
    font-family: var(--ui);
    letter-spacing: 0.02em;
  }
  .nw-chip-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--t3);
    flex-shrink: 0;
  }
  .nw-chip.active .nw-chip-dot { background: var(--acc); }
  .nw-input {
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md, 6px);
    padding: 7px 10px;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 12.5px;
    outline: none;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .nw-input:focus {
    border-color: var(--acc);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 12%, transparent);
  }

  .nw-recent {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 10px;
  }
  .nw-recent-label {
    font-family: var(--ui);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--t4);
  }
  .nw-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .nw-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    border-radius: 12px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11px;
    font-family: var(--ui);
    cursor: default;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    transition: border-color 0.12s, background 0.12s, color 0.12s;
  }
  .nw-chip:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .nw-chip.active {
    border-color: var(--acc);
    background: color-mix(in srgb, var(--acc) 15%, transparent);
    color: var(--t1);
  }

  .nw-path-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .nw-input-flex {
    flex: 1;
    min-width: 0;
  }
  .nw-browse {
    padding: 7px 12px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11.5px;
    font-family: var(--ui);
    cursor: default;
    transition: border-color 0.12s, color 0.12s;
  }
  .nw-browse:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .nw-clear {
    width: 26px;
    height: 26px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t3);
    font-size: 16px;
    line-height: 1;
    cursor: default;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .nw-clear:hover {
    border-color: var(--err);
    color: var(--err);
  }

  .nw-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
  .nw-btn-cancel {
    height: 30px;
    padding: 0 16px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: border-color 0.1s, color 0.1s;
  }
  .nw-btn-cancel:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .nw-btn-create {
    height: 30px;
    padding: 0 16px;
    border-radius: 6px;
    border: none;
    background: var(--acc);
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    font-family: var(--ui);
    cursor: default;
    transition: opacity 0.1s;
  }
  .nw-btn-create:hover:not(:disabled) {
    opacity: 0.9;
  }
  .nw-btn-create:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>

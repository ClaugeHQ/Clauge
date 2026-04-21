<script>
  import { invoke } from "@tauri-apps/api/core";
  import { contextsStore } from "$lib/stores/contexts.svelte";
  import "$lib/styles/app.css";

  let {
    profile = $bindable(),
    onSave,
    onClose,
  } = $props();

  let title = $state('');
  let skipPermissions = $state(false);
  let gitEnabled = $state(false);
  let gitName = $state('');
  let gitEmail = $state('');
  let contextPrompt = $state('');
  let saving = $state(false);

  $effect(() => {
    if (profile) {
      title = profile.title || '';
      skipPermissions = profile.skipPermissions || false;
      gitEnabled = !!(profile.gitName || profile.gitEmail);
      gitName = profile.gitName || '';
      gitEmail = profile.gitEmail || '';
      contextPrompt = profile.contextPrompt || '';
      contextsStore.loadContextSnippets();
    }
  });

  async function handleSave() {
    if (!profile || !title.trim()) return;
    saving = true;
    try {
      await invoke("update_profile", {
        id: profile.id,
        title: title.trim(),
        skipPermissions,
        gitName: gitEnabled ? gitName : null,
        gitEmail: gitEnabled ? gitEmail : null,
        contextPrompt: contextPrompt,
      });
      onSave?.();
      onClose?.();
    } catch (e) {
      console.error("Failed to update profile:", e);
    } finally {
      saving = false;
    }
  }

  function handleBackdropClick(e) {
    if (e.target === e.currentTarget) onClose?.();
  }

  function handleKeydown(e) {
    if (e.key === 'Escape') onClose?.();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if profile}
<!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
<div class="modal-backdrop" onclick={handleBackdropClick}>
  <div class="edit-modal">
    <div class="edit-header">
      <h3>Edit Session</h3>
      <button class="close-btn" onclick={onClose}>
        <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor"><path d="M3.72 3.72a.75.75 0 011.06 0L8 6.94l3.22-3.22a.75.75 0 111.06 1.06L9.06 8l3.22 3.22a.75.75 0 11-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 01-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 010-1.06z"/></svg>
      </button>
    </div>

    <div class="edit-body">
      <div class="field">
        <label for="edit-title">Title</label>
        <input id="edit-title" type="text" bind:value={title} placeholder="Session name" />
      </div>

      <div class="field">
        <label>Purpose</label>
        <div class="readonly-value">{profile.purpose}</div>
      </div>

      <div class="field">
        <label>Project</label>
        <div class="readonly-value">{profile.projectPath}</div>
      </div>

      {#if profile.purpose === 'Custom'}
        <div class="field">
          <label for="edit-prompt">System Prompt</label>
          <textarea id="edit-prompt" bind:value={contextPrompt} rows="4" placeholder="Custom system prompt (optional)"></textarea>
        </div>
      {/if}

      <div class="field">
        <label class="toggle-label">
          <span>Skip Permissions</span>
          <button class="toggle" class:on={skipPermissions} onclick={() => skipPermissions = !skipPermissions}>
            <span class="toggle-thumb"></span>
          </button>
        </label>
        <div class="field-hint">Uses --dangerously-skip-permissions flag. Takes effect on next session launch.</div>
      </div>

      <div class="field">
        <label class="toggle-label">
          <span>Git Identity</span>
          <button class="toggle" class:on={gitEnabled} onclick={() => gitEnabled = !gitEnabled}>
            <span class="toggle-thumb"></span>
          </button>
        </label>
        <div class="field-hint">Set a per-session git author name and email. Takes effect on next session launch.</div>
        {#if gitEnabled}
          <div class="git-fields">
            <input type="text" bind:value={gitName} placeholder="Git name" />
            <input type="text" bind:value={gitEmail} placeholder="Git email" />
          </div>
        {/if}
      </div>

      <div class="field">
        <label class="toggle-label">
          <span>Contexts</span>
          <button class="ctx-manage-btn" onclick={() => { contextsStore.showContextPicker = profile; }}>
            Manage
          </button>
        </label>
        {#if profile.contexts?.length > 0}
          <div class="ctx-tags">
            {#each profile.contexts as ctx}
              <span class="ctx-tag">{ctx}</span>
            {/each}
          </div>
        {:else}
          <div class="field-hint">No contexts attached.</div>
        {/if}
      </div>
    </div>

    <div class="edit-footer">
      <button class="btn-cancel" onclick={onClose}>Cancel</button>
      <button class="btn-save" onclick={handleSave} disabled={!title.trim() || saving}>
        {saving ? 'Saving...' : 'Save'}
      </button>
    </div>
  </div>
</div>
{/if}

<style>
  .edit-modal {
    background: var(--bg-primary, #0d1117);
    border: 1px solid var(--border, #30363d);
    border-radius: 12px;
    width: 440px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 16px 48px rgba(0,0,0,0.5);
    animation: modalUp 0.15s ease-out;
  }

  .edit-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border, #30363d);
  }
  .edit-header h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary, #e6edf3);
  }
  .close-btn {
    background: none;
    border: none;
    color: var(--text-secondary, #8b949e);
    cursor: pointer;
    padding: 4px;
    border-radius: 4px;
    display: flex;
    align-items: center;
  }
  .close-btn:hover { background: rgba(255,255,255,0.06); color: var(--text-primary); }

  .edit-body {
    padding: 16px 20px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .field label {
    display: block;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary, #8b949e);
    margin-bottom: 6px;
  }

  .field input[type="text"], .field textarea {
    width: 100%;
    padding: 8px 10px;
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    background: rgba(255,255,255,0.04);
    color: var(--text-primary, #e6edf3);
    font-size: 13px;
    font-family: inherit;
    outline: none;
    box-sizing: border-box;
  }
  .field input:focus, .field textarea:focus {
    border-color: var(--accent, #58a6ff);
  }
  .field textarea { resize: vertical; }

  .readonly-value {
    font-size: 13px;
    color: var(--text-primary, #e6edf3);
    opacity: 0.7;
    padding: 2px 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .field-hint {
    font-size: 11px;
    color: var(--text-secondary, #8b949e);
    opacity: 0.7;
    margin-top: 4px;
  }

  .toggle-label {
    display: flex !important;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0 !important;
  }

  .toggle {
    width: 36px;
    height: 20px;
    border-radius: 10px;
    border: none;
    background: rgba(255,255,255,0.1);
    cursor: pointer;
    position: relative;
    transition: background 0.2s;
    padding: 0;
    flex-shrink: 0;
  }
  .toggle.on { background: var(--accent, #58a6ff); }
  .toggle-thumb {
    display: block;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: white;
    position: absolute;
    top: 2px;
    left: 2px;
    transition: transform 0.2s;
  }
  .toggle.on .toggle-thumb { transform: translateX(16px); }

  .git-fields {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }

  .ctx-manage-btn {
    font-size: 11px;
    padding: 3px 10px;
    border-radius: 4px;
    border: 1px solid var(--border, #30363d);
    background: transparent;
    color: var(--text-secondary, #8b949e);
    cursor: pointer;
    font-family: inherit;
  }
  .ctx-manage-btn:hover { background: rgba(255,255,255,0.06); color: var(--text-primary); }

  .ctx-tags { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .ctx-tag {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 4px;
    background: rgba(88,166,255,0.1);
    color: var(--accent, #58a6ff);
  }

  .edit-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px;
    border-top: 1px solid var(--border, #30363d);
  }
  .btn-cancel, .btn-save {
    padding: 7px 16px;
    border-radius: 6px;
    font-size: 13px;
    font-family: inherit;
    cursor: pointer;
    border: none;
  }
  .btn-cancel {
    background: transparent;
    color: var(--text-secondary, #8b949e);
    border: 1px solid var(--border, #30363d);
  }
  .btn-cancel:hover { background: rgba(255,255,255,0.04); }
  .btn-save {
    background: var(--accent, #58a6ff);
    color: #fff;
    font-weight: 500;
  }
  .btn-save:hover { filter: brightness(1.1); }
  .btn-save:disabled { opacity: 0.5; cursor: not-allowed; }
</style>

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
  let selectedContexts = $state([]);
  let showContextDropdown = $state(false);
  let saving = $state(false);

  // Available contexts not yet selected
  let availableContexts = $derived(
    contextsStore.contextSnippets.filter(s => !selectedContexts.includes(s.name))
  );

  $effect(() => {
    if (profile) {
      title = profile.title || '';
      skipPermissions = profile.skipPermissions || false;
      gitEnabled = !!(profile.gitName || profile.gitEmail);
      gitName = profile.gitName || '';
      gitEmail = profile.gitEmail || '';
      contextPrompt = profile.contextPrompt || '';
      selectedContexts = [...(profile.contexts || [])];
      showContextDropdown = false;
      contextsStore.loadContextSnippets();
    }
  });

  function addContext(name) {
    if (!selectedContexts.includes(name)) {
      selectedContexts = [...selectedContexts, name];
    }
    showContextDropdown = false;
  }

  function removeContext(name) {
    selectedContexts = selectedContexts.filter(c => c !== name);
  }

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

      // Sync contexts — compare with original to decide inject/remove
      const original = profile.contexts || [];
      const changed = JSON.stringify(original.sort()) !== JSON.stringify([...selectedContexts].sort());
      if (changed) {
        if (selectedContexts.length > 0) {
          await contextsStore.attachContextsToSession(profile.id, profile.worktreePath || profile.projectPath, selectedContexts);
        } else {
          await contextsStore.detachContextsFromSession(profile.id, profile.worktreePath || profile.projectPath);
        }
      }

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
        <label>Contexts</label>
        {#if selectedContexts.length > 0}
          <div class="ctx-tags">
            {#each selectedContexts as ctx}
              <span class="ctx-tag">
                {ctx}
                <button class="ctx-remove" onclick={() => removeContext(ctx)} title="Remove">
                  <svg width="10" height="10" viewBox="0 0 16 16" fill="currentColor"><path d="M3.72 3.72a.75.75 0 011.06 0L8 6.94l3.22-3.22a.75.75 0 111.06 1.06L9.06 8l3.22 3.22a.75.75 0 11-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 01-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 010-1.06z"/></svg>
                </button>
              </span>
            {/each}
          </div>
        {/if}
        <div class="ctx-add-row">
          {#if availableContexts.length > 0}
            <button class="ctx-add-btn" onclick={(e) => { e.stopPropagation(); showContextDropdown = !showContextDropdown; }}>
              + Add context
            </button>
            {#if showContextDropdown}
              <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
              <div class="ctx-dropdown" onclick={(e) => e.stopPropagation()}>
                {#each availableContexts as snippet}
                  <button class="ctx-dropdown-item" onclick={() => addContext(snippet.name)}>
                    {snippet.name}
                  </button>
                {/each}
              </div>
            {/if}
          {:else if selectedContexts.length === 0}
            <div class="field-hint">No context snippets available. Create them in Settings.</div>
          {/if}
        </div>
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

  .ctx-tags { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 8px; }
  .ctx-tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    padding: 3px 8px;
    border-radius: 5px;
    background: rgba(88,166,255,0.1);
    color: var(--accent, #58a6ff);
  }
  .ctx-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    color: var(--text-secondary, #8b949e);
    cursor: pointer;
    padding: 1px;
    border-radius: 3px;
    opacity: 0.6;
    transition: opacity 0.15s, color 0.15s;
  }
  .ctx-remove:hover { opacity: 1; color: #f85149; }

  .ctx-add-row { position: relative; }
  .ctx-add-btn {
    font-size: 12px;
    padding: 4px 10px;
    border-radius: 5px;
    border: 1px dashed var(--border, #30363d);
    background: transparent;
    color: var(--text-secondary, #8b949e);
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, color 0.15s;
  }
  .ctx-add-btn:hover { border-color: var(--accent, #58a6ff); color: var(--accent, #58a6ff); }

  .ctx-dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    background: #1c2128;
    border: 1px solid var(--border, #30363d);
    border-radius: 8px;
    padding: 4px;
    min-width: 180px;
    max-height: 160px;
    overflow-y: auto;
    box-shadow: 0 8px 24px rgba(0,0,0,0.4);
    z-index: 20;
    animation: fadeIn 0.1s ease-out;
  }
  .ctx-dropdown-item {
    display: block;
    width: 100%;
    padding: 6px 10px;
    border: none;
    background: transparent;
    color: var(--text-secondary, #8b949e);
    font-size: 12px;
    font-family: inherit;
    cursor: pointer;
    border-radius: 5px;
    text-align: left;
    transition: background 0.12s;
  }
  .ctx-dropdown-item:hover { background: rgba(255,255,255,0.06); color: var(--text-primary, #e6edf3); }
  @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }

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

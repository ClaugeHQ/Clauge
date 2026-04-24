<script lang="ts">
  import Modal from '$lib/components/shared/Modal.svelte';
  import { agentCreateSession } from '$lib/commands/agent';
  import { loadAgentSessions } from '$lib/stores/agent';
  import { showToast } from '$lib/components/shared/toast';

  let { show = $bindable(false) } = $props();

  let title = $state('');
  let purpose = $state('Development');
  let projectPath = $state('');
  let skipPermissions = $state(false);
  let customPrompt = $state('');
  let gitName = $state('');
  let gitEmail = $state('');
  let loading = $state(false);

  const purposes = [
    { name: 'Brainstorming', color: '#d2a8ff' },
    { name: 'Development', color: '#3fb950' },
    { name: 'Code Review', color: '#58a6ff' },
    { name: 'PR Review', color: '#d29922' },
    { name: 'Debugging', color: '#f85149' },
    { name: 'Custom', color: '#8b949e' },
  ];

  async function pickFolder() {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const selected = await open({ directory: true, multiple: false });
    if (selected) projectPath = selected as string;
  }

  async function handleCreate() {
    if (!title.trim() || !projectPath.trim()) return;
    loading = true;
    try {
      await agentCreateSession({
        title: title.trim(),
        purpose,
        projectPath: projectPath.trim(),
        projectName: projectPath.trim().split('/').pop() || title.trim(),
        claudeSessionId: null,
        contextPrompt: purpose === 'Custom' && customPrompt.trim() ? customPrompt.trim() : '',
        worktreePath: null,
        worktreeBranch: null,
        skipPermissions: skipPermissions ? 1 : 0,
        gitName: gitName.trim() || null,
        gitEmail: gitEmail.trim() || null,
      });
      await loadAgentSessions();
      show = false;
      resetForm();
    } catch (e: any) {
      showToast(String(e), 'error');
    } finally {
      loading = false;
    }
  }

  function resetForm() {
    title = '';
    purpose = 'Development';
    projectPath = '';
    skipPermissions = false;
    customPrompt = '';
    gitName = '';
    gitEmail = '';
  }
</script>

<Modal bind:show title="New Agent Session" width="520px">
  <div class="ns-form">
    <label class="ns-field">
      <span class="ns-label">Project Path</span>
      <div class="ns-path-row">
        <input
          class="ns-input ns-path-input"
          type="text"
          bind:value={projectPath}
          placeholder="/path/to/project"
        />
        <button class="ns-btn outline" onclick={pickFolder}>Browse</button>
      </div>
    </label>

    <label class="ns-field">
      <span class="ns-label">Title</span>
      <input class="ns-input" type="text" bind:value={title} placeholder="Session title" />
    </label>

    <div class="ns-field">
      <span class="ns-label">Purpose</span>
      <div class="ns-chips">
        {#each purposes as p}
          <button
            class="ns-chip"
            class:active={purpose === p.name}
            style="--chip-color: {p.color}"
            onclick={() => purpose = p.name}
          >
            <span class="ns-chip-dot" style="background: {p.color}"></span>
            {p.name}
          </button>
        {/each}
      </div>
    </div>

    {#if purpose === 'Custom'}
      <label class="ns-field">
        <span class="ns-label">Custom Prompt</span>
        <textarea
          class="ns-textarea"
          bind:value={customPrompt}
          placeholder="Describe the purpose of this session..."
          rows="3"
        ></textarea>
      </label>
    {/if}

    <label class="ns-check">
      <input type="checkbox" bind:checked={skipPermissions} />
      <span>Skip permission prompts</span>
    </label>

    <div class="ns-section">
      <span class="ns-section-title">Git Identity (optional)</span>
      <div class="ns-row">
        <label class="ns-field" style="flex:1">
          <span class="ns-label">Name</span>
          <input class="ns-input" type="text" bind:value={gitName} placeholder="John Doe" />
        </label>
        <label class="ns-field" style="flex:1">
          <span class="ns-label">Email</span>
          <input class="ns-input" type="text" bind:value={gitEmail} placeholder="john@example.com" />
        </label>
      </div>
    </div>

    <div class="ns-actions">
      <button class="ns-btn outline" onclick={() => show = false}>Cancel</button>
      <button
        class="ns-btn primary"
        onclick={handleCreate}
        disabled={!title.trim() || !projectPath.trim() || loading}
      >
        {loading ? 'Creating...' : 'Create Session'}
      </button>
    </div>
  </div>
</Modal>

<style>
  .ns-form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .ns-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ns-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--t2);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .ns-input {
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
  .ns-input:focus {
    border-color: var(--acc);
  }
  .ns-input::placeholder {
    color: var(--t3);
  }
  .ns-textarea {
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 8px 10px;
    font-size: 12.5px;
    font-family: var(--mono);
    color: var(--t1);
    outline: none;
    resize: vertical;
    transition: border-color 0.15s;
  }
  .ns-textarea:focus {
    border-color: var(--acc);
  }
  .ns-textarea::placeholder {
    color: var(--t3);
  }
  .ns-path-row {
    display: flex;
    gap: 8px;
  }
  .ns-path-input {
    flex: 1;
  }
  .ns-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 2px;
  }
  .ns-chip {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 12px;
    border-radius: 14px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11.5px;
    font-family: var(--ui);
    cursor: default;
    transition: border-color 0.15s, color 0.15s, background 0.15s;
  }
  .ns-chip:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .ns-chip.active {
    border-color: var(--chip-color);
    color: var(--t1);
    background: color-mix(in srgb, var(--chip-color) 12%, transparent);
  }
  .ns-chip-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .ns-check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--t2);
    font-family: var(--ui);
    cursor: default;
  }
  .ns-check input {
    accent-color: var(--acc);
  }
  .ns-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .ns-section-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--t3);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .ns-row {
    display: flex;
    gap: 10px;
  }
  .ns-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--b1);
  }
  .ns-btn {
    height: 34px;
    padding: 0 20px;
    border-radius: 8px;
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: opacity 0.12s, border-color 0.12s, color 0.12s;
  }
  .ns-btn.outline {
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
  }
  .ns-btn.outline:hover:not(:disabled) {
    border-color: var(--b2);
    color: var(--t1);
  }
  .ns-btn.outline:disabled {
    opacity: 0.5;
  }
  .ns-btn.primary {
    border: none;
    background: var(--acc);
    color: #fff;
    font-weight: 600;
  }
  .ns-btn.primary:hover:not(:disabled) {
    opacity: 0.85;
  }
  .ns-btn.primary:disabled {
    opacity: 0.4;
  }
</style>

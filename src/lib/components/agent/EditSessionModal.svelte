<script lang="ts">
  import Modal from '$lib/components/shared/Modal.svelte';
  import { agentUpdateSession } from '$lib/commands/agent';
  import { loadAgentSessions } from '$lib/stores/agent';
  import { showToast } from '$lib/components/shared/toast';
  import type { AgentSession } from '$lib/types/agent';

  let { show = $bindable(false), session = $bindable<AgentSession | null>(null) } = $props();

  let title = $state('');
  let skipPermissions = $state(false);
  let gitName = $state('');
  let gitEmail = $state('');
  let contextPrompt = $state('');
  let loading = $state(false);

  $effect(() => {
    if (session) {
      title = session.title;
      skipPermissions = session.skipPermissions === 1;
      gitName = session.gitName || '';
      gitEmail = session.gitEmail || '';
      contextPrompt = session.contextPrompt;
    }
  });

  async function handleSave() {
    if (!session || !title.trim()) return;
    loading = true;
    try {
      await agentUpdateSession({
        ...session,
        title: title.trim(),
        skipPermissions: skipPermissions ? 1 : 0,
        gitName: gitName.trim() || null,
        gitEmail: gitEmail.trim() || null,
        contextPrompt: contextPrompt,
      });
      await loadAgentSessions();
      show = false;
    } catch (e: any) {
      showToast(String(e), 'error');
    } finally {
      loading = false;
    }
  }
</script>

<Modal bind:show title="Edit Session" width="500px">
  {#if session}
    <div class="es-form">
      <label class="es-field">
        <span class="es-label">Title</span>
        <input class="es-input" type="text" bind:value={title} placeholder="Session title" />
      </label>

      <label class="es-field">
        <span class="es-label">System Prompt</span>
        <textarea
          class="es-textarea"
          bind:value={contextPrompt}
          placeholder="Custom instructions for this session..."
          rows="4"
        ></textarea>
      </label>

      <label class="es-check">
        <input type="checkbox" bind:checked={skipPermissions} />
        <span>Skip permission prompts</span>
      </label>

      <div class="es-section">
        <span class="es-section-title">Git Identity (optional)</span>
        <div class="es-row">
          <label class="es-field" style="flex:1">
            <span class="es-label">Name</span>
            <input class="es-input" type="text" bind:value={gitName} placeholder="John Doe" />
          </label>
          <label class="es-field" style="flex:1">
            <span class="es-label">Email</span>
            <input class="es-input" type="text" bind:value={gitEmail} placeholder="john@example.com" />
          </label>
        </div>
      </div>

      <div class="es-actions">
        <button class="es-btn outline" onclick={() => show = false}>Cancel</button>
        <button
          class="es-btn primary"
          onclick={handleSave}
          disabled={!title.trim() || loading}
        >
          {loading ? 'Saving...' : 'Save Changes'}
        </button>
      </div>
    </div>
  {/if}
</Modal>

<style>
  .es-form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .es-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .es-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--t2);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .es-input {
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
  .es-input:focus {
    border-color: var(--acc);
  }
  .es-input::placeholder {
    color: var(--t3);
  }
  .es-textarea {
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
  .es-textarea:focus {
    border-color: var(--acc);
  }
  .es-textarea::placeholder {
    color: var(--t3);
  }
  .es-check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--t2);
    font-family: var(--ui);
    cursor: default;
  }
  .es-check input {
    accent-color: var(--acc);
  }
  .es-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .es-section-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--t3);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .es-row {
    display: flex;
    gap: 10px;
  }
  .es-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--b1);
  }
  .es-btn {
    height: 34px;
    padding: 0 20px;
    border-radius: 8px;
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
    transition: opacity 0.12s, border-color 0.12s, color 0.12s;
  }
  .es-btn.outline {
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
  }
  .es-btn.outline:hover:not(:disabled) {
    border-color: var(--b2);
    color: var(--t1);
  }
  .es-btn.primary {
    border: none;
    background: var(--acc);
    color: #fff;
    font-weight: 600;
  }
  .es-btn.primary:hover:not(:disabled) {
    opacity: 0.85;
  }
  .es-btn.primary:disabled {
    opacity: 0.4;
  }
</style>

<script lang="ts">
  import Modal from '$lib/components/shared/Modal.svelte';
  import { agentCreateSession, agentDiscoverSessions, agentListContexts, agentAttachContext, agentUpdateSessionId } from '$lib/commands/agent';
  import type { AgentContext, DiscoveredSession } from '$lib/types/agent';
  import { loadAgentSessions, agentSessions, activeAgentSession } from '$lib/stores/agent';
  import { tabs as tabsStore, addTab, activateTab } from '$lib/stores/tabs';
  import { showToast } from '$lib/components/shared/toast';
  import { get } from 'svelte/store';

  // Mirror PURPOSE_COLORS used in +layout.svelte's session picker so the new
  // tab gets the right dot color matching the rest of Agent UI.
  const PURPOSE_COLORS: Record<string, string> = {
    Brainstorming: '#d2a8ff',
    Development: '#3fb950',
    'Code Review': '#58a6ff',
    'PR Review': '#d29922',
    Debugging: '#f85149',
    Custom: '#8b949e',
  };

  let { show = $bindable(false) } = $props();

  // Form state — matches original Clauge exactly
  let projectPath = $state('');
  let title = $state('');
  let purpose = $state('');  // Empty by default — user must pick
  let skipPermissions = $state(false);
  let customPrompt = $state('');
  let gitEnabled = $state(false);
  let gitName = $state('');
  let gitEmail = $state('');
  let loading = $state(false);

  // Resume existing session (Custom purpose only)
  let discoveredSessions = $state<DiscoveredSession[]>([]);
  let selectedSessionId = $state('');

  // Context attachment
  let contextEnabled = $state(false);
  let availableContexts = $state<AgentContext[]>([]);
  let attachedContextNames = $state<string[]>([]);
  let showContextDropdown = $state(false);

  const purposes = [
    { label: 'Brainstorming', color: '#d2a8ff' },
    { label: 'Development', color: '#3fb950' },
    { label: 'Code Review', color: '#58a6ff' },
    { label: 'PR Review', color: '#d29922' },
    { label: 'Debugging', color: '#f85149' },
    { label: 'Custom', color: '#8b949e' },
  ];

  // Check if a purpose is already active for this project
  function isPurposeUsed(purposeLabel: string): boolean {
    if (!projectPath.trim()) return false;
    const sessions = get(agentSessions);
    return sessions.some(s => s.projectPath === projectPath.trim() && s.purpose === purposeLabel);
  }

  async function loadDiscoveredSessions(path: string) {
    try {
      const sessions = await agentDiscoverSessions(path);
      // Filter out sessions already linked to a profile
      const allSessions = get(agentSessions);
      const linkedIds = new Set(allSessions.filter(s => s.claudeSessionId).map(s => s.claudeSessionId));
      discoveredSessions = sessions.filter(s => !linkedIds.has(s.sessionId));
      selectedSessionId = '';
    } catch (_) {
      discoveredSessions = [];
    }
  }

  async function loadContexts() {
    try {
      availableContexts = await agentListContexts();
    } catch (_) {
      availableContexts = [];
    }
  }

  async function pickFolder() {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true, multiple: false, title: 'Select Project Folder' });
      if (selected) {
        projectPath = selected as string;
        if (!title) title = (selected as string).split('/').filter(Boolean).pop() || '';
        loadDiscoveredSessions(selected as string);
      }
    } catch (_) {}
  }

  async function handleCreate() {
    if (!projectPath.trim() || !title.trim() || !purpose) return;
    if (gitEnabled && (!gitName.trim() || !gitEmail.trim())) return;
    loading = true;
    try {
      const session = await agentCreateSession({
        title: title.trim(),
        purpose,
        projectPath: projectPath.trim(),
        skipPermissions: skipPermissions || undefined,
        customPrompt: purpose === 'Custom' && customPrompt.trim() ? customPrompt.trim() : undefined,
        gitName: gitEnabled && gitName.trim() ? gitName.trim() : undefined,
        gitEmail: gitEnabled && gitEmail.trim() ? gitEmail.trim() : undefined,
      });

      // Link resumed Claude session if selected
      if (selectedSessionId) {
        await agentUpdateSessionId(session.id, selectedSessionId);
      }

      // Attach selected contexts
      if (contextEnabled && attachedContextNames.length > 0) {
        for (const ctx of availableContexts) {
          if (attachedContextNames.includes(ctx.name)) {
            await agentAttachContext(session.id, ctx.id);
          }
        }
      }

      await loadAgentSessions();

      // Auto-open: open the new session in a tab and activate it. Mirrors the
      // session-picker open flow in +layout.svelte so behavior is identical.
      const allTabs = get(tabsStore);
      const existing = allTabs.find((t) => t.mode === 'agent' && t.key === session.id);
      if (existing) {
        activateTab(existing.id);
      } else {
        addTab(session.title, 'agent', session.id, PURPOSE_COLORS[session.purpose] ?? PURPOSE_COLORS.Custom);
      }
      activeAgentSession.set(session);

      show = false;
      resetForm();
    } catch (e: any) {
      showToast(String(e), 'error');
    } finally {
      loading = false;
    }
  }

  function resetForm() {
    projectPath = ''; title = ''; purpose = ''; skipPermissions = false;
    customPrompt = ''; gitEnabled = false; gitName = ''; gitEmail = '';
    discoveredSessions = []; selectedSessionId = '';
    contextEnabled = false; attachedContextNames = []; showContextDropdown = false;
  }

  // Derived: can we enable the create button?
  let canCreate = $derived(
    projectPath.trim() !== '' &&
    title.trim() !== '' &&
    purpose !== '' &&
    (!gitEnabled || (gitName.trim() !== '' && gitEmail.trim() !== ''))
  );
</script>

<Modal bind:show title="New Session" width="440px">
  <div class="ns-form">
    <label class="ns-field">
      <span class="ns-label">Project Folder</span>
      <div class="ns-path-row">
        <input
          class="ns-input ns-path-input"
          type="text"
          bind:value={projectPath}
          placeholder="/path/to/project"
          onblur={() => { if (projectPath.trim()) loadDiscoveredSessions(projectPath.trim()); }}
        />
        <button class="ns-btn-browse" onclick={pickFolder}>Browse</button>
      </div>
    </label>

    <label class="ns-field">
      <span class="ns-label">Title</span>
      <input class="ns-input" type="text" bind:value={title} placeholder="e.g. Auth Refactor" />
    </label>

    <div class="ns-field">
      <span class="ns-label-text">Purpose</span>
      <div class="ns-chips">
        {#each purposes as p}
          {#if !projectPath.trim()}
            <span class="ns-chip disabled">{p.label}</span>
          {:else if p.label !== 'Custom' && isPurposeUsed(p.label)}
            <span class="ns-chip disabled" title="{p.label} already active for this project">{p.label}</span>
          {:else}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <span
              class="ns-chip"
              class:selected={purpose === p.label}
              style={purpose === p.label ? `background:${p.color}33;color:${p.color};border-color:${p.color}` : ''}
              onclick={() => { purpose = p.label; if (p.label === 'Custom' && projectPath.trim()) loadDiscoveredSessions(projectPath.trim()); }}
            >{p.label}</span>
          {/if}
        {/each}
      </div>
    </div>

    {#if discoveredSessions.length > 0 && purpose !== 'Custom'}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="ns-hint" onclick={() => { purpose = 'Custom'; }}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--acc)" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/></svg>
        <span>{discoveredSessions.length} previous session{discoveredSessions.length > 1 ? 's' : ''} found — <strong style="color:var(--acc);cursor:pointer;">resume via Custom</strong></span>
      </div>
    {/if}

    {#if purpose === 'Custom'}
      {#if discoveredSessions.length > 0}
        <label class="ns-field">
          <span class="ns-label">Resume Existing Session</span>
          <select class="ns-select" bind:value={selectedSessionId}>
            <option value="">Start fresh</option>
            {#each discoveredSessions as s}
              <option value={s.sessionId}>{s.preview || s.sessionId.slice(0, 8)} — {new Date(s.modifiedAt).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}</option>
            {/each}
          </select>
        </label>
      {/if}
      <label class="ns-field">
        <span class="ns-label">System Prompt <span class="ns-optional">(optional)</span></span>
        <textarea class="ns-textarea" bind:value={customPrompt} placeholder="Custom instructions for this session..." rows="2"></textarea>
      </label>
    {/if}

    <div class="ns-adv-label">Advanced</div>

    <!-- Skip Permissions toggle -->
    <div class="ns-toggle-row">
      <div class="ns-toggle-info">
        <span class="ns-toggle-text">Skip permissions</span>
        <span class="ns-toggle-hint">Auto-approve all tool calls without confirmation</span>
      </div>
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <button class="ns-toggle" class:on={skipPermissions} onclick={() => skipPermissions = !skipPermissions}>
        <span class="ns-toggle-knob"></span>
      </button>
    </div>

    <!-- Git Identity toggle -->
    <div class="ns-toggle-row">
      <div class="ns-toggle-info">
        <span class="ns-toggle-text">Git Identity</span>
        <span class="ns-toggle-hint">Override git author name and email for this session</span>
      </div>
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <button class="ns-toggle" class:on={gitEnabled} onclick={() => gitEnabled = !gitEnabled}>
        <span class="ns-toggle-knob"></span>
      </button>
    </div>
    {#if gitEnabled}
      <div class="ns-adv-body">
        <div class="ns-row">
          <label class="ns-adv-field">
            <span class="ns-adv-label-sm">Name <span class="ns-required">*</span></span>
            <input type="text" class="ns-input" bind:value={gitName} placeholder="e.g. John Doe" />
          </label>
          <label class="ns-adv-field">
            <span class="ns-adv-label-sm">Email <span class="ns-required">*</span></span>
            <input type="text" class="ns-input" bind:value={gitEmail} placeholder="e.g. john@example.com" />
          </label>
        </div>
      </div>
    {/if}

    <!-- Attach Contexts toggle -->
    <div class="ns-toggle-row">
      <div class="ns-toggle-info">
        <span class="ns-toggle-text">Attach Contexts</span>
        <span class="ns-toggle-hint">Inject context snippets into CLAUDE.md before each spawn</span>
      </div>
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <button class="ns-toggle" class:on={contextEnabled} onclick={() => { contextEnabled = !contextEnabled; if (contextEnabled) loadContexts(); }}>
        <span class="ns-toggle-knob"></span>
      </button>
    </div>
    {#if contextEnabled}
      <div class="ns-adv-body">
        {#if attachedContextNames.length > 0}
          <div class="ns-ctx-chips">
            {#each attachedContextNames as name}
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <span class="ns-ctx-chip">
                {name}
                <span class="ns-ctx-x" onclick={() => { attachedContextNames = attachedContextNames.filter(n => n !== name); }}>×</span>
              </span>
            {/each}
          </div>
        {/if}
        <div class="ns-ctx-add-wrap">
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
          <button class="ns-ctx-add-btn" onclick={(e) => { e.stopPropagation(); showContextDropdown = !showContextDropdown; }}>
            <svg width="10" height="10" viewBox="0 0 16 16" fill="currentColor"><path d="M7.75 2a.75.75 0 01.75.75V7h4.25a.75.75 0 010 1.5H8.5v4.25a.75.75 0 01-1.5 0V8.5H2.75a.75.75 0 010-1.5H7V2.75A.75.75 0 017.75 2z"/></svg>
            Add
          </button>
          {#if showContextDropdown}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <div class="ns-ctx-backdrop" onclick={() => showContextDropdown = false}></div>
            <div class="ns-ctx-dropdown">
              {#each availableContexts.filter(c => !attachedContextNames.includes(c.name)) as ctx}
                <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                <div class="ns-ctx-dd-item" onclick={() => { attachedContextNames = [...attachedContextNames, ctx.name]; showContextDropdown = false; }}>
                  <span class="ns-ctx-dd-name">{ctx.name}</span>
                  <span class="ns-ctx-dd-preview">{ctx.content.slice(0, 60)}</span>
                </div>
              {:else}
                <div class="ns-ctx-dd-empty">No more contexts available</div>
              {/each}
            </div>
          {/if}
        </div>
      </div>
    {/if}

    <div class="ns-actions">
      <button class="ns-btn-cancel" onclick={() => { show = false; resetForm(); }}>Cancel</button>
      <button class="ns-btn-create" onclick={handleCreate} disabled={!canCreate || loading}>
        {loading ? 'Creating...' : 'Create'}
      </button>
    </div>
  </div>
</Modal>

<style>
  .ns-form { display: flex; flex-direction: column; gap: 12px; }
  .ns-field { display: flex; flex-direction: column; gap: 4px; }
  .ns-label { font-size: 12px; font-weight: 600; color: var(--t2); text-transform: uppercase; font-family: var(--ui); }
  .ns-label-text { font-size: 13px; color: var(--t1); font-family: var(--ui); }
  .ns-optional { font-size: 10px; color: var(--t3); font-weight: normal; text-transform: none; }
  .ns-input {
    width: 100%; background: var(--e); border: 1px solid var(--b1); border-radius: 6px;
    padding: 8px 10px; font-size: 13px; color: var(--t1); outline: none; box-sizing: border-box;
    font-family: var(--mono); transition: border-color 0.15s;
  }
  .ns-input:focus { border-color: var(--acc); }
  .ns-input::placeholder { color: var(--t3); }
  .ns-textarea {
    width: 100%; padding: 8px 10px; border-radius: 6px; border: 1px solid var(--b1);
    background: var(--e); color: var(--t1); font-size: 12px; font-family: var(--mono);
    resize: vertical; min-height: 50px; line-height: 1.5; outline: none; box-sizing: border-box;
  }
  .ns-textarea:focus { border-color: var(--acc); }
  .ns-textarea::placeholder { color: var(--t3); }
  .ns-select {
    width: 100%; padding: 7px 10px; border-radius: 6px; border: 1px solid var(--b1);
    background: var(--e); color: var(--t1); font-size: 12px; font-family: var(--ui);
    appearance: none; cursor: pointer; outline: none;
  }
  .ns-select option { background: var(--n); color: var(--t1); }
  .ns-path-row { display: flex; gap: 8px; }
  .ns-path-input { flex: 1; }
  .ns-btn-browse {
    background: var(--n); border: 1px solid var(--b1); border-radius: 6px;
    padding: 8px 12px; color: var(--t1); font-size: 13px; cursor: pointer;
    white-space: nowrap; font-family: var(--ui); transition: border-color 0.15s;
  }
  .ns-btn-browse:hover { border-color: var(--b2); }
  .ns-chips { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 4px; }
  .ns-chip {
    padding: 5px 12px; border-radius: 14px; border: 1px solid var(--b1);
    background: transparent; color: var(--t2); font-size: 12px; cursor: pointer;
    font-family: var(--ui); transition: background 0.15s, color 0.15s; user-select: none;
  }
  .ns-chip:hover:not(.selected):not(.disabled) { background: rgba(255,255,255,0.06); }
  .ns-chip.disabled { opacity: 0.3; cursor: not-allowed; }
  .ns-chip.selected { font-weight: 600; }
  .ns-hint {
    display: flex; align-items: flex-start; gap: 8px; padding: 8px 10px; border-radius: 6px;
    background: color-mix(in srgb, var(--acc) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--acc) 20%, transparent);
  }
  .ns-hint svg { flex-shrink: 0; margin-top: 1px; }
  .ns-hint span { font-size: 11px; color: var(--t2); line-height: 1.4; }
  .ns-adv-label {
    font-size: 11px; font-weight: 600; color: var(--t3); text-transform: uppercase;
    letter-spacing: 0.1em; margin-top: 6px; font-family: var(--ui);
  }
  .ns-toggle-row {
    display: flex; align-items: center; justify-content: space-between; margin-top: 4px;
  }
  .ns-toggle-info { display: flex; flex-direction: column; gap: 2px; }
  .ns-toggle-text { font-size: 12px; color: var(--t2); font-family: var(--ui); }
  .ns-toggle-hint { font-size: 10px; color: var(--t4); font-family: var(--ui); }
  .ns-toggle {
    width: 36px; height: 20px; border-radius: 10px; border: 1px solid var(--b1);
    background: rgba(255,255,255,0.06); cursor: pointer; position: relative;
    transition: all 0.2s; padding: 0;
  }
  .ns-toggle.on { background: var(--acc); border-color: var(--acc); }
  .ns-toggle-knob {
    position: absolute; top: 2px; left: 2px; width: 14px; height: 14px;
    border-radius: 50%; background: var(--t3); transition: all 0.2s;
  }
  .ns-toggle.on .ns-toggle-knob { left: 18px; background: #fff; }
  .ns-adv-body {
    display: flex; flex-direction: column; gap: 8px; padding: 4px 0 0;
    animation: advIn 0.12s ease;
  }
  @keyframes advIn { from { opacity: 0; } to { opacity: 1; } }
  .ns-row { display: flex; gap: 8px; }
  .ns-adv-field { flex: 1; display: flex; flex-direction: column; gap: 4px; }
  .ns-adv-label-sm { font-size: 11px; color: var(--t3); font-family: var(--ui); }
  .ns-required { color: var(--err, #f85149); font-weight: 600; }
  .ns-ctx-chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .ns-ctx-chip {
    display: flex; align-items: center; gap: 4px; padding: 3px 6px 3px 10px;
    border-radius: 12px; background: color-mix(in srgb, var(--acc) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--acc) 25%, transparent);
    color: var(--acc); font-size: 11px; font-weight: 500;
  }
  .ns-ctx-x { cursor: pointer; font-size: 14px; line-height: 1; opacity: 0.6; transition: opacity 0.1s; }
  .ns-ctx-x:hover { opacity: 1; }
  .ns-ctx-backdrop { position: fixed; inset: 0; z-index: 99; }
  .ns-ctx-add-wrap { position: relative; }
  .ns-ctx-add-btn {
    display: flex; align-items: center; gap: 4px; padding: 4px 10px; border-radius: 4px;
    border: 1px dashed var(--b1); background: transparent; color: var(--t3);
    font-size: 11px; font-family: var(--ui); cursor: pointer; transition: all 0.1s;
  }
  .ns-ctx-add-btn:hover { border-color: var(--acc); color: var(--acc); }
  .ns-ctx-dropdown {
    position: absolute; top: calc(100% + 4px); left: 0; width: 250px;
    background: var(--n); border: 1px solid var(--b1); border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0,0,0,0.4); z-index: 100; max-height: 180px;
    overflow-y: auto; padding: 4px;
  }
  .ns-ctx-dd-item { padding: 6px 10px; border-radius: 4px; cursor: pointer; transition: background 0.1s; }
  .ns-ctx-dd-item:hover { background: rgba(255,255,255,0.06); }
  .ns-ctx-dd-name { font-size: 12px; font-weight: 500; color: var(--t1); display: block; }
  .ns-ctx-dd-preview { font-size: 10px; color: var(--t3); display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ns-ctx-dd-empty { padding: 10px; text-align: center; font-size: 11px; color: var(--t3); }
  .ns-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; padding-top: 12px; border-top: 1px solid var(--b1); }
  .ns-btn-cancel {
    padding: 7px 16px; border-radius: 6px; font-size: 13px; cursor: pointer;
    border: 1px solid var(--b1); background: transparent; color: var(--t2); font-family: var(--ui);
  }
  .ns-btn-cancel:hover { background: rgba(255,255,255,0.04); }
  .ns-btn-create {
    padding: 7px 16px; border-radius: 6px; font-size: 13px; cursor: pointer;
    border: none; background: var(--acc); color: #fff; font-weight: 600; font-family: var(--ui);
  }
  .ns-btn-create:hover:not(:disabled) { filter: brightness(1.1); }
  .ns-btn-create:disabled { opacity: 0.4; cursor: not-allowed; }
</style>

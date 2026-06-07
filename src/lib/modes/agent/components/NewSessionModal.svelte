<script lang="ts">
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { agentCreateSession, agentDiscoverSessions, agentListContexts, agentAttachContext, agentUpdateSessionId, agentValidateBinary } from '../commands';
  import type { AgentContext, DiscoveredSession, AgentProvider } from '../types';
  import { AGENT_PROVIDERS } from '../types';
  import { providerStatus, providerStatusReady, refreshProviderStatus } from '$lib/shared/stores/providerStatus';
  import ProviderNotInstalledModal from '$lib/shared/agent/ProviderNotInstalledModal.svelte';

  // Provider tile icons live in /static. Same brand assets you see in
  // the agent nav session-row, so the New Session picker matches the
  // session list visually.
  const PROVIDER_ICON: Record<AgentProvider, string> = {
    claude: '/code-no-action.svg',
    codex: '/codex.svg',
    gemini: '/gemini.svg',
    opencode: '/opencode-dark.svg',
  };
  // Vendor subtitles for the left-rail provider list. Surfaced only
  // for visual orientation — not stored, not sent to the backend.
  const PROVIDER_VENDOR: Record<AgentProvider, string> = {
    claude: 'Anthropic',
    codex: 'OpenAI',
    gemini: 'Google',
    opencode: 'Open source',
  };
  import { loadAgentSessions, agentSessions, activeAgentSession, agentFooterProvider } from '../stores';
  import { tabs as tabsStore, addTab, activateTab } from '$lib/shared/stores/tabs';
  import { showToast } from '$lib/shared/primitives/toast';
  import { SESSION_PURPOSES, getPurposeColor, getPurposePrompt } from '../ai/prompt';
  import { get } from 'svelte/store';

  let { show = $bindable(false) } = $props();

  // Form state — matches original Clauge exactly
  let projectPath = $state('');
  let title = $state('');
  let purpose = $state('');  // Empty by default — user must pick
  // Which CLI backs this session. Defaults to the footer-selected provider
  // (whichever the user last looked at usage for) so the typical
  // "open another session in the same CLI" flow is one click. Coerced to
  // an `AgentProvider` since the footer store can technically widen.
  let provider = $state<AgentProvider>(
    (['claude', 'codex', 'gemini', 'opencode'] as const).includes(($agentFooterProvider as any))
      ? ($agentFooterProvider as AgentProvider)
      : 'claude',
  );
  let skipPermissions = $state(false);
  let customPrompt = $state('');
  let gitEnabled = $state(false);
  let gitName = $state('');
  let gitEmail = $state('');
  let loading = $state(false);

  // Right-pane tab: General (project + purpose + custom extras) vs
  // Advanced (toggles for skip-permissions / git / binary / contexts).
  // Defaults to General; reset back to General on close.
  let activeTab = $state<'general' | 'advanced'>('general');

  // Advanced > Custom binary path. Off by default — the typical user
  // has the CLI on $PATH and shouldn't see this. When enabled, the
  // value is forwarded to the new session row's `binary_path` column;
  // empty string is treated as "not set" (falls back to $PATH).
  let useCustomBinary = $state(false);
  let customBinaryPath = $state('');
  let binaryProbeResult = $state<{ kind: 'idle' | 'ok' | 'err' | 'probing'; msg: string }>({ kind: 'idle', msg: '' });

  // Resume existing session (Custom purpose only)
  let discoveredSessions = $state<DiscoveredSession[]>([]);
  let selectedSessionId = $state('');

  // Context attachment
  let contextEnabled = $state(false);
  let availableContexts = $state<AgentContext[]>([]);
  let attachedContextNames = $state<string[]>([]);
  let showContextDropdown = $state(false);

  const purposes = SESSION_PURPOSES.map(p => ({ label: p.id, color: p.color }));

  // Check if a purpose is already active for this project
  function isPurposeUsed(purposeLabel: string): boolean {
    if (!projectPath.trim()) return false;
    const sessions = get(agentSessions);
    return sessions.some(s => s.projectPath === projectPath.trim() && s.purpose === purposeLabel);
  }

  async function loadDiscoveredSessions(path: string) {
    try {
      // Pass the selected provider so the backend queries the right
      // session store (Claude per-project jsonl dir, Codex date-tree
      // sessions filtered by cwd, or OpenCode SQLite by directory).
      const sessions = await agentDiscoverSessions(path, provider);
      // Filter out sessions already linked to a profile of the same
      // provider AND same project. Without the project-scope clause a
      // stale claudeSessionId on an unrelated project's row could
      // theoretically hide a disk session here. (`claudeSessionId` is
      // the historical column name for the CLI session id.)
      const allSessions = get(agentSessions).filter(
        (s) =>
          (s.provider ?? 'claude') === provider &&
          s.projectPath === path,
      );
      const linkedIds = new Set(
        allSessions.filter((s) => s.claudeSessionId).map((s) => s.claudeSessionId),
      );
      discoveredSessions = sessions.filter((s) => !linkedIds.has(s.sessionId));
      selectedSessionId = '';
    } catch (_) {
      discoveredSessions = [];
    }
  }

  // Reload discovered sessions whenever the user picks a different
  // provider OR re-types the project path — keeps the "Custom" purpose
  // picker honest across provider switches.
  $effect(() => {
    const _ = provider; // dependency
    if (projectPath.trim()) {
      loadDiscoveredSessions(projectPath.trim());
    } else {
      discoveredSessions = [];
    }
  });

  async function loadContexts() {
    try {
      availableContexts = await agentListContexts();
    } catch (_) {
      availableContexts = [];
    }
  }

  async function pickBinaryFile() {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const isWindows = navigator.userAgent.toLowerCase().includes('windows');
      const selected = await open({
        directory: false,
        multiple: false,
        title: 'Select CLI binary',
        filters: isWindows
          ? [{ name: 'Executables', extensions: ['exe', 'cmd', 'bat'] }, { name: 'All files', extensions: ['*'] }]
          : [{ name: 'All files', extensions: ['*'] }],
      });
      if (typeof selected === 'string' && selected) {
        customBinaryPath = selected;
        await probeBinary(selected);
      }
    } catch (_) {}
  }

  async function probeBinary(path: string) {
    const trimmed = path.trim();
    if (!trimmed) {
      binaryProbeResult = { kind: 'idle', msg: '' };
      return;
    }
    binaryProbeResult = { kind: 'probing', msg: 'Checking…' };
    try {
      const banner = await agentValidateBinary(trimmed);
      binaryProbeResult = { kind: 'ok', msg: banner.split('\n')[0].slice(0, 80) };
    } catch (e: any) {
      const msg = (typeof e === 'string' ? e : e?.message ?? String(e)).split('\n')[0].slice(0, 120);
      binaryProbeResult = { kind: 'err', msg };
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

  let showProviderNotInstalled = $state(false);
  let rechecking = $state(false);

  async function recheck() {
    if (rechecking) return;
    rechecking = true;
    try {
      await refreshProviderStatus();
    } finally {
      rechecking = false;
    }
  }

  const missingCount = $derived(
    $providerStatusReady
      ? AGENT_PROVIDERS.filter((p) => !$providerStatus[p.id]).length
      : 0,
  );

  async function handleCreate() {
    if (!projectPath.trim() || !title.trim() || !purpose) return;
    if (gitEnabled && (!gitName.trim() || !gitEmail.trim())) return;

    // Pre-flight: the boot-time probe (providerStatus) tells us instantly
    // whether the chosen CLI is on PATH. If it's not, show the install
    // modal here instead of letting the spawn fail later.
    if ($providerStatusReady && !$providerStatus[provider]) {
      showProviderNotInstalled = true;
      return;
    }

    loading = true;
    try {
      const session = await agentCreateSession({
        title: title.trim(),
        purpose,
        projectPath: projectPath.trim(),
        skipPermissions: skipPermissions || undefined,
        customPrompt: purpose === 'Custom'
          ? (customPrompt.trim() || undefined)
          : (getPurposePrompt(purpose) ?? undefined),
        gitName: gitEnabled && gitName.trim() ? gitName.trim() : undefined,
        gitEmail: gitEnabled && gitEmail.trim() ? gitEmail.trim() : undefined,
        provider,
        binaryPath: useCustomBinary && customBinaryPath.trim()
          ? customBinaryPath.trim()
          : undefined,
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
        addTab(session.title, 'agent', session.id, getPurposeColor(session.purpose));
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
    useCustomBinary = false; customBinaryPath = '';
    binaryProbeResult = { kind: 'idle', msg: '' };
    activeTab = 'general';
  }

  // Derived: can we enable the create button?
  let canCreate = $derived(
    projectPath.trim() !== '' &&
    title.trim() !== '' &&
    purpose !== '' &&
    (!gitEnabled || (gitName.trim() !== '' && gitEmail.trim() !== ''))
  );
</script>

<Modal bind:show title="New session" width="680px">
  <!-- Bleed the Modal primitive's body padding so the left rail + right
       pane go edge-to-edge, and the footer reaches the card borders. -->
  <div class="ns-shell">
    <div class="ns-body">
      <!-- LEFT RAIL — vertical provider list -->
      <aside class="ns-rail">
        {#each AGENT_PROVIDERS as p}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <button
            class="ns-prov-row"
            class:selected={provider === p.id}
            class:not-installed={$providerStatusReady && !$providerStatus[p.id]}
            title={$providerStatusReady && !$providerStatus[p.id] ? `${p.label} not found on PATH` : `Use ${p.label}`}
            onclick={() => { provider = p.id; }}
          >
            <span class="ns-prov-icon">
              <img src={PROVIDER_ICON[p.id]} alt="" width="22" height="22" />
            </span>
            <span class="ns-prov-text">
              <span class="ns-prov-name">{p.label}</span>
              <span class="ns-prov-vendor">{PROVIDER_VENDOR[p.id]}</span>
            </span>
            {#if $providerStatusReady && !$providerStatus[p.id]}
              <span class="ns-prov-missing" title="Not installed">!</span>
            {/if}
          </button>
        {/each}
        {#if missingCount > 0}
          <div class="ns-prov-foot">
            <span class="ns-prov-foot-text">
              {missingCount === 1 ? '1 CLI not on PATH' : `${missingCount} CLIs not on PATH`}
            </span>
            <button
              class="ns-prov-foot-link"
              class:rechecking
              type="button"
              disabled={rechecking}
              title="Re-check installed CLIs"
              onclick={recheck}
            >
              {#if rechecking}
                <svg class="ns-spin" viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 12a9 9 0 1 1-6.219-8.56"/>
                </svg>
                Checking…
              {:else}
                <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="23 4 23 10 17 10"/>
                  <polyline points="1 20 1 14 7 14"/>
                  <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
                </svg>
                Re-check
              {/if}
            </button>
          </div>
        {/if}
      </aside>

      <!-- RIGHT PANE — tabs + form -->
      <section class="ns-pane">
        <!-- Tab bar: General (form fields) · Advanced (toggles cards) -->
        <div class="ns-tabs" role="tablist">
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <button
            class="ns-tab"
            class:active={activeTab === 'general'}
            role="tab"
            aria-selected={activeTab === 'general'}
            onclick={() => activeTab = 'general'}
          >General</button>
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <button
            class="ns-tab"
            class:active={activeTab === 'advanced'}
            role="tab"
            aria-selected={activeTab === 'advanced'}
            onclick={() => activeTab = 'advanced'}
          >Advanced</button>
        </div>

        {#if activeTab === 'general'}
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
              <button class="ns-btn-browse" onclick={pickFolder}>
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>
                Browse
              </button>
            </div>
          </label>

          <label class="ns-field">
            <span class="ns-label">Session Title</span>
            <input class="ns-input" type="text" bind:value={title} placeholder="e.g. Auth Refactor" />
          </label>

          <div class="ns-field">
            <span class="ns-label">Purpose</span>
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
        {:else}
          <!-- ADVANCED TAB — each option is a card with a tinted icon
               square, title, description, and a toggle. When toggled
               ON, the inline inputs slide in directly below the card. -->
          <div class="ns-adv-list">
            <!-- Skip permissions -->
            <div class="ns-adv-card">
              <div class="ns-adv-row">
                <span class="ns-adv-icon" style="background: color-mix(in srgb, var(--err, #f85149) 14%, transparent); color: var(--err, #f85149);">
                  <!-- shield-off (lucide-style) -->
                  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M19.69 14a6.9 6.9 0 00.31-2V5l-8-3-3.16 1.18"/><path d="M4.73 4.73L4 5v7c0 6 8 10 8 10a20.29 20.29 0 005.62-4.38"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
                </span>
                <div class="ns-adv-info">
                  <span class="ns-adv-title">Skip permissions</span>
                  <span class="ns-adv-desc">Auto-approve all tool calls without confirmation</span>
                </div>
                <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                <button class="ns-toggle" class:on={skipPermissions} onclick={() => skipPermissions = !skipPermissions}>
                  <span class="ns-toggle-knob"></span>
                </button>
              </div>
            </div>

            <!-- Git identity -->
            <div class="ns-adv-card">
              <div class="ns-adv-row">
                <span class="ns-adv-icon" style="background: color-mix(in srgb, #6aa9ff 14%, transparent); color: #6aa9ff;">
                  <!-- git branch (lucide-style) -->
                  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><line x1="6" y1="3" x2="6" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 01-9 9"/></svg>
                </span>
                <div class="ns-adv-info">
                  <span class="ns-adv-title">Git identity</span>
                  <span class="ns-adv-desc">Override git author name and email for this session</span>
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
            </div>

            <!-- Custom binary path -->
            <div class="ns-adv-card">
              <div class="ns-adv-row">
                <span class="ns-adv-icon" style="background: color-mix(in srgb, #c4a7e7 14%, transparent); color: #c4a7e7;">
                  <!-- terminal/binary glyph -->
                  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>
                </span>
                <div class="ns-adv-info">
                  <span class="ns-adv-title">Custom binary path</span>
                  <span class="ns-adv-desc">Use a specific {provider} binary for this session instead of $PATH</span>
                </div>
                <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                <button class="ns-toggle" class:on={useCustomBinary} onclick={() => { useCustomBinary = !useCustomBinary; if (!useCustomBinary) { customBinaryPath = ''; binaryProbeResult = { kind: 'idle', msg: '' }; } }}>
                  <span class="ns-toggle-knob"></span>
                </button>
              </div>
              {#if useCustomBinary}
                <div class="ns-adv-body">
                  <div class="ns-path-row">
                    <input
                      class="ns-input ns-path-input"
                      type="text"
                      bind:value={customBinaryPath}
                      placeholder="/usr/local/bin/{provider}"
                      onblur={() => probeBinary(customBinaryPath)}
                    />
                    <button class="ns-btn-browse" onclick={pickBinaryFile}>
                      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>
                      Browse
                    </button>
                  </div>
                  {#if binaryProbeResult.kind === 'probing'}
                    <div class="ns-probe-hint">{binaryProbeResult.msg}</div>
                  {:else if binaryProbeResult.kind === 'ok'}
                    <div class="ns-probe-hint ns-probe-ok">✓ {binaryProbeResult.msg}</div>
                  {:else if binaryProbeResult.kind === 'err'}
                    <div class="ns-probe-hint ns-probe-err">⚠ {binaryProbeResult.msg} — save anyway?</div>
                  {/if}
                </div>
              {/if}
            </div>

            <!-- Attach contexts -->
            <div class="ns-adv-card">
              <div class="ns-adv-row">
                <span class="ns-adv-icon" style="background: color-mix(in srgb, #2ee08a 14%, transparent); color: #2ee08a;">
                  <!-- file-text (lucide-style) -->
                  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/></svg>
                </span>
                <div class="ns-adv-info">
                  <span class="ns-adv-title">Attach contexts</span>
                  <span class="ns-adv-desc">Inject context snippets into the project's agent file before each spawn</span>
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
            </div>
          </div>
        {/if}
      </section>
    </div>

    <!-- FOOTER — full-width action bar. Same pattern as ConfirmDialog so
         buttons match the rest of the app visually. -->
    <div class="ns-actions">
      <button class="ns-btn-cancel" onclick={() => { show = false; resetForm(); }}>Cancel</button>
      <button class="ns-btn-create" onclick={handleCreate} disabled={!canCreate || loading}>
        {loading ? 'Creating…' : 'Create session'}
      </button>
    </div>
  </div>
</Modal>

<ProviderNotInstalledModal bind:show={showProviderNotInstalled} {provider} />

<style>
  /* Shell bleeds the Modal primitive's body padding (20px 24px) so the
     two-column layout reaches the card edges and the footer's top
     divider lines up with the card sides. */
  .ns-shell {
    margin: -20px -24px;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  /* Two-column body: provider rail (left, fixed) + form (right, fluid). */
  .ns-body {
    display: flex;
    align-items: stretch;
    min-height: 0;
  }

  /* LEFT RAIL ------------------------------------------------------- */
  .ns-rail {
    flex: 0 0 220px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 18px 14px;
    border-right: 1px solid var(--b1);
    background: color-mix(in srgb, var(--e) 60%, transparent);
  }
  .ns-prov-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 10px;
    color: var(--t2);
    font-family: var(--ui);
    cursor: pointer;
    text-align: left;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
    position: relative;
  }
  .ns-prov-row:hover {
    background: var(--surface-hover);
    color: var(--t1);
  }
  .ns-prov-row.selected {
    background: color-mix(in srgb, var(--acc) 8%, transparent);
    border-color: color-mix(in srgb, var(--acc) 30%, var(--b1));
    color: var(--t1);
  }
  /* Right-edge accent bar on the selected row — matches the wireframe. */
  .ns-prov-row.selected::after {
    content: '';
    position: absolute;
    top: 8px; bottom: 8px; right: -1px;
    width: 3px;
    background: var(--acc);
    border-radius: 2px;
  }
  .ns-prov-row.not-installed .ns-prov-name,
  .ns-prov-row.not-installed .ns-prov-vendor {
    opacity: 0.55;
  }
  .ns-prov-missing {
    margin-left: auto;
    width: 16px;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: color-mix(in srgb, var(--warn, #d97706) 22%, transparent);
    color: var(--warn, #d97706);
    font-size: 10px;
    font-weight: 700;
    font-family: var(--ui);
    line-height: 1;
  }
  .ns-prov-foot {
    margin-top: auto;
    padding: 10px 8px 4px;
    border-top: 1px solid var(--b1);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ns-prov-foot-text {
    font: 10.5px var(--ui);
    color: var(--t3);
    letter-spacing: 0.01em;
  }
  .ns-prov-foot-link {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 0;
    background: transparent;
    border: none;
    color: var(--acc);
    font: 11px var(--ui);
    cursor: pointer;
    align-self: flex-start;
    transition: opacity 0.1s;
  }
  .ns-prov-foot-link:hover:not(:disabled) {
    text-decoration: underline;
  }
  .ns-prov-foot-link:disabled {
    cursor: default;
    opacity: 0.7;
  }
  .ns-prov-foot-link.rechecking {
    color: var(--t3);
  }
  .ns-spin {
    animation: ns-spin 0.8s linear infinite;
  }
  @keyframes ns-spin {
    to { transform: rotate(360deg); }
  }
  .ns-prov-icon {
    flex-shrink: 0;
    width: 32px; height: 32px;
    display: flex; align-items: center; justify-content: center;
    background: var(--surface-hover);
    border-radius: 8px;
  }
  .ns-prov-icon img { display: block; }
  .ns-prov-text { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .ns-prov-name {
    font-size: 13px; font-weight: 600; color: var(--t1);
    line-height: 1.2;
  }
  .ns-prov-vendor {
    font-size: 11px; color: var(--t4);
    line-height: 1.2;
  }

  /* RIGHT PANE ------------------------------------------------------ */
  .ns-pane {
    flex: 1;
    padding: 20px 22px 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  .ns-field { display: flex; flex-direction: column; gap: 6px; }

  /* Binary-probe inline status — neutral / green / amber. Lives inside
     the same ns-adv-body block as the path input. */
  .ns-probe-hint {
    margin-top: 6px;
    font-size: 11px;
    font-family: var(--mono);
    color: var(--t3);
    line-height: 1.4;
    word-break: break-word;
  }
  .ns-probe-ok { color: var(--ok, #2ee08a); }
  .ns-probe-err { color: var(--warn, #f4c150); }
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
    width: 100%; padding: 7px 10px; padding-right: 28px; border-radius: 6px; border: 1px solid var(--b1);
    background: var(--e); color: var(--t1); font-size: 12px; font-family: var(--ui);
    -webkit-appearance: none; appearance: none; cursor: pointer; outline: none;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23b0b0c8' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'><polyline points='3 5 6 8 9 5'/></svg>");
    background-repeat: no-repeat; background-position: right 10px center; background-size: 10px 10px;
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
  .ns-chip:hover:not(.selected):not(.disabled) { background: var(--surface-hover); }
  .ns-chip.disabled { opacity: 0.3; cursor: not-allowed; }
  .ns-chip.selected { font-weight: 600; }
  .ns-hint {
    display: flex; align-items: flex-start; gap: 8px; padding: 8px 10px; border-radius: 6px;
    background: color-mix(in srgb, var(--acc) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--acc) 20%, transparent);
  }
  .ns-hint svg { flex-shrink: 0; margin-top: 1px; }
  .ns-hint span { font-size: 11px; color: var(--t2); line-height: 1.4; }

  /* Tab bar (General / Advanced). Underline indicator on the active
     tab; same pattern as the rest of the app's tabbed surfaces. */
  .ns-tabs {
    display: flex;
    gap: 4px;
    margin: -4px -4px 4px;
    border-bottom: 1px solid var(--b1);
  }
  .ns-tab {
    background: transparent;
    border: none;
    padding: 10px 16px;
    font-size: 13px;
    font-family: var(--ui);
    color: var(--t3);
    cursor: pointer;
    position: relative;
    transition: color 0.12s;
    border-radius: 0;
  }
  .ns-tab:hover { color: var(--t1); }
  .ns-tab.active {
    color: var(--t1);
    font-weight: 600;
  }
  .ns-tab.active::after {
    content: '';
    position: absolute;
    left: 12px; right: 12px; bottom: -1px;
    height: 2px;
    background: var(--acc);
    border-radius: 2px 2px 0 0;
  }

  /* Advanced-tab content — list of option "cards" with tinted icon,
     title, description, and a toggle. Matches the wireframe. */
  .ns-adv-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    animation: advIn 0.14s ease;
  }
  .ns-adv-card {
    border: 1px solid var(--b1);
    border-radius: 12px;
    padding: 14px 16px;
    background: color-mix(in srgb, var(--e) 25%, transparent);
    transition: border-color 0.12s;
  }
  .ns-adv-card:hover { border-color: var(--b2, color-mix(in srgb, var(--t1) 15%, var(--b1))); }
  .ns-adv-row {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .ns-adv-icon {
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border-radius: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .ns-adv-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .ns-adv-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
    line-height: 1.2;
  }
  .ns-adv-desc {
    font-size: 12px;
    color: var(--t3);
    font-family: var(--ui);
    line-height: 1.4;
  }
  .ns-toggle-row {
    display: flex; align-items: center; justify-content: space-between; margin-top: 4px;
  }
  .ns-toggle-info { display: flex; flex-direction: column; gap: 2px; }
  .ns-toggle-text { font-size: 12px; color: var(--t2); font-family: var(--ui); }
  .ns-toggle-hint { font-size: 10px; color: var(--t4); font-family: var(--ui); }
  .ns-toggle {
    width: 36px; height: 20px; border-radius: 10px; border: 1px solid var(--b1);
    background: var(--surface-hover); cursor: pointer; position: relative;
    transition: all 0.2s; padding: 0;
  }
  .ns-toggle.on { background: var(--acc); border-color: var(--acc); }
  .ns-toggle-knob {
    position: absolute; top: 2px; left: 2px; width: 14px; height: 14px;
    border-radius: 50%; background: var(--t3); transition: all 0.2s;
  }
  .ns-toggle.on .ns-toggle-knob { left: 18px; background: #fff; }
  /* Inline-expand pane below a card's icon+title row (e.g. Git fields,
     binary picker, attached contexts). Top divider separates it from
     the card's headline visually. */
  .ns-adv-body {
    display: flex; flex-direction: column; gap: 10px;
    margin-top: 14px;
    padding-top: 14px;
    border-top: 1px solid var(--b1);
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
  /* Click-outside catch for the dropdown. Stays visually invisible (no
     scrim) because we're already inside a modal — dimming the viewport
     a second time would imply a second modal. The transparent fill
     still receives clicks. z-index sits above sibling modal content
     but below the dropdown itself. */
  .ns-ctx-backdrop { position: fixed; inset: 0; z-index: var(--z-popover); background: transparent; }
  .ns-ctx-add-wrap { position: relative; }
  .ns-ctx-add-btn {
    display: flex; align-items: center; gap: 4px; padding: 4px 10px; border-radius: 4px;
    border: 1px dashed var(--b1); background: transparent; color: var(--t3);
    font-size: 11px; font-family: var(--ui); cursor: pointer; transition: all 0.1s;
  }
  .ns-ctx-add-btn:hover { border-color: var(--acc); color: var(--acc); }
  .ns-ctx-dropdown {
    position: absolute; top: calc(100% + 4px); left: 0; width: 250px;
    background: var(--modal-bg, var(--n)); border: 1px solid var(--b1); border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0,0,0,0.4); z-index: calc(var(--z-popover) + 1); max-height: 180px;
    overflow-y: auto; padding: 4px;
  }
  .ns-ctx-dd-item { padding: 6px 10px; border-radius: 4px; cursor: pointer; transition: background 0.1s; }
  .ns-ctx-dd-item:hover { background: var(--surface-hover); }
  .ns-ctx-dd-name { font-size: 12px; font-weight: 500; color: var(--t1); display: block; }
  .ns-ctx-dd-preview { font-size: 10px; color: var(--t3); display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ns-ctx-dd-empty { padding: 10px; text-align: center; font-size: 11px; color: var(--t3); }
  /* Full-width footer. Lives outside the two-column body so the
     border-top spans both rail + pane. */
  .ns-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 22px;
    border-top: 1px solid var(--b1);
    background: color-mix(in srgb, var(--e) 35%, transparent);
    flex-shrink: 0;
  }
  .ns-btn-cancel {
    height: 32px;
    padding: 0 18px;
    border-radius: 8px;
    font-size: 13px;
    cursor: pointer;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    transition: border-color 0.12s, color 0.12s;
  }
  .ns-btn-cancel:hover { border-color: var(--b2); color: var(--t1); }
  .ns-btn-create {
    height: 32px;
    padding: 0 20px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    border: none;
    background: var(--acc);
    color: #fff;
    font-family: var(--ui);
    transition: opacity 0.12s, filter 0.12s;
  }
  .ns-btn-create:hover:not(:disabled) { filter: brightness(1.08); }
  .ns-btn-create:disabled { opacity: 0.4; cursor: not-allowed; }
</style>

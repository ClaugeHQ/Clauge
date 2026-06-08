<script lang="ts">
  // Create / edit a coworker (persona). Same form serves both —
  // `existing` decides which command runs on save.
  //
  // Avatar UX: live preview reflects (seed, style). Re-roll = generate
  // a fresh seed (we use crypto.randomUUID for entropy, but any string
  // works — dicebear hashes the seed). Style switcher shows the four
  // most useful collections for "team member" vibes.

  import { onMount } from 'svelte';
  import {
    workspaceCoworkerCreate,
    workspaceCoworkerUpdate,
    workspaceCoworkerDelete,
    workspaceCoworkerList,
  } from '../commands';
  import { currentUserActor } from '../attribution';
  import { loadCoworkers } from '../stores';
  import type { WorkspaceCoworker } from '../types';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import CoworkerAvatar from './CoworkerAvatar.svelte';
  import { cloudPlan, upgradeModalOpen } from '$lib/stores/cloud';
  import {
    providerStatus,
    providerStatusReady,
    refreshProviderStatus,
  } from '$lib/shared/stores/providerStatus';
  import ProviderNotInstalledModal from '$lib/shared/agent/ProviderNotInstalledModal.svelte';
  import type { AgentProvider } from '$lib/shared/agent/providers';

  interface Props {
    show: boolean;
    /** When set, the modal is in EDIT mode for this coworker. */
    existing?: WorkspaceCoworker | null;
    onclose?: () => void;
    onsaved?: (cw: WorkspaceCoworker) => void;
  }

  let { show = $bindable(false), existing = null, onclose, onsaved }: Props = $props();

  let name = $state('');
  let role = $state('');
  let systemPrompt = $state('');
  let avatarSeed = $state('');
  let avatarStyle = $state('bottts');
  let provider = $state('claude');
  let saving = $state(false);
  let confirmingDelete = $state(false);
  let showProRequired = $state(false);
  let showProviderNotInstalled = $state(false);

  const PROVIDER_OPTIONS = [
    { id: 'claude', label: 'Claude', color: '#d4a96a' },
    { id: 'codex', label: 'Codex', color: '#10a37f' },
    { id: 'gemini', label: 'Antigravity', color: '#3186ff' },
    { id: 'opencode', label: 'OpenCode', color: '#f1ecec' },
  ];

  /** Available styles + their human labels for the picker. Only the
   *  collections we ship; user-friendly subset of dicebear's catalogue. */
  const STYLE_OPTIONS = [
    { id: 'personas',   label: 'Personas (cartoon)' },
    { id: 'avataaars',  label: 'Avataaars (Sketch)' },
    { id: 'bottts',     label: 'Bots' },
    { id: 'lorelei',    label: 'Lorelei' },
    { id: 'micah',      label: 'Micah' },
    { id: 'adventurer', label: 'Adventurer' },
    { id: 'big-smile',  label: 'Big smile' },
    { id: 'thumbs',     label: 'Thumbs (mascot)' },
  ];

  const isEdit = $derived(!!existing);
  /** Validation: name must be non-empty AND match the @-mention
   *  charset (lowercase letters only — keeps `@alex` but rejects
   *  `@Alex.dev` which would never match the @-popover regex). */
  const NAME_RX = /^[a-z]+$/;
  const nameError = $derived.by(() => {
    const n = name.trim();
    if (!n) return null; // empty handled by Save-disabled
    if (!NAME_RX.test(n)) return 'Use lowercase letters only — no spaces, digits, or punctuation.';
    return null;
  });
  const canSave = $derived(name.trim().length > 0 && !nameError && !saving);

  /** Auto-sanitise as the user types: strip anything that isn't a
   *  lowercase letter. Cheaper than a "you can't save" wall. */
  function onNameInput() {
    const cleaned = name.toLowerCase().replace(/[^a-z]/g, '');
    if (cleaned !== name) name = cleaned;
  }

  // Re-init the form when the modal opens with new data.
  $effect(() => {
    if (show) {
      name         = existing?.name ?? '';
      role         = existing?.role ?? '';
      systemPrompt = existing?.systemPrompt ?? '';
      avatarSeed   = existing?.avatarSeed ?? '';
      avatarStyle  = existing?.avatarStyle ?? 'bottts';
      provider     = existing?.provider ?? 'claude';
      confirmingDelete = false;
      showProRequired = false;
    }
  });

  /** Default seed when the user hasn't customised it: their typed name.
   *  Once they re-roll, the field becomes fixed (seed is preserved). */
  const effectiveSeed = $derived(
    avatarSeed.trim() || name.trim() || 'coworker',
  );

  function reroll() {
    avatarSeed = (typeof crypto !== 'undefined' && 'randomUUID' in crypto)
      ? crypto.randomUUID().slice(0, 12)
      : Math.random().toString(36).slice(2, 14);
  }

  async function save() {
    if (!canSave) return;
    if (!isEdit && $cloudPlan !== 'pro') {
      const all = await workspaceCoworkerList();
      const activeCount = all.filter((c) => c.disabledAt == null).length;
      if (activeCount >= 3) {
        showProRequired = true;
        return;
      }
    }
    // Pre-flight: surface the install guide here rather than at first
    // run when the coworker is actually invoked.
    if ($providerStatusReady && !$providerStatus[provider as AgentProvider]) {
      showProviderNotInstalled = true;
      return;
    }
    saving = true;
    try {
      const seedToSave = avatarSeed.trim() || name.trim();
      const cw = isEdit
        ? await workspaceCoworkerUpdate({
            id: existing!.id,
            name: name.trim(),
            role: role.trim(),
            systemPrompt: systemPrompt.trim(),
            provider,
            avatarSeed: seedToSave,
            avatarStyle,
          })
        : await workspaceCoworkerCreate({
            name: name.trim(),
            role: role.trim(),
            systemPrompt: systemPrompt.trim(),
            provider,
            avatarSeed: seedToSave,
            avatarStyle,
            actor: currentUserActor(),
          });
      await loadCoworkers();
      onsaved?.(cw);
      show = false;
    } catch (e) {
      errorToast('Save failed', e);
    } finally {
      saving = false;
    }
  }

  async function doDelete() {
    if (!isEdit) return;
    saving = true;
    try {
      await workspaceCoworkerDelete(existing!.id);
      await loadCoworkers();
      show = false;
    } catch (e) {
      errorToast('Delete failed', e);
    } finally {
      saving = false;
      confirmingDelete = false;
    }
  }

  function handleKey(e: KeyboardEvent) {
    if (!show) return;
    if (e.key === 'Escape') { e.preventDefault(); show = false; onclose?.(); }
  }

  /** Move the overlay to <body> so it renders relative to the viewport and
   *  DOM mutations inside the modal can't trigger repaints in the ancestor
   *  workspace layout (header, sidebar, footer). */
  function teleportToBody(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentElement === document.body) node.remove();
      },
    };
  }
</script>

<svelte:window onkeydown={handleKey} />

{#if show}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="cm-overlay" use:teleportToBody onclick={() => { show = false; onclose?.(); }}>
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="cm-modal modal-card" onclick={(e) => e.stopPropagation()}>
      <div class="cm-head">
        <h2>{isEdit ? 'Edit coworker' : 'New coworker'}</h2>
        <button class="cm-x" onclick={() => { show = false; onclose?.(); }}>×</button>
      </div>

      <div class="cm-body">
        <!-- ─── Avatar preview + actions ─── -->
        <div class="cm-avatar-row">
          <CoworkerAvatar seed={effectiveSeed} style={avatarStyle} size={92} ring />
          <div class="cm-avatar-actions">
            <label class="cm-field">
              <span class="cm-label">Avatar style</span>
              <select class="cm-input" bind:value={avatarStyle}>
                {#each STYLE_OPTIONS as opt}
                  <option value={opt.id}>{opt.label}</option>
                {/each}
              </select>
            </label>
            <button class="cm-btn-secondary" onclick={reroll} type="button">↻ Re-roll</button>
          </div>
        </div>

        <!-- ─── Name + role ─── -->
        <div class="cm-row">
          <label class="cm-field cm-field-grow">
            <span class="cm-label">
              Name <span class="cm-req">*</span>
              <span class="cm-hint">lowercase letters only</span>
            </span>
            <input
              class="cm-input"
              bind:value={name}
              oninput={onNameInput}
              placeholder="e.g. alex"
              spellcheck="false"
              autocapitalize="none"
              autofocus
            />
            {#if nameError}
              <span class="cm-error">{nameError}</span>
            {/if}
          </label>
          <label class="cm-field cm-field-grow">
            <span class="cm-label">Role / skill</span>
            <input
              class="cm-input"
              bind:value={role}
              placeholder="e.g. Code Reviewer, Technical Lead"
              spellcheck="false"
            />
          </label>
        </div>

        <!-- ─── System prompt ─── -->
        <label class="cm-field">
          <span class="cm-label">
            How should @{name.trim() || 'them'} behave?
            <span class="cm-hint">Appended to the agent's system prompt on every run.</span>
          </span>
          <textarea
            class="cm-textarea"
            bind:value={systemPrompt}
            placeholder="Focus on architecture and trade-offs. Ask clarifying questions before suggesting code. Be concise."
          ></textarea>
        </label>

        <!-- ─── Provider ─── -->
        <div class="cm-field">
          <span class="cm-label">Powered by</span>
          <div class="cm-provider-list">
            {#each PROVIDER_OPTIONS as p}
              {@const missing = $providerStatusReady && !$providerStatus[p.id as AgentProvider]}
              <button
                class="cm-provider-pill"
                class:selected={provider === p.id}
                class:missing
                title={missing ? `${p.label} not found on PATH` : `Use ${p.label}`}
                onclick={() => provider = p.id}
                type="button"
              >
                <span class="cm-provider-dot" style="background:{p.color}"></span>
                {p.label}
                {#if missing}<span class="cm-provider-missing">!</span>{/if}
              </button>
            {/each}
            <button
              class="cm-provider-refresh"
              title="Re-check installed CLIs"
              type="button"
              onclick={() => { void refreshProviderStatus(); }}
            >
              <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="23 4 23 10 17 10"/>
                <polyline points="1 20 1 14 7 14"/>
                <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
              </svg>
              Re-check
            </button>
          </div>
        </div>
      </div>

      {#if showProRequired}
        <div class="cm-pro-gate">
          <span class="cm-pro-gate-text">Free plan supports up to 3 coworkers.</span>
          <button
            class="cm-btn-primary"
            onclick={() => { upgradeModalOpen.set(true); showProRequired = false; show = false; }}
          >Upgrade to Pro</button>
          <button class="cm-btn-secondary" onclick={() => (showProRequired = false)}>Cancel</button>
        </div>
      {:else}
      <div class="cm-foot">
        {#if isEdit}
          {#if confirmingDelete}
            <span class="cm-confirm-text">Delete this coworker?</span>
            <button class="cm-btn-danger" onclick={doDelete} disabled={saving}>Delete</button>
            <button class="cm-btn-secondary" onclick={() => (confirmingDelete = false)}>Cancel</button>
          {:else}
            <button class="cm-btn-quiet" onclick={() => (confirmingDelete = true)}>Delete</button>
          {/if}
        {/if}
        <span class="cm-spacer"></span>
        <button class="cm-btn-secondary" onclick={() => { show = false; onclose?.(); }}>Cancel</button>
        <button class="cm-btn-primary" onclick={save} disabled={!canSave}>
          {saving ? 'Saving…' : isEdit ? 'Save' : 'Create coworker'}
        </button>
      </div>
      {/if}
    </div>
  </div>
{/if}

<ProviderNotInstalledModal
  bind:show={showProviderNotInstalled}
  provider={provider as AgentProvider}
/>

<style>
  .cm-overlay {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 300;
    display: flex; align-items: center; justify-content: center;
    animation: fadeIn 0.15s ease both;
  }
  @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }
  .cm-modal {
    width: 540px;
    max-width: 92%;
    max-height: 92vh;
    display: flex; flex-direction: column;
    animation: slideUp 0.18s ease both;
  }
  @keyframes slideUp { from { transform: translateY(8px); opacity: 0.6; } to { transform: none; opacity: 1; } }

  .cm-head {
    display: flex; align-items: center; justify-content: space-between;
    padding: 14px 18px;
    border-bottom: 1px solid var(--b1);
  }
  .cm-head h2 {
    margin: 0;
    font-family: var(--ui);
    font-size: 14.5px;
    font-weight: 600;
    color: var(--t1);
  }
  .cm-x {
    width: 28px; height: 28px;
    border: none; background: transparent;
    color: var(--t3); font-size: 20px; line-height: 1;
    border-radius: 5px; cursor: default;
  }
  .cm-x:hover { background: var(--surface-hover); color: var(--t1); }

  .cm-body {
    flex: 1;
    overflow-y: auto;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .cm-avatar-row {
    display: flex;
    gap: 16px;
    align-items: center;
    padding: 8px 12px;
    border: 1px dashed var(--b1);
    border-radius: 8px;
  }
  .cm-avatar-actions {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .cm-row { display: flex; gap: 12px; }
  .cm-field { display: flex; flex-direction: column; gap: 5px; }
  .cm-field-grow { flex: 1; min-width: 0; }
  .cm-label {
    font-family: var(--ui);
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: var(--t4);
    text-transform: uppercase;
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  .cm-req { color: var(--err, #f87171); }
  .cm-hint { color: var(--t4); font-weight: 500; letter-spacing: 0; text-transform: none; font-size: 10px; }
  .cm-error {
    margin-top: 3px;
    font-family: var(--ui);
    font-size: 10.5px;
    color: var(--err, #f87171);
  }

  .cm-input, .cm-textarea {
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 7px 10px;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 12px;
    outline: none;
    transition: border-color 0.12s;
    box-sizing: border-box;
    width: 100%;
  }
  .cm-input:focus, .cm-textarea:focus { border-color: var(--acc); }
  select.cm-input {
    -webkit-appearance: none;
    appearance: none;
    padding-right: 28px;
    cursor: default;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23b0b0c8' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'><polyline points='3 5 6 8 9 5'/></svg>");
    background-repeat: no-repeat;
    background-position: right 10px center;
    background-size: 10px 10px;
  }
  select.cm-input option {
    background: var(--n);
    color: var(--t1);
  }
  .cm-textarea {
    min-height: 100px;
    line-height: 1.5;
    resize: vertical;
  }

  .cm-provider-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .cm-provider-pill {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 6px 14px;
    border-radius: 20px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12px;
    cursor: default;
    transition: border-color 0.12s, background 0.12s, color 0.12s;
  }
  .cm-provider-pill:hover { border-color: var(--b2); color: var(--t1); }
  .cm-provider-pill.selected {
    border-color: var(--acc);
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    color: var(--t1);
  }
  .cm-provider-dot {
    width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0;
  }
  .cm-provider-pill.missing {
    color: var(--t3);
    border-style: dashed;
  }
  .cm-provider-missing {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--warn, #d97706) 22%, transparent);
    color: var(--warn, #d97706);
    font-size: 9px;
    font-weight: 700;
    line-height: 1;
  }
  .cm-provider-refresh {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 9px;
    border: 1px dashed var(--b1);
    background: transparent;
    border-radius: 14px;
    color: var(--t3);
    font: 10.5px var(--ui);
    cursor: pointer;
    transition: color 0.1s, border-color 0.1s;
  }
  .cm-provider-refresh:hover { color: var(--t1); border-color: var(--b2); }

  .cm-foot {
    display: flex; align-items: center; gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--b1);
  }
  .cm-spacer { flex: 1; }
  .cm-btn-primary, .cm-btn-secondary, .cm-btn-quiet, .cm-btn-danger {
    height: 30px;
    padding: 0 14px;
    border-radius: 6px;
    font-family: var(--ui);
    font-size: 12px;
    cursor: default;
    transition: opacity 0.12s, border-color 0.12s, color 0.12s, background 0.12s;
  }
  .cm-btn-primary {
    border: none; background: var(--acc); color: #fff; font-weight: 600;
  }
  .cm-btn-primary:hover:not(:disabled) { opacity: 0.9; }
  .cm-btn-primary:disabled { opacity: 0.4; }
  .cm-btn-secondary {
    border: 1px solid var(--b2); background: transparent; color: var(--t2);
  }
  .cm-btn-secondary:hover:not(:disabled) { color: var(--t1); border-color: var(--acc); }
  .cm-btn-quiet {
    border: none; background: transparent; color: var(--t3); font-size: 11.5px;
  }
  .cm-btn-quiet:hover { color: var(--err, #f87171); }
  .cm-btn-danger {
    border: 1px solid var(--err, #f87171);
    background: transparent;
    color: var(--err, #f87171);
    font-weight: 600;
  }
  .cm-btn-danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--err, #f87171) 14%, transparent);
  }
  .cm-confirm-text {
    font-family: var(--ui);
    font-size: 11.5px;
    color: var(--t2);
  }
  .cm-pro-gate {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--b1);
    background: color-mix(in srgb, var(--acc) 6%, transparent);
  }
  .cm-pro-gate-text {
    flex: 1;
    font-family: var(--ui);
    font-size: 12px;
    color: var(--t2);
  }
</style>

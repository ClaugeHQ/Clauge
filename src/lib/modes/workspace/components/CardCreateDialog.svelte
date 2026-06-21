<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import TagInput from './TagInput.svelte';
  import GhNotInstalledModal from './GhNotInstalledModal.svelte';
  import GlabNotInstalledModal from './GlabNotInstalledModal.svelte';
  import {
    workspaceCardCreate,
    workspaceCardPushToRepo,
    workspaceCloudTarget,
  } from '../commands';
  import type { CloudTarget } from '../types';
  import { currentUserActor } from '../attribution';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { imagePaste, insertAtCaret, pickImageFile, imageMarkdown } from '../imageAttach';

  interface Props {
    columnId: string;
    boardId: string;
    position?: number;
    initialTitle?: string;
    onclose?: () => void;
    oncreated?: () => void;
  }
  let { columnId, boardId, position = 0, initialTitle = '', onclose, oncreated }: Props = $props();

  let show = $state(true);
  let title = $state(initialTitle);
  let descEl = $state<HTMLTextAreaElement>();
  async function addImage() {
    const f = await pickImageFile();
    if (!f || !descEl) return;
    try {
      const md = await imageMarkdown(f);
      if (md) description = insertAtCaret(descEl, description, md);
    } catch (e) { showToast(friendlyError(e), 'error'); }
  }
  let description = $state('');
  let priority = $state<string | null>(null);
  let tags = $state<string[]>([]);
  let mode = $state<'local' | 'cloud'>('local');
  let creating = $state(false);

  let target = $state<CloudTarget | null>(null);
  let showGh = $state(false);
  let showGlab = $state(false);

  const providerLabel = $derived(
    target?.provider === 'github' ? 'GitHub' : target?.provider === 'gitlab' ? 'GitLab' : 'cloud',
  );
  /** Cloud creation is only possible when a supported repo is configured
   *  AND the matching CLI is installed. */
  const cloudReady = $derived(
    !!target?.repoConfigured &&
      (target?.provider === 'github' || target?.provider === 'gitlab') &&
      !!target?.toolInstalled,
  );
  /** Why cloud is unavailable — drives the inline hint + action. */
  const cloudBlocker = $derived.by<null | 'no-repo' | 'unsupported' | 'no-tool'>(() => {
    if (!target) return null;
    if (!target.repoConfigured) return 'no-repo';
    if (target.provider !== 'github' && target.provider !== 'gitlab') return 'unsupported';
    if (!target.toolInstalled) return 'no-tool';
    return null;
  });

  onMount(async () => {
    try {
      target = await workspaceCloudTarget(boardId);
    } catch {
      target = null;
    }
  });

  function openToolInstall() {
    if (target?.tool === 'gh') showGh = true;
    else if (target?.tool === 'glab') showGlab = true;
  }

  function close() {
    show = false;
    onclose?.();
  }

  async function create() {
    const t = title.trim();
    if (!t || creating) return;
    creating = true;
    try {
      const card = await workspaceCardCreate({
        columnId,
        title: t,
        description: description.trim() || undefined,
        priority: priority || undefined,
        tags,
        position,
        actor: currentUserActor(),
      });
      if (mode === 'cloud' && cloudReady) {
        try {
          const r = await workspaceCardPushToRepo(card.id, currentUserActor());
          showToast(`Created ${providerLabel} issue ${r.externalId}`, 'success');
        } catch (e) {
          // Card was created locally; only the issue push failed. Surface
          // the right guidance and leave the card so no work is lost.
          const msg = `${e}`;
          if (/^gh is not installed/.test(msg)) showGh = true;
          else if (/^glab is not installed/.test(msg)) showGlab = true;
          else showToast(`Card created locally, but ${providerLabel} issue failed: ${friendlyError(e)}`, 'error');
        }
      } else {
        showToast('Card created', 'success');
      }
      oncreated?.();
      close();
    } catch (e) {
      showToast(`Create failed: ${friendlyError(e)}`, 'error');
    } finally {
      creating = false;
    }
  }
</script>

<Modal bind:show title="New ticket" width="520px" onclose={close}>
  <div class="cc">
    <label class="cc-field">
      <span class="cc-label">Title</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="cc-input" type="text" bind:value={title} placeholder="What needs doing?" autofocus spellcheck="false" />
    </label>

    <div class="cc-field">
      <div class="cc-label-row">
        <span class="cc-label">Description</span>
        <button type="button" class="cc-img-btn" onclick={addImage} title="Attach image (or paste a screenshot)">
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="M21 15l-5-5L5 21"/></svg>
          Image
        </button>
      </div>
      <textarea class="cc-textarea" bind:value={description} bind:this={descEl} use:imagePaste={(md) => { if (descEl) description = insertAtCaret(descEl, description, md); }} rows="5" placeholder="Details. Markdown supported. Paste or attach images." spellcheck="false"></textarea>
    </div>

    <div class="cc-row">
      <label class="cc-field cc-field-sm">
        <span class="cc-label">Priority</span>
        <select class="cc-input cc-select" bind:value={priority}>
          <option value={null}>—</option>
          <option value="P0">P0</option>
          <option value="P1">P1</option>
          <option value="P2">P2</option>
          <option value="P3">P3</option>
        </select>
      </label>
      <div class="cc-field cc-field-grow">
        <span class="cc-label">Tags</span>
        <TagInput bind:value={tags} />
      </div>
    </div>

    <!-- Where to create -->
    <div class="cc-field">
      <span class="cc-label">Create as</span>
      <div class="cc-seg">
        <button class="cc-seg-btn" class:on={mode === 'local'} onclick={() => (mode = 'local')}>
          Local card
        </button>
        <button
          class="cc-seg-btn"
          class:on={mode === 'cloud'}
          disabled={!cloudReady}
          onclick={() => cloudReady && (mode = 'cloud')}
          title={cloudReady ? `Creates a real ${providerLabel} issue` : 'Cloud unavailable'}
        >
          {providerLabel} issue
        </button>
      </div>
      {#if cloudBlocker === 'no-repo'}
        <div class="cc-hint">No repo linked to this workspace — set a repo URL to create issues. Saving as a local card.</div>
      {:else if cloudBlocker === 'unsupported'}
        <div class="cc-hint">This repo isn't GitHub or GitLab — only local cards are supported here.</div>
      {:else if cloudBlocker === 'no-tool'}
        <div class="cc-hint">
          {target?.tool} isn't installed, so {providerLabel} issues can't be created.
          <button class="cc-hint-link" onclick={openToolInstall}>How to install</button>
        </div>
      {:else if mode === 'cloud'}
        <div class="cc-hint cc-hint-ok">Creates a real {providerLabel} issue and links this card to it.</div>
      {/if}
    </div>

    <div class="cc-actions">
      <button class="cc-btn" onclick={close}>Cancel</button>
      <button class="cc-btn primary" onclick={create} disabled={!title.trim() || creating}>
        {creating ? 'Creating…' : mode === 'cloud' && cloudReady ? `Create ${providerLabel} issue` : 'Create card'}
      </button>
    </div>
  </div>
</Modal>

<GhNotInstalledModal bind:show={showGh} />
<GlabNotInstalledModal bind:show={showGlab} />

<style>
  .cc { display: flex; flex-direction: column; gap: 14px; }
  .cc-field { display: flex; flex-direction: column; gap: 5px; }
  .cc-row { display: flex; gap: 12px; }
  .cc-field-sm { min-width: 90px; }
  .cc-field-grow { flex: 1; min-width: 0; }
  .cc-label {
    font-family: var(--ui); font-size: 10px; font-weight: 700;
    letter-spacing: 0.06em; text-transform: uppercase; color: var(--t4);
  }
  .cc-label-row { display: flex; align-items: center; justify-content: space-between; }
  .cc-img-btn {
    display: inline-flex; align-items: center; gap: 4px;
    border: 1px solid var(--b1); background: transparent; color: var(--t3);
    font-family: var(--ui); font-size: 10px; padding: 2px 7px; border-radius: 5px; cursor: pointer;
    transition: color 0.12s, border-color 0.12s;
  }
  .cc-img-btn:hover { color: var(--t1); border-color: var(--acc); }
  .cc-input, .cc-textarea {
    background: var(--surface-hover); border: 1px solid var(--b1);
    border-radius: 6px; padding: 8px 10px; color: var(--t1);
    font-family: var(--ui); font-size: 13px; outline: none;
    transition: border-color 0.12s; box-sizing: border-box; width: 100%;
  }
  .cc-textarea { resize: vertical; min-height: 96px; line-height: 1.5; }
  .cc-input:focus, .cc-textarea:focus { border-color: var(--acc); }
  .cc-select { -webkit-appearance: none; appearance: none; font-family: var(--mono); font-size: 12px; }
  .cc-seg {
    display: inline-flex; border: 1px solid var(--b1); border-radius: 6px; overflow: hidden; width: fit-content;
  }
  .cc-seg-btn {
    padding: 6px 14px; border: none; background: transparent; color: var(--t3);
    font-family: var(--ui); font-size: 12px; font-weight: 600; cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }
  .cc-seg-btn + .cc-seg-btn { border-left: 1px solid var(--b1); }
  .cc-seg-btn.on { background: var(--acc); color: #fff; }
  .cc-seg-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .cc-hint { font-family: var(--ui); font-size: 11px; color: var(--t3); line-height: 1.45; }
  .cc-hint-ok { color: var(--acc); }
  .cc-hint-link {
    border: none; background: transparent; color: var(--acc);
    font-family: var(--ui); font-size: 11px; cursor: pointer; padding: 0; text-decoration: underline;
  }
  .cc-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 2px; }
  .cc-btn {
    padding: 7px 14px; border-radius: 6px; border: 1px solid var(--b1);
    background: transparent; color: var(--t2); font-family: var(--ui);
    font-size: 12.5px; cursor: pointer; transition: background 0.12s, color 0.12s, border-color 0.12s, filter 0.12s;
  }
  .cc-btn:hover:not(:disabled) { background: var(--surface-hover); color: var(--t1); }
  .cc-btn.primary { background: var(--acc); border-color: var(--acc); color: #fff; }
  .cc-btn.primary:hover:not(:disabled) { filter: brightness(1.1); }
  .cc-btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>

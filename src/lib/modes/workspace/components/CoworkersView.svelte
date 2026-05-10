<script lang="ts">
  // Workspace main panel — Co-workers grid. Shows every persona the
  // user has set up; clicking a card opens it for edit. The first tile
  // is the "+ New coworker" affordance.
  //
  // Empty state explains the concept (one paragraph, no fluff): a
  // coworker is a named persona that drives an agent under the hood.
  // Tag them on cards instead of generic @claude.

  import { onMount } from 'svelte';
  import { coworkers, loadCoworkers } from '../stores';
  import CoworkerAvatar from './CoworkerAvatar.svelte';
  import CoworkerModal from './CoworkerModal.svelte';
  import type { WorkspaceCoworker } from '../types';

  let modalOpen = $state(false);
  let editing = $state<WorkspaceCoworker | null>(null);

  onMount(() => { loadCoworkers(); });

  function openNew() {
    editing = null;
    modalOpen = true;
  }
  function openEdit(cw: WorkspaceCoworker) {
    editing = cw;
    modalOpen = true;
  }
</script>

<div class="cv">
  <header class="cv-head">
    <div class="cv-head-row">
      <span class="cv-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="9" cy="8" r="3.5"/>
          <path d="M2.5 19a6.5 6.5 0 0 1 13 0"/>
          <circle cx="17" cy="6" r="2.6"/>
          <path d="M14 13a4.5 4.5 0 0 1 8.5 2"/>
        </svg>
      </span>
      <h1 class="cv-title">Co-workers</h1>
      <span class="cv-count">{$coworkers.length} {$coworkers.length === 1 ? 'persona' : 'personas'}</span>
      <button class="cv-new" onclick={openNew}>+ New coworker</button>
    </div>
    <p class="cv-sub">Agent personas with custom roles and instructions. Assign them to cards to get focused, consistent responses.</p>
  </header>

  <div class="cv-body">
    {#if $coworkers.length === 0}
      <div class="cv-empty">
        <svg viewBox="0 0 24 24" width="42" height="42" fill="none" stroke="var(--t4)" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="9" cy="8" r="3.5"/><path d="M2.5 19a6.5 6.5 0 0 1 13 0"/><circle cx="17" cy="6" r="2.6"/><path d="M14 13a4.5 4.5 0 0 1 8.5 2"/>
        </svg>
        <h3>No coworkers yet</h3>
        <p>Create a persona, give it a role and instructions, then assign it to a card.</p>
        <button class="cv-cta" onclick={openNew}>+ Create your first coworker</button>
      </div>
    {:else}
      <div class="cv-grid">
        <!-- "Add" tile -->
        <button class="cv-tile cv-tile-add" onclick={openNew}>
          <span class="cv-tile-add-plus">+</span>
          <span class="cv-tile-add-label">New coworker</span>
        </button>

        {#each $coworkers as cw (cw.id)}
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
          <button class="cv-tile" onclick={() => openEdit(cw)}>
            <CoworkerAvatar seed={cw.avatarSeed} style={cw.avatarStyle} size={64} ring />
            <div class="cv-tile-name">@{cw.name}</div>
            {#if cw.role}
              <div class="cv-tile-role">{cw.role}</div>
            {/if}
            {#if cw.systemPrompt}
              <div class="cv-tile-prompt">{cw.systemPrompt}</div>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<CoworkerModal bind:show={modalOpen} existing={editing} />

<style>
  .cv { flex: 1; display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
  .cv-head {
    flex-shrink: 0;
    padding: 16px 22px 14px;
    border-bottom: 1px solid var(--b1);
    background: var(--n2);
  }
  .cv-head-row { display: flex; align-items: center; gap: 10px; }
  .cv-icon { color: var(--acc); display: inline-flex; }
  .cv-title { margin: 0; font-size: 16px; font-weight: 600; color: var(--t1); font-family: var(--ui); }
  .cv-count { font-size: 11px; color: var(--t3); font-family: var(--ui); }
  .cv-new {
    margin-left: auto;
    height: 28px; padding: 0 14px; border-radius: 6px;
    border: none; background: var(--acc); color: #fff;
    font-family: var(--ui); font-size: 12px; font-weight: 600;
    cursor: default;
  }
  .cv-new:hover { opacity: 0.9; }
  .cv-sub { margin: 8px 0 0; font-size: 11.5px; color: var(--t3); font-family: var(--ui); line-height: 1.55; max-width: 720px; }
  .cv-sub strong { color: var(--t2); font-weight: 600; }

  .cv-body { flex: 1; overflow-y: auto; min-height: 0; padding: 18px 22px 28px; }

  .cv-empty {
    display: flex; flex-direction: column; align-items: center; gap: 10px;
    padding: 60px 40px; color: var(--t3); text-align: center;
  }
  .cv-empty h3 { margin: 6px 0 0; font-size: 14px; font-weight: 600; color: var(--t2); font-family: var(--ui); }
  .cv-empty p { margin: 0; font-size: 12px; color: var(--t3); font-family: var(--ui); max-width: 420px; line-height: 1.6; }
  .cv-cta {
    margin-top: 6px;
    padding: 8px 18px; border-radius: 8px;
    border: 1px solid var(--acc);
    background: color-mix(in srgb, var(--acc) 18%, transparent);
    color: var(--t1);
    font-size: 12.5px; font-family: var(--ui); font-weight: 500;
    cursor: default;
  }
  .cv-cta:hover { background: color-mix(in srgb, var(--acc) 28%, transparent); }

  .cv-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 12px;
  }
  .cv-tile {
    border: 1px solid var(--b1);
    background: rgba(255, 255, 255, 0.02);
    border-radius: 10px;
    padding: 16px 14px 14px;
    display: flex; flex-direction: column; align-items: flex-start; gap: 8px;
    cursor: default;
    text-align: left;
    transition: border-color 0.12s, background 0.12s;
  }
  .cv-tile:hover { border-color: var(--acc); background: rgba(255,255,255,0.04); }
  .cv-tile-name {
    font-family: var(--ui);
    font-size: 13.5px;
    font-weight: 600;
    color: var(--t1);
  }
  .cv-tile-role {
    font-family: var(--ui);
    font-size: 11px;
    color: var(--acc);
    font-weight: 500;
  }
  .cv-tile-prompt {
    font-family: var(--ui);
    font-size: 11px;
    color: var(--t3);
    line-height: 1.5;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .cv-tile-add {
    border: 1px dashed var(--b1);
    background: transparent;
    color: var(--t3);
    align-items: center;
    justify-content: center;
    min-height: 156px;
  }
  .cv-tile-add:hover {
    border-color: var(--acc);
    color: var(--t1);
    background: color-mix(in srgb, var(--acc) 4%, transparent);
  }
  .cv-tile-add-plus { font-size: 28px; line-height: 1; font-weight: 300; }
  .cv-tile-add-label { font-family: var(--ui); font-size: 12px; }
</style>

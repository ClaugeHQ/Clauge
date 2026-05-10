<script lang="ts">
  // Inbox — items recently mutated by an agent (any `updated_by` not
  // starting with 'user'). Reads workspace_inbox_list once per visit
  // plus on a refresh button. Click an item → opens its tab.

  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { workspaceInboxList } from '../commands';
  import type { InboxItem } from '../types';
  import { describeActor, formatAttribution } from '../attribution';
  import { tabs as sharedTabs, addTab, activateTab } from '$lib/shared/stores/tabs';
  import { activeWorkspaceId, markInboxRead } from '../stores';
  import { mode } from '$lib/stores/app';

  let items = $state<InboxItem[]>([]);
  let loading = $state(true);
  let lastLoaded = $state<number | null>(null);

  async function refresh() {
    loading = true;
    try {
      items = await workspaceInboxList(50);
      lastLoaded = Date.now();
      // Anything visible in the inbox is "seen" — bump lastReadAt so
      // the side-panel count clears.
      markInboxRead();
    } catch (e) {
      console.error('inbox load failed:', e);
    } finally {
      loading = false;
    }
  }

  onMount(refresh);

  function open(item: InboxItem) {
    activeWorkspaceId.set(item.workspaceId);
    // Cards open the parent board; notes open themselves.
    const kind = item.kind === 'card' ? 'board' : 'note';
    const id = item.kind === 'card' ? item.boardId! : item.id;
    const label = item.kind === 'card' ? (item.boardName ?? 'Board') : item.label;
    const dot = item.kind === 'card' ? '#a78bfa' : 'var(--acc)';
    const key = `${kind}:${id}`;
    const existing = get(sharedTabs).find(t => t.mode === 'workspace' && t.key === key);
    if (existing) activateTab(existing.id);
    else addTab(label, 'workspace', key, dot);
    mode.set('workspace');
  }

  function relTime(iso: string): string {
    const d = new Date(iso).getTime();
    if (isNaN(d)) return '';
    const diff = Date.now() - d;
    if (diff < 60_000) return 'just now';
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
    if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
    return `${Math.floor(diff / 86_400_000)}d ago`;
  }
</script>

<div class="ib">
  <header class="ib-head">
    <div class="ib-head-row">
      <span class="ib-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/></svg>
      </span>
      <h1 class="ib-title">Inbox</h1>
      <span class="ib-count">{items.length} item{items.length === 1 ? '' : 's'}</span>
      <button class="ib-refresh" onclick={refresh} disabled={loading} title="Refresh">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" class:spinning={loading}><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
      </button>
    </div>
    <p class="ib-sub">Items recently created or updated by an agent. Click to open.</p>
  </header>

  <div class="ib-body">
    {#if loading && items.length === 0}
      <div class="ib-empty">Loading…</div>
    {:else if items.length === 0}
      <div class="ib-empty-state">
        <svg viewBox="0 0 24 24" width="42" height="42" fill="none" stroke="var(--t4)" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/></svg>
        <h3>Nothing new</h3>
        <p>When an agent edits a note or card, it'll show up here.</p>
      </div>
    {:else}
      <div class="ib-list">
        {#each items as it (it.kind + ':' + it.id)}
          {@const actor = describeActor(it.updatedBy)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="ib-row" onclick={() => open(it)}>
            <span class="ib-row-ico" class:ib-row-board={it.kind === 'card'}>
              {#if it.kind === 'card'}
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="5" height="16" rx="1"/><rect x="9.5" y="4" width="5" height="9" rx="1"/><rect x="16" y="4" width="5" height="13" rx="1"/></svg>
              {:else}
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><polyline points="14 3 14 9 20 9"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="13" y2="17"/></svg>
              {/if}
            </span>
            <div class="ib-row-body">
              <div class="ib-row-top">
                <span class="ib-row-label">{it.label}</span>
                <span class="ib-row-time">{relTime(it.updatedAt)}</span>
              </div>
              <div class="ib-row-meta">
                <span class="ib-row-actor">
                  <svg viewBox="0 0 24 24" width="9" height="9" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l1.6 4.8L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.2L12 3z"/></svg>
                  {actor.label}
                </span>
                <span class="ib-row-sep">·</span>
                <span class="ib-row-ws">{it.workspaceName}</span>
                {#if it.boardName}
                  <span class="ib-row-sep">/</span>
                  <span class="ib-row-board-name">{it.boardName}</span>
                {/if}
              </div>
            </div>
            <span class="ib-row-arrow">→</span>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .ib { flex: 1; display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
  .ib-head {
    flex-shrink: 0;
    padding: 16px 22px 14px;
    border-bottom: 1px solid var(--b1);
    background: var(--n2);
  }
  .ib-head-row { display: flex; align-items: center; gap: 10px; }
  .ib-icon { color: var(--acc); display: inline-flex; }
  .ib-title { margin: 0; font-size: 16px; font-weight: 600; color: var(--t1); font-family: var(--ui); }
  .ib-count { margin-left: auto; font-size: 11px; color: var(--t3); font-family: var(--ui); }
  .ib-refresh {
    width: 26px; height: 26px; border-radius: 5px; border: none; background: transparent; color: var(--t3);
    display: inline-flex; align-items: center; justify-content: center; cursor: default;
    transition: background 0.1s, color 0.1s;
  }
  .ib-refresh:hover:not(:disabled) { background: rgba(255, 255, 255, 0.06); color: var(--t1); }
  .ib-refresh:disabled { opacity: 0.5; }
  .spinning { animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .ib-sub { margin: 6px 0 0; font-size: 11.5px; color: var(--t3); font-family: var(--ui); line-height: 1.5; }

  .ib-body { flex: 1; overflow: auto; min-height: 0; }
  .ib-body::-webkit-scrollbar { width: 4px; }
  .ib-body::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }
  .ib-empty {
    padding: 40px; color: var(--t4); font-family: var(--ui); font-size: 12px; text-align: center;
  }
  .ib-empty-state {
    display: flex; flex-direction: column; align-items: center; gap: 10px;
    padding: 60px 40px; color: var(--t3); text-align: center;
  }
  .ib-empty-state h3 { margin: 6px 0 0; font-size: 14px; font-weight: 600; color: var(--t2); font-family: var(--ui); }
  .ib-empty-state p { margin: 0; font-size: 12px; color: var(--t3); font-family: var(--ui); max-width: 360px; line-height: 1.6; }

  .ib-list { display: flex; flex-direction: column; }
  .ib-row {
    display: flex; align-items: flex-start; gap: 12px;
    padding: 12px 22px; cursor: default; transition: background 0.08s;
    border-bottom: 1px solid var(--b1);
  }
  .ib-row:hover { background: var(--n2); }
  .ib-row-ico {
    flex-shrink: 0; width: 18px; height: 18px;
    display: inline-flex; align-items: center; justify-content: center;
    color: var(--acc); margin-top: 2px;
  }
  .ib-row-ico.ib-row-board { color: #a78bfa; }
  .ib-row-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .ib-row-top { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
  .ib-row-label {
    flex: 1; font-size: 12.5px; color: var(--t1); font-family: var(--ui); font-weight: 500;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .ib-row-time { flex-shrink: 0; font-size: 10.5px; color: var(--t4); font-family: var(--mono); }
  .ib-row-meta { display: flex; align-items: center; gap: 5px; font-size: 10.5px; color: var(--t3); font-family: var(--ui); }
  .ib-row-actor {
    display: inline-flex; align-items: center; gap: 4px;
    color: var(--acc); font-weight: 500;
  }
  .ib-row-sep { color: var(--t4); }
  .ib-row-ws { color: var(--t3); }
  .ib-row-board-name { color: var(--t2); }
  .ib-row-arrow { color: var(--t4); flex-shrink: 0; align-self: center; transition: color 0.1s, transform 0.12s; }
  .ib-row:hover .ib-row-arrow { color: var(--acc); transform: translateX(2px); }
</style>

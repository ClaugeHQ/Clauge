<script lang="ts">
  // Hardcoded "AI Meeting Notes" accordion above the user workspaces —
  // same ncoll header/body idiom as WorkspaceItem. Rows open a
  // `meeting:<id>` workspace tab (MeetingView routes off it in
  // WorkspacePanel).

  import { get } from 'svelte/store';
  import { meetings, recordingStatus, loadMeetings, openMeetingTab } from '../stores';
  import {
    workspaceMeetingStart,
    workspaceMeetingStop,
    workspaceMeetingUpdateTitle,
    workspaceMeetingDelete,
    MEETING_MODEL_MISSING,
  } from '../commands';
  import { relativeTime } from '../attribution';
  import type { WorkspaceMeeting } from '../types';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast } from '$lib/utils/errors';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import InlineInput from '$lib/components/nav/InlineInput.svelte';
  import { tabs as sharedTabs, updateTab, closeTab } from '$lib/shared/stores/tabs';

  interface Props {
    searchQuery?: string;
  }

  let { searchQuery = '' }: Props = $props();

  let expanded = $state(false);
  let starting = $state(false);
  let stopping = $state(false);
  let renamingId = $state<string | null>(null);
  let deleteTarget = $state<WorkspaceMeeting | null>(null);
  let showDeleteConfirm = $state(false);

  const filtered = $derived(
    searchQuery.trim()
      ? $meetings.filter(m => m.title.toLowerCase().includes(searchQuery.toLowerCase()))
      : $meetings,
  );

  const isRecording = $derived($recordingStatus.recording);
  const isStopping = $derived($recordingStatus.stopping);

  // 1s tick drives the live duration of the active recording row.
  let now = $state(Date.now());
  $effect(() => {
    if (!isRecording) return;
    const t = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(t);
  });

  function fmtDuration(m: WorkspaceMeeting): string {
    const start = new Date(m.startedAt).getTime();
    if (isNaN(start)) return '';
    const live = m.status === 'recording' && $recordingStatus.meetingId === m.id;
    const end = m.endedAt ? new Date(m.endedAt).getTime() : (live ? now : NaN);
    if (isNaN(end)) return '';
    const secs = Math.max(0, Math.floor((end - start) / 1000));
    const h = Math.floor(secs / 3600);
    const mm = Math.floor((secs % 3600) / 60);
    const ss = secs % 60;
    if (h > 0) return `${h}h ${mm}m`;
    if (mm > 0) return `${mm}m ${ss}s`;
    return `${ss}s`;
  }

  function subline(m: WorkspaceMeeting): string {
    const when = relativeTime(m.startedAt);
    const dur = fmtDuration(m);
    return dur ? `${when} · ${dur}` : when;
  }

  async function handleRecordBtn(e: MouseEvent) {
    e.stopPropagation();
    if (isStopping || stopping || starting) return;
    if (isRecording) {
      stopping = true;
      try {
        await workspaceMeetingStop();
      } catch (err) {
        errorToast('Failed to stop recording', err);
      } finally {
        stopping = false;
      }
      return;
    }
    starting = true;
    try {
      const id = await workspaceMeetingStart({});
      expanded = true;
      await loadMeetings();
      const m = get(meetings).find(x => x.id === id);
      if (m) openMeetingTab(m);
    } catch (err) {
      if (String(err).includes(MEETING_MODEL_MISSING)) {
        showToast('Download a transcription model in Settings first', 'error');
      } else {
        errorToast('Failed to start recording', err);
      }
    } finally {
      starting = false;
    }
  }

  async function handleRename(m: WorkspaceMeeting, newTitle: string) {
    renamingId = null;
    const trimmed = newTitle.trim();
    if (!trimmed || trimmed === m.title) return;
    try {
      await workspaceMeetingUpdateTitle(m.id, trimmed);
      await loadMeetings();
      const t = get(sharedTabs).find(x => x.mode === 'workspace' && x.key === `meeting:${m.id}`);
      if (t) updateTab(t.id, { label: trimmed });
    } catch (err) {
      errorToast('Rename failed', err);
    }
  }

  function requestDelete(m: WorkspaceMeeting) {
    const rs = $recordingStatus;
    if (rs.meetingId === m.id && (rs.recording || rs.stopping)) {
      showToast('Stop the recording first', 'error');
      return;
    }
    deleteTarget = m;
    showDeleteConfirm = true;
  }

  async function confirmDelete() {
    const m = deleteTarget;
    if (!m) return;
    try {
      await workspaceMeetingDelete(m.id);
      const t = get(sharedTabs).find(x => x.mode === 'workspace' && x.key === `meeting:${m.id}`);
      if (t) closeTab(t.id);
      await loadMeetings();
      showToast(`Deleted "${m.title || 'Untitled meeting'}"`, 'success');
    } catch (err) {
      errorToast('Delete failed', err);
    }
    deleteTarget = null;
  }

  function showRowMenu(e: MouseEvent, m: WorkspaceMeeting) {
    e.preventDefault();
    e.stopPropagation();
    showContextMenu(e.clientX, e.clientY, [
      {
        label: 'Open',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M15 3h6v6"/><path d="M10 14L21 3"/><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/></svg>',
        action: () => openMeetingTab(m),
      },
      {
        label: 'Rename',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
        action: () => { renamingId = m.id; },
      },
      { label: '', action: () => {}, separator: true },
      {
        label: 'Delete',
        danger: true,
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/></svg>',
        action: () => requestDelete(m),
      },
    ]);
  }

  const subText = $derived(
    `${$meetings.length} meeting${$meetings.length === 1 ? '' : 's'}`,
  );
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="ncoll">
  <div class="ncoll-hdr" onclick={() => { expanded = !expanded; }}>
    <div class="coll-icon coll-icon-accent">
      <svg viewBox="0 0 24 24"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" y1="19" x2="12" y2="22"/></svg>
    </div>
    <div class="ncoll-text">
      <div class="ncoll-row-top">
        <span class="ncoll-name">AI Meeting Notes</span>
        {#if isRecording}
          <span class="mtg-rec-dot" title="Recording"></span>
        {/if}
      </div>
      <div class="ncoll-row-bot">
        <span class="ncoll-sub">{subText}</span>
      </div>
    </div>
    <button
      class="coll-add"
      class:mtg-stop={isRecording}
      class:mtg-busy={isStopping || stopping || starting}
      title={isStopping || stopping ? 'Stopping…' : isRecording ? 'Stop recording' : 'Record meeting now'}
      disabled={isStopping || stopping || starting}
      onclick={handleRecordBtn}
    >
      {#if isStopping || stopping || starting}
        <svg class="mtg-spin" viewBox="0 0 24 24"><path d="M21 12a9 9 0 1 1-6.2-8.56"/></svg>
      {:else if isRecording}
        <svg viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>
      {:else}
        <svg viewBox="0 0 24 24"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" y1="19" x2="12" y2="22"/></svg>
      {/if}
    </button>
    <svg class="ncoll-arr" class:open={expanded} viewBox="0 0 24 24">
      <path d="M9 18l6-6-6-6" stroke="currentColor" fill="none" stroke-width="1.8" stroke-linecap="round"/>
    </svg>
  </div>

  <div
    class="ncoll-body"
    style="max-height:{expanded ? Math.max(filtered.length, 1) * 44 + 24 + 'px' : '0'}"
  >
    {#each filtered as m (m.id)}
      <div class="ws-leaf" onclick={() => openMeetingTab(m)} oncontextmenu={(e) => showRowMenu(e, m)}>
        <span
          class="mtg-dot"
          class:mtg-dot-rec={m.status === 'recording'}
          class:mtg-dot-ready={m.status === 'notes_ready'}
          aria-hidden="true"
        ></span>
        {#if renamingId === m.id}
          <InlineInput
            value={m.title}
            placeholder="Meeting title…"
            onsubmit={(v) => handleRename(m, v)}
            oncancel={() => renamingId = null}
          />
        {:else}
          <span class="mtg-leaf-text">
            <span class="ws-leaf-name">{m.title || 'Untitled meeting'}</span>
            <span class="mtg-leaf-sub">{subline(m)}</span>
          </span>
        {/if}
      </div>
    {/each}

    {#if expanded && filtered.length === 0}
      <div class="ws-leaf-empty">
        {#if searchQuery.trim()}
          No meetings match "{searchQuery}"
        {:else}
          Meetings you record appear here.
        {/if}
      </div>
    {/if}
  </div>
</div>

<ConfirmDialog
  bind:show={showDeleteConfirm}
  title="Delete meeting"
  message={`Delete "${deleteTarget?.title || 'Untitled meeting'}"? The transcript and notes will be removed. This cannot be undone.`}
  confirmText="Delete"
  onconfirm={confirmDelete}
/>

<style>
  .ncoll {
    border-bottom: 1px solid var(--b1);
  }
  .ncoll-hdr {
    min-height: 44px;
    padding: 6px 8px;
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    transition: background 0.1s;
    user-select: none;
    position: relative;
  }
  .ncoll-hdr:hover { background: var(--n2); }
  .ncoll-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ncoll-row-top, .ncoll-row-bot {
    display: flex;
    align-items: center;
    min-width: 0;
    gap: 6px;
  }
  .ncoll-name {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--t2);
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ncoll-sub {
    font-size: 10.5px;
    font-family: var(--mono);
    color: var(--t4);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .coll-icon {
    width: 22px;
    height: 22px;
    border-radius: 5px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .coll-icon-accent {
    background: color-mix(in srgb, var(--acc) 18%, transparent);
    color: var(--acc);
  }
  .coll-icon svg {
    width: 13px;
    height: 13px;
    stroke: currentColor;
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
  }
  .coll-add {
    width: 18px;
    height: 18px;
    border-radius: 4px;
    border: none;
    background: transparent;
    display: none;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    flex-shrink: 0;
    color: var(--t3);
    transition: background 0.1s, color 0.1s;
    padding: 0;
  }
  .ncoll-hdr:hover .coll-add { display: flex; }
  .coll-add:hover { background: var(--b1); color: var(--t1); }
  .coll-add svg {
    width: 12px;
    height: 12px;
    stroke: currentColor;
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
  }
  /* Stop button stays visible while recording — the user shouldn't
     need to hover to find the way out of a live capture. */
  .coll-add.mtg-stop {
    display: flex;
    color: var(--err, #f87171);
  }
  .coll-add.mtg-stop:hover {
    background: color-mix(in srgb, var(--err, #f87171) 18%, transparent);
    color: var(--err, #f87171);
  }
  .coll-add.mtg-stop svg { fill: currentColor; stroke: none; }
  .coll-add.mtg-busy {
    display: flex;
    cursor: default;
    opacity: 0.7;
  }
  .mtg-spin {
    animation: mtg-rotate 0.8s linear infinite;
  }
  @keyframes mtg-rotate {
    to { transform: rotate(360deg); }
  }

  .mtg-rec-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--err, #f87171);
    flex-shrink: 0;
    animation: mtg-pulse 1.2s ease-in-out infinite;
  }
  @keyframes mtg-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.35; }
  }

  .ncoll-arr {
    width: 12px;
    height: 12px;
    stroke: var(--t3);
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
    flex-shrink: 0;
    transition: transform 0.18s;
  }
  .ncoll-arr.open { transform: rotate(90deg); }

  .ncoll-body {
    overflow: hidden;
    background: var(--e);
    transition: max-height 0.2s ease;
  }

  .ws-leaf {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px 6px 20px;
    cursor: pointer;
    transition: background 0.08s;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12.5px;
    min-height: 40px;
  }
  .ws-leaf:hover {
    background: var(--n2);
    color: var(--t1);
  }
  .mtg-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--t4);
    flex-shrink: 0;
  }
  .mtg-dot-rec {
    background: var(--err, #f87171);
    animation: mtg-pulse 1.2s ease-in-out infinite;
  }
  .mtg-dot-ready { background: var(--acc); }
  .mtg-leaf-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ws-leaf-name {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mtg-leaf-sub {
    font-size: 10.5px;
    font-family: var(--mono);
    color: var(--t4);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ws-leaf-empty {
    padding: 8px 10px 12px 20px;
    color: var(--t4);
    font-size: 11px;
    font-style: italic;
    font-family: var(--ui);
  }
</style>

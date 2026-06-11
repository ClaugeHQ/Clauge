<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { get } from 'svelte/store';
  import { listen } from '@tauri-apps/api/event';
  import MilkdownEditor from './MilkdownEditor.svelte';
  import { recordingStatus, liveSegmentsByMeeting, clearLiveSegments, loadMeetings } from '../stores';
  import {
    workspaceMeetingGet,
    workspaceMeetingUpdateTitle,
    workspaceMeetingUpdateNotes,
  } from '../commands';
  import { parseTranscript } from '../types';
  import type { TranscriptSegment, WorkspaceMeeting } from '../types';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast } from '$lib/utils/errors';
  import { tabs as sharedTabs, updateTab } from '$lib/shared/stores/tabs';
  import { MEETING_EVENT } from '$lib/shared/constants/events';

  interface Props {
    meetingId: string;
  }

  let { meetingId }: Props = $props();

  let meeting = $state<WorkspaceMeeting | null>(null);
  let notFound = $state(false);
  let title = $state('');
  let view = $state<'notes' | 'transcript'>('transcript');
  /** Editor visibility is decided once per load, not derived — deleting
   *  every character mid-edit must not unmount Crepe under the cursor. */
  let showEditor = $state(false);

  let currentNotes = $state('');
  let saving = $state(false);
  let dirty = $state(false);
  let saveTimeout: ReturnType<typeof setTimeout> | null = null;
  // Same phantom-update guard as NoteView: Crepe fires markdownUpdated
  // on initial parse and on cursor-placement ops. The first emit becomes
  // the baseline; identical re-emits are ignored so they never trip the
  // autosave.
  let baseline = $state<string | null>(null);

  const rec = $derived($recordingStatus);
  const recordingThis = $derived(rec.meetingId === meetingId && (rec.recording || rec.stopping));

  const liveSegs = $derived($liveSegmentsByMeeting.get(meetingId) ?? []);
  /** Survives the gap between the store clearing live segments and the
   *  post-stop refetch landing — without it the transcript flashes
   *  empty if the clear wins the race. */
  let liveSnapshot = $state<TranscriptSegment[]>([]);
  $effect(() => {
    if (liveSegs.length) liveSnapshot = liveSegs;
  });

  const parsed = $derived(meeting ? parseTranscript(meeting) : []);
  const segments = $derived.by(() => {
    if (recordingThis) {
      // The backend flushes segments to the DB mid-recording while the
      // live store keeps accumulating from recording start, so a tab
      // opened mid-recording would show flushed segments twice. Drop
      // live segments already present in the persisted transcript.
      const persisted = new Set(parsed.map(s => `${s.startMs}:${s.endMs}:${s.source}`));
      return [...parsed, ...liveSegs.filter(s => !persisted.has(`${s.startMs}:${s.endMs}:${s.source}`))];
    }
    return parsed.length ? parsed : liveSnapshot;
  });

  // 1s tick drives the live timer + duration while recording.
  let now = $state(Date.now());
  $effect(() => {
    if (!recordingThis) return;
    const t = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(t);
  });

  async function bootstrap(id: string) {
    if (saveTimeout) { clearTimeout(saveTimeout); saveTimeout = null; }
    // The component instance is reused across meeting tabs, so a pending
    // edit on the outgoing meeting must be flushed before state resets —
    // fire-and-forget so the incoming meeting loads immediately.
    if (dirty && meeting) {
      const prevId = meeting.id;
      const prevNotes = currentNotes;
      workspaceMeetingUpdateNotes(prevId, prevNotes).catch((e) => errorToast('Save failed', e));
    }
    dirty = false;
    meeting = null;
    notFound = false;
    baseline = null;
    liveSnapshot = [];
    try {
      const fetched = await workspaceMeetingGet(id);
      meeting = fetched;
      title = fetched.title;
      currentNotes = fetched.notesMd ?? '';
      showEditor = !!fetched.notesMd?.trim();
      view = showEditor ? 'notes' : 'transcript';
    } catch {
      notFound = true;
    }
  }

  // untrack: bootstrap reads dirty/meeting/currentNotes for the flush,
  // and those must not retrigger the effect — only the id may.
  $effect(() => {
    const id = meetingId;
    untrack(() => bootstrap(id));
  });

  onMount(() => {
    let destroyed = false;
    const stoppedPromise = listen<{ meetingId: string }>(MEETING_EVENT.RECORDING_STOPPED, async (e) => {
      if (destroyed || e.payload.meetingId !== meetingId) return;
      // Refetch FIRST so the full transcript replaces the live list
      // without a flash of missing segments, then drop the live entry.
      try {
        const fresh = await workspaceMeetingGet(meetingId);
        meeting = fresh;
        if (!dirty) currentNotes = fresh.notesMd ?? '';
      } catch { /* deleted while recording stopped elsewhere */ }
      clearLiveSegments(meetingId);
      liveSnapshot = [];
    });
    // Refetch on start too — the status chip reads meeting.status, which
    // is stale if recording begins while this tab is already open.
    const startedPromise = listen<{ meetingId: string }>(MEETING_EVENT.RECORDING_STARTED, async (e) => {
      if (destroyed || e.payload.meetingId !== meetingId) return;
      try {
        meeting = await workspaceMeetingGet(meetingId);
      } catch { /* deleted concurrently */ }
    });
    return () => {
      destroyed = true;
      // Awaiting the promise covers destroy-before-listen-resolves; the
      // flag covers callbacks already in flight.
      stoppedPromise.then((u) => u());
      startedPromise.then((u) => u());
    };
  });

  // ── Title ──────────────────────────────────────────────────────────

  async function saveTitle() {
    if (!meeting) return;
    const trimmed = title.trim() || 'Untitled meeting';
    if (trimmed === meeting.title) { title = trimmed; return; }
    try {
      await workspaceMeetingUpdateTitle(meeting.id, trimmed);
      meeting = { ...meeting, title: trimmed };
      title = trimmed;
      await loadMeetings();
      const t = get(sharedTabs).find(x => x.mode === 'workspace' && x.key === `meeting:${meeting!.id}`);
      if (t) updateTab(t.id, { label: trimmed });
    } catch (e) {
      errorToast('Rename failed', e);
    }
  }

  function onTitleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur();
  }

  // ── Notes ──────────────────────────────────────────────────────────

  function onNotesChange(markdown: string) {
    if (baseline === null) {
      baseline = markdown;
      currentNotes = markdown;
      return;
    }
    if (markdown === baseline) return;
    currentNotes = markdown;
    dirty = true;
    scheduleSave();
  }

  function scheduleSave() {
    if (saveTimeout) clearTimeout(saveTimeout);
    saveTimeout = setTimeout(saveNotes, 600);
  }

  async function saveNotes() {
    if (!meeting || saving) return;
    saving = true;
    try {
      await workspaceMeetingUpdateNotes(meeting.id, currentNotes);
      dirty = false;
      baseline = currentNotes;
      meeting = { ...meeting, notesMd: currentNotes };
    } catch (e) {
      errorToast('Save failed', e);
    } finally {
      saving = false;
    }
  }

  onDestroy(() => {
    if (saveTimeout) clearTimeout(saveTimeout);
    if (dirty) saveNotes();
  });

  // T15 swaps this body for the real AI generation flow — keep the
  // button wired through this single function.
  function onGenerate() {
    showToast('Coming in the next update', 'info');
  }

  // ── Transcript ─────────────────────────────────────────────────────

  let scrollEl = $state<HTMLDivElement | null>(null);
  let atBottom = true;

  function onScroll() {
    if (!scrollEl) return;
    atBottom = scrollEl.scrollTop + scrollEl.clientHeight >= scrollEl.scrollHeight - 40;
  }

  $effect(() => {
    void segments.length;
    if (!recordingThis || !atBottom) return;
    requestAnimationFrame(() => {
      scrollEl?.scrollTo({ top: scrollEl.scrollHeight });
    });
  });

  function fmtTs(ms: number): string {
    const secs = Math.max(0, Math.floor(ms / 1000));
    return `${String(Math.floor(secs / 60)).padStart(2, '0')}:${String(secs % 60).padStart(2, '0')}`;
  }

  async function copyTranscript() {
    const text = segments.map(s => `[${fmtTs(s.startMs)}] ${s.text}`).join('\n');
    try {
      await navigator.clipboard.writeText(text);
      showToast('Transcript copied', 'success');
    } catch (e) {
      errorToast('Copy failed', e);
    }
  }

  // ── Header bits ────────────────────────────────────────────────────

  function sourceLabel(s: string | null): string {
    if (!s) return 'Manual';
    if (s === 'browser') return 'Browser call';
    return s.charAt(0).toUpperCase() + s.slice(1);
  }

  const statusLabel = $derived.by(() => {
    switch (meeting?.status) {
      case 'recording': return 'Recording…';
      case 'notes_ready': return 'Notes ready';
      default: return 'Transcribed';
    }
  });

  const startedText = $derived(
    meeting ? new Date(meeting.startedAt).toLocaleString() : '',
  );

  const durationText = $derived.by(() => {
    if (!meeting) return '';
    const start = new Date(meeting.startedAt).getTime();
    if (isNaN(start)) return '';
    const end = meeting.endedAt
      ? new Date(meeting.endedAt).getTime()
      : (recordingThis ? now : NaN);
    if (isNaN(end)) return '';
    const secs = Math.max(0, Math.floor((end - start) / 1000));
    const h = Math.floor(secs / 3600);
    const mm = Math.floor((secs % 3600) / 60);
    if (h > 0) return `${h}h ${mm}m`;
    if (mm > 0) return `${mm}m ${secs % 60}s`;
    return `${secs}s`;
  });

  const liveElapsed = $derived.by(() => {
    if (!recordingThis) return '';
    const startStr = rec.startedAt ?? meeting?.startedAt;
    const start = startStr ? new Date(startStr).getTime() : NaN;
    if (isNaN(start)) return '';
    const secs = Math.max(0, Math.floor((now - start) / 1000));
    return `${String(Math.floor(secs / 60)).padStart(2, '0')}:${String(secs % 60).padStart(2, '0')}`;
  });
</script>

{#if notFound}
  <div class="mv-empty-pane">
    <svg viewBox="0 0 24 24" width="42" height="42" fill="none" stroke="var(--t4)" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" y1="19" x2="12" y2="22"/>
    </svg>
    <p>This meeting no longer exists.</p>
  </div>
{:else if !meeting}
  <div class="mv-loading">Loading…</div>
{:else}
  <div class="mv">
    <div class="mv-meta">
      <span class="mv-crumb">meetings</span>
      <span class="mv-sep">/</span>
      <span class="mv-crumb-active">{meeting.title || 'untitled'}</span>
      <span style="flex:1"></span>
      {#if showEditor && view === 'notes'}
        {#if saving}
          <span class="mv-saving">saving…</span>
        {:else if dirty}
          <span class="mv-dirty">unsaved</span>
        {:else}
          <span class="mv-saved">saved</span>
        {/if}
      {/if}
    </div>

    <input
      class="mv-title"
      bind:value={title}
      onblur={saveTitle}
      onkeydown={onTitleKeydown}
      placeholder="Untitled meeting"
      spellcheck="false"
    />

    <div class="mv-badges">
      <span class="mv-chip">{sourceLabel(meeting.sourceApp)}</span>
      <span class="mv-when">{startedText}</span>
      {#if durationText}
        <span class="mv-when">· {durationText}</span>
      {/if}
      <span class="mv-chip mv-status" class:mv-status-rec={meeting.status === 'recording'} class:mv-status-ready={meeting.status === 'notes_ready'}>
        {statusLabel}
      </span>
      {#if recordingThis}
        <span class="mv-live">
          <span class="mv-live-dot"></span>
          {rec.stopping ? 'Saving…' : liveElapsed}
        </span>
      {/if}
    </div>

    <div class="mv-segment">
      <button type="button" class="mv-seg-btn" class:active={view === 'notes'} onclick={() => (view = 'notes')}>Notes</button>
      <button type="button" class="mv-seg-btn" class:active={view === 'transcript'} onclick={() => (view = 'transcript')}>Transcript</button>
    </div>

    <!-- Both tab bodies stay mounted (display toggle, #9): Crepe keeps
         its editing state and the transcript keeps its scroll offset. -->
    <div class="mv-pane" class:hidden={view !== 'notes'}>
      {#if showEditor}
        <div class="mv-editor">
          {#key meeting.id}
            <MilkdownEditor value={meeting.notesMd ?? ''} onChange={onNotesChange} />
          {/key}
        </div>
      {:else if recordingThis}
        <div class="mv-notes-empty">
          <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="var(--t4)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" y1="19" x2="12" y2="22"/>
          </svg>
          <h3>Recording in progress…</h3>
          <p>Notes can be generated once the recording stops.</p>
        </div>
      {:else}
        <div class="mv-notes-empty">
          <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="var(--t4)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="13" y2="17"/>
          </svg>
          <h3>No meeting notes yet</h3>
          <p>The transcript was captured. Generate notes from it with AI.</p>
          <button class="mv-generate" onclick={onGenerate}>Generate meeting notes</button>
        </div>
      {/if}
    </div>

    <div class="mv-pane" class:hidden={view !== 'transcript'}>
      <div class="mv-tr-head">
        <span class="mv-tr-count">{segments.length} segment{segments.length === 1 ? '' : 's'}</span>
        <span class="mv-legend">
          <span class="mv-legend-dot mv-legend-mic"></span>Mic — you
          <span class="mv-legend-gap">·</span>
          <span class="mv-legend-dot mv-legend-sys"></span>System — other participants
        </span>
        <span style="flex:1"></span>
        <button class="mv-copy" onclick={copyTranscript} disabled={segments.length === 0}>
          <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
          Copy transcript
        </button>
      </div>
      <div class="mv-tr-list" bind:this={scrollEl} onscroll={onScroll}>
        {#each segments as s, i (i)}
          <div class="mv-seg" class:mv-seg-sys={s.source === 'system'}>
            <span class="mv-seg-ts">[{fmtTs(s.startMs)}]</span>
            <span class="mv-seg-text">{s.text}</span>
          </div>
        {/each}
        {#if segments.length === 0}
          <div class="mv-tr-empty">
            {#if recordingThis}
              Listening… segments appear here as speech is transcribed.
            {:else}
              No transcript was captured for this meeting.
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .mv-loading {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 12.5px;
  }
  .mv-empty-pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 40px;
    color: var(--t3);
    text-align: center;
  }
  .mv-empty-pane p {
    margin: 0;
    font-size: 12.5px;
    color: var(--t3);
    font-family: var(--ui);
  }

  .mv {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    padding: 16px 28px 0;
  }
  .mv-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 14px;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t4);
  }
  .mv-crumb { color: var(--t3); }
  .mv-crumb-active { color: var(--t2); }
  .mv-sep { color: var(--t4); }
  .mv-saving { color: var(--warn, #f5a623); font-style: italic; }
  .mv-dirty { color: var(--t4); font-style: italic; }
  .mv-saved { color: var(--state-saved); }

  .mv-title {
    border: none;
    background: transparent;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.01em;
    outline: none;
    padding: 0;
    margin: 0 0 12px;
    width: 100%;
  }
  .mv-title::placeholder { color: var(--t4); }

  .mv-badges {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 16px;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--b1);
  }
  .mv-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    border-radius: 12px;
    border: 1px solid var(--b1);
    background: var(--surface-hover);
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t1);
  }
  .mv-when {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t4);
  }
  .mv-status {
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    border-color: color-mix(in srgb, var(--acc) 30%, transparent);
    color: var(--t2);
  }
  .mv-status-rec {
    background: color-mix(in srgb, var(--err, #f87171) 12%, transparent);
    border-color: color-mix(in srgb, var(--err, #f87171) 35%, transparent);
    color: var(--err, #f87171);
  }
  .mv-status-ready {
    color: var(--acc);
  }
  .mv-live {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: var(--mono);
    font-size: 10.5px;
    font-variant-numeric: tabular-nums;
    color: var(--err, #f87171);
  }
  .mv-live-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--err, #f87171);
    animation: mv-pulse 1.2s ease-in-out infinite;
  }
  @keyframes mv-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.35; }
  }

  .mv-segment {
    display: inline-flex;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 9px;
    padding: 3px;
    gap: 2px;
    align-self: flex-start;
    margin-bottom: 14px;
  }
  .mv-seg-btn {
    border: none;
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 500;
    padding: 5px 16px;
    border-radius: 7px;
    cursor: default;
    transition: background 0.12s, color 0.12s;
  }
  .mv-seg-btn:hover { color: var(--t1); }
  .mv-seg-btn.active {
    background: var(--surface-hover);
    color: var(--t1);
  }

  .mv-pane {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .mv-pane.hidden { display: none; }

  .mv-editor {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    margin: 0 -28px;
    padding: 0 28px;
  }

  .mv-notes-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px;
    text-align: center;
  }
  .mv-notes-empty h3 {
    margin: 6px 0 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--t2);
    font-family: var(--ui);
  }
  .mv-notes-empty p {
    margin: 0;
    max-width: 380px;
    font-size: 12px;
    color: var(--t4);
    font-family: var(--ui);
    line-height: 1.6;
  }
  .mv-generate {
    margin-top: 10px;
    padding: 6px 16px;
    border-radius: 7px;
    border: 1px dashed var(--b2, var(--b1));
    background: transparent;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--ui);
    font-weight: 500;
    cursor: default;
    transition: border-color 0.12s, color 0.12s;
  }
  .mv-generate:hover { border-color: var(--acc); color: var(--t2); }

  .mv-tr-head {
    position: sticky;
    top: 0;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 0 8px;
    border-bottom: 1px solid var(--b1);
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t3);
    flex-shrink: 0;
  }
  .mv-tr-count { color: var(--t2); }
  .mv-legend {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--t4);
  }
  .mv-legend-gap { margin: 0 2px; }
  .mv-legend-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .mv-legend-mic { background: var(--acc); }
  .mv-legend-sys { background: var(--warn, #f5a623); }
  .mv-copy {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 11px;
    cursor: default;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }
  .mv-copy:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--t1);
    border-color: var(--b2);
  }
  .mv-copy:disabled { opacity: 0.45; }

  .mv-tr-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px 0 40px;
  }
  .mv-tr-list::-webkit-scrollbar { width: 6px; }
  .mv-tr-list::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 3px; }

  .mv-seg {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 4px 10px;
    border-left: 2px solid color-mix(in srgb, var(--acc) 45%, transparent);
    margin-bottom: 4px;
  }
  .mv-seg-sys {
    border-left-color: color-mix(in srgb, var(--warn, #f5a623) 55%, transparent);
  }
  .mv-seg-ts {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t4);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .mv-seg-text {
    font-family: var(--ui);
    font-size: 13px;
    line-height: 1.6;
    color: var(--t1);
    min-width: 0;
  }
  .mv-tr-empty {
    padding: 28px 0;
    text-align: center;
    color: var(--t4);
    font-size: 12px;
    font-style: italic;
    font-family: var(--ui);
  }
</style>

<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { get } from 'svelte/store';
  import { listen } from '@tauri-apps/api/event';
  import MilkdownEditor from './MilkdownEditor.svelte';
  import {
    recordingStatus,
    liveSegmentsByMeeting,
    clearLiveSegments,
    loadMeetings,
    stopActiveRecording,
    markGenerationStart,
    markGenerationEnd,
  } from '../stores';
  import {
    workspaceMeetingGet,
    workspaceMeetingUpdateTitle,
    workspaceMeetingUpdateNotes,
    workspaceMeetingGenerateNotes,
  } from '../commands';
  import { parseTranscript, sortSegments } from '../types';
  import type { TranscriptSegment, WorkspaceMeeting } from '../types';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast } from '$lib/utils/errors';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import { tabs as sharedTabs, updateTab, openSettingsTab } from '$lib/shared/stores/tabs';
  import { MEETING_EVENT } from '$lib/shared/constants/events';
  import { settings } from '$lib/stores/settings';
  import { cloudPlan, cloudCredits, upgradeModalOpen } from '$lib/stores/cloud';
  import { PROVIDERS } from '$lib/shared/ai/providers';

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

  let stopBusy = $state(false);
  async function stopThisRecording() {
    if (stopBusy || rec.stopping) return;
    stopBusy = true;
    try {
      await stopActiveRecording();
    } finally {
      stopBusy = false;
    }
  }

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
      return sortSegments([...parsed, ...liveSegs.filter(s => !persisted.has(`${s.startMs}:${s.endMs}:${s.source}`))]);
    }
    return parsed.length ? parsed : sortSegments(liveSnapshot);
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
    // Generation keeps running backend-side across tab switches; the
    // notes-progress listener re-raises the flag if this meeting is
    // still being summarized.
    generating = false;
    progress = null;
    pickerOpen = false;
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

  // Component scope (not onMount) so startGeneration's catch — which can
  // outlive the component when a tab is closed mid-generation — can
  // check it too.
  let destroyed = false;

  onMount(() => {
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
    // Emitted only on multi-chunk runs. Also re-raises `generating` when
    // this tab was closed and reopened mid-generation — the backend keeps
    // going regardless.
    const progressPromise = listen<{ meetingId: string; done: number; total: number }>(
      MEETING_EVENT.NOTES_PROGRESS,
      (e) => {
        if (destroyed || e.payload.meetingId !== meetingId) return;
        generating = true;
        progress = { done: e.payload.done, total: e.payload.total };
      },
    );
    // Sole completion path — the invoke success resolves around the same
    // time but defers here so a run that outlived its original tab still
    // lands when the meeting is reopened.
    const readyPromise = listen<{ meetingId: string }>(MEETING_EVENT.NOTES_READY, async (e) => {
      if (destroyed || e.payload.meetingId !== meetingId) return;
      generating = false;
      progress = null;
      // Kill any debounced manual save BEFORE the refetch await — a stale
      // save firing in that gap would overwrite the generated notes.
      if (saveTimeout) { clearTimeout(saveTimeout); saveTimeout = null; }
      dirty = false;
      try {
        const fresh = await workspaceMeetingGet(meetingId);
        meeting = fresh;
        currentNotes = fresh.notesMd ?? '';
        baseline = null;
        showEditor = !!fresh.notesMd?.trim();
        notesEpoch += 1;
        view = 'notes';
        showToast('Meeting notes ready', 'success');
      } catch { /* deleted while generating */ }
    });
    // Failure twin of notes-ready. Owns the error UI for every failure
    // past the backend's in-flight guard; `errorHandledFor` dedupes
    // against the invoke catch when this tab is the one that started the
    // run (event vs rejection ordering is not guaranteed).
    const failedPromise = listen<{ meetingId: string; message: string }>(
      MEETING_EVENT.NOTES_ERROR,
      (e) => {
        if (destroyed || e.payload.meetingId !== meetingId) return;
        if (errorHandledFor === e.payload.meetingId) { errorHandledFor = null; return; }
        errorHandledFor = e.payload.meetingId;
        handleGenerationFailure(e.payload.message);
      },
    );
    return () => {
      destroyed = true;
      // Awaiting the promise covers destroy-before-listen-resolves; the
      // flag covers callbacks already in flight.
      stoppedPromise.then((u) => u());
      startedPromise.then((u) => u());
      progressPromise.then((u) => u());
      readyPromise.then((u) => u());
      failedPromise.then((u) => u());
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
    // Bypass saveNotes(): its in-flight guard would drop the latest edit
    // if a save is already running when the tab closes. Fire-and-forget —
    // the component is gone, so no state to update.
    if (dirty && meeting) {
      workspaceMeetingUpdateNotes(meeting.id, currentNotes).catch((e) =>
        console.error('Failed to save meeting notes on close', e),
      );
    }
  });

  // ── Generate notes ─────────────────────────────────────────────────

  const CLAUGE = 'clauge';
  const isPro = $derived($cloudPlan === 'pro');

  /** Advisory only — the server is authoritative on every call. Flags
   *  zero/near-zero (≤5% of allowance) remaining Clauge AI credits so
   *  the picker can hint before a run fails with a 402. */
  const lowCredits = $derived.by(() => {
    const c = $cloudCredits;
    if (!isPro || !c) return false;
    return c.remaining <= 0 || (c.allowance > 0 && c.remaining <= c.allowance * 0.05);
  });

  // Same configured-provider rule as AIConfigSelector: only BYOK
  // providers with a key set are offered. Deduped to the first registry
  // entry per provider — that entry is the provider's default model.
  const configuredProviders = $derived.by(() => {
    const seen = new Set<string>();
    return PROVIDERS.filter((p) => {
      if (seen.has(p.providerId) || !$settings[p.keySettingName]?.trim()) return false;
      seen.add(p.providerId);
      return true;
    });
  });

  let pickerOpen = $state(false);
  let pickerEl = $state<HTMLDivElement | null>(null);
  let generating = $state(false);
  let progress = $state<{ done: number; total: number } | null>(null);
  let showRegenConfirm = $state(false);
  /** Bumping this remounts Crepe so generated notes replace the buffer —
   *  updating `value` alone doesn't reach an already-mounted editor. */
  let notesEpoch = $state(0);

  const hasNotes = $derived(!!meeting?.notesMd?.trim());
  const canGenerate = $derived(
    !!meeting && parsed.length > 0 && !recordingThis && meeting.status !== 'recording',
  );

  const generateLabel = $derived.by(() => {
    if (generating) {
      return progress ? `Summarizing ${progress.done}/${progress.total}…` : 'Generating…';
    }
    return hasNotes ? 'Regenerate' : 'Generate notes';
  });

  /** Single entry point for both the header button and the empty-state
   *  hint button. */
  function onGenerate() {
    if (generating) return;
    if (parsed.length === 0) {
      showToast('No transcript to generate notes from', 'error');
      return;
    }
    if (hasNotes) {
      showRegenConfirm = true;
      return;
    }
    pickerOpen = !pickerOpen;
  }

  function pickClaugeAI() {
    if (!isPro) {
      pickerOpen = false;
      upgradeModalOpen.set(true);
      return;
    }
    startGeneration(CLAUGE);
  }

  function openAIKeySettings() {
    pickerOpen = false;
    openSettingsTab('ai:byok');
  }

  /** Failure-dedupe token: a backend failure reaches an open tab TWICE —
   *  once as the invoke rejection, once as the notes-error event — in no
   *  guaranteed order. Whichever path runs first handles the UI and sets
   *  this to the meeting id; the second sees its own id and only clears
   *  the token. Reset on every new generation so a token orphaned by a
   *  pre-guard rejection (which emits no event) can't swallow the next
   *  failure's toast. */
  let errorHandledFor: string | null = null;

  /** Shared error UI for the invoke catch and the notes-error listener:
   *  resets the in-flight state and routes the message to the right
   *  toast/modal. Callers guard meeting identity + dedupe first. */
  function handleGenerationFailure(message: string) {
    generating = false;
    progress = null;
    if (message.includes('pro_required')) {
      showToast('Clauge AI needs an active Pro plan — upgrade to generate notes', 'error');
      upgradeModalOpen.set(true);
    } else if (message.toLowerCase().includes('credit')) {
      // Backend maps upstream 402s to a message that always contains
      // "credits" (shared/ai/clients/errors.rs). There is no separate
      // top-up purchase — the Account settings card shows the balance
      // and when the cycle's credits come back.
      showToast('Out of Clauge AI credits', 'error');
      openSettingsTab('account');
    } else if (message.includes('no_api_key')) {
      showToast('No API key for this provider — add one in Settings → AI', 'error');
      openSettingsTab('ai:byok');
    } else {
      errorToast('Notes generation failed', message);
    }
  }

  async function startGeneration(providerId: string, model?: string) {
    if (!meeting || generating) return;
    // The catch below can fire after the user has switched this reused
    // component to another meeting — everything in it must compare
    // against the id the run was started for, never `meeting`/`meetingId`
    // read at catch time.
    const id = meeting.id;
    pickerOpen = false;
    generating = true;
    progress = null;
    errorHandledFor = null;
    // Eager add so the nav's delete guard sees the run before the
    // backend's first notes-progress event lands; the global
    // ready/error listeners in stores.ts remove it.
    markGenerationStart(id);
    try {
      await workspaceMeetingGenerateNotes(id, providerId, model);
      // Success is applied by the notes-ready listener (refetch + tab
      // switch + toast) — nothing to do here.
    } catch (e) {
      const msg = String(e);
      // Pre-guard rejections emit no notes-error event, so the store
      // entry must be released here — except when the rejection means a
      // real run from another tab instance is still going.
      if (!msg.includes('generation already in progress')) markGenerationEnd(id);
      // Another meeting is displayed (or the component is gone): skip ALL
      // of it — state reset, toasts, modals. If the failed meeting is
      // reopened later, the notes-error listener shows the failure there.
      if (destroyed || meetingId !== id) return;
      if (msg.includes('generation already in progress')) {
        // A run started from a previous tab instance is still going; keep
        // the button in its in-flight state and let notes-ready land it.
        showToast('Notes are already being generated for this meeting', 'info');
        return;
      }
      if (errorHandledFor === id) { errorHandledFor = null; return; }
      errorHandledFor = id;
      handleGenerationFailure(msg);
    }
  }

  // Same dismiss idiom as AIConfigSelector — deferred a frame so the
  // click that opened the popover doesn't immediately close it.
  function pickerOutside(e: MouseEvent) {
    if (!pickerOpen) return;
    if (pickerEl?.contains(e.target as Node)) return;
    pickerOpen = false;
  }
  function pickerKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && pickerOpen) pickerOpen = false;
  }
  $effect(() => {
    if (!pickerOpen) return;
    const t = setTimeout(() => {
      window.addEventListener('mousedown', pickerOutside);
      window.addEventListener('keydown', pickerKey);
    }, 0);
    return () => {
      clearTimeout(t);
      window.removeEventListener('mousedown', pickerOutside);
      window.removeEventListener('keydown', pickerKey);
    };
  });

  const provenanceProvider = $derived.by(() => {
    const p = meeting?.notesProvider;
    if (!p) return '';
    if (p === CLAUGE || p === 'clauge-ai') return 'Clauge AI';
    return PROVIDERS.find((x) => x.providerId === p)?.providerLabel ?? p;
  });

  // Same names AIConfigSelector shows: registry `modelLabel` for BYOK
  // models, "Managed" for the Clauge AI sentinel id; unknown ids fall
  // back to the raw stored value.
  const provenanceModel = $derived.by(() => {
    const m = meeting?.notesModel;
    if (!m) return '';
    if (m === 'clauge-managed') return 'Managed';
    return PROVIDERS.find((x) => x.modelId === m)?.modelLabel ?? m;
  });

  // ── Transcript ─────────────────────────────────────────────────────

  let scrollEl = $state<HTMLDivElement | null>(null);
  let atBottom = true;

  /** Mic-only recordings have no system rows — the "other participants"
   *  legend entry would only mislead. */
  const hasSystemSegments = $derived(segments.some((s) => s.source === 'system'));

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
        {#if !rec.stopping}
          <button
            type="button"
            class="mv-stop-btn"
            onclick={stopThisRecording}
            disabled={stopBusy}
          >
            <svg viewBox="0 0 12 12" width="8" height="8" fill="currentColor" aria-hidden="true">
              <rect x="1" y="1" width="10" height="10" rx="2" />
            </svg>
            {stopBusy ? 'Stopping…' : 'Stop'}
          </button>
        {/if}
      {/if}
    </div>

    <div class="mv-toolbar">
      <div class="mv-segment">
        <button type="button" class="mv-seg-btn" class:active={view === 'notes'} onclick={() => (view = 'notes')}>Notes</button>
        <button type="button" class="mv-seg-btn" class:active={view === 'transcript'} onclick={() => (view = 'transcript')}>Transcript</button>
      </div>
      <span style="flex:1"></span>
      {#if canGenerate}
        <div class="mv-gen-wrap" bind:this={pickerEl}>
          <button
            type="button"
            class="mv-gen-btn"
            onclick={onGenerate}
            disabled={generating}
            aria-haspopup="listbox"
            aria-expanded={pickerOpen}
          >
            {#if generating}
              <span class="mv-gen-spinner" aria-hidden="true"></span>
            {:else}
              <svg viewBox="0 0 24 24" width="11" height="11" fill="currentColor" aria-hidden="true">
                <path d="M12 2l2.6 7.4L22 12l-7.4 2.6L12 22l-2.6-7.4L2 12l7.4-2.6L12 2z" />
              </svg>
            {/if}
            {generateLabel}
          </button>
          {#if pickerOpen}
            <div class="mv-gen-pop" role="listbox" aria-label="Choose AI provider">
              <!-- Clauge AI — pinned at top, gated by Pro. Mirrors AIConfigSelector. -->
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="mv-gen-row is-clauge"
                role="option"
                tabindex="0"
                aria-selected="false"
                aria-disabled={!isPro}
                onclick={pickClaugeAI}
                onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); pickClaugeAI(); } }}
              >
                <span class="mv-gen-dot mv-gen-dot-clauge" aria-hidden="true">
                  <svg viewBox="0 0 24 24" width="9" height="9" fill="currentColor">
                    <path d="M12 2l2.6 7.4L22 12l-7.4 2.6L12 22l-2.6-7.4L2 12l7.4-2.6L12 2z" />
                  </svg>
                </span>
                <span class="mv-gen-text">
                  <span class="mv-gen-name">Clauge AI</span>
                  <span class="mv-gen-sub">
                    {#if isPro}Managed · no API key needed{:else}Requires Pro{/if}
                  </span>
                </span>
                {#if !isPro}
                  <span class="mv-gen-pro">PRO</span>
                {:else if lowCredits}
                  <span class="mv-gen-low" title="Clauge AI credits are nearly used up — the server decides per run">Low credits</span>
                {/if}
              </div>
              {#if configuredProviders.length > 0}
                <div class="mv-gen-sep" aria-hidden="true"><span>Your providers</span></div>
                {#each configuredProviders as p (p.providerId)}
                  <!-- svelte-ignore a11y_click_events_have_key_events -->
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div
                    class="mv-gen-row"
                    role="option"
                    tabindex="0"
                    aria-selected="false"
                    onclick={() => startGeneration(p.providerId, p.modelId)}
                    onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); startGeneration(p.providerId, p.modelId); } }}
                  >
                    <span class="mv-gen-dot" aria-hidden="true"></span>
                    <span class="mv-gen-text">
                      <span class="mv-gen-name">{p.providerLabel}</span>
                      <span class="mv-gen-sub">{p.modelLabel}</span>
                    </span>
                  </div>
                {/each}
              {:else if !isPro}
                <div class="mv-gen-empty">
                  Add an API key in Settings or upgrade to Clauge Pro.
                  <button type="button" class="mv-gen-settings" onclick={openAIKeySettings}>Open Settings</button>
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <!-- Both tab bodies stay mounted (display toggle, #9): Crepe keeps
         its editing state and the transcript keeps its scroll offset. -->
    <div class="mv-pane" class:hidden={view !== 'notes'}>
      {#if showEditor}
        {#if meeting.notesProvider && meeting.notesGeneratedAt}
          <div class="mv-provenance">
            Generated by {provenanceProvider}
            {#if provenanceModel}· {provenanceModel}{/if}
            · {new Date(meeting.notesGeneratedAt).toLocaleString()}
          </div>
        {/if}
        <div class="mv-editor">
          {#key `${meeting.id}:${notesEpoch}`}
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
      {:else if segments.length === 0}
        <div class="mv-notes-empty">
          <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="var(--t4)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="13" y2="17"/>
          </svg>
          <h3>No meeting notes</h3>
          <p>No speech was captured in this meeting.</p>
        </div>
      {:else}
        <div class="mv-notes-empty">
          <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="var(--t4)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="13" y2="17"/>
          </svg>
          <h3>No meeting notes yet</h3>
          <p>The transcript was captured. Generate notes from it with AI.</p>
          {#if parsed.length > 0}
            <button class="mv-generate" onclick={onGenerate}>Generate meeting notes</button>
          {/if}
        </div>
      {/if}
    </div>

    <div class="mv-pane" class:hidden={view !== 'transcript'}>
      <div class="mv-tr-head">
        <span class="mv-tr-count">{segments.length} segment{segments.length === 1 ? '' : 's'}</span>
        <span class="mv-legend">
          <span class="mv-legend-dot mv-legend-mic"></span>Mic — you
          {#if hasSystemSegments}
            <span class="mv-legend-gap">·</span>
            <span class="mv-legend-dot mv-legend-sys"></span>System — other participants
          {/if}
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

<ConfirmDialog
  bind:show={showRegenConfirm}
  title="Regenerate notes"
  message="Regenerate notes? Current notes (including manual edits) will be replaced."
  confirmText="Regenerate"
  confirmColor="var(--acc)"
  onconfirm={() => { pickerOpen = true; }}
/>

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
  .mv-stop-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 10px;
    border-radius: 12px;
    border: 1px solid color-mix(in srgb, var(--err, #f87171) 35%, transparent);
    background: color-mix(in srgb, var(--err, #f87171) 12%, transparent);
    color: var(--err, #f87171);
    font-family: var(--ui);
    font-size: 11px;
    font-weight: 500;
    cursor: default;
    transition: background 0.12s;
  }
  .mv-stop-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--err, #f87171) 22%, transparent);
  }
  .mv-stop-btn:disabled { opacity: 0.6; }

  .mv-toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 14px;
  }
  .mv-segment {
    display: inline-flex;
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 9px;
    padding: 3px;
    gap: 2px;
  }

  .mv-gen-wrap {
    position: relative;
  }
  .mv-gen-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 12px;
    border-radius: 7px;
    border: 1px solid var(--b1);
    background: var(--surface-hover);
    color: var(--t1);
    font-family: var(--ui);
    font-size: 11.5px;
    font-weight: 500;
    cursor: default;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .mv-gen-btn:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--acc) 45%, transparent);
    color: var(--t1);
  }
  .mv-gen-btn:disabled {
    opacity: 0.7;
    color: var(--t3);
  }
  .mv-gen-btn svg { color: var(--acc); }
  .mv-gen-spinner {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid var(--b2, var(--b1));
    border-top-color: var(--acc);
    animation: mv-spin 0.8s linear infinite;
    flex-shrink: 0;
  }
  @keyframes mv-spin {
    to { transform: rotate(360deg); }
  }

  .mv-gen-pop {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 30;
    min-width: 240px;
    padding: 5px;
    border-radius: 10px;
    border: 1px solid var(--b1);
    background: var(--e);
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.35);
  }
  .mv-gen-row {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 7px 9px;
    border-radius: 7px;
    cursor: default;
  }
  .mv-gen-row:hover {
    background: var(--surface-hover);
  }
  .mv-gen-row[aria-disabled='true'] .mv-gen-name,
  .mv-gen-row[aria-disabled='true'] .mv-gen-sub {
    color: var(--t4);
  }
  .mv-gen-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--acc);
    flex-shrink: 0;
  }
  .mv-gen-dot-clauge {
    width: auto;
    height: auto;
    background: transparent;
    color: var(--acc);
    display: inline-flex;
  }
  .mv-gen-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .mv-gen-name {
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 500;
    color: var(--t1);
  }
  .mv-gen-sub {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--t4);
  }
  .mv-gen-pro {
    margin-left: auto;
    padding: 1px 6px;
    border-radius: 5px;
    border: 1px solid color-mix(in srgb, var(--acc) 40%, transparent);
    color: var(--acc);
    font-family: var(--mono);
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.06em;
  }
  .mv-gen-low {
    margin-left: auto;
    padding: 1px 6px;
    border-radius: 5px;
    border: 1px solid color-mix(in srgb, var(--warn, #f5a623) 45%, transparent);
    color: var(--warn, #f5a623);
    font-family: var(--mono);
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }
  .mv-gen-sep {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 5px 4px 3px;
    font-family: var(--mono);
    font-size: 9.5px;
    color: var(--t4);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .mv-gen-sep::after {
    content: '';
    flex: 1;
    height: 1px;
    background: var(--b1);
  }
  .mv-gen-empty {
    padding: 9px 10px 10px;
    font-family: var(--ui);
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--t3);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 7px;
  }
  .mv-gen-settings {
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 11px;
    cursor: default;
    transition: background 0.12s, color 0.12s;
  }
  .mv-gen-settings:hover {
    background: var(--surface-hover);
    color: var(--t1);
  }

  .mv-provenance {
    flex-shrink: 0;
    padding: 2px 0 8px;
    font-family: var(--mono);
    font-size: 10px;
    color: var(--t4);
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

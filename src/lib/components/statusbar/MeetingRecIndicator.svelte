<script lang="ts">
  // Global recording indicator — visible from any mode while a meeting
  // capture is live. Click jumps to the meeting tab in workspace mode.

  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import {
    meetings,
    recordingStatus,
    loadMeetings,
    loadRecordingStatus,
    initMeetingListeners,
    openMeetingTab,
  } from '$lib/modes/workspace/stores';

  onMount(() => {
    // Idempotent — workspace mode may never have been opened, but the
    // indicator still needs the recorder events + status snapshot.
    initMeetingListeners();
    loadRecordingStatus();
  });

  const active = $derived($recordingStatus.recording || $recordingStatus.stopping);
  const meeting = $derived($meetings.find(m => m.id === $recordingStatus.meetingId) ?? null);

  // The meetings list is loaded lazily by workspace nav; fetch it here
  // when a recording is visible but its title isn't resolvable yet.
  let loadTried = false;
  $effect(() => {
    if (active && !meeting && !loadTried) {
      loadTried = true;
      loadMeetings();
    }
  });

  let now = $state(Date.now());
  $effect(() => {
    if (!$recordingStatus.recording) return;
    const t = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(t);
  });

  const elapsed = $derived.by(() => {
    const startStr = $recordingStatus.startedAt;
    if (!startStr) return '';
    const start = new Date(startStr).getTime();
    if (isNaN(start)) return '';
    const secs = Math.max(0, Math.floor((now - start) / 1000));
    return `${String(Math.floor(secs / 60)).padStart(2, '0')}:${String(secs % 60).padStart(2, '0')}`;
  });

  function open() {
    const id = get(recordingStatus).meetingId;
    if (!id) return;
    const m = get(meetings).find(x => x.id === id);
    openMeetingTab(m ?? { id, title: 'Untitled meeting' });
  }
</script>

{#if active}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="rec-ind" onclick={open} title="Open meeting">
    <span class="rec-dot" class:saving={$recordingStatus.stopping}></span>
    {#if $recordingStatus.stopping}
      <span class="rec-label">Saving…</span>
    {:else}
      <span class="rec-time">{elapsed}</span>
    {/if}
    <span class="rec-title">{meeting?.title || 'Untitled meeting'}</span>
  </div>
{/if}

<style>
  .rec-ind {
    font-size: 10px;
    color: var(--t3);
    display: flex;
    align-items: center;
    gap: 5px;
    font-family: var(--mono);
    cursor: default;
    padding: 2px 6px;
    border-radius: 4px;
    transition: background 0.1s, color 0.1s;
  }
  .rec-ind:hover {
    background: var(--surface-hover);
    color: var(--t1);
  }
  .rec-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--err, #ff5f57);
    box-shadow: 0 0 6px var(--err, #ff5f57);
    animation: recPulse 1.2s ease-in-out infinite;
  }
  .rec-dot.saving {
    animation-duration: 0.6s;
  }
  @keyframes recPulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.35; }
  }
  .rec-time {
    color: var(--err, #ff5f57);
    font-variant-numeric: tabular-nums;
  }
  .rec-label {
    color: var(--err, #ff5f57);
  }
  .rec-title {
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>

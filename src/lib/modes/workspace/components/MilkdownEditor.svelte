<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    attachNoteEditor,
    detachNoteEditor,
    subscribeNoteMarkdown,
  } from '../services/noteEditorReparent';

  import '@milkdown/crepe/theme/common/style.css';
  import '@milkdown/crepe/theme/frame-dark.css';

  interface Props {
    noteId: string;
    onChange?: (markdown: string) => void;
  }

  let { noteId, onChange }: Props = $props();

  let host: HTMLDivElement;
  let mountedNoteId: string | null = null;
  let unsubscribe: (() => void) | null = null;

  async function mount(id: string) {
    if (!host) return;
    await attachNoteEditor(id, host);
    mountedNoteId = id;
    if (onChange) {
      unsubscribe = subscribeNoteMarkdown(id, onChange);
    }
  }

  function unmount() {
    if (unsubscribe) {
      unsubscribe();
      unsubscribe = null;
    }
    if (mountedNoteId && host) {
      detachNoteEditor(mountedNoteId, host);
    }
    mountedNoteId = null;
  }

  onMount(() => {
    void mount(noteId);
  });

  $effect(() => {
    if (mountedNoteId && mountedNoteId !== noteId) {
      unmount();
      void mount(noteId);
    }
  });

  onDestroy(() => {
    unmount();
  });
</script>

<div class="md-host-slot" bind:this={host}></div>

<style>
  .md-host-slot {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  :global(.md-host) {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  :global(.md-host::-webkit-scrollbar) { width: 6px; }
  :global(.md-host::-webkit-scrollbar-thumb) { background: var(--b1); border-radius: 3px; }

  :global(.md-host .milkdown) {
    background: transparent;
    color: var(--t1);
    font-family: var(--ui);
  }
  :global(.md-host .milkdown .ProseMirror) {
    padding: 12px 0 80px;
    max-width: none;
    margin: 0;
    font-size: 14px;
    line-height: 1.7;
    outline: none;
  }
  :global(.md-host .milkdown) { padding: 0 !important; }
  :global(.md-host .milkdown h1),
  :global(.md-host .milkdown h2),
  :global(.md-host .milkdown h3) {
    color: var(--t1);
    letter-spacing: -0.01em;
  }
  :global(.md-host .milkdown code) {
    background: var(--surface-hover);
    color: var(--acc);
    font-family: var(--mono);
    font-size: 12.5px;
    padding: 1px 5px;
    border-radius: 4px;
  }
  :global(.md-host .milkdown pre) {
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid var(--b1);
    border-radius: 6px;
  }
  :global(.md-host .milkdown blockquote) {
    border-left: 2px solid var(--acc);
    color: var(--t2);
  }
  :global(.md-host .milkdown a) {
    color: var(--acc);
  }
  :global(.md-host .milkdown hr) {
    border-color: var(--b1);
  }
</style>

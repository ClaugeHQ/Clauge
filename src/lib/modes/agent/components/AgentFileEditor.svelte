<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { EditorView, keymap } from '@codemirror/view';
  import { EditorState, type Extension } from '@codemirror/state';
  import { basicSetup } from 'codemirror';
  import { oneDark } from '@codemirror/theme-one-dark';
  import { json } from '@codemirror/lang-json';
  import { markdown } from '@codemirror/lang-markdown';
  import { xml } from '@codemirror/lang-xml';
  import { javascript } from '@codemirror/lang-javascript';
  import { python } from '@codemirror/lang-python';
  import { rust } from '@codemirror/lang-rust';
  import { go } from '@codemirror/lang-go';
  import { html } from '@codemirror/lang-html';
  import { css } from '@codemirror/lang-css';
  import { java } from '@codemirror/lang-java';
  import { cpp } from '@codemirror/lang-cpp';
  import { php } from '@codemirror/lang-php';
  import { yaml } from '@codemirror/lang-yaml';
  import { sass } from '@codemirror/lang-sass';
  import { vue } from '@codemirror/lang-vue';
  import { StreamLanguage } from '@codemirror/language';
  import { shell } from '@codemirror/legacy-modes/mode/shell';
  import { ruby } from '@codemirror/legacy-modes/mode/ruby';
  import { lua } from '@codemirror/legacy-modes/mode/lua';
  import { perl } from '@codemirror/legacy-modes/mode/perl';
  import { swift } from '@codemirror/legacy-modes/mode/swift';
  import { toml } from '@codemirror/legacy-modes/mode/toml';
  import { dockerFile } from '@codemirror/legacy-modes/mode/dockerfile';
  import { properties } from '@codemirror/legacy-modes/mode/properties';
  import { csharp, kotlin, scala, dart, objectiveC } from '@codemirror/legacy-modes/mode/clike';
  import { agentEditorFile, agentFsChanged } from '../stores';
  import { agentFsReadFile, agentFsWriteFile } from '../commands';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';

  let { file }: { file: { path: string; name: string } } = $props();

  let host = $state<HTMLDivElement>();
  let view: EditorView | null = null;
  let loading = $state(true);
  let readOnly = $state(false);
  let readOnlyReason = $state('');
  let dirty = $state(false);
  let externalChange = $state(false);
  let loadedPath = $state('');
  let confirmDiscard = $state(false);
  // Epoch ms of our last save; suppresses the watcher echo of our own write.
  let lastSaveAt = 0;

  function langFor(name: string): Extension[] {
    const lower = name.toLowerCase();
    const ext = lower.split('.').pop() ?? '';
    // Extension-less / special filenames.
    if (lower === 'dockerfile' || lower.endsWith('.dockerfile')) return [StreamLanguage.define(dockerFile)];
    switch (ext) {
      case 'js': case 'jsx': case 'mjs': case 'cjs': return [javascript({ jsx: true })];
      case 'ts': return [javascript({ typescript: true })];
      case 'tsx': return [javascript({ jsx: true, typescript: true })];
      case 'json': return [json()];
      case 'md': case 'markdown': return [markdown()];
      case 'xml': case 'svg': return [xml()];
      case 'py': return [python()];
      case 'rs': return [rust()];
      case 'go': return [go()];
      case 'html': case 'htm': case 'svelte': return [html()];
      case 'vue': return [vue()];
      case 'css': case 'less': return [css()];
      case 'scss': case 'sass': return [sass()];
      case 'java': return [java()];
      case 'c': case 'h': case 'cpp': case 'cc': case 'cxx': case 'hpp': case 'hh': return [cpp()];
      case 'php': return [php()];
      case 'yaml': case 'yml': return [yaml()];
      case 'cs': return [StreamLanguage.define(csharp)];
      case 'kt': case 'kts': return [StreamLanguage.define(kotlin)];
      case 'scala': case 'sc': return [StreamLanguage.define(scala)];
      case 'dart': return [StreamLanguage.define(dart)];
      case 'm': return [StreamLanguage.define(objectiveC)];
      case 'swift': return [StreamLanguage.define(swift)];
      case 'rb': return [StreamLanguage.define(ruby)];
      case 'lua': return [StreamLanguage.define(lua)];
      case 'pl': case 'pm': return [StreamLanguage.define(perl)];
      case 'sh': case 'bash': case 'zsh': case 'fish': return [StreamLanguage.define(shell)];
      case 'toml': return [StreamLanguage.define(toml)];
      case 'ini': case 'conf': case 'cfg': case 'properties': case 'env': return [StreamLanguage.define(properties)];
      default: return [];
    }
  }

  async function save() {
    if (!view || readOnly || !dirty) return;
    try {
      await agentFsWriteFile(file.path, view.state.doc.toString());
      lastSaveAt = Date.now();
      dirty = false;
      externalChange = false;
    } catch (e) {
      showToast(friendlyError(e), 'error');
    }
  }

  function destroyView() {
    view?.destroy();
    view = null;
  }

  async function load(path: string, name: string) {
    loading = true;
    readOnly = false;
    readOnlyReason = '';
    dirty = false;
    externalChange = false;
    destroyView();
    try {
      const res = await agentFsReadFile(path);
      if (res.tooLarge) {
        readOnly = true;
        readOnlyReason = `File too large to edit (${(res.size / 1024 / 1024).toFixed(1)} MB)`;
      } else if (res.isBinary || res.content === null) {
        readOnly = true;
        readOnlyReason = 'Binary file — preview not available';
      }
      loadedPath = path;
      loading = false;
      // Wait a tick for the host div to render now that loading=false.
      requestAnimationFrame(() => mountView(res.content ?? ''));
    } catch (e) {
      loading = false;
      readOnly = true;
      readOnlyReason = friendlyError(e);
    }
  }

  function mountView(content: string) {
    if (!host) return;
    destroyView();
    const extensions: Extension[] = [
      basicSetup,
      oneDark,
      ...langFor(file.name),
      keymap.of([
        { key: 'Mod-s', preventDefault: true, run: () => { save(); return true; } },
      ]),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) { dirty = true; externalChange = false; }
      }),
    ];
    if (readOnly) extensions.push(EditorState.readOnly.of(true));
    view = new EditorView({
      state: EditorState.create({ doc: content, extensions }),
      parent: host,
    });
  }

  async function reloadFromDisk() {
    await load(file.path, file.name);
  }

  // Load whenever the open file changes.
  $effect(() => {
    const f = file;
    if (f && f.path !== loadedPath) load(f.path, f.name);
  });

  // React to fs watcher: if the open file changed on disk, reload when
  // clean, otherwise flag a conflict so we never clobber unsaved edits.
  $effect(() => {
    const changed = $agentFsChanged;
    if (!changed.length || !loadedPath) return;
    if (!changed.includes(loadedPath)) return;
    // Ignore the watcher echo of our own save (avoids cursor-resetting reload).
    if (Date.now() - lastSaveAt < 1000) return;
    if (dirty) externalChange = true;
    else reloadFromDisk();
  });

  function close() {
    if (dirty) { confirmDiscard = true; return; }
    doClose();
  }
  function doClose() {
    confirmDiscard = false;
    agentEditorFile.set(null);
  }

  // Cmd/Ctrl+W closes the open FILE (not the tab) when focus is in the files
  // panel. Capture-phase on window beats the global bubble-phase tab handler.
  function onKeydownCapture(e: KeyboardEvent) {
    if (e.key !== 'w' || !(e.metaKey || e.ctrlKey)) return;
    const ae = document.activeElement as HTMLElement | null;
    if (ae?.closest('.editor-col, .explorer-col')) {
      e.preventDefault();
      e.stopImmediatePropagation();
      close();
    }
  }
  onMount(() => window.addEventListener('keydown', onKeydownCapture, true));
  onDestroy(() => {
    window.removeEventListener('keydown', onKeydownCapture, true);
    destroyView();
  });
</script>

<div class="editor-col">
  <div class="editor-head">
    <span class="editor-name" title={file.path}>
      {file.name}{#if dirty}<span class="dirty-dot" title="Unsaved changes"></span>{/if}
    </span>
    {#if externalChange}
      <button class="reload-btn" onclick={reloadFromDisk} title="File changed on disk — discard your edits and reload">
        Changed on disk · Reload
      </button>
    {/if}
    <div class="editor-head-spacer"></div>
    {#if !readOnly}
      <button class="ed-save" class:active={dirty} onclick={save} disabled={!dirty} title="Save (⌘S)">
        <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/><polyline points="17 21 17 13 7 13 7 21"/><polyline points="7 3 7 8 15 8"/></svg>
        <span>Save</span>
      </button>
    {/if}
    <button class="ed-close" onclick={close} title="Close (⌘W)" aria-label="Close file">
      <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
    </button>
  </div>
  {#if loading}
    <div class="editor-msg">Loading…</div>
  {:else if readOnly && readOnlyReason}
    <div class="editor-msg">{readOnlyReason}</div>
  {:else}
    <div class="editor-host" bind:this={host}></div>
  {/if}
</div>

<ConfirmDialog
  bind:show={confirmDiscard}
  title="Discard changes?"
  message={`You have unsaved changes in ${file.name}. Close without saving?`}
  confirmText="Discard"
  onconfirm={doClose}
/>

<style>
  .editor-col {
    flex: 1;
    min-width: 240px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-right: 1px solid var(--b1);
    background: var(--s);
  }
  .editor-head {
    height: 28px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    border-bottom: 1px solid var(--b1);
    font-size: 11px;
    font-family: var(--mono);
    color: var(--t2);
  }
  .editor-name {
    display: flex;
    align-items: center;
    gap: 5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dirty-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--acc, #7c5cf8);
    flex-shrink: 0;
  }
  .editor-head-spacer { flex: 1; }
  .ed-save {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    font-family: var(--mono);
    font-weight: 600;
    color: var(--t3);
    background: transparent;
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 3px 10px;
    cursor: default;
    transition: all 0.12s;
  }
  .ed-save.active {
    color: #fff;
    background: var(--acc, #7c5cf8);
    border-color: var(--acc, #7c5cf8);
    cursor: pointer;
    box-shadow: 0 1px 6px color-mix(in srgb, var(--acc, #7c5cf8) 45%, transparent);
  }
  .ed-save.active:hover { filter: brightness(1.08); }
  .ed-save:disabled { opacity: 0.55; }
  .ed-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 22px;
    color: var(--t3);
    background: transparent;
    border: none;
    border-radius: 5px;
    cursor: pointer;
    transition: color 0.12s, background 0.12s;
  }
  .ed-close:hover { color: var(--err, #ff5f57); background: var(--surface-hover); }
  .reload-btn {
    font-size: 10px;
    font-family: var(--mono);
    color: var(--warn, #fa0);
    background: transparent;
    border: 1px solid var(--warn, #fa0);
    border-radius: 4px;
    padding: 2px 8px;
    cursor: pointer;
  }
  .editor-msg {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
  }
  .editor-host {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .editor-host :global(.cm-editor) {
    height: 100%;
  }
  .editor-host :global(.cm-scroller) {
    overflow: auto;
  }
</style>

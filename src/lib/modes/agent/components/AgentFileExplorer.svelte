<script lang="ts">
  import { onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { Channel } from '@tauri-apps/api/core';
  import {
    setAgentExplorerOpen,
    agentExplorerWidth,
    agentEditorFile,
    agentFsChanged,
    agentFileDragging,
    agentTerminalIds,
    activeAgentSession,
  } from '../stores';
  import {
    agentFsListDir,
    agentFsCreate,
    agentFsRename,
    agentFsDelete,
    agentFsReveal,
    agentFsReadFile,
    agentSaveContext,
    agentAttachContext,
    agentInjectContexts,
    agentWriteToTerminal,
    agentFileReference,
    type FsEntry,
    type FsChange,
  } from '../commands';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import { showToast } from '$lib/shared/primitives/toast';
  import { friendlyError } from '$lib/utils/errors';

  let { root }: { root: string } = $props();

  // dirPath → its children (lazily loaded)
  let childrenMap = $state<Map<string, FsEntry[]>>(new Map());
  let expanded = $state<Set<string>>(new Set());
  let selected = $state('');
  let rootEntries = $state<FsEntry[]>([]);
  let colEl = $state<HTMLDivElement>();

  let watchChannel: Channel<FsChange> | null = null;
  let watchedRoot = '';
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;

  function relPath(abs: string): string {
    if (abs.startsWith(root)) {
      return abs.slice(root.length).replace(/^[/\\]/, '');
    }
    return abs;
  }

  async function loadDir(path: string): Promise<FsEntry[]> {
    try {
      const entries = await agentFsListDir(path);
      childrenMap.set(path, entries);
      childrenMap = new Map(childrenMap);
      return entries;
    } catch (e) {
      showToast(friendlyError(e), 'error');
      return [];
    }
  }

  async function loadRoot() {
    rootEntries = await loadDir(root);
  }

  async function toggleDir(entry: FsEntry) {
    if (expanded.has(entry.path)) {
      expanded.delete(entry.path);
    } else {
      expanded.add(entry.path);
      if (!childrenMap.has(entry.path)) await loadDir(entry.path);
    }
    expanded = new Set(expanded);
  }

  function openFile(entry: FsEntry) {
    selected = entry.path;
    agentEditorFile.set({ path: entry.path, name: entry.name });
  }

  function onItemClick(entry: FsEntry) {
    // A pointer-drag just happened on this row — swallow the trailing click
    // so we don't also open/expand the item.
    if (didDrag) { didDrag = false; return; }
    if (entry.isDir) toggleDir(entry);
    else openFile(entry);
  }

  // ---- Pointer-based drag (Tauri's native drag-drop blocks HTML5 DnD
  //      drop events in the webview, so we drive the drag with mouse
  //      events and hit-test the terminal region on mouseup) ------------
  let dragGhost = $state<{ name: string; x: number; y: number } | null>(null);
  let pendingDrag: { entry: FsEntry; x: number; y: number } | null = null;
  let didDrag = false;

  function onRowMouseDown(e: MouseEvent, entry: FsEntry) {
    if (e.button !== 0) return;
    // Focus the panel so Cmd/Ctrl+W targets the open file, not the tab.
    colEl?.focus({ preventScroll: true });
    pendingDrag = { entry, x: e.clientX, y: e.clientY };
    didDrag = false;
    window.addEventListener('mousemove', onDragMove);
    window.addEventListener('mouseup', onDragUp);
  }

  function onDragMove(e: MouseEvent) {
    if (!pendingDrag) return;
    if (!didDrag && Math.hypot(e.clientX - pendingDrag.x, e.clientY - pendingDrag.y) < 5) return;
    if (!didDrag) { didDrag = true; agentFileDragging.set(true); }
    dragGhost = { name: pendingDrag.entry.name, x: e.clientX, y: e.clientY };
  }

  async function onDragUp(e: MouseEvent) {
    window.removeEventListener('mousemove', onDragMove);
    window.removeEventListener('mouseup', onDragUp);
    const drag = pendingDrag;
    pendingDrag = null;
    dragGhost = null;
    agentFileDragging.set(false);
    if (!didDrag || !drag || drag.entry.isDir) return;
    const el = document.elementFromPoint(e.clientX, e.clientY);
    if (!el || !el.closest('.agent-terminal-region')) return;
    await insertIntoTerminal(drag.entry);
  }

  async function insertIntoTerminal(entry: FsEntry) {
    const session = get(activeAgentSession);
    if (!session) return;
    const termId = get(agentTerminalIds).get(session.id);
    if (!termId) return;
    try {
      const ref = await agentFileReference(session.provider || 'claude', relPath(entry.path));
      await agentWriteToTerminal(termId, ref + ' ');
    } catch { /* ignore */ }
  }

  // ---- File operations ------------------------------------------------

  async function refreshParentOf(path: string) {
    const parent = path.replace(/[/\\][^/\\]+$/, '');
    if (parent === root || childrenMap.has(parent)) {
      if (parent === root) await loadRoot();
      else await loadDir(parent);
    } else {
      await loadRoot();
    }
  }

  async function doRename(entry: FsEntry) {
    const next = prompt('Rename to:', entry.name);
    if (!next || next === entry.name) return;
    const target = entry.path.replace(/[^/\\]+$/, next);
    try {
      await agentFsRename(entry.path, target);
      await refreshParentOf(entry.path);
    } catch (e) { showToast(friendlyError(e), 'error'); }
  }

  async function doDelete(entry: FsEntry) {
    if (!confirm(`Delete ${entry.name}?${entry.isDir ? ' This removes the folder and its contents.' : ''}`)) return;
    try {
      await agentFsDelete(entry.path);
      if (get(agentEditorFile)?.path === entry.path) agentEditorFile.set(null);
      await refreshParentOf(entry.path);
    } catch (e) { showToast(friendlyError(e), 'error'); }
  }

  async function doCreate(dir: string, isDir: boolean) {
    const name = prompt(isDir ? 'New folder name:' : 'New file name:');
    if (!name) return;
    const target = `${dir}/${name}`;
    try {
      await agentFsCreate(target, isDir);
      if (!expanded.has(dir) && dir !== root) { expanded.add(dir); expanded = new Set(expanded); }
      if (dir === root) await loadRoot(); else await loadDir(dir);
    } catch (e) { showToast(friendlyError(e), 'error'); }
  }

  async function addToContext(entry: FsEntry) {
    const session = $activeAgentSession;
    if (!session) return;
    try {
      const fc = await agentFsReadFile(entry.path);
      if (fc.content == null) { showToast('Cannot add a binary/large file to context', 'error'); return; }
      const ctx = await agentSaveContext({ name: relPath(entry.path), content: fc.content });
      await agentAttachContext(session.id, ctx.id);
      await agentInjectContexts(session.worktreePath || session.projectPath, [ctx.id], session.provider || 'claude');
      showToast(`Added ${entry.name} to session context`, 'success');
    } catch (e) { showToast(friendlyError(e), 'error'); }
  }

  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); } catch { /* ignore */ }
  }

  function onContextMenu(e: MouseEvent, entry: FsEntry) {
    e.preventDefault();
    const dirForNew = entry.isDir ? entry.path : entry.path.replace(/[/\\][^/\\]+$/, '');
    const items = [
      ...(entry.isDir ? [] : [{ label: 'Open', action: () => openFile(entry) }]),
      { label: 'Add to session context', action: () => addToContext(entry) },
      { label: 'Rename', action: () => doRename(entry) },
      { label: 'Delete', danger: true, action: () => doDelete(entry) },
      { label: '', separator: true, action: () => {} },
      { label: 'New file…', action: () => doCreate(dirForNew, false) },
      { label: 'New folder…', action: () => doCreate(dirForNew, true) },
      { label: '', separator: true, action: () => {} },
      { label: 'Copy path', action: () => copy(entry.path) },
      { label: 'Copy relative path', action: () => copy(relPath(entry.path)) },
      { label: 'Reveal in file manager', action: () => agentFsReveal(entry.path).catch((er) => showToast(friendlyError(er), 'error')) },
    ];
    showContextMenu(e.clientX, e.clientY, items);
  }

  // ---- Watcher --------------------------------------------------------

  function scheduleRefresh(paths: string[]) {
    agentFsChanged.set(paths);
    if (refreshTimer) clearTimeout(refreshTimer);
    refreshTimer = setTimeout(async () => {
      refreshTimer = null;
      await loadRoot();
      for (const dir of Array.from(childrenMap.keys())) {
        if (dir !== root) await loadDir(dir);
      }
    }, 150);
  }

  function startWatch(path: string) {
    stopWatch();
    watchedRoot = path;
    watchChannel = new Channel<FsChange>();
    watchChannel.onmessage = (msg) => { if (msg?.paths?.length) scheduleRefresh(msg.paths); };
    import('../commands').then(({ agentFsWatchStart }) => {
      agentFsWatchStart(path, watchChannel).catch(() => {});
    });
  }

  function stopWatch() {
    if (!watchedRoot) return;
    watchedRoot = '';
    watchChannel = null;
    import('../commands').then(({ agentFsWatchStop }) => { agentFsWatchStop().catch(() => {}); });
  }

  // (Re)load + (re)watch whenever the root changes.
  $effect(() => {
    const r = root;
    if (!r) return;
    childrenMap = new Map();
    expanded = new Set();
    rootEntries = [];
    selected = get(agentEditorFile)?.path ?? '';
    loadRoot();
    startWatch(r);
  });

  onDestroy(stopWatch);

  // ---- Resize handle --------------------------------------------------

  function onResizeStart(e: MouseEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = $agentExplorerWidth;
    function move(ev: MouseEvent) {
      agentExplorerWidth.set(Math.max(160, Math.min(560, startW + (ev.clientX - startX))));
    }
    function up() {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    }
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  }
</script>

{#snippet node(entry: FsEntry, depth: number)}
  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
  <div
    class="row"
    class:selected={selected === entry.path}
    style="padding-left:{6 + depth * 12}px"
    onmousedown={(e) => onRowMouseDown(e, entry)}
    onclick={() => onItemClick(entry)}
    oncontextmenu={(e) => onContextMenu(e, entry)}
    title={entry.name}
  >
    {#if entry.isDir}
      <svg class="chev" class:open={expanded.has(entry.path)} viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="9 6 15 12 9 18"/></svg>
      <svg class="ic" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>
    {:else}
      <span class="chev-spacer"></span>
      <svg class="ic" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
    {/if}
    <span class="label">{entry.name}</span>
  </div>
  {#if entry.isDir && expanded.has(entry.path)}
    {#each childrenMap.get(entry.path) ?? [] as child (child.path)}
      {@render node(child, depth + 1)}
    {/each}
  {/if}
{/snippet}

<div class="explorer-col" bind:this={colEl} tabindex="-1" style="width:{$agentExplorerWidth}px">
  <div class="explorer-head">
    <span class="explorer-title">EXPLORER</span>
    <div class="head-spacer"></div>
    <button class="head-btn" onclick={() => doCreate(root, false)} title="New file">
      <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><line x1="12" y1="11" x2="12" y2="17"/><line x1="9" y1="14" x2="15" y2="14"/></svg>
    </button>
    <button class="head-btn" onclick={loadRoot} title="Refresh">
      <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
    </button>
    <button class="head-btn" onclick={() => { const s = get(activeAgentSession); if (s) setAgentExplorerOpen(s.id, false); }} title="Close explorer">
      <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
    </button>
  </div>
  <div class="explorer-tree">
    {#each rootEntries as entry (entry.path)}
      {@render node(entry, 0)}
    {/each}
    {#if rootEntries.length === 0}
      <div class="empty">Empty folder</div>
    {/if}
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="resize-handle" onmousedown={onResizeStart}></div>
</div>

{#if dragGhost}
  <div class="drag-ghost" style="left:{dragGhost.x + 12}px; top:{dragGhost.y + 6}px">{dragGhost.name}</div>
{/if}

<style>
  .explorer-col {
    position: relative;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-right: 1px solid var(--b1);
    background: var(--s);
    min-width: 160px;
  }
  .explorer-col:focus { outline: none; }
  .explorer-head {
    height: 28px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 6px;
    border-bottom: 1px solid var(--b1);
  }
  .explorer-title {
    font-size: 10px;
    letter-spacing: 0.06em;
    font-weight: 700;
    color: var(--t3);
    font-family: var(--mono);
  }
  .head-spacer { flex: 1; }
  .head-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    color: var(--t3);
    cursor: pointer;
    padding: 3px;
    border-radius: 4px;
    transition: color 0.1s, background 0.1s;
  }
  .head-btn:hover { color: var(--t1); background: var(--surface-hover); }
  .explorer-tree {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 4px 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding-right: 6px;
    cursor: pointer;
    font-size: 12px;
    color: var(--t2);
    font-family: var(--mono);
    user-select: none;
    white-space: nowrap;
  }
  .row:hover { background: var(--surface-hover); }
  .row.selected { background: color-mix(in srgb, var(--acc, #7c5cf8) 22%, transparent); color: var(--t1); }
  .chev {
    flex-shrink: 0;
    color: var(--t4);
    transition: transform 0.1s;
  }
  .chev.open { transform: rotate(90deg); }
  .chev-spacer { width: 10px; flex-shrink: 0; }
  .ic { flex-shrink: 0; color: var(--t3); }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .empty {
    padding: 12px;
    color: var(--t4);
    font-size: 11px;
    font-family: var(--mono);
  }
  .resize-handle {
    position: absolute;
    top: 0;
    right: -2px;
    width: 5px;
    height: 100%;
    cursor: col-resize;
    z-index: 5;
  }
  .resize-handle:hover { background: var(--acc, #7c5cf8); opacity: 0.4; }
  .drag-ghost {
    position: fixed;
    z-index: 9999;
    pointer-events: none;
    padding: 3px 8px;
    border-radius: 4px;
    background: var(--acc, #7c5cf8);
    color: #fff;
    font-size: 11px;
    font-family: var(--mono);
    box-shadow: 0 4px 12px rgba(0,0,0,0.3);
    white-space: nowrap;
  }
</style>

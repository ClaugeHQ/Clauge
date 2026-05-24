<script lang="ts">
  import {
    fsList,
    fsMkdir,
    fsDelete,
    fsRename,
    openSession,
    closeSession,
    fsHomeDir,
    uploadFile,
    downloadFile,
  } from '$lib/modes/explorer/commands';
  import { onMount, onDestroy } from 'svelte';
  import { explorerConnections, explorerConnStates } from '$lib/modes/explorer/stores';
  import { tabs as tabsStore, closeTab } from '$lib/shared/stores/tabs';
  import { get } from 'svelte/store';
  import { posixDirname, posixJoin, pathSegments } from '$lib/modes/explorer/utils/path';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import type { DirEntry } from '$lib/modes/explorer/types';

  interface Props {
    tabKey: string;
    connectionId: string;
  }

  let { tabKey, connectionId }: Props = $props();

  // Initial path: SFTP/FTP start at "/", S3/Azure start at "/<bucket-or-container>".
  const conn = $derived($explorerConnections.find((c) => c.id === connectionId));
  /** 'connecting' | 'connected' | 'error' | 'disconnected'. Drives whether
   *  we mount the file listing or show the connect-progress UI. Untracked
   *  tab keys (e.g. tabs restored from disk on app reload) default to
   *  'disconnected' so we render a "reconnect" prompt rather than firing
   *  fsList against a session that doesn't exist. */
  const connState = $derived($explorerConnStates.get(tabKey) ?? 'disconnected');
  let cwd = $state('/');
  let entries = $state<DirEntry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let initialLoaded = $state(false);

  // Pending delete confirm.
  let pendingDeletePaths = $state<string[]>([]);
  let showDeleteConfirm = $state(false);

  // In-app prompt dialog. Tauri's webview blocks window.prompt() / confirm()
  // so we render our own. One dialog covers both "new folder name" and
  // "rename" — `promptOnSubmit` decides which action to take.
  let promptShow = $state(false);
  let promptTitle = $state('');
  let promptLabel = $state('');
  let promptValue = $state('');
  let promptPlaceholder = $state('');
  let promptOnSubmit = $state<((v: string) => void) | null>(null);
  let promptInputEl = $state<HTMLInputElement | null>(null);
  // After Modal teleports to body, autofocus the input. We can't rely on
  // bind:this firing pre-mount + use:teleport rearranging the DOM.
  $effect(() => {
    if (promptShow && promptInputEl) {
      requestAnimationFrame(() => {
        promptInputEl?.focus();
        promptInputEl?.select();
      });
    }
  });

  /** Synchronous best-guess of the starting path. Used for S3/Azure
   *  (always derivable from config) and as the fallback for SFTP/FTP if
   *  the user *did* supply a working dir. For SFTP/FTP without a working
   *  dir we instead resolve the server-side home asynchronously below. */
  function initialPath(): string {
    if (!conn) return '/';
    if (conn.kind === 's3' && conn.s3Bucket) return `/${conn.s3Bucket}`;
    if (conn.kind === 'azure_blob' && conn.azureContainer) return `/${conn.azureContainer}`;
    if (conn.kind === 'sftp' && conn.sftpWorkingDir) return conn.sftpWorkingDir;
    return '/';
  }

  /** Whether we should ask the server for its default directory rather
   *  than starting at `/`. Mirrors what FileZilla / WinSCP do — most SFTP
   *  servers chroot users into `/home/<user>` and refuse to list `/`,
   *  so the naive start fails with "permission denied" on first list. */
  function shouldResolveHomeDir(): boolean {
    if (!conn) return false;
    if (conn.kind === 'sftp') return !conn.sftpWorkingDir;
    if (conn.kind === 'ftp') return true; // FTP has no per-conn working dir field
    return false;
  }

  async function refresh() {
    loading = true;
    error = null;
    try {
      const data = await fsList(tabKey, cwd);
      entries = data.sort((a, b) => {
        // Directories first, then alphabetical.
        if (a.kind === 'dir' && b.kind !== 'dir') return -1;
        if (a.kind !== 'dir' && b.kind === 'dir') return 1;
        return a.name.localeCompare(b.name);
      });
    } catch (e: any) {
      error = String(e);
      entries = [];
    } finally {
      loading = false;
    }
  }

  function go(path: string) {
    cwd = path;
    refresh();
  }

  function goUp() {
    if (cwd === '/' || cwd === '') return;
    go(posixDirname(cwd));
  }

  function openEntry(e: DirEntry) {
    if (e.kind === 'dir') go(e.path);
    // File preview is a v2 polish — Stage 6 ships navigation only.
  }

  function openPrompt(opts: { title: string; label: string; value?: string; placeholder?: string; onSubmit: (v: string) => void }) {
    promptTitle = opts.title;
    promptLabel = opts.label;
    promptValue = opts.value ?? '';
    promptPlaceholder = opts.placeholder ?? '';
    promptOnSubmit = opts.onSubmit;
    promptShow = true;
  }

  function submitPrompt() {
    const v = promptValue.trim();
    if (!v) return;
    const fn = promptOnSubmit;
    promptShow = false;
    promptOnSubmit = null;
    fn?.(v);
  }

  function handleMkdir() {
    openPrompt({
      title: 'New folder',
      label: 'Folder name',
      placeholder: 'e.g. uploads',
      onSubmit: async (name) => {
        try {
          await fsMkdir(tabKey, posixJoin(cwd, name));
          await refresh();
        } catch (err: any) {
          errorToast('mkdir failed', err);
        }
      },
    });
  }

  function handleDeleteEntry(e: DirEntry) {
    pendingDeletePaths = [e.path];
    showDeleteConfirm = true;
  }

  async function confirmDelete() {
    if (pendingDeletePaths.length === 0) return;
    try {
      await fsDelete(tabKey, pendingDeletePaths);
      await refresh();
      showToast(`Deleted ${pendingDeletePaths.length} item(s)`, 'success');
    } catch (err: any) {
      errorToast('Delete failed', err);
    }
    pendingDeletePaths = [];
  }

  function handleRowContextMenu(ev: MouseEvent, e: DirEntry) {
    ev.preventDefault();
    ev.stopPropagation();
    const items: any[] = [];
    if (e.kind === 'dir') {
      items.push({ label: 'Open', action: () => openEntry(e) });
    } else if (e.kind === 'file') {
      items.push({ label: 'Download to…', action: () => handleDownload(e) });
    }
    items.push({ label: 'Rename', action: () => handleRename(e) });
    items.push({ label: '', action: () => {}, separator: true });
    items.push({ label: 'Delete', danger: true, action: () => handleDeleteEntry(e) });
    showContextMenu(ev.clientX, ev.clientY, items);
  }

  /** Right-click → "Download to…" — picks a local destination via the
   *  Tauri save dialog, then kicks off the download. Progress is rendered
   *  by TransfersPanel on the global `explorer:transfer` channel. */
  async function handleDownload(e: DirEntry) {
    try {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const dest = await save({ defaultPath: e.name, title: `Download ${e.name}` });
      if (typeof dest !== 'string' || !dest) return;
      const id = (crypto.randomUUID?.() ?? `${Date.now()}-${Math.random()}`) as string;
      downloadFile(id, tabKey, e.path, dest).catch((err) =>
        errorToast('Download failed', err),
      );
    } catch (err: any) {
      errorToast('Download failed', err);
    }
  }

  /** Drag-and-drop upload. We use Tauri's window-level onDragDropEvent so
   *  we get OS file paths (HTML5 drag-and-drop in webview only gives the
   *  filename, not the absolute path — useless for native uploads).
   *
   *  Multi-file drop fires N parallel uploadFile calls; the backend caps
   *  per-tab concurrency by the SFTP / FTP / S3 client itself. v1 ignores
   *  folder drops to keep behaviour predictable — caller will see the
   *  files inside the folder dropped one level deep, but not nested.
   */
  let dragOver = $state(false);
  let dragDepth = 0; // counts enter/leave so child elements don't flicker
  let unlistenDrop: (() => void) | null = null;
  onMount(async () => {
    try {
      const { getCurrentWebview } = await import('@tauri-apps/api/webview');
      const win = getCurrentWebview();
      const unsub = await win.onDragDropEvent((event) => {
        // Only react to drops while this tab is the active one — otherwise
        // every visible explorer tab would race to upload to its own cwd.
        const t = (event.payload as any).type;
        if (t === 'enter') {
          dragDepth++;
          dragOver = true;
        } else if (t === 'over') {
          dragOver = true;
        } else if (t === 'leave') {
          dragDepth = Math.max(0, dragDepth - 1);
          if (dragDepth === 0) dragOver = false;
        } else if (t === 'drop') {
          dragDepth = 0;
          dragOver = false;
          if (connState !== 'connected') {
            showToast('Connect first before dropping files', 'error');
            return;
          }
          const paths: string[] = (event.payload as any).paths ?? [];
          if (paths.length === 0) return;
          startUploads(paths);
        }
      });
      unlistenDrop = unsub;
    } catch {
      // Drag-drop API only available in Tauri webview — silent in web/dev.
    }
  });
  onDestroy(() => {
    try { unlistenDrop?.(); } catch { /* noop */ }
  });

  async function startUploads(paths: string[]) {
    // Folder uploads are out of scope for v1 — the backend's File::open will
    // fail cleanly for a directory path and surface as a transfer "failed"
    // entry, which is enough for now without pulling in plugin-fs just for
    // a stat call. (See the design doc for recursive-upload roadmap.)
    for (const p of paths) {
      const name = p.replace(/\\/g, '/').split('/').pop() ?? 'upload';
      const id = (crypto.randomUUID?.() ?? `${Date.now()}-${Math.random()}-${name}`) as string;
      const dest = posixJoin(cwd, name);
      uploadFile(id, tabKey, p, dest)
        .then(() => refresh())
        .catch((err) => errorToast('Upload failed', err));
    }
  }

  function handleRename(e: DirEntry) {
    openPrompt({
      title: 'Rename',
      label: 'New name',
      value: e.name,
      placeholder: e.name,
      onSubmit: async (next) => {
        if (next === e.name) return;
        try {
          await fsRename(tabKey, e.path, posixJoin(posixDirname(e.path), next));
          await refresh();
        } catch (err: any) {
          errorToast('Rename failed', err);
        }
      },
    });
  }

  function formatSize(b: number | null): string {
    if (b === null || b === undefined) return '—';
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
    if (b < 1024 * 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MB`;
    return `${(b / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  // Only kick off the first list once the Rust session is live. Mounting
  // before openSession completes would race and produce "no active session
  // for this tab" errors that confuse users into thinking the connection
  // failed. Gating here means the spinner shows until it's actually safe
  // to list, and a single fsList runs the moment we transition to 'connected'.
  $effect(() => {
    if (connState === 'connected' && !initialLoaded) {
      initialLoaded = true;
      void resolveStartingDirAndList();
    }
  });

  /** Resolve the starting path then list it. For SFTP/FTP without an
   *  explicit working dir, ask the server for its default (realpath / PWD)
   *  before listing — naively starting at `/` triggers "permission denied"
   *  on chrooted accounts. */
  async function resolveStartingDirAndList() {
    let start = initialPath();
    if (shouldResolveHomeDir()) {
      try {
        const remote = await fsHomeDir(tabKey);
        if (remote && remote.length > 0) start = remote;
      } catch {
        // Server doesn't support realpath/PWD — fall back to "/".
      }
    }
    cwd = start;
    await refresh();
  }

  /** Auto-reconnect a tab that's been restored from disk on app launch
   *  (state defaults to 'disconnected' because the in-memory session map
   *  was cleared). Mirrors SshPanel's auto-attach behaviour — the user
   *  clicked once to open the tab; they shouldn't have to click again
   *  just because the binary restarted. */
  let attempted = $state(false);
  $effect(() => {
    if (!attempted && connectionId && connState === 'disconnected') {
      attempted = true;
      explorerConnStates.update((m) => {
        const next = new Map(m);
        next.set(tabKey, 'connecting');
        return next;
      });
      openSession(connectionId, tabKey)
        .then(() => {
          explorerConnStates.update((m) => {
            const next = new Map(m);
            next.set(tabKey, 'connected');
            return next;
          });
        })
        .catch((err) => {
          explorerConnStates.update((m) => {
            const next = new Map(m);
            next.set(tabKey, 'error');
            return next;
          });
          errorToast('Connection failed', err);
        });
    }
  });

  async function retryConnect() {
    if (!connectionId) return;
    attempted = false; // allow the auto-reconnect effect to fire again
    explorerConnStates.update((m) => {
      const next = new Map(m);
      next.set(tabKey, 'disconnected');
      return next;
    });
  }

  /** Cancel an in-flight connect — mirrors SshPanel.cancelConnect. Tears
   *  down the Rust session (if openSession ended up succeeding after the
   *  user gave up) and closes the tab. */
  async function cancelConnect() {
    try { await closeSession(tabKey); } catch { /* ignore */ }
    explorerConnStates.update((m) => {
      const next = new Map(m);
      next.delete(tabKey);
      return next;
    });
    const tab = get(tabsStore).find((t) => t.mode === 'explorer' && t.key === tabKey);
    if (tab) closeTab(tab.id);
  }

  /** Sub-line shown under the connection name on the loading overlay —
   *  matches the per-kind sub-line shown in the nav row, so the loader
   *  visibly identifies *which* remote we're hitting. */
  function loadingSubLine(): string {
    if (!conn) return '';
    if (conn.kind === 'sftp' || conn.kind === 'ftp') {
      const u = conn.username ?? '';
      const h = conn.host ?? '';
      const portSuffix = conn.port && ((conn.kind === 'sftp' && conn.port !== 22) || (conn.kind === 'ftp' && conn.port !== 21))
        ? `:${conn.port}` : '';
      return h ? `${u}${u ? '@' : ''}${h}${portSuffix}` : '';
    }
    if (conn.kind === 's3') return conn.s3Bucket ?? '';
    if (conn.kind === 'azure_blob') return `${conn.azureAccount ?? ''}/${conn.azureContainer ?? ''}`;
    return '';
  }

  function loadingIcon(): string {
    if (!conn) return '';
    if (conn.kind === 'sftp' || conn.kind === 'ftp') {
      return '<svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="var(--acc)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="4" width="20" height="6" rx="1"/><rect x="2" y="14" width="20" height="6" rx="1"/><line x1="6" y1="7" x2="6.01" y2="7"/><line x1="6" y1="17" x2="6.01" y2="17"/></svg>';
    }
    return '<svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="var(--acc)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M18 10h-1.26A8 8 0 109 20h9a5 5 0 000-10z"/></svg>';
  }

  const breadcrumbs = $derived.by(() => {
    const segs = pathSegments(cwd);
    let acc = '';
    return segs.map((s) => {
      acc = acc + '/' + s;
      return { name: s, path: acc };
    });
  });
</script>

<div class="fb">
  <!-- Path / actions toolbar -->
  <div class="fb-toolbar">
    <button class="fb-btn fb-icon-btn" onclick={goUp} disabled={cwd === '/' || cwd === ''} title="Up">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 19V5"/><path d="M5 12l7-7 7 7"/>
      </svg>
    </button>
    <button class="fb-btn fb-icon-btn" onclick={refresh} title="Refresh">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 12a9 9 0 0115.5-6.36L21 8"/><path d="M21 3v5h-5"/>
        <path d="M21 12a9 9 0 01-15.5 6.36L3 16"/><path d="M3 21v-5h5"/>
      </svg>
    </button>
    <div class="fb-crumbs">
      <button class="fb-crumb fb-crumb-root" onclick={() => go('/')} title="Root">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
          <path d="M3 12l9-9 9 9"/><path d="M5 10v10a1 1 0 001 1h4v-6h4v6h4a1 1 0 001-1V10"/>
        </svg>
      </button>
      {#each breadcrumbs as c (c.path)}
        <span class="fb-sep">/</span>
        <button class="fb-crumb" onclick={() => go(c.path)}>{c.name}</button>
      {/each}
    </div>
    <div class="fb-spacer"></div>
    <button class="fb-btn fb-btn-text" onclick={handleMkdir} title="Create new folder">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
        <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
        <line x1="12" y1="11" x2="12" y2="17"/><line x1="9" y1="14" x2="15" y2="14"/>
      </svg>
      <span>New folder</span>
    </button>
  </div>

  <!-- Entries table -->
  <div class="fb-body">
    {#if loading}
      <div class="fb-state">
        <div class="fb-spin" aria-hidden="true"></div>
        <div class="fb-state-text">
          <span class="fb-state-title">Loading directory</span>
          <span class="fb-state-sub"><span class="fb-state-path">{cwd}</span><span class="fb-dots"></span></span>
        </div>
      </div>
    {:else if error}
      <div class="fb-state fb-state-err">
        <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="var(--err)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10"/>
          <line x1="12" y1="8" x2="12" y2="13"/>
          <circle cx="12" cy="16.5" r="0.6" fill="var(--err)"/>
        </svg>
        <div class="fb-state-text">
          <span class="fb-state-title">Couldn't load directory</span>
          <span class="fb-state-sub fb-state-err-detail">{error}</span>
        </div>
      </div>
    {:else if entries.length === 0}
      <div class="fb-state">
        <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="var(--acc)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
        </svg>
        <div class="fb-state-text">
          <span class="fb-state-title">Empty directory</span>
          <span class="fb-state-sub">Nothing in <span class="fb-state-path">{cwd}</span> — drop files here to upload</span>
        </div>
      </div>
    {:else}
      <table class="fb-table">
        <thead>
          <tr>
            <th class="fb-col-name">Name</th>
            <th class="fb-col-size">Size</th>
            <th class="fb-col-mod">Modified</th>
            <th class="fb-col-perm">Perms</th>
          </tr>
        </thead>
        <tbody>
          {#each entries as e (e.path)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <tr
              class="fb-row"
              ondblclick={() => openEntry(e)}
              oncontextmenu={(ev) => handleRowContextMenu(ev, e)}
            >
              <td class="fb-col-name">
                <span class="fb-icon" class:dir={e.kind === 'dir'} class:link={e.kind === 'symlink'}>
                  {#if e.kind === 'dir'}
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
                    </svg>
                  {:else if e.kind === 'symlink'}
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M10 13a5 5 0 007.54.54l3-3a5 5 0 00-7.07-7.07l-1.72 1.71"/>
                      <path d="M14 11a5 5 0 00-7.54-.54l-3 3a5 5 0 007.07 7.07l1.71-1.71"/>
                    </svg>
                  {:else}
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/>
                      <polyline points="14 2 14 8 20 8"/>
                    </svg>
                  {/if}
                </span>
                {e.name}
              </td>
              <td class="fb-col-size">{formatSize(e.size)}</td>
              <td class="fb-col-mod">{e.modified ?? '—'}</td>
              <td class="fb-col-perm">{e.permissions ?? '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>

  <!-- Connect-state overlay — same design pattern as SshPanel.ssh-loading
       (centered icon + title + sub@host with animated dots + cancel).
       Shown for connecting / disconnected (auto-reconnecting) / error.
       Sits above the toolbar+body so the user only sees one thing at a time
       while we're not yet ready to list. -->
  <!-- Drag-and-drop overlay — shown while the OS is hovering files over
       the window. Only enabled when the session is live, otherwise the
       drop handler raises a toast instead of starting failing uploads. -->
  {#if dragOver && connState === 'connected'}
    <div class="fb-drop" aria-hidden="true">
      <div class="fb-drop-card">
        <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="var(--acc)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 19V5"/>
          <path d="M5 12l7-7 7 7"/>
          <path d="M5 21h14"/>
        </svg>
        <div class="fb-drop-title">Drop to upload</div>
        <div class="fb-drop-sub">into <span class="fb-drop-path">{cwd}</span></div>
      </div>
    </div>
  {/if}

  {#if connState === 'connecting' || connState === 'disconnected' || connState === 'error'}
    <div class="fb-overlay" role="status">
      {#if connState === 'error'}
        <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="var(--err)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10"/>
          <line x1="15" y1="9" x2="9" y2="15"/>
          <line x1="9" y1="9" x2="15" y2="15"/>
        </svg>
      {:else}
        {@html loadingIcon()}
      {/if}
      <div class="fb-overlay-text">
        {#if connState === 'error'}
          <span class="fb-overlay-title">Failed to connect to {conn?.name ?? 'server'}</span>
          <span class="fb-overlay-sub">{loadingSubLine()}</span>
        {:else}
          <span class="fb-overlay-title">Connecting to {conn?.name ?? 'server'}</span>
          <span class="fb-overlay-sub">{loadingSubLine()}<span class="fb-dots"></span></span>
        {/if}
      </div>
      {#if connState === 'error'}
        <div class="fb-overlay-actions">
          <button class="fb-overlay-btn primary" onclick={retryConnect}>Retry</button>
          <button class="fb-overlay-btn" onclick={cancelConnect}>Close tab</button>
        </div>
      {:else}
        <button class="fb-overlay-btn" onclick={cancelConnect}>Cancel</button>
      {/if}
    </div>
  {/if}
</div>

<ConfirmDialog
  bind:show={showDeleteConfirm}
  title="Delete"
  message={pendingDeletePaths.length === 1
    ? `Delete "${pendingDeletePaths[0]}"?`
    : `Delete ${pendingDeletePaths.length} items?`}
  confirmText="Delete"
  onconfirm={confirmDelete}
/>

<Modal bind:show={promptShow} title={promptTitle} width="380px">
  <form class="fb-prompt" onsubmit={(e) => { e.preventDefault(); submitPrompt(); }}>
    <label class="fb-prompt-row">
      <span class="fb-prompt-label">{promptLabel}</span>
      <input
        bind:this={promptInputEl}
        bind:value={promptValue}
        class="fb-prompt-input"
        type="text"
        placeholder={promptPlaceholder}
      />
    </label>
    <div class="fb-prompt-actions">
      <button type="button" class="fb-prompt-btn" onclick={() => (promptShow = false)}>Cancel</button>
      <button type="submit" class="fb-prompt-btn primary">OK</button>
    </div>
  </form>
</Modal>

<style>
  .fb {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
    position: relative; /* needed for the absolute connect overlay */
  }
  .fb-toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--b1);
    background: var(--n2);
    flex-shrink: 0;
  }
  .fb-btn {
    height: 28px;
    padding: 0 10px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    border-radius: 5px;
    cursor: default;
    font-size: 12px;
    font-family: var(--ui);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    transition: background 0.1s, border-color 0.1s, color 0.1s;
  }
  .fb-btn:hover:not(:disabled) {
    color: var(--t1);
    border-color: var(--b2);
    background: var(--surface-hover);
  }
  .fb-btn:disabled { opacity: 0.4; cursor: default; }
  .fb-icon-btn { width: 28px; padding: 0; }
  .fb-icon-btn svg { width: 14px; height: 14px; }
  .fb-btn-text svg { width: 13px; height: 13px; }
  .fb-crumbs {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    overflow: hidden;
    margin-left: 6px;
    font-family: var(--mono);
  }
  .fb-crumb {
    border: none;
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    padding: 2px 4px;
    cursor: default;
    border-radius: 3px;
  }
  .fb-crumb:hover { color: var(--t1); background: var(--surface-hover); }
  .fb-sep { color: var(--t4); }
  .fb-spacer { flex: 1; }
  .fb-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    /* Flex column so .fb-state (a single child in those branches) can
       flex-fill and centre vertically. Table case is unaffected — it
       grows naturally and the body keeps scrolling. */
    display: flex;
    flex-direction: column;
  }
  /* Centered state block — empty / loading / error all use this so the
     in-body messages match the connect overlay's "icon + title + sub"
     visual language. Inline (not absolute) so the toolbar stays usable. */
  .fb-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    padding: 24px;
  }
  /* Plain CSS spinner — single source of truth for the in-body "loading"
     indicator. Sized to match the 40px icon footprint of empty/error
     states so the visual rhythm stays consistent. */
  .fb-spin {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 2px solid var(--b1);
    border-top-color: var(--acc);
    animation: fb-spin 0.7s linear infinite;
  }
  @keyframes fb-spin {
    to { transform: rotate(360deg); }
  }
  .fb-state-text {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    text-align: center;
  }
  .fb-state-title {
    font-size: 14px;
    font-weight: 500;
    color: var(--t2);
    font-family: var(--ui);
  }
  .fb-state-sub {
    font-size: 11.5px;
    color: var(--t4);
    font-family: var(--mono);
    max-width: 480px;
    word-break: break-word;
    white-space: pre-wrap;
  }
  .fb-state-path { color: var(--t3); }
  .fb-state-err .fb-state-title { color: var(--err); }
  .fb-state-err-detail { color: var(--err); }
  .fb-crumb-root {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--t3);
    padding: 3px 5px;
  }
  .fb-crumb-root:hover { color: var(--acc); background: var(--surface-hover); }

  /* Connect-state overlay — design ported from SshPanel.ssh-loading so
     SSH and Explorer use the same visual language for "we're talking to
     a remote, please wait". */
  .fb-overlay {
    position: absolute;
    inset: 0;
    /* --modal-bg is the only surface token guaranteed opaque on every
       theme, including dark-glass. The previous `var(--n)` resolved to
       a 40% translucent panel on glass, letting the "Empty Directory"
       empty-state bleed through during connect. */
    background: var(--modal-bg);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 16px;
    z-index: 5;
    cursor: default;
    animation: fb-overlay-fade 0.3s ease;
  }
  :global(body.glass-mode) .fb-overlay {
    backdrop-filter: blur(20px) saturate(180%);
    -webkit-backdrop-filter: blur(20px) saturate(180%);
  }
  .fb-overlay-text {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
  .fb-overlay-title {
    font-size: 14px;
    font-weight: 500;
    color: var(--t2);
    font-family: var(--ui);
  }
  .fb-overlay-sub {
    font-size: 11px;
    color: var(--t4);
    font-family: var(--mono);
  }
  .fb-dots::after {
    content: '';
    animation: fb-dots 1.5s steps(4, end) infinite;
  }
  @keyframes fb-dots {
    0%   { content: ''; }
    25%  { content: '.'; }
    50%  { content: '..'; }
    75%  { content: '...'; }
  }
  @keyframes fb-overlay-fade {
    from { opacity: 0; transform: scale(0.97); }
    to   { opacity: 1; transform: scale(1); }
  }
  .fb-overlay-actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }
  .fb-overlay-btn {
    margin-top: 14px;
    padding: 6px 16px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .fb-overlay-btn:hover {
    background: var(--surface-hover);
    border-color: var(--b2);
    color: var(--t1);
  }
  .fb-overlay-btn.primary {
    background: var(--acc);
    border-color: transparent;
    color: #fff;
  }
  .fb-overlay-btn.primary:hover {
    filter: brightness(1.1);
    background: var(--acc);
    color: #fff;
  }
  .fb-overlay-actions .fb-overlay-btn { margin-top: 0; }
  .fb-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
    font-family: var(--mono);
  }
  .fb-table th {
    text-align: left;
    color: var(--t3);
    font-weight: 500;
    font-size: 11px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--b1);
    position: sticky;
    top: 0;
    background: var(--n2);
  }
  .fb-row td {
    padding: 5px 12px;
    border-bottom: 1px solid #111128;
    color: var(--t1);
  }
  .fb-row:hover { background: var(--surface-hover); }
  .fb-col-name { width: auto; }
  .fb-col-size { width: 100px; color: var(--t2); }
  .fb-col-mod { width: 200px; color: var(--t3); }
  .fb-col-perm { width: 110px; color: var(--t3); }
  .fb-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-right: 8px;
    width: 16px;
    height: 16px;
    color: var(--t3); /* default for files */
    vertical-align: -3px;
  }
  .fb-icon :global(svg) { width: 16px; height: 16px; }
  /* Folders + symlinks pick up the user's accent so they pop visually
     against muted file rows — same role as REST collection folder. */
  .fb-icon.dir { color: var(--acc); }
  .fb-icon.link { color: var(--acc); opacity: 0.85; }

  /* Prompt modal — used for "New folder" / "Rename" since Tauri's webview
     blocks the native window.prompt(). Mirrors the connection-modal form
     style so it reads as part of the app, not a generic OS dialog. */
  .fb-prompt { display: flex; flex-direction: column; gap: 14px; }
  .fb-prompt-row { display: flex; flex-direction: column; gap: 4px; font-family: var(--ui); font-size: 12px; color: var(--t2); }
  .fb-prompt-input {
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 5px;
    color: var(--t1);
    height: 32px;
    padding: 0 10px;
    font-size: 13px;
    font-family: var(--mono);
    outline: none;
    transition: border-color 0.12s;
    box-sizing: border-box;
    width: 100%;
  }
  .fb-prompt-input:focus { border-color: var(--acc); }
  .fb-prompt-actions { display: flex; gap: 8px; justify-content: flex-end; }
  .fb-prompt-btn {
    padding: 7px 14px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
  }
  .fb-prompt-btn:hover { border-color: var(--b2); color: var(--t1); }
  .fb-prompt-btn.primary { background: var(--acc); color: #fff; border-color: transparent; }
  .fb-prompt-btn.primary:hover { filter: brightness(1.1); }

  /* Drop-zone overlay. Sits between the file table and the connect overlay
     in z-index so it never appears while disconnected (logically guarded
     in the template too). Pointer-events:none so the OS drag operation
     keeps reaching the Tauri webview underneath. */
  .fb-drop {
    position: absolute;
    inset: 0;
    z-index: 4;
    pointer-events: none;
    background: color-mix(in srgb, var(--acc) 10%, rgba(0,0,0,0.35));
    display: flex;
    align-items: center;
    justify-content: center;
    animation: fb-drop-fade 0.12s ease;
  }
  @keyframes fb-drop-fade {
    from { opacity: 0; }
    to   { opacity: 1; }
  }
  .fb-drop-card {
    background: var(--n);
    border: 2px dashed var(--acc);
    border-radius: 12px;
    padding: 28px 36px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    box-shadow: 0 12px 36px rgba(0,0,0,0.4);
    animation: fb-drop-pulse 1.2s ease-in-out infinite;
  }
  @keyframes fb-drop-pulse {
    0%, 100% { transform: scale(1);    box-shadow: 0 12px 36px rgba(0,0,0,0.4); }
    50%      { transform: scale(1.02); box-shadow: 0 16px 48px rgba(0,0,0,0.55); }
  }
  .fb-drop-title {
    font-size: 15px;
    font-family: var(--ui);
    font-weight: 600;
    color: var(--t1);
    margin-top: 4px;
  }
  .fb-drop-sub {
    font-size: 12px;
    font-family: var(--ui);
    color: var(--t3);
  }
  .fb-drop-path {
    font-family: var(--mono);
    color: var(--acc);
  }
</style>

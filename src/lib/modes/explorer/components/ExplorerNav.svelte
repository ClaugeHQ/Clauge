<script lang="ts">
  import { get } from 'svelte/store';
  import { tabs, addTab, activateTab, closeTab } from '$lib/shared/stores/tabs';
  import {
    explorerConnections,
    activeExplorerConnection,
    explorerConnStates,
    loadExplorerConnections,
  } from '$lib/modes/explorer/stores';
  import { newExplorerTabKey, connectionIdFromTabKey } from '$lib/modes/explorer/tabkey';
  import { openSession, closeSession, deleteConnection, deleteSecrets } from '$lib/modes/explorer/commands';
  import { showToast } from '$lib/shared/primitives/toast';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import NewSftpConnectionModal from './connection-modal/NewSftpConnectionModal.svelte';
  import NewFtpConnectionModal from './connection-modal/NewFtpConnectionModal.svelte';
  import NewS3ConnectionModal from './connection-modal/NewS3ConnectionModal.svelte';
  import NewAzureBlobConnectionModal from './connection-modal/NewAzureBlobConnectionModal.svelte';
  import type { ExplorerKind, ExplorerConnection } from '$lib/modes/explorer/types';

  // NavPanel passes the search query down (mirrors SshNav's contract).
  interface Props { searchQuery?: string }
  let { searchQuery = '' }: Props = $props();

  // Filter chips at the top of the nav.
  let filter = $state<'all' | ExplorerKind>('all');

  /** Best-effort connection details summary for the row sub-line. */
  function subLine(conn: ExplorerConnection): string {
    switch (conn.kind) {
      case 'sftp':
        if (conn.sshProfileId) return 'via SSH profile';
        if (conn.host) {
          const port = conn.port && conn.port !== 22 ? `:${conn.port}` : '';
          return `${conn.username || ''}@${conn.host}${port}`;
        }
        return '—';
      case 'ftp': {
        const port = conn.port && conn.port !== 21 ? `:${conn.port}` : '';
        return `${conn.username || 'anonymous'}@${conn.host || ''}${port}`;
      }
      case 's3': {
        const preset = conn.s3Preset && conn.s3Preset !== 'custom' ? `${conn.s3Preset.toUpperCase()} · ` : '';
        return `${preset}${conn.s3Bucket || '(no bucket)'}`;
      }
      case 'azure_blob':
        return `${conn.azureAccount || '?'} / ${conn.azureContainer || '?'}`;
    }
  }

  /** Apply name + sub-line text search on top of the kind filter. */
  const filtered = $derived.by(() => {
    const byKind = filter === 'all' ? $explorerConnections : $explorerConnections.filter((c) => c.kind === filter);
    if (!searchQuery.trim()) return byKind;
    const q = searchQuery.toLowerCase();
    return byKind.filter((c) =>
      c.name.toLowerCase().includes(q) ||
      subLine(c).toLowerCase().includes(q),
    );
  });

  /** True if this connection has any live tab/session. */
  function isConnected(conn: ExplorerConnection, allTabs = get(tabs), states = get(explorerConnStates)): boolean {
    return allTabs.some((t) =>
      t.mode === 'explorer'
      && t.key
      && connectionIdFromTabKey(t.key) === conn.id
      && states.get(t.key) === 'connected'
    );
  }
  // Reactive set of connected connection IDs so the row template can
  // light up the dot without each row triggering its own subscription.
  const connectedIds = $derived(new Set(
    $tabs
      .filter((t) => t.mode === 'explorer' && t.key && $explorerConnStates.get(t.key) === 'connected')
      .map((t) => connectionIdFromTabKey(t.key as string)),
  ));

  /** Format an RFC3339 timestamp as "just now / 3m / 4h / 2d / DD MMM". */
  function relativeTime(iso: string | null): string {
    if (!iso) return '';
    const t = Date.parse(iso);
    if (!Number.isFinite(t)) return '';
    const diff = Math.max(0, Date.now() - t);
    if (diff < 60_000) return 'just now';
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m`;
    if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h`;
    if (diff < 7 * 86_400_000) return `${Math.floor(diff / 86_400_000)}d`;
    const d = new Date(t);
    return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  }

  function kindIcon(kind: ExplorerKind): string {
    // Inline SVG strings rendered via {@html} — kept as data so the row
    // markup stays compact. Stroke uses currentColor to inherit the row
    // colour (idle = t3, hover/active = t1, connected = explorer accent).
    switch (kind) {
      case 'sftp':
      case 'ftp':
        // Server / rack icon — represents a remote host
        return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="4" width="20" height="6" rx="1"/><rect x="2" y="14" width="20" height="6" rx="1"/><line x1="6" y1="7" x2="6.01" y2="7"/><line x1="6" y1="17" x2="6.01" y2="17"/></svg>';
      case 's3':
      case 'azure_blob':
        // Cloud/bucket icon — represents object storage
        return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M18 10h-1.26A8 8 0 109 20h9a5 5 0 000-10z"/></svg>';
    }
  }

  let showDeleteConfirm = $state(false);
  let pendingDelete = $state<ExplorerConnection | null>(null);
  let showDisconnectConfirm = $state(false);
  let pendingDisconnect = $state<ExplorerConnection | null>(null);

  async function confirmDisconnect() {
    if (!pendingDisconnect) return;
    await disconnectAllForConnection(pendingDisconnect);
    pendingDisconnect = null;
  }

  // The "+ New connection" button lives in NavPanel's header (consistent
  // with every other mode); it dispatches the `explorer:add-connection`
  // event below so this component can pop the kind-picker.
  let showKindPicker = $state(false);
  let showSftpModal = $state(false);
  let showFtpModal = $state(false);
  let showS3Modal = $state(false);
  let showAzureModal = $state(false);

  /** When set, the matching kind's modal opens in edit mode. */
  let editingTarget = $state<ExplorerConnection | null>(null);

  // Teleport the kind-picker overlay to <body>. NavPanel.overlay's slide-in
  // animation sets `transform`, which creates a containing block that
  // clips `position: fixed` descendants. Same root cause the Modal
  // primitive now handles internally; the kind-picker is hand-rolled
  // (not a Modal) so it needs the action explicitly.
  function teleportToBody(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentElement === document.body) node.remove();
      },
    };
  }

  function handleAdd() {
    showKindPicker = true;
  }
  $effect(() => {
    const handler = () => handleAdd();
    window.addEventListener('explorer:add-connection', handler);
    // Topbar "+" picker (in +layout.svelte) dispatches OPEN_TAB with a
    // connection in the detail payload — route it through the same
    // handleConnect path so all "open" entry points behave identically.
    const openHandler = (e: Event) => {
      const conn = (e as CustomEvent).detail as ExplorerConnection | undefined;
      if (conn) handleConnect(conn);
    };
    window.addEventListener('explorer:open-tab', openHandler);
    return () => {
      window.removeEventListener('explorer:add-connection', handler);
      window.removeEventListener('explorer:open-tab', openHandler);
    };
  });
  function pickKind(k: ExplorerKind) {
    showKindPicker = false;
    if (k === 'sftp') showSftpModal = true;
    else if (k === 'ftp') showFtpModal = true;
    else if (k === 's3') showS3Modal = true;
    else if (k === 'azure_blob') showAzureModal = true;
  }

  /**
   * Open a connection. Default behaviour: activate an existing tab if one
   * exists for this connection; otherwise create a new tab. `force=true`
   * always creates a new tab (used by Duplicate Session).
   */
  async function handleConnect(conn: ExplorerConnection, force = false) {
    if (!force) {
      const allTabs = get(tabs);
      const existing = allTabs.find((t) => t.mode === 'explorer' && t.key && t.key.startsWith(`${conn.id}#`));
      if (existing) {
        activateTab(existing.id);
        return;
      }
    }
    const tabKey = newExplorerTabKey(conn.id);
    explorerConnStates.update((m) => {
      const next = new Map(m);
      next.set(tabKey, 'connecting');
      return next;
    });
    addTab(conn.name || conn.kind.toUpperCase(), 'explorer', tabKey, conn.accentColor || 'var(--explorer)');
    try {
      await openSession(conn.id, tabKey);
      explorerConnStates.update((m) => {
        const next = new Map(m);
        next.set(tabKey, 'connected');
        return next;
      });
    } catch (err: any) {
      explorerConnStates.update((m) => {
        const next = new Map(m);
        next.set(tabKey, 'error');
        return next;
      });
      showToast(`Connection failed: ${err}`, 'error');
    }
  }

  /** Close every tab + session for a given connection. Used by the
   *  "Disconnect" menu entry; matches `disconnectAllForProfile` in SshNav. */
  async function disconnectAllForConnection(conn: ExplorerConnection) {
    const allTabs = get(tabs);
    const matching = allTabs.filter((t) => t.mode === 'explorer' && t.key && connectionIdFromTabKey(t.key) === conn.id);
    for (const tab of matching) {
      if (tab.key) {
        try { await closeSession(tab.key); } catch { /* fire-and-forget */ }
        explorerConnStates.update((m) => {
          const next = new Map(m);
          next.delete(tab.key as string);
          return next;
        });
      }
      closeTab(tab.id);
    }
    if (get(activeExplorerConnection)?.id === conn.id) {
      activeExplorerConnection.set(null);
    }
  }

  function openEditModal(conn: ExplorerConnection) {
    editingTarget = conn;
    if (conn.kind === 'sftp') showSftpModal = true;
    else if (conn.kind === 'ftp') showFtpModal = true;
    else if (conn.kind === 's3') showS3Modal = true;
    else if (conn.kind === 'azure_blob') showAzureModal = true;
  }

  function handleContextMenu(e: MouseEvent, conn: ExplorerConnection) {
    e.preventDefault();
    e.stopPropagation();
    const connected = isConnected(conn);
    // Icons mirror SshNav so the two modes' context menus read identically.
    const items: any[] = [];
    if (connected) {
      items.push({
        label: 'Disconnect',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18.36 6.64a9 9 0 11-12.73 0"/><line x1="12" y1="2" x2="12" y2="12"/></svg>',
        action: () => {
          pendingDisconnect = conn;
          showDisconnectConfirm = true;
        },
      });
      items.push({
        label: 'Duplicate Session',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>',
        action: () => handleConnect(conn, true),
      });
    } else {
      items.push({
        label: 'Connect',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/><path d="M12 5l7 7-7 7"/></svg>',
        action: () => handleConnect(conn),
      });
    }
    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: 'Edit',
      icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
      action: () => openEditModal(conn),
    });
    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: 'Delete',
      icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>',
      danger: true,
      action: () => { pendingDelete = conn; showDeleteConfirm = true; },
    });
    showContextMenu(e.clientX, e.clientY, items);
  }

  async function confirmDelete() {
    if (!pendingDelete) return;
    // Close any live tabs for this connection before deletion.
    await disconnectAllForConnection(pendingDelete);
    try {
      await deleteSecrets(pendingDelete.id);
      await deleteConnection(pendingDelete.id);
      await loadExplorerConnections();
      showToast(`Deleted ${pendingDelete.name}`, 'success');
    } catch (err: any) {
      showToast(`Delete failed: ${err}`, 'error');
    }
    pendingDelete = null;
  }

  // Brand-accurate identity colors per remote-storage kind. Hardcoded by
  // design — the badge identifies *what* the thing is, not the theme.
  function kindColor(k: ExplorerKind): string {
    switch (k) {
      case 'sftp':       return '#06b6d4'; // terminal cyan
      case 'ftp':        return '#94a3b8'; // muted slate (legacy protocol)
      case 's3':         return '#FF9900'; // AWS orange
      case 'azure_blob': return '#0078D4'; // Azure blue
      default:           return 'var(--t3)';
    }
  }

  function kindBadge(k: ExplorerKind): string {
    switch (k) {
      case 'sftp': return 'SFTP';
      case 'ftp': return 'FTP';
      case 's3': return 'S3';
      case 'azure_blob': return 'AZURE';
    }
  }
</script>

<div class="ex-nav">
  <div class="ex-nav-filters">
    {#each ['all', 'sftp', 'ftp', 's3', 'azure_blob'] as f (f)}
      <button class="ex-nav-chip" class:on={filter === f} onclick={() => (filter = f as typeof filter)}>
        {f === 'all' ? 'All' : kindBadge(f as ExplorerKind)}
      </button>
    {/each}
  </div>

  <div class="ex-nav-list">
    {#if filtered.length === 0}
      {#if searchQuery}
        <div class="ex-nav-empty">No results for "{searchQuery}"</div>
      {:else}
        <div class="ex-nav-empty">
          <span>No connections yet</span>
          <button class="ex-nav-empty-btn" onclick={() => (showKindPicker = true)}>+ New connection</button>
        </div>
      {/if}
    {:else}
      {#each filtered as conn (conn.id)}
        {@const connected = connectedIds.has(conn.id)}
        <button
          class="ex-row"
          class:active={$activeExplorerConnection?.id === conn.id}
          class:connected
          onclick={() => handleConnect(conn)}
          oncontextmenu={(e) => handleContextMenu(e, conn)}
        >
          <span class="ex-row-icon">
            {@html kindIcon(conn.kind)}
          </span>
          <div class="ex-row-body">
            <div class="ex-row-top">
              <span class="ex-row-name">{conn.name}</span>
              <span
                class="ex-row-badge"
                style:color={kindColor(conn.kind)}
                style:background="color-mix(in srgb, {kindColor(conn.kind)} 12%, transparent)"
              >{kindBadge(conn.kind)}</span>
            </div>
            <div class="ex-row-bot">
              <span class="ex-row-sub">{subLine(conn)}</span>
              <span class="ex-row-spacer"></span>
              <span class="ex-row-time">{relativeTime(conn.lastUsedAt)}</span>
            </div>
          </div>
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span
            class="ex-row-ellipsis"
            role="button"
            tabindex="-1"
            title="More"
            onclick={(e) => handleContextMenu(e as unknown as MouseEvent, conn)}
          >
            <svg viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="5" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="12" cy="19" r="1.5"/></svg>
          </span>
        </button>
      {/each}
    {/if}
  </div>
</div>

<ConfirmDialog
  bind:show={showDeleteConfirm}
  title="Delete connection"
  message={pendingDelete ? `Delete "${pendingDelete.name}"? Stored credentials will also be removed.` : ''}
  confirmText="Delete"
  onconfirm={confirmDelete}
/>

<ConfirmDialog
  bind:show={showDisconnectConfirm}
  title="Disconnect"
  message={pendingDisconnect ? `Disconnect from "${pendingDisconnect.name}"? Any open file browser tabs for this connection will be closed.` : ''}
  confirmText="Disconnect"
  confirmColor="var(--acc)"
  onconfirm={confirmDisconnect}
/>

<NewSftpConnectionModal
  bind:show={showSftpModal}
  editing={editingTarget?.kind === 'sftp' ? editingTarget : null}
  onclose={() => (editingTarget = null)}
/>
<NewFtpConnectionModal
  bind:show={showFtpModal}
  editing={editingTarget?.kind === 'ftp' ? editingTarget : null}
  onclose={() => (editingTarget = null)}
/>
<NewS3ConnectionModal
  bind:show={showS3Modal}
  editing={editingTarget?.kind === 's3' ? editingTarget : null}
  onclose={() => (editingTarget = null)}
/>
<NewAzureBlobConnectionModal
  bind:show={showAzureModal}
  editing={editingTarget?.kind === 'azure_blob' ? editingTarget : null}
  onclose={() => (editingTarget = null)}
/>

{#if showKindPicker}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- Match the global Modal primitive: teleport to body, no click-outside
       close, X button + Escape only. -->
  <div class="ex-kind-overlay" use:teleportToBody>
    <div class="ex-kind-card">
      <div class="ex-kind-hdr">
        <span class="ex-kind-title">New connection</span>
        <button class="ex-kind-close" onclick={() => (showKindPicker = false)} title="Close">&times;</button>
      </div>
      <div class="ex-kind-body">
        <p class="ex-kind-sub">What kind of remote storage?</p>
        <div class="ex-kind-grid">
          <button onclick={() => pickKind('sftp')}><strong>SFTP</strong><span>Secure shell file transfer</span></button>
          <button onclick={() => pickKind('s3')}><strong>S3-compatible</strong><span>AWS, R2, MinIO, Wasabi, B2, GCS, Custom</span></button>
          <button onclick={() => pickKind('azure_blob')}><strong>Azure Blob</strong><span>Microsoft Azure Storage</span></button>
          <button onclick={() => pickKind('ftp')}><strong>FTP</strong><span>Plain FTP (legacy)</span></button>
        </div>
      </div>
    </div>
  </div>
{/if}

<svelte:window onkeydown={(e) => { if (e.key === 'Escape' && showKindPicker) { showKindPicker = false; } }} />

<style>
  .ex-nav {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 0;
  }
  .ex-nav-filters {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--b1);
  }
  .ex-nav-chip {
    padding: 3px 8px;
    border-radius: 11px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t3);
    font-size: 10.5px;
    font-family: var(--mono);
    cursor: default;
  }
  .ex-nav-chip.on { color: #fff; background: var(--acc); border-color: var(--acc); }
  .ex-nav-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 4px;
  }
  .ex-nav-list::-webkit-scrollbar { width: 4px; }
  .ex-nav-list::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }
  .ex-nav-empty {
    padding: 24px 12px;
    color: var(--t3);
    font-size: 12px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  .ex-nav-empty-btn {
    background: var(--acc);
    color: #fff;
    border: none;
    padding: 6px 12px;
    border-radius: 5px;
    font-size: 11.5px;
    font-family: var(--ui);
    cursor: default;
  }
  .ex-nav-empty-btn:hover { opacity: 0.88; }

  /* Row — mirrors SSH `.profile-item` layout: icon + body + ellipsis */
  .ex-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: none;
    background: transparent;
    border-radius: 6px;
    cursor: default;
    transition: background 0.08s;
    width: 100%;
    text-align: left;
    color: var(--t3);
    position: relative;
  }
  .ex-row:hover {
    background: rgba(255, 255, 255, 0.04);
    color: var(--t1);
  }
  .ex-row.active {
    background: color-mix(in srgb, var(--acc) 14%, transparent);
    color: var(--t1);
  }
  .ex-row.connected { color: var(--t1); }
  .ex-row-icon {
    position: relative;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    /* Idle icon tint — matches SSH `.profile-icon`. SVG uses
       currentColor, so this propagates. .connected state overrides below. */
    color: var(--acc);
  }
  .ex-row-icon :global(svg) { width: 18px; height: 18px; }
  /* Connected state — mirrors SSH `.profile-item.connected`:
     border-only ring around the icon (no fill, gentle), small pulsing
     status dot, icon stroke turns green. Same dialled-down intensity
     as SSH (3s cadence, smaller dot, smaller pulse spread). */
  .ex-row.connected .ex-row-icon { color: var(--ok, #1dc880); }
  .ex-row.connected .ex-row-icon :global(svg) {
    stroke: var(--ok, #1dc880);
    position: relative;
    z-index: 1;
  }
  .ex-row.connected .ex-row-icon::before {
    content: '';
    position: absolute;
    inset: -2px;
    border-radius: 8px;
    border: 1px solid color-mix(in srgb, var(--ok, #1dc880) 28%, transparent);
    z-index: 0;
  }
  .ex-row.connected .ex-row-icon::after {
    content: '';
    position: absolute;
    top: 1px;
    right: 1px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok, #1dc880);
    box-shadow: 0 0 0 1.5px var(--n);
    z-index: 2;
    animation: exRowConnPulse 3s ease-in-out infinite;
  }
  @keyframes exRowConnPulse {
    0%, 100% { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 2px color-mix(in srgb, var(--ok, #1dc880) 30%, transparent); }
    50%      { box-shadow: 0 0 0 1.5px var(--n), 0 0 0 5px color-mix(in srgb, var(--ok, #1dc880) 0%, transparent); }
  }
  .ex-row-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ex-row-top {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .ex-row-name {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--t1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .ex-row-badge {
    /* color + background set inline via kindColor(kind) — see template. */
    flex-shrink: 0;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 3px;
    font-family: var(--mono);
  }
  .ex-row-bot {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .ex-row-sub {
    font-size: 11px;
    color: var(--t3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--mono);
    flex-shrink: 1;
    min-width: 0;
  }
  .ex-row-spacer { flex: 1; }
  .ex-row-time {
    flex-shrink: 0;
    font-size: 10.5px;
    color: var(--t4);
    font-family: var(--mono);
  }
  .ex-row-ellipsis {
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    color: var(--t3);
    opacity: 0;
    transition: opacity 0.1s, background 0.1s, color 0.1s;
    cursor: default;
  }
  .ex-row:hover .ex-row-ellipsis,
  .ex-row.active .ex-row-ellipsis { opacity: 1; }
  .ex-row-ellipsis:hover {
    background: var(--b1);
    color: var(--t1);
  }
  .ex-row-ellipsis :global(svg) { width: 14px; height: 14px; }

  /* Kind picker — visually matches the global Modal primitive
     (header bar with title + X, no click-outside close). */
  .ex-kind-overlay {
    position: fixed; inset: 0;
    background: rgba(0,0,0,0.4);
    backdrop-filter: blur(8px);
    z-index: 1000;
    display: flex; align-items: center; justify-content: center;
    animation: ex-kind-fade 0.15s ease;
  }
  @keyframes ex-kind-fade { from { opacity: 0; } to { opacity: 1; } }
  .ex-kind-card {
    width: 460px;
    background: var(--modal-bg, #101016);
    border: 1px solid var(--b1);
    border-radius: var(--radius-xl, 12px);
    box-shadow: 0 24px 48px rgba(0,0,0,0.5);
    color: var(--t1);
    overflow: hidden;
    animation: ex-kind-up 0.18s ease;
  }
  @keyframes ex-kind-up {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to { opacity: 1; transform: none; }
  }
  .ex-kind-hdr {
    display: flex;
    align-items: center;
    padding: 14px 18px;
    border-bottom: 1px solid var(--b1);
    background: var(--e);
  }
  .ex-kind-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }
  .ex-kind-close {
    margin-left: auto;
    width: 28px; height: 28px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    display: flex; align-items: center; justify-content: center;
    cursor: default;
    color: var(--t3);
    font-size: 16px;
    transition: background 0.12s, color 0.12s;
  }
  .ex-kind-close:hover { background: var(--c); color: var(--t1); }
  .ex-kind-body { padding: 20px 24px; }
  .ex-kind-sub { margin: 0 0 14px; font-size: 12.5px; color: var(--t3); }
  .ex-kind-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .ex-kind-grid button {
    text-align: left;
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    color: var(--t1);
    padding: 12px 14px;
    border-radius: 8px;
    cursor: default;
    display: flex; flex-direction: column; gap: 3px;
    transition: border-color 0.12s, background 0.12s;
  }
  .ex-kind-grid button:hover { border-color: var(--acc); background: rgba(255,255,255,0.06); }
  .ex-kind-grid strong { font-size: 13px; font-weight: 600; }
  .ex-kind-grid span { font-size: 11.5px; color: var(--t3); }
</style>

<script lang="ts">
  import { mode, navOpen } from '$lib/stores/app';
  import RestNav from './RestNav.svelte';
  import SqlNav from '$lib/modes/sql/components/SqlNav.svelte';
  import NoSqlNav from '$lib/modes/nosql/components/NoSqlNav.svelte';
  import AgentNav from '$lib/modes/agent/components/AgentNav.svelte';
  import SshNav from '$lib/modes/ssh/components/SshNav.svelte';
  import ExplorerNav from '$lib/modes/explorer/components/ExplorerNav.svelte';
  import HistoryPanel from './HistoryPanel.svelte';
  import ImportExportModal from '$lib/shared/primitives/ImportExportModal.svelte';
  import { getNavPinned, setNavPinned } from '$lib/shared/constants/storage';
  import { AGENT_EVENT } from '$lib/shared/constants/events';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';

  let searchPerMode = $state<Record<string, string>>({ rest: '', sql: '', nosql: '', agent: '', ssh: '' });
  let searchQuery = $derived(searchPerMode[$mode] ?? '');
  let restNavRef: ReturnType<typeof RestNav> | undefined = $state();
  let sqlNavRef: ReturnType<typeof SqlNav> | undefined = $state();
  let nosqlNavRef: ReturnType<typeof NoSqlNav> | undefined = $state();
  let agentNavRef: ReturnType<typeof AgentNav> | undefined = $state();
  let sshNavRef: ReturnType<typeof SshNav> | undefined = $state();
  let showImportExport = $state(false);

  // Pin/unpin: pinned = always visible in layout, unpinned = overlay panel.
  let navPinned = $state(getNavPinned());

  function togglePin() {
    navPinned = !navPinned;
    setNavPinned(navPinned);
    navOpen.set(navPinned);
  }

  let navPanelEl: HTMLElement;

  function handleMouseLeavePanel(e: MouseEvent) {
    if (navPinned) return;
    if (!navPanelEl) return;
    const rect = navPanelEl.getBoundingClientRect();
    if (e.clientX >= rect.right - 2) {
      navOpen.set(false);
    }
  }

  function handleOverlayDismiss() {
    if (!navPinned) navOpen.set(false);
  }

  import { onMount, onDestroy } from 'svelte';
  onMount(() => {
    window.addEventListener(AGENT_EVENT.EDIT_SESSION, handleOverlayDismiss);
    window.addEventListener(AGENT_EVENT.RESET_SESSION, handleOverlayDismiss);
    window.addEventListener(AGENT_EVENT.RELAUNCH_SESSION, handleOverlayDismiss);
    window.addEventListener(AGENT_EVENT.NEW_SESSION, handleOverlayDismiss);
  });
  onDestroy(() => {
    window.removeEventListener(AGENT_EVENT.EDIT_SESSION, handleOverlayDismiss);
    window.removeEventListener(AGENT_EVENT.RESET_SESSION, handleOverlayDismiss);
    window.removeEventListener(AGENT_EVENT.RELAUNCH_SESSION, handleOverlayDismiss);
    window.removeEventListener(AGENT_EVENT.NEW_SESSION, handleOverlayDismiss);
  });

  function setSearch(val: string) {
    searchPerMode[$mode] = val;
  }

  const searchPlaceholders = {
    rest: 'Search collections…',
    sql: 'Search connections…',
    nosql: 'Search connections…',
    agent: 'Search sessions…',
    ssh: 'Search SSH profiles…',
    explorer: 'Search connections…',
    history: 'Search history…',
  } as const;

  function handleAddClick() {
    if ($mode === 'rest') {
      restNavRef?.showAddCollection();
    } else if ($mode === 'sql') {
      sqlNavRef?.showAddConnection();
    } else if ($mode === 'nosql') {
      nosqlNavRef?.showAddConnection();
    } else if ($mode === 'agent') {
      window.dispatchEvent(new CustomEvent(AGENT_EVENT.NEW_SESSION));
    } else if ($mode === 'ssh') {
      sshNavRef?.showAddProfile();
    } else if ($mode === 'explorer') {
      window.dispatchEvent(new CustomEvent('explorer:add-connection'));
    }
  }

  /** Per-mode add button tooltip — drives the title attr only. */
  const addLabels = {
    rest: 'New collection',
    sql: 'New connection',
    nosql: 'New connection',
    agent: 'New session',
    ssh: 'New SSH profile',
    explorer: 'New connection',
  } as const;

  /** Open the overflow menu at a button's position. Per-mode items are
   *  appended above the always-present Pin/Unpin entry so each mode's
   *  secondary action (REST: import/export, SSH: import ssh_config)
   *  stays one click away without polluting the bar with extra icons. */
  function openOverflow(ev: MouseEvent) {
    ev.stopPropagation();
    const rect = (ev.currentTarget as HTMLElement).getBoundingClientRect();
    const items: any[] = [];
    if ($mode === 'rest') {
      items.push({
        label: 'Import / Export',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>',
        action: () => (showImportExport = true),
      });
      items.push({ label: '', action: () => {}, separator: true });
    }
    items.push({
      label: navPinned ? 'Unpin sidebar' : 'Pin sidebar',
      icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3h14a2 2 0 012 2v14a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2z"/><path d="M9 3v18"/></svg>',
      action: togglePin,
    });
    showContextMenu(rect.right - 4, rect.bottom + 4, items);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<nav
  bind:this={navPanelEl}
  class="nav-panel glass-surface-light"
  class:shut={!$navOpen}
  class:overlay={!navPinned && $navOpen}
  onmouseleave={handleMouseLeavePanel}
>
  <!-- Single combined header — search bar with inline (+) and (⋯) buttons.
       Replaces the old two-row "title bar + search row" layout. The data-drag-region
       attr lets users drag the window from blank header chrome; the global
       mousedown handler in +layout.svelte excludes inputs/buttons so clicks
       still register on the controls. -->
  <div class="nav-header" data-drag-region>
    <div class="nav-search-wrap">
      <svg class="nav-search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="11" cy="11" r="8"/>
        <line x1="21" y1="21" x2="16.65" y2="16.65"/>
      </svg>
      <input
        type="text"
        class="nav-search-input"
        placeholder={searchPlaceholders[$mode] ?? 'Search…'}
        value={searchQuery}
        oninput={(e) => setSearch((e.target as HTMLInputElement).value)}
      />
    </div>
    {#if $mode !== 'history'}
      <button class="nav-action nav-add" title={addLabels[$mode]} onclick={handleAddClick}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 5v14M5 12h14"/>
        </svg>
      </button>
    {/if}
    <button class="nav-action nav-overflow" title="More" onclick={openOverflow}>
      <svg viewBox="0 0 24 24" fill="currentColor">
        <circle cx="5" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="19" cy="12" r="1.6"/>
      </svg>
    </button>
  </div>

  <div class="nav-body">
    {#if $mode === 'history'}
      <HistoryPanel />
    {:else if $mode === 'rest'}
      <RestNav bind:this={restNavRef} {searchQuery} />
    {:else if $mode === 'sql'}
      <SqlNav bind:this={sqlNavRef} {searchQuery} />
    {:else if $mode === 'agent'}
      <AgentNav bind:this={agentNavRef} {searchQuery} />
    {:else if $mode === 'ssh'}
      <SshNav bind:this={sshNavRef} {searchQuery} />
    {:else if $mode === 'explorer'}
      <ExplorerNav {searchQuery} />
    {:else}
      <NoSqlNav bind:this={nosqlNavRef} {searchQuery} />
    {/if}
  </div>
</nav>

<ImportExportModal bind:show={showImportExport} />

<style>
  .nav-panel {
    width: 300px;
    min-width: 300px;
    background: var(--n);
    border-right: 1px solid var(--b1);
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    transition: width 0.2s cubic-bezier(0.4, 0, 0.2, 1),
                min-width 0.2s cubic-bezier(0.4, 0, 0.2, 1),
                opacity 0.15s ease,
                transform 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  }
  .nav-panel.shut {
    width: 0;
    min-width: 0;
    border-right-width: 0;
    overflow: hidden;
  }

  /* Overlay mode: floats on top of content, doesn't take layout space */
  .nav-panel.overlay {
    position: absolute;
    top: 0;
    left: 72px; /* after sidebar */
    bottom: 0;
    z-index: 100;
    box-shadow: 8px 0 24px rgba(0, 0, 0, 0.3);
    animation: navSlideIn 0.15s ease;
  }
  @keyframes navSlideIn {
    from { opacity: 0; transform: translateX(-8px); }
    to   { opacity: 1; transform: translateX(0); }
  }

  /* Single combined header. Search input is the centerpiece, with the
     primary (+) action and a (⋯) overflow on the right. No mode title —
     that's already shown by the highlighted sidebar mode button + the
     topbar tab marker, so a third repeat would just be noise. */
  .nav-header {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 48px;
    flex-shrink: 0;
    padding: 8px 10px;
    border-bottom: 1px solid var(--b1);
    background: var(--n2);
  }
  .nav-search-wrap {
    flex: 1;
    min-width: 0;
    position: relative;
    display: flex;
    align-items: center;
  }
  .nav-search-icon {
    position: absolute;
    left: 9px;
    width: 14px;
    height: 14px;
    color: var(--t4);
    pointer-events: none;
  }
  .nav-search-input {
    width: 100%;
    height: 32px;
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md, 6px);
    padding: 0 10px 0 30px;
    font-size: 12.5px;
    color: var(--t1);
    font-family: var(--ui);
    outline: none;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .nav-search-input::placeholder { color: var(--t3); }
  .nav-search-input:focus {
    border-color: var(--acc);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 12%, transparent);
  }

  /* Action buttons — same 32px height as search to read as one bar.
     (+) is the primary action so it gets accent treatment. */
  .nav-action {
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border: 1px solid var(--b1);
    background: transparent;
    border-radius: var(--radius-md, 6px);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: default;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
    color: var(--t3);
  }
  .nav-action svg { width: 14px; height: 14px; }
  .nav-action:hover { color: var(--t1); border-color: var(--b2); background: rgba(255,255,255,0.04); }
  .nav-add {
    color: var(--acc);
    border-color: color-mix(in srgb, var(--acc) 35%, var(--b1));
    background: color-mix(in srgb, var(--acc) 10%, transparent);
  }
  .nav-add:hover {
    color: #fff;
    background: var(--acc);
    border-color: var(--acc);
  }
  .nav-overflow svg { width: 16px; height: 16px; }

  .nav-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .nav-body::-webkit-scrollbar { width: 3px; }
  .nav-body::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }
</style>

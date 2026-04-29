<script lang="ts">
  import { mode, navOpen } from '$lib/stores/app';
  import RestNav from './RestNav.svelte';
  import SqlNav from './SqlNav.svelte';
  import NoSqlNav from '$lib/modes/nosql/components/NoSqlNav.svelte';
  import AgentNav from '$lib/components/agent/AgentNav.svelte';
  import SshNav from '$lib/modes/ssh/components/SshNav.svelte';
  import HistoryPanel from './HistoryPanel.svelte';
  import ImportExportModal from '$lib/components/shared/ImportExportModal.svelte';
  import { getNavPinned, setNavPinned } from '$lib/shared/constants/storage';
  import { AGENT_EVENT } from '$lib/shared/constants/events';

  let searchPerMode = $state<Record<string, string>>({ rest: '', sql: '', nosql: '', agent: '', ssh: '' });
  let searchQuery = $derived(searchPerMode[$mode] ?? '');
  let restNavRef: ReturnType<typeof RestNav> | undefined = $state();
  let sqlNavRef: ReturnType<typeof SqlNav> | undefined = $state();
  let nosqlNavRef: ReturnType<typeof NoSqlNav> | undefined = $state();
  let agentNavRef: ReturnType<typeof AgentNav> | undefined = $state();
  let sshNavRef: ReturnType<typeof SshNav> | undefined = $state();
  let showImportExport = $state(false);

  // Pin/unpin: pinned = always visible in layout, unpinned = overlay on hover (Arc browser style)
  let navPinned = $state(getNavPinned());
  let hoverVisible = $state(false);
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;

  function togglePin() {
    navPinned = !navPinned;
    setNavPinned(navPinned);
    if (navPinned) {
      navOpen.set(true);
      hoverVisible = false;
    } else {
      navOpen.set(false);
    }
  }

  let navPanelEl: HTMLElement;

  function handleMouseEnterZone() {
    if (navPinned) return;
    hoverVisible = true;
  }

  function handleMouseLeavePanel(e: MouseEvent) {
    if (navPinned) return;
    if (!navPanelEl) return;
    const rect = navPanelEl.getBoundingClientRect();
    // Only hide when mouse exits from the RIGHT edge (into content area)
    if (e.clientX >= rect.right - 2) {
      hoverVisible = false;
    }
    // Exiting left (toward sidebar) — keep visible
  }

  // Close overlay when any session action dispatches (edit/reset/relaunch opens modal or respawns terminal)
  function handleOverlayDismiss() {
    if (!navPinned) hoverVisible = false;
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

  const modeLabels = {
    rest: 'REST Collections',
    sql: 'SQL Connections',
    nosql: 'NoSQL Connections',
    agent: 'Agent Sessions',
    ssh: 'SSH Connections',
  } as const;

  const modeColors = {
    rest: 'var(--rest)',
    sql: 'var(--sql)',
    nosql: 'var(--nosql)',
    agent: 'var(--agent, var(--acc))',
    ssh: 'var(--ssh)',
  } as const;

  const searchPlaceholders = {
    rest: 'Search requests...',
    sql: 'Search tables...',
    nosql: 'Search collections...',
    agent: 'Search sessions...',
    ssh: 'Search SSH profiles...',
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
    }
  }
</script>

<!-- Hover trigger zone: thin strip on left edge when unpinned -->
{#if !navPinned && !hoverVisible}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="nav-hover-zone" onmouseenter={handleMouseEnterZone}></div>
{/if}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<nav
  bind:this={navPanelEl}
  class="nav-panel glass-surface-light"
  class:shut={navPinned ? !$navOpen : !hoverVisible}
  class:overlay={!navPinned && hoverVisible}
  onmouseleave={handleMouseLeavePanel}
>
  {#if $mode === 'history'}
    <div class="nav-top" data-drag-region>
      <span class="nav-mode-label" style="color:var(--t2)">HISTORY</span>
      <button class="nav-btn" class:pin-active={navPinned} title={navPinned ? 'Unpin sidebar' : 'Pin sidebar'} onclick={togglePin}>
        <svg viewBox="0 0 24 24"><path d="M5 3h14a2 2 0 012 2v14a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2z"/><path d="M9 3v18"/><path d="M13 8h4M13 12h4"/></svg>
      </button>
    </div>
    <div class="nav-body">
      <HistoryPanel />
    </div>
  {:else}
    <div class="nav-top" data-drag-region>
      <span class="nav-mode-label" style="color:{modeColors[$mode]}">{modeLabels[$mode]}</span>
      <div class="nav-btns">
        {#if $mode === 'rest'}
          <button class="nav-btn" title="Import / Export" onclick={() => showImportExport = true}>
            <svg viewBox="0 0 24 24"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
          </button>
        {/if}
        <button class="nav-btn" title={$mode === 'rest' ? 'New collection' : 'New connection'} onclick={handleAddClick}>
          {#if $mode === 'rest'}
            <svg viewBox="0 0 24 24"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/><path d="M12 11v6M9 14h6"/></svg>
          {:else}
            <svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>
          {/if}
        </button>
        <button class="nav-btn" class:pin-active={navPinned} title={navPinned ? 'Unpin sidebar' : 'Pin sidebar'} onclick={togglePin}>
          <svg viewBox="0 0 24 24"><path d="M5 3h14a2 2 0 012 2v14a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2z"/><path d="M9 3v18"/><path d="M13 8h4M13 12h4"/></svg>
        </button>
      </div>
    </div>
    <div class="nav-search">
      <input type="text" placeholder={searchPlaceholders[$mode]} value={searchQuery} oninput={(e) => setSearch((e.target as HTMLInputElement).value)} />
    </div>
    <div class="nav-body">
      {#if $mode === 'rest'}
        <RestNav bind:this={restNavRef} {searchQuery} />
      {:else if $mode === 'sql'}
        <SqlNav bind:this={sqlNavRef} {searchQuery} />
      {:else if $mode === 'agent'}
        <AgentNav bind:this={agentNavRef} {searchQuery} />
      {:else if $mode === 'ssh'}
        <SshNav bind:this={sshNavRef} {searchQuery} />
      {:else}
        <NoSqlNav bind:this={nosqlNavRef} {searchQuery} />
      {/if}
    </div>
  {/if}
</nav>

<ImportExportModal bind:show={showImportExport} />

<style>
  /* Hover trigger zone — invisible strip at sidebar right edge */
  .nav-hover-zone {
    position: fixed;
    top: 0;
    left: 72px;
    width: 8px;
    height: 100%;
    z-index: 90;
  }

  .nav-panel {
    width: 240px;
    min-width: 240px;
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
    to { opacity: 1; transform: translateX(0); }
  }

  .nav-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .nav-top {
    height: 46px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    padding: 0 12px;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .nav-mode-label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--t3);
    white-space: nowrap;
  }
  .nav-btns {
    display: flex;
    gap: 4px;
  }
  .nav-btn {
    width: 24px;
    height: 24px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: default;
    transition: background 0.12s, border-color 0.12s;
  }
  .nav-btn:hover {
    background: var(--c);
    border-color: var(--b2);
  }
  .nav-btn.pin-active {
    border-color: var(--acc);
    background: color-mix(in srgb, var(--acc) 10%, transparent);
  }
  .nav-btn.pin-active svg {
    stroke: var(--acc);
  }
  .nav-btn svg {
    width: 12px;
    height: 12px;
    stroke: var(--t3);
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
  }
  .nav-search {
    padding: 8px 10px;
    border-bottom: 1px solid var(--b1);
    flex-shrink: 0;
  }
  .nav-search input {
    width: 100%;
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md);
    padding: 7px 11px;
    font-size: 12.5px;
    color: var(--t1);
    font-family: var(--ui);
    outline: none;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .nav-search input::placeholder {
    color: var(--t3);
  }
  .nav-search input:focus {
    border-color: var(--acc);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 12%, transparent);
  }
  .nav-body {
    flex: 1;
    overflow-y: auto;
  }
  .nav-body::-webkit-scrollbar {
    width: 3px;
  }
  .nav-body::-webkit-scrollbar-thumb {
    background: var(--b1);
    border-radius: 2px;
  }
  .nav-placeholder {
    padding: 24px 12px;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    text-align: center;
  }
</style>

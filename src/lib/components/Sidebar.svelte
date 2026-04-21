<script>
  import { invoke } from "@tauri-apps/api/core";
  import { terminalStore } from "$lib/stores/terminal.svelte";
  import { contextsStore } from "$lib/stores/contexts.svelte";
  import EditSessionModal from "./EditSessionModal.svelte";

  let {
    profiles = $bindable(),
    activeProfile,
    claudePlan = '',
    onSelect,
    onNewSession,
    onDeleteProfile,
    onRefitTerminals,
  } = $props();

  let editingProfile = $state(null);

  let sidebarCollapsed = $state(
    typeof localStorage !== 'undefined' ? localStorage.getItem('clauge-sidebar-collapsed') === 'true' : false
  );
  let expandedGroups = $state(
    typeof localStorage !== 'undefined'
      ? JSON.parse(localStorage.getItem('clauge-expanded') || '{}')
      : {}
  );
  let menuProfile = $state(null);

  let grouped = $derived(groupByProject(profiles));

  const purposes = [
    { label: "Brainstorming", color: "#d2a8ff" },
    { label: "Development", color: "#3fb950" },
    { label: "Code Review", color: "#58a6ff" },
    { label: "PR Review", color: "#d29922" },
    { label: "Debugging", color: "#f85149" },
    { label: "Custom", color: "#8b949e" },
  ];
  const purposeColors = Object.fromEntries(purposes.map(p => [p.label, p.color]));

  function toggleSidebar() {
    sidebarCollapsed = !sidebarCollapsed;
    localStorage.setItem('clauge-sidebar-collapsed', String(sidebarCollapsed));
    setTimeout(() => {
      onRefitTerminals?.();
    }, 250);
  }

  function toggleGroup(name) {
    expandedGroups[name] = !expandedGroups[name];
    expandedGroups = { ...expandedGroups };
    localStorage.setItem('clauge-expanded', JSON.stringify(expandedGroups));
  }

  function isGroupExpanded(name) {
    return expandedGroups[name] !== false;
  }

  function groupByProject(list) {
    const g = {};
    for (const p of list) {
      const name = p.projectName || "Unknown";
      if (!g[name]) g[name] = [];
      g[name].push(p);
    }
    return g;
  }

  function relativeTime(iso) {
    if (!iso) return "";
    const sec = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
    if (sec < 60) return "just now";
    if (sec < 3600) return Math.floor(sec/60) + "m ago";
    if (sec < 86400) return Math.floor(sec/3600) + "h ago";
    return Math.floor(sec/86400) + "d ago";
  }

  function handleWindowClick() {
    if (menuProfile) menuProfile = null;
  }

  async function handleEditSave() {
    try {
      profiles = await invoke("get_profiles");
    } catch (_) {}
  }

  // Expose toggleSidebar so parent can call it (e.g. keyboard shortcut Cmd+B)
  export { toggleSidebar };
</script>

<svelte:window onclick={handleWindowClick} />

<EditSessionModal profile={editingProfile} onSave={handleEditSave} onClose={() => editingProfile = null} />

<aside class="sidebar" class:collapsed={sidebarCollapsed}>
  <div class="sidebar-header">
    <span class="app-title">Clauge {#if claudePlan}<span class="plan-badge">{claudePlan}</span>{/if}</span>
    <div class="header-actions">
      <button class="new-btn" onclick={() => { onNewSession?.(); contextsStore.loadContextSnippets(); }} title="New Session (Cmd+N)">+</button>
    </div>
  </div>
  <div class="sidebar-content">
    {#if profiles.length === 0}
      <div class="empty-sidebar">No sessions yet. Click + to create one.</div>
    {:else}
      {#each Object.entries(grouped) as [projectName, items]}
        <div class="project-group">
          <button class="project-header" onclick={() => toggleGroup(projectName)}>
            <svg class="chevron" class:collapsed={!isGroupExpanded(projectName)} width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
              <path d="M3 2l4 3-4 3z"/>
            </svg>
            {projectName}
            <span class="project-count">{items.length}</span>
          </button>
          {#if isGroupExpanded(projectName)}
            {#each items as profile}
              <button
                class="profile-item"
                class:active={activeProfile?.id === profile.id}
                onclick={() => onSelect?.(profile)}
              >
                <div class="profile-title">
                  <span class="status-dot" class:active={activeProfile?.id === profile.id} class:bg-active={terminalStore.sessionActivity[profile.id] === 'active'} class:bg-done={terminalStore.sessionActivity[profile.id] === 'done'}></span>
                  {profile.title}
                </div>
                <div class="profile-meta">
                  <span class="badge" style="color:{purposeColors[profile.purpose] || '#8b949e'}; background:{purposeColors[profile.purpose] || '#8b949e'}22">{profile.purpose}</span>
                  {#if profile.worktreeBranch}
                    <span class="wt-badge" title="Isolated worktree: {profile.worktreeBranch}">WT</span>
                  {/if}
                  <span class="time">{relativeTime(profile.lastUsedAt)}</span>
                </div>
                <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
                <span class="ellipsis-btn" onclick={(e) => { e.stopPropagation(); menuProfile = menuProfile?.id === profile.id ? null : profile; }}>
                  <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor"><circle cx="8" cy="3" r="1.5"/><circle cx="8" cy="8" r="1.5"/><circle cx="8" cy="13" r="1.5"/></svg>
                </span>
                {#if menuProfile?.id === profile.id}
                  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
                  <div class="session-menu" onclick={(e) => e.stopPropagation()}>
                    <button class="session-menu-item" onclick={() => { editingProfile = profile; menuProfile = null; }}>
                      <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><path d="M11.013 1.427a1.75 1.75 0 012.474 0l1.086 1.086a1.75 1.75 0 010 2.474l-8.61 8.61c-.21.21-.47.364-.756.445l-3.251.93a.75.75 0 01-.927-.928l.929-3.25a1.75 1.75 0 01.445-.758l8.61-8.61zm1.414 1.06a.25.25 0 00-.354 0L3.463 11.1a.25.25 0 00-.064.108l-.563 1.97 1.971-.564a.25.25 0 00.108-.064l8.61-8.61a.25.25 0 000-.353L12.427 2.488z"/></svg>
                      Edit
                    </button>
                    <button class="session-menu-item danger" onclick={() => { menuProfile = null; onDeleteProfile?.(null, profile); }}>
                      <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><path d="M6.5 1.75a.25.25 0 01.25-.25h2.5a.25.25 0 01.25.25V3h-3V1.75zM11 3V1.75A1.75 1.75 0 009.25 0h-2.5A1.75 1.75 0 005 1.75V3H2.75a.75.75 0 000 1.5h.928l.747 10.218A1.75 1.75 0 006.172 16h3.656a1.75 1.75 0 001.747-1.282L12.322 4.5h.928a.75.75 0 000-1.5H11zm-5.522 1.5l.735 10.06a.25.25 0 00.249.19h3.076a.25.25 0 00.249-.19l.735-10.06H5.478z"/></svg>
                      Delete
                    </button>
                  </div>
                {/if}
              </button>
            {/each}
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</aside>

<button class="sidebar-toggle" onclick={toggleSidebar} title={sidebarCollapsed ? 'Expand sidebar (Cmd+B)' : 'Collapse sidebar (Cmd+B)'}>
  <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
    {#if sidebarCollapsed}
      <path d="M4 1l5 5-5 5z"/>
    {:else}
      <path d="M8 1L3 6l5 5z"/>
    {/if}
  </svg>
</button>

<style>
  .sidebar { width: 220px; min-width: 220px; background: var(--sidebar-bg); border-right: 1px solid var(--border); display: flex; flex-direction: column; user-select: none; transition: width 0.2s ease, min-width 0.2s ease, opacity 0.2s ease; overflow: hidden; }
  .sidebar.collapsed { width: 0; min-width: 0; border-right: none; opacity: 0; pointer-events: none; }
  .sidebar-toggle { position: absolute; left: 220px; top: 50%; transform: translateY(-50%); z-index: 50; width: 12px; height: 28px; border: none; border-radius: 0 4px 4px 0; background: transparent; color: var(--text-secondary); cursor: pointer; display: flex; align-items: center; justify-content: center; transition: left 0.2s ease, background 0.15s, opacity 0.15s; -webkit-app-region: no-drag; opacity: 0; }
  .sidebar-toggle:hover, :global(.app):hover .sidebar-toggle { opacity: 1; background: rgba(255,255,255,0.06); color: var(--text-primary); }
  .sidebar.collapsed ~ .sidebar-toggle { left: 0; }
  .sidebar-header { display: flex; align-items: center; justify-content: space-between; padding: 14px; padding-top: 38px; border-bottom: 1px solid var(--border); }
  .app-title { font-size: 15px; font-weight: 700; color: var(--text-primary); display: flex; align-items: center; gap: 6px; }
  .plan-badge { font-size: 9px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.8px; padding: 2px 6px; border-radius: 4px; background: linear-gradient(135deg, rgba(255,215,0,0.15), rgba(255,170,50,0.1)); color: #ffd700; border: 1px solid rgba(255,215,0,0.3); position: relative; overflow: hidden; }
  .plan-badge::after { content: ''; position: absolute; top: -50%; left: -100%; width: 60%; height: 200%; background: linear-gradient(90deg, transparent, rgba(255,215,0,0.2), transparent); animation: shine 3s ease-in-out infinite; }
  @keyframes shine { 0% { left: -100%; } 50% { left: 150%; } 100% { left: 150%; } }
  .header-actions { display: flex; gap: 6px; align-items: center; -webkit-app-region: no-drag; }
  .new-btn { width: 28px; height: 28px; border-radius: 6px; border: 1px solid var(--border); background: transparent; color: var(--text-primary); font-size: 18px; cursor: pointer; display: flex; align-items: center; justify-content: center; -webkit-app-region: no-drag; transition: all 0.2s cubic-bezier(0.34, 1.56, 0.64, 1); }
  .new-btn:hover { background: var(--border); transform: scale(1.1); }
  .new-btn:active { transform: scale(0.95); }

  .sidebar-content { flex: 1; overflow-y: auto; padding: 8px 0; -webkit-app-region: no-drag; }
  .empty-sidebar { padding: 24px 14px; text-align: center; color: var(--text-secondary); font-size: 13px; }
  .project-group { margin-bottom: 2px; }
  .project-header { display: flex; align-items: center; gap: 4px; width: 100%; padding: 6px 14px; font-size: 11px; font-weight: 600; color: var(--text-secondary); text-transform: uppercase; letter-spacing: 0.5px; border: none; background: transparent; cursor: pointer; font-family: inherit; transition: color 0.15s; }
  .project-header:hover { color: var(--text-primary); }
  .chevron { transition: transform 0.15s; flex-shrink: 0; }
  .chevron.collapsed { transform: rotate(0deg); }
  .chevron:not(.collapsed) { transform: rotate(90deg); }
  .project-count { margin-left: auto; font-size: 10px; color: var(--text-secondary); opacity: 0.6; font-weight: 400; }
  .profile-item { width: 100%; display: block; text-align: left; padding: 8px 14px; padding-right: 28px; border: none; background: transparent; cursor: pointer; border-left: 3px solid transparent; font-family: inherit; -webkit-app-region: no-drag; position: relative; }
  .profile-item:hover { background: var(--hover-bg, rgba(255,255,255,0.06)); }
  .profile-item.active { background: rgba(31,111,235,0.15); border-left-color: var(--accent); box-shadow: inset 0 0 20px rgba(88, 166, 255, 0.08); }
  .profile-title { font-size: 13px; font-weight: 500; color: var(--text-primary); margin-bottom: 3px; }
  .profile-meta { display: flex; align-items: center; justify-content: space-between; }
  .badge { font-size: 10px; font-weight: 600; padding: 1px 6px; border-radius: 10px; }
  .wt-badge { font-size: 8px; font-weight: 700; padding: 1px 4px; border-radius: 3px; background: rgba(210, 168, 255, 0.2); color: #d2a8ff; letter-spacing: 0.5px; }
  .ellipsis-btn { position: absolute; right: 6px; top: 50%; transform: translateY(-50%); opacity: 0; padding: 4px; border-radius: 4px; color: var(--text-secondary); cursor: pointer; transition: opacity 0.15s, background 0.15s; z-index: 2; }
  .profile-item:hover .ellipsis-btn { opacity: 1; }
  .ellipsis-btn:hover { background: var(--hover-bg, rgba(255,255,255,0.08)); color: var(--text-primary); }
  .session-menu { position: absolute; right: 6px; top: calc(50% + 14px); background: #1c2128; border: 1px solid var(--border); border-radius: 8px; padding: 4px; min-width: 110px; box-shadow: 0 8px 24px rgba(0,0,0,0.4); z-index: 10; animation: fadeIn 0.1s ease-out; }
  .session-menu-item { display: flex; align-items: center; gap: 6px; width: 100%; padding: 6px 10px; border: none; background: transparent; color: var(--text-secondary); font-size: 12px; font-family: inherit; cursor: pointer; border-radius: 5px; transition: background 0.12s; }
  .session-menu-item:hover { background: rgba(255,255,255,0.06); }
  .session-menu-item.danger:hover { background: rgba(248,81,73,0.12); color: #f85149; }
  .time { font-size: 11px; color: var(--text-secondary); }
  .status-dot { display: inline-block; width: 6px; height: 6px; border-radius: 50%; background: #484f58; margin-right: 6px; vertical-align: middle; transition: background 0.3s; }
  .status-dot.active { background: #3fb950; box-shadow: 0 0 6px rgba(63, 185, 80, 0.5); }
  .status-dot.bg-active { background: var(--accent); box-shadow: 0 0 6px color-mix(in srgb, var(--accent) 50%, transparent); animation: bgPulse 0.8s ease-in-out infinite; }
  .status-dot.bg-done { background: #d29922; box-shadow: 0 0 6px rgba(210, 153, 34, 0.5); }
  @keyframes bgPulse { 0%, 100% { opacity: 1; transform: scale(1); } 50% { opacity: 0.4; transform: scale(0.7); } }
  @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }
</style>

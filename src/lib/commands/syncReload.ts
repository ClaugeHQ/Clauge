/**
 * Reload every synced domain store from SQLite after a bulk import
 * (restore, merge, snapshot restore). Dynamic imports keep this module
 * free of eager dependencies on every mode's store graph.
 */
export async function reloadSyncedStores(): Promise<void> {
  const [r, s, n, ssh, agent, explorer, workspace] = await Promise.all([
    import('$lib/modes/rest/stores'),
    import('$lib/modes/sql/stores'),
    import('$lib/modes/nosql/stores'),
    import('$lib/modes/ssh/stores'),
    import('$lib/modes/agent/stores'),
    import('$lib/modes/explorer/stores'),
    import('$lib/modes/workspace/stores'),
  ]);
  await Promise.all([
    r.loadCollections(),
    r.loadEnvironments(),
    s.loadConnections(),
    s.loadSqlScripts(),
    n.loadNoSqlConnections(),
    ssh.loadSshProfiles(),
    agent.loadAgentSessions(),
    agent.loadAgentContexts(),
    explorer.loadExplorerConnections(),
    workspace.loadWorkspaces(),
    workspace.loadCoworkers(),
  ]);

  // Notes + boards are cached per-workspace and loaded lazily, so a bulk
  // import would otherwise leave stale entries behind. Refresh every
  // workspace whose cache is already populated; never-opened workspaces
  // load fresh on first open anyway.
  const { get } = await import('svelte/store');
  const noteWs = [...get(workspace.notesByWorkspace).keys()];
  const boardWs = [...get(workspace.boardsByWorkspace).keys()];
  await Promise.all([
    ...noteWs.map(id => workspace.loadNotes(id)),
    ...boardWs.map(id => workspace.loadBoards(id)),
  ]);
}

<script lang="ts">
  import { onMount } from 'svelte';
  import { dndzone, type DndEvent } from 'svelte-dnd-action';
  import {
    columnsByBoard,
    cardsByBoard,
    loadBoardContents,
    workspaces as workspacesStore,
    inflightMentions,
    cardLastSeenAt,
    isCardUnread,
  } from '../stores';
  import { agentIcon } from '../agentIcon';
  import {
    workspaceBoardGet,
    workspaceBoardRename,
    workspaceCardCreate,
    workspaceBoardDismissedExternals,
    workspaceCardUpdate,
    workspaceCardMove,
    workspaceCardClearReview,
    workspaceCardDelete,
    workspaceScanProjectIssues,
    workspaceScanProjectIssuesByUrl,
    workspaceCardPushToRepo,
  } from '../commands';
  import type { ProjectScanResult } from '../types';
  import BoardSyncBanner from './BoardSyncBanner.svelte';
  import BoardConfigDialog from './BoardConfigDialog.svelte';
  import { tagColor } from '../tagColor';
  import { cardSourceBadge } from '../cardSource';
  import { coworkers } from '../stores';
  import CoworkerAvatar from './CoworkerAvatar.svelte';
  import { tabs as sharedTabs, updateTab } from '$lib/shared/stores/tabs';
  import { get } from 'svelte/store';
  import { currentUserActor, describeActor, formatAttribution } from '../attribution';
  import type { WorkspaceBoard, WorkspaceBoardCard } from '../types';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import { showContextMenu } from '$lib/shared/primitives/contextmenu';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import CardEditorDrawer from './CardEditorDrawer.svelte';
  import CardCreateDialog from './CardCreateDialog.svelte';
  import GhNotInstalledModal from './GhNotInstalledModal.svelte';
  import GlabNotInstalledModal from './GlabNotInstalledModal.svelte';

  // ── PR / branch + repo-aware actions ──────────────────────────
  // BoardView mirrors the install-modal pattern from CardEditorDrawer
  // so the right-click and bulk actions can prompt the user to install
  // `gh` / `glab` instead of showing a raw stderr toast.
  function detectMissingCli(errMsg: string): 'gh' | 'glab' | null {
    const m = errMsg.match(/^(gh|glab) is not installed/);
    return m ? (m[1] as 'gh' | 'glab') : null;
  }
  let showGhNotInstalled = $state(false);
  let showGlabNotInstalled = $state(false);

  interface Props {
    boardId: string;
  }

  let { boardId }: Props = $props();

  let board = $state<WorkspaceBoard | null>(null);
  // Column id the "new ticket" dialog is open for (null = closed), plus
  // a title to prefill (from the quick-add input).
  let createDialogColumn = $state<string | null>(null);
  let createDialogTitle = $state('');

  /** Open the unified create dialog for a column, optionally prefilled
   *  with the quick-add text. Keeps the create flow uniform — typing +
   *  Enter no longer silently creates a card. */
  function openCreateDialog(columnId: string) {
    createDialogTitle = (inlineNewByColumn[columnId] ?? '').trim();
    inlineNewByColumn = { ...inlineNewByColumn, [columnId]: '' };
    createDialogColumn = columnId;
  }
  let nameDraft = $state('');
  let inlineNewByColumn = $state<Record<string, string>>({});
  let editingCard = $state<WorkspaceBoardCard | null>(null);
  let scanState = $state<ProjectScanResult | null>(null);
  let scanBusy = $state(false);
  let scanDismissed = $state(false);
  let lastSyncedAt = $state<number | null>(null);
  let showConfigDialog = $state(false);

  /** Effective project context for sync. Two fields:
   *    path — local clone (preferred when present; sync runs from cwd)
   *    url  — direct GitHub/GitLab URL (used when path is null)
   *  Resolution order, per field:
   *    1. Per-board override in source_config JSON ({path?, url?})
   *    2. Parent workspace's project_path (path only — workspaces don't
   *       carry a URL field today; can be added later if needed)
   *  Both null → no project bound, banner hidden. */
  const boardConfig = $derived.by(() => {
    const b = board;
    if (!b) return { path: null as string | null, url: null as string | null };
    let path: string | null = null;
    let url: string | null = null;
    if (b.sourceConfig) {
      try {
        const cfg = JSON.parse(b.sourceConfig);
        if (cfg && typeof cfg.project_path === 'string' && cfg.project_path.trim()) path = cfg.project_path;
        if (cfg && typeof cfg.project_url  === 'string' && cfg.project_url.trim())  url  = cfg.project_url;
      } catch { /* ignore */ }
    }
    if (!path) {
      path = $workspacesStore.find(w => w.id === b.workspaceId)?.projectPath ?? null;
    }
    return { path, url };
  });
  const projectPath = $derived(boardConfig.path);
  const projectUrl  = $derived(boardConfig.url);
  /** Banner shows when EITHER path or url is set. */
  const hasProject  = $derived(!!(projectPath || projectUrl));

  function showHeaderMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const items: any[] = [
      {
        label: hasProject ? 'Change project' : 'Set project',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/><path d="M9 13l2 2 4-4"/></svg>',
        action: () => { showConfigDialog = true; },
      },
    ];
    // Bulk "push every local card as an issue" — only when a) the
    // workspace has a repo URL and b) there's at least one unlinked
    // card. Otherwise this entry would be dead clickable noise.
    if (repoUrl && localCards.length > 0) {
      items.push({ label: '', action: () => {}, separator: true });
      items.push({
        label: bulkPushing
          ? `Pushing ${localCards.length} card${localCards.length === 1 ? '' : 's'}…`
          : `Create ${repoLabel} issues for ${localCards.length} local card${localCards.length === 1 ? '' : 's'}`,
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="12 5 12 19"/><polyline points="5 12 12 19 19 12"/></svg>',
        action: () => bulkPushLocalCards(),
      });
    }
    showContextMenu(rect.left, rect.bottom + 4, items);
  }

  async function onProjectChanged() {
    // Re-fetch the board so the new source_config flows back into
    // `board` and the derived `projectPath` updates.
    if (board) {
      try { board = await workspaceBoardGet(board.id); } catch { /* ignore */ }
    }
    // Reset scan state so the banner offers a fresh sync against the
    // new project URL.
    scanState = null;
    lastSyncedAt = null;
    scanDismissed = false;
  }

  let confirmShow = $state(false);
  let confirmTarget = $state<WorkspaceBoardCard | null>(null);

  const columns = $derived($columnsByBoard.get(boardId) ?? []);
  const cards = $derived($cardsByBoard.get(boardId) ?? []);

  // Workspace + repo context for the new push / PR actions surfaced
  // via the header menu and the per-card right-click menu.
  const workspace = $derived.by(() => {
    const b = board;
    if (!b) return null;
    return $workspacesStore.find(w => w.id === b.workspaceId) ?? null;
  });
  const repoUrl = $derived(workspace?.repoUrl ?? null);
  const repoLabel = $derived.by(() => {
    const u = (repoUrl ?? '').toLowerCase();
    if (u.includes('github.com')) return 'GitHub';
    if (u.includes('gitlab')) return 'GitLab';
    return 'repo';
  });
  /** Cards that haven't been pushed to a remote issue tracker yet. */
  const localCards = $derived(
    cards.filter(c => !c.externalId || !c.externalId.trim()),
  );

  /** Bulk: turn every local card on this board into a real issue.
   *  Run sequentially so we don't hammer gh/glab rate limits and so
   *  errors per card surface cleanly. Stops on the first
   *  "not installed" error since every subsequent card would fail the
   *  same way. */
  let bulkPushing = $state(false);
  async function bulkPushLocalCards() {
    if (bulkPushing) return;
    if (!repoUrl) {
      showToast(`Set the workspace repo URL first`, 'error');
      return;
    }
    const targets = localCards.slice();
    if (targets.length === 0) {
      showToast('No local cards to push', 'info');
      return;
    }
    bulkPushing = true;
    let ok = 0;
    let failed = 0;
    for (const c of targets) {
      try {
        await workspaceCardPushToRepo(c.id, currentUserActor());
        ok += 1;
      } catch (e) {
        const msg = `${e}`;
        const missing = detectMissingCli(msg);
        if (missing === 'gh') { showGhNotInstalled = true; break; }
        if (missing === 'glab') { showGlabNotInstalled = true; break; }
        failed += 1;
      }
    }
    await loadBoardContents(boardId);
    bulkPushing = false;
    if (ok > 0 && failed === 0) showToast(`Created ${ok} issue${ok === 1 ? '' : 's'} on ${repoLabel}`, 'success');
    else if (ok > 0 && failed > 0) showToast(`Created ${ok}, failed ${failed}`, 'info');
    else if (failed > 0) showToast(`Failed to push ${failed} card${failed === 1 ? '' : 's'}`, 'error');
  }

  /** Single-card create-issue from the right-click menu. */
  async function pushOneCardAsIssue(c: WorkspaceBoardCard) {
    if (!repoUrl) {
      showToast(`Set the workspace repo URL first`, 'error');
      return;
    }
    try {
      const r = await workspaceCardPushToRepo(c.id, currentUserActor());
      showToast(`Created issue ${r.externalId}`, 'success');
      await loadBoardContents(boardId);
    } catch (e) {
      const msg = `${e}`;
      const missing = detectMissingCli(msg);
      if (missing === 'gh') showGhNotInstalled = true;
      else if (missing === 'glab') showGlabNotInstalled = true;
      else showToast(`Issue creation failed: ${msg}`, 'error');
    }
  }

  async function openExternalUrl(url: string) {
    try {
      const { openUrl } = await import('@tauri-apps/plugin-opener');
      await openUrl(url);
    } catch {
      window.open(url, '_blank', 'noopener,noreferrer');
    }
  }

  async function copyToClipboard(text: string, label: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast(`Copied ${label}`, 'success');
    } catch {
      showToast('Copy failed', 'error');
    }
  }

  /** Group cards by column for rendering. Stable order (column.position
   *  then card.position) is preserved by `cardsByBoard`'s SQL ORDER BY. */
  const cardsByColumn = $derived.by(() => {
    const m = new Map<string, WorkspaceBoardCard[]>();
    for (const col of columns) m.set(col.id, []);
    for (const c of cards) {
      const arr = m.get(c.columnId) ?? [];
      arr.push(c);
      m.set(c.columnId, arr);
    }
    return m;
  });

  async function bootstrap(id: string) {
    board = null;
    // Reset sync-banner state on every board switch. Without this, the
    // previous board's URL + sync timestamp + dismissed flag leak into
    // the new board (BoardView is reused across boards / workspaces —
    // Svelte updates the `boardId` prop rather than remounting). Most
    // visibly: open Workspace A → board with synced GitHub repo, switch
    // to Workspace B's board, see Workspace A's repo URL still showing.
    scanState = null;
    scanBusy = false;
    scanDismissed = false;
    lastSyncedAt = null;
    try {
      board = await workspaceBoardGet(id);
      nameDraft = board.name;
      await loadBoardContents(id);
    } catch (e) {
      errorToast('Failed to load board', e);
    }
  }

  async function commitNameChange() {
    const current = board;
    if (!current) return;
    const trimmed = nameDraft.trim();
    if (!trimmed || trimmed === current.name) return;
    try {
      await workspaceBoardRename(current.id, trimmed);
      board = { ...current, name: trimmed };
      const myTab = get(sharedTabs).find(t => t.mode === 'workspace' && t.key === `board:${current.id}`);
      if (myTab) updateTab(myTab.id, { label: trimmed });
    } catch (e) {
      errorToast('Rename failed', e);
      nameDraft = current.name;
    }
  }

  $effect(() => { bootstrap(boardId); });

  /** Handler used by both the in-zone consider phase and the finalize
   *  drop. We update the in-memory store eagerly during consider so the
   *  ghost-card lands smoothly, then persist on finalize. */
  function handleConsider(columnId: string, e: CustomEvent<DndEvent<WorkspaceBoardCard>>) {
    const next = new Map($cardsByBoard);
    // Replace this column's slice with the proposed items, leave other
    // columns alone, and recompute the flat list with positions.
    const flat: WorkspaceBoardCard[] = [];
    for (const col of columns) {
      const slice = col.id === columnId ? e.detail.items : (cardsByColumn.get(col.id) ?? []);
      slice.forEach((c, idx) => flat.push({ ...c, columnId: col.id, position: idx }));
    }
    next.set(boardId, flat);
    cardsByBoard.set(next);
  }

  async function handleFinalize(columnId: string, e: CustomEvent<DndEvent<WorkspaceBoardCard>>) {
    handleConsider(columnId, e);
    const moved = e.detail.info.id as string;
    const items = e.detail.items;
    const idx = items.findIndex(c => c.id === moved);
    if (idx < 0) return;
    try {
      await workspaceCardMove({
        id: moved,
        columnId,
        position: idx,
        actor: currentUserActor(),
      });
      // Server clears review_pending on user moves; mirror that
      // optimistically so the "Pending review" badge clears
      // immediately when the user drags an agent-flagged card out
      // of the Review column. Without this, the local store still
      // shows the stale flag until a full reload.
      const map = $cardsByBoard;
      const list = map.get(boardId) ?? [];
      const next = list.map(c =>
        c.id === moved ? { ...c, columnId, reviewPending: 0 } : c,
      );
      cardsByBoard.set(new Map(map).set(boardId, next));
    } catch (err) {
      errorToast('Move failed', err);
      await loadBoardContents(boardId);
    }
  }

  async function approveCard(card: WorkspaceBoardCard) {
    // Approve = clear review_pending. Doesn't move the card; user can
    // separately drag it to Done if they want.
    try {
      await workspaceCardClearReview(card.id, currentUserActor());
      await loadBoardContents(boardId);
      showToast('Approved', 'success');
    } catch (e) {
      errorToast('Approve failed', e);
    }
  }

  async function requestChanges(card: WorkspaceBoardCard) {
    // Request changes = clear review_pending + move back to the
    // "In Progress" active-work column. Same matcher Rust uses for
    // is_active_class.
    const active = columns.find(c => c.name.trim().toLowerCase() === 'in progress')
      ?? columns.find(c => c.name.trim().toLowerCase() === 'todo')
      ?? columns[0];
    if (!active) return;
    try {
      await workspaceCardClearReview(card.id, currentUserActor());
      await workspaceCardMove({
        id: card.id,
        columnId: active.id,
        position: 0,
        actor: currentUserActor(),
      });
      await loadBoardContents(boardId);
      showToast(`Moved back to ${active.name}`, 'success');
    } catch (e) {
      errorToast('Request changes failed', e);
    }
  }

  function showCardMenu(e: MouseEvent, card: WorkspaceBoardCard) {
    e.preventDefault();
    e.stopPropagation();
    const isLocal = !card.externalId || !card.externalId.trim();
    const items: any[] = [
      {
        label: 'Edit',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
        action: () => editingCard = card,
      },
    ];

    // ── Issue + PR shortcuts ────────────────────────────────────
    // Keep these together as a related group so users learn the
    // lifecycle (issue first, then PR) by reading the menu.
    items.push({ label: '', action: () => {}, separator: true });

    if (isLocal && repoUrl) {
      items.push({
        label: `Create issue on ${repoLabel}`,
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="16"/><line x1="8" y1="12" x2="16" y2="12"/></svg>',
        action: () => pushOneCardAsIssue(card),
      });
    }
    if (card.externalUrl) {
      items.push({
        label: 'View issue on host',
        sub: card.externalId ?? undefined,
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>',
        action: () => openExternalUrl(card.externalUrl!),
      });
    }
    if (card.prUrl) {
      items.push({
        label: 'View PR on host',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="18" r="2"/><path d="M6 8v8M11 6h4a3 3 0 0 1 3 3v7"/></svg>',
        action: () => openExternalUrl(card.prUrl!),
      });
    }

    // Copy actions — most useful link is the host URL when present,
    // falling back to a clauge:// card link so other panes can
    // navigate back here.
    items.push({
      label: card.prUrl ? 'Copy PR URL' : card.externalUrl ? 'Copy issue URL' : 'Copy card title',
      icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>',
      action: () =>
        copyToClipboard(
          card.prUrl ?? card.externalUrl ?? card.title,
          card.prUrl ? 'PR URL' : card.externalUrl ? 'issue URL' : 'title',
        ),
    });

    if (card.reviewPending === 1) {
      items.push({ label: '', action: () => {}, separator: true });
      items.push({
        label: 'Approve (clear review)',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="20 6 9 17 4 12"/></svg>',
        action: () => approveCard(card),
      });
      items.push({
        label: 'Request changes',
        icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 102.13-9.36L1 10"/></svg>',
        action: () => requestChanges(card),
      });
    }
    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: 'Delete',
      danger: true,
      icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/></svg>',
      action: () => {
        confirmTarget = card;
        confirmShow = true;
      },
    });
    showContextMenu(e.clientX, e.clientY, items);
  }

  async function handleConfirmDelete() {
    if (!confirmTarget) return;
    const target = confirmTarget;
    confirmTarget = null;
    try {
      await workspaceCardDelete(target.id);
      await loadBoardContents(boardId);
      showToast(`Deleted "${target.title}"`, 'success');
    } catch (e) {
      errorToast('Delete failed', e);
    }
  }

  async function onCardSaved() {
    // Refresh the board so column moves / new comments / pushes show
    // up on the kanban tiles. Crucially: DO NOT close the drawer.
    // `onsave` fires on every chat turn (auto-save, comment, agent
    // reply); closing the drawer mid-conversation was the bug behind
    // the "drawer slams shut after Alex replies" complaint.
    await loadBoardContents(boardId);
    // Keep editingCard pointing at the freshest version of this card
    // so the drawer's prop stays valid (was the source of the
    // null-card crash earlier).
    if (editingCard) {
      const list = $cardsByBoard.get(boardId) ?? [];
      const fresh = list.find(c => c.id === editingCard!.id);
      if (fresh) editingCard = fresh;
    }
  }

  /** Trigger a scan and, on success, bulk-create cards for every
   *  fetched issue that isn't already represented (matched by
   *  external_id). Cards land in the first column whose name suggests
   *  "Todo"; falls back to the very first column. */
  async function runScanAndImport() {
    if (!board || !hasProject || scanBusy) return;
    scanBusy = true;
    scanDismissed = false;
    try {
      // Path wins over URL — a local clone gives strictly more accurate
      // remote info than parsing a URL string. URL flow is the fallback
      // for users who haven't cloned (or don't want to).
      const result = projectPath
        ? await workspaceScanProjectIssues(projectPath)
        : await workspaceScanProjectIssuesByUrl(projectUrl as string);
      scanState = result;
      if (result.kind !== 'success') return;
      lastSyncedAt = Date.now();
      // Pick a target column. "Todo" preferred, else first available.
      const target = columns.find(c => c.name.toLowerCase().includes('todo')) ?? columns[0];
      if (!target) return;
      // Two skip-sets: cards already on the board (de-dup) AND
      // tombstoned externals the user previously deleted (so a Done
      // issue they removed doesn't keep coming back to Todo).
      const existingExternalIds = new Set(cards.map(c => c.externalId).filter(Boolean));
      const dismissed = new Set(await workspaceBoardDismissedExternals(boardId).catch(() => []));
      let pos = (cardsByColumn.get(target.id) ?? []).length;
      for (const issue of result.issues) {
        if (existingExternalIds.has(issue.externalId)) continue;
        if (dismissed.has(issue.externalId)) continue;
        try {
          await workspaceCardCreate({
            columnId: target.id,
            title: `${issue.externalId} ${issue.title}`,
            description: issue.body,
            tags: issue.labels,
            position: pos++,
            externalId: issue.externalId,
            externalUrl: issue.url,
            actor: currentUserActor(),
            createdAt: issue.createdAt || undefined,
          });
        } catch (e) { console.warn('skip issue:', issue.externalId, e); }
      }
      await loadBoardContents(boardId);
    } catch (e) {
      scanState = { kind: 'apiError', message: String(e) };
    } finally {
      scanBusy = false;
    }
  }
</script>

{#if !board}
  <div class="bv-loading">Loading board…</div>
{:else}
  <div class="bv">
    <header class="bv-header">
      <div class="bv-header-row">
        <span class="bv-icon">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="6" height="16" rx="1"/><rect x="11" y="4" width="6" height="10" rx="1"/><rect x="19" y="4" width="2" height="14" rx="1"/></svg>
        </span>
        <input
          class="bv-title-input"
          bind:value={nameDraft}
          size={Math.min(Math.max(nameDraft.length, 8), 38)}
          onblur={commitNameChange}
          onkeydown={(e) => { if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur(); }}
          placeholder="Board name"
          spellcheck="false"
        />
        <span class="bv-count">{cards.length} card{cards.length === 1 ? '' : 's'}</span>
        <button class="bv-menu" onclick={showHeaderMenu} title="Board options" aria-label="Board options">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><circle cx="5" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="19" cy="12" r="1.6"/></svg>
        </button>
      </div>
    </header>

    {#if !scanDismissed}
      <BoardSyncBanner
        projectPath={hasProject ? (projectPath ?? projectUrl) : null}
        state={scanState}
        busy={scanBusy}
        {lastSyncedAt}
        onsync={runScanAndImport}
        ondismiss={() => scanDismissed = true}
      />
    {/if}

    <div class="bv-board">
      {#each columns as col (col.id)}
        {@const colCards = cardsByColumn.get(col.id) ?? []}
        <div class="bv-col">
          <div class="bv-col-header">
            <span class="bv-col-dot" style="background: {col.color || 'var(--t3)'}"></span>
            <span class="bv-col-name">{col.name}</span>
            <span class="bv-col-count">{colCards.length}</span>
          </div>

          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="bv-col-body"
            use:dndzone={{ items: colCards, type: 'workspace-card', flipDurationMs: 150 }}
            onconsider={(e) => handleConsider(col.id, e as CustomEvent<DndEvent<WorkspaceBoardCard>>)}
            onfinalize={(e) => handleFinalize(col.id, e as CustomEvent<DndEvent<WorkspaceBoardCard>>)}
          >
            {#each colCards as card (card.id)}
              {@const editor = describeActor(card.updatedBy)}
              {@const tags = (() => { try { return JSON.parse(card.tags) as string[]; } catch { return []; } })()}
              {@const src = cardSourceBadge(card)}
              {@const unread = isCardUnread(card, $cardLastSeenAt)}
              {@const creatorCw = card.createdByCoworkerId
                ? $coworkers.find(c => c.id === card.createdByCoworkerId) ?? null
                : null}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              {@const inflight = $inflightMentions.get(card.id) ?? null}
              {@const inflightProvider = inflight?.provider ?? null}
              {@const inflightIco = inflightProvider ? agentIcon(inflightProvider) : null}
              <div
                class="bv-card"
                class:bv-card-review={card.reviewPending === 1}
                class:bv-card-inflight={!!inflightProvider}
                class:bv-card-unread={unread}
                onclick={() => editingCard = card}
                oncontextmenu={(e) => showCardMenu(e, card)}
              >
                <div class="bv-card-top">
                  {#if unread}
                    <span class="bv-card-unread-dot" title="New activity from {editor.label}"></span>
                  {/if}
                  {#if card.priority}
                    <span class="bv-priority bv-priority-{card.priority.toLowerCase()}">{card.priority}</span>
                  {/if}
                  <span class="bv-card-title">{card.title}</span>
                  {#if card.reviewPending === 1}
                    <span class="bv-review-badge" title="Pending review">Pending review</span>
                  {/if}
                </div>
                {#if tags.length > 0}
                  <div class="bv-card-tags">
                    {#each tags as t}
                      {@const c = tagColor(t)}
                      <span class="bv-card-tag" style="color:{c.fg};background:{c.bg};border-color:{c.border};">{t}</span>
                    {/each}
                  </div>
                {/if}
                <div class="bv-card-foot">
                  {#if creatorCw}
                    <!-- Creator chip wins when a persona made the card —
                         signals "this work is owned by @alex." Avatar +
                         name is more informative than the generic agent
                         star, and lets the user scan the board by who's
                         doing what. -->
                    <span class="bv-card-creator" title="Created by @{creatorCw.name}">
                      <CoworkerAvatar seed={creatorCw.avatarSeed} style={creatorCw.avatarStyle} size={12} />
                      <span>@{creatorCw.name}</span>
                    </span>
                  {:else if editor.kind === 'coworker'}
                    <span class="bv-card-creator" title="Last edit by @{editor.label}">
                      <CoworkerAvatar seed={editor.coworkerSeed ?? editor.label} style={editor.coworkerStyle ?? 'personas'} size={12} />
                      <span>@{editor.label}</span>
                    </span>
                  {:else if editor.kind === 'agent'}
                    <span class="bv-card-actor bv-card-actor-agent" title="Last edit by {editor.label}">
                      <svg viewBox="0 0 24 24" width="9" height="9" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l1.6 4.8L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.2L12 3z"/></svg>
                      {editor.label}
                    </span>
                  {:else if editor.kind === 'user'}
                    <span class="bv-card-creator" title="Last edit by @{editor.label}">
                      {#if editor.avatarUrl}
                        <img class="bv-card-actor-avatar" src={editor.avatarUrl} alt="" width="12" height="12" />
                      {:else}
                        <span class="bv-card-actor-initials">{editor.label.slice(0, 2).toUpperCase()}</span>
                      {/if}
                      <span>@{editor.label}</span>
                    </span>
                  {:else}
                    <span class="bv-card-actor bv-card-actor-anon" title="Last edit by you">
                      <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 21a8 8 0 10-16 0"/><circle cx="12" cy="7" r="4"/></svg>
                      {editor.label}
                    </span>
                  {/if}
                  <span class="bv-card-time">· {formatAttribution(card.updatedBy, card.updatedAt).split('· ')[1] ?? ''}</span>
                  {#if card.commentCount > 0}
                    <span class="bv-card-comments" title="{card.commentCount} {card.commentCount === 1 ? 'comment' : 'comments'}">
                      <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
                      {card.commentCount}
                    </span>
                  {/if}
                  {#if inflightIco}
                    <span
                      class="bv-card-inflight-chip"
                      style={`color: ${inflightIco.color}; background: color-mix(in srgb, ${inflightIco.color} 16%, transparent); border-color: color-mix(in srgb, ${inflightIco.color} 45%, transparent);`}
                      title={`@${inflightProvider} is working on this card…`}
                    >
                      <span class="bv-card-inflight-pulse">
                        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                        {@html inflightIco.svg}
                      </span>
                      <span>thinking…</span>
                    </span>
                  {/if}
                  {#if card.prUrl}
                    <a
                      class="bv-card-pr"
                      href={card.prUrl}
                      target="_blank"
                      rel="noreferrer noopener"
                      title="Open PR · {card.prUrl}"
                      onclick={(e) => e.stopPropagation()}
                    >
                      <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <circle cx="6" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="18" r="2"/>
                        <path d="M6 8v8M11 6h4a3 3 0 0 1 3 3v7"/>
                      </svg>
                      <span>PR</span>
                    </a>
                  {/if}
                  {#if src.kind === 'github' || src.kind === 'gitlab' || src.kind === 'external'}
                    <a
                      class="bv-card-source bv-card-source-{src.kind}"
                      href={src.url ?? '#'}
                      target="_blank"
                      rel="noreferrer noopener"
                      title="Linked issue · open in browser"
                      onclick={(e) => e.stopPropagation()}
                    >
                      {#if src.kind === 'github'}
                        <svg viewBox="0 0 16 16" width="10" height="10" fill="currentColor" aria-hidden="true"><path d="M8 0C3.58 0 0 3.58 0 8a8 8 0 005.47 7.59c.4.07.55-.17.55-.38v-1.49c-2.23.48-2.7-1.07-2.7-1.07-.36-.93-.89-1.18-.89-1.18-.73-.5.06-.49.06-.49.81.06 1.24.83 1.24.83.72 1.23 1.88.87 2.34.67.07-.52.28-.87.51-1.07-1.78-.2-3.65-.89-3.65-3.96 0-.88.31-1.59.83-2.15-.08-.2-.36-1.03.08-2.14 0 0 .67-.21 2.2.82a7.6 7.6 0 014 0c1.53-1.04 2.2-.82 2.2-.82.44 1.11.16 1.94.08 2.14.52.56.83 1.27.83 2.15 0 3.07-1.87 3.75-3.66 3.95.29.25.54.73.54 1.48v2.2c0 .21.15.46.55.38A8 8 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>
                      {:else if src.kind === 'gitlab'}
                        <svg viewBox="0 0 16 16" width="10" height="10" fill="currentColor" aria-hidden="true"><path d="M8 14.41 11.07 5h-6.14L8 14.41z" opacity=".7"/><path d="M3.86 6.93 2.93 9.79a.62.62 0 00.23.7L8 14.41 4.93 5H1.79l2.07 1.93z" opacity=".5"/><path d="M8 14.41l3.07-9.41h-3.07v9.41z" opacity=".9"/><path d="M12.14 6.93l.93 2.86a.62.62 0 01-.23.7L8 14.41 11.07 5h3.14l-2.07 1.93z" opacity=".5"/></svg>
                      {:else}
                        <svg viewBox="0 0 16 16" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 11l6-6M7 5h4v4"/></svg>
                      {/if}
                      <span>{src.label}</span>
                    </a>
                  {:else}
                    <span class="bv-card-source bv-card-source-local" title="Local card — not pushed to a repo">local</span>
                  {/if}
                </div>
              </div>
            {/each}
          </div>

          <div class="bv-col-add">
            <input
              class="bv-add-input"
              type="text"
              placeholder="+ Add a card"
              value={inlineNewByColumn[col.id] ?? ''}
              oninput={(e) => inlineNewByColumn = { ...inlineNewByColumn, [col.id]: e.currentTarget.value }}
              onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); openCreateDialog(col.id); } }}
              spellcheck="false"
            />
            <button class="bv-add-more" title="New ticket (description, priority, local/cloud)" onclick={() => openCreateDialog(col.id)} aria-label="New ticket">
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
            </button>
          </div>
        </div>
      {/each}
    </div>
  </div>
{/if}

{#if editingCard}
  <CardEditorDrawer
    card={editingCard}
    workspace={board ? ($workspacesStore.find(w => w.id === board!.workspaceId) ?? null) : null}
    onclose={() => editingCard = null}
    onsave={onCardSaved}
  />
{/if}

{#if createDialogColumn}
  <CardCreateDialog
    columnId={createDialogColumn}
    boardId={boardId}
    initialTitle={createDialogTitle}
    position={(cardsByColumn.get(createDialogColumn) ?? []).length}
    onclose={() => (createDialogColumn = null)}
    oncreated={() => loadBoardContents(boardId)}
  />
{/if}

{#if board}
  <BoardConfigDialog
    bind:show={showConfigDialog}
    boardId={board.id}
    initialPath={projectPath}
    initialUrl={projectUrl}
    onsaved={onProjectChanged}
  />
{/if}

<ConfirmDialog
  bind:show={confirmShow}
  title="Delete card"
  message={confirmTarget ? `Delete "${confirmTarget.title}"? This cannot be undone.` : ''}
  confirmText="Delete"
  onconfirm={handleConfirmDelete}
/>

<GhNotInstalledModal bind:show={showGhNotInstalled} />
<GlabNotInstalledModal bind:show={showGlabNotInstalled} />

<style>
  .bv-loading {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 12.5px;
  }
  .bv {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }
  .bv-header {
    flex-shrink: 0;
    padding: 14px 22px 12px;
    border-bottom: 1px solid var(--b1);
    background: var(--n2);
  }
  .bv-header-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .bv-icon {
    display: inline-flex;
    color: var(--acc);
  }
  /* Title input sizes to its content (size={n} attr in markup) so it
     doesn't stretch the full header. Hover/focus give a subtle box so
     it's discoverable as editable without looking like a form field
     when idle. */
  .bv-title-input {
    flex: 0 1 auto;
    border: none;
    background: transparent;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 16px;
    font-weight: 600;
    outline: none;
    padding: 2px 6px;
    border-radius: 5px;
    min-width: 0;
    max-width: 100%;
    width: auto;
    text-overflow: ellipsis;
    transition: background 0.12s;
  }
  .bv-title-input:hover { background: var(--surface-hover); }
  .bv-title-input:focus { background: var(--surface-hover); }
  .bv-title-input::placeholder { color: var(--t4); }
  .bv-count {
    margin-left: auto;
    font-size: 11px;
    color: var(--t3);
    font-family: var(--ui);
  }
  .bv-menu {
    width: 24px;
    height: 24px;
    border-radius: 5px;
    border: none;
    background: transparent;
    color: var(--t3);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: default;
    transition: background 0.1s, color 0.1s;
    flex-shrink: 0;
  }
  .bv-menu:hover { background: var(--surface-hover); color: var(--t1); }

  .bv-board {
    flex: 1;
    display: flex;
    gap: 14px;
    padding: 18px 18px 4px;
    overflow-x: auto;
    overflow-y: hidden;
    min-height: 0;
  }
  .bv-board::-webkit-scrollbar { height: 6px; }
  .bv-board::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 3px; }

  .bv-col {
    flex: 0 0 300px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    background: var(--n);
    border: 1px solid var(--b1);
    border-radius: 12px;
    overflow: hidden;
  }
  .bv-col-header {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 11px 14px;
    border-bottom: 1px solid var(--b1);
    background: var(--n2);
    min-width: 0;
  }
  .bv-col-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .bv-col-name {
    font-family: var(--ui);
    font-size: 11.5px;
    font-weight: 600;
    color: var(--t1);
    letter-spacing: 0.02em;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bv-col-count {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t4);
    flex-shrink: 0;
  }

  .bv-col-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    overflow-y: auto;
    min-height: 80px;
  }
  .bv-col-body::-webkit-scrollbar { width: 4px; }
  .bv-col-body::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .bv-card {
    background: var(--surface-card);
    border: 1px solid var(--b1);
    border-radius: 10px;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 9px;
    cursor: default;
    min-width: 0;
    transition: border-color 0.14s ease, background 0.14s ease, box-shadow 0.14s ease, transform 0.08s ease;
  }
  .bv-card:hover {
    border-color: color-mix(in srgb, var(--acc) 50%, var(--b1));
    background: var(--surface-hover);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.18);
  }
  .bv-card:active { transform: translateY(1px); }
  .bv-card-review {
    border-color: var(--acc);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--acc) 25%, transparent);
  }
  .bv-card-top {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    min-width: 0;
  }
  .bv-priority {
    flex-shrink: 0;
    font-family: var(--mono);
    font-size: 9px;
    font-weight: 700;
    padding: 2px 6px;
    border-radius: 4px;
    letter-spacing: 0.04em;
    color: #fff;
    line-height: 1.45;
  }
  .bv-priority-p0 { background: #f85149; }
  .bv-priority-p1 { background: #d29922; }
  .bv-priority-p2 { background: #58a6ff; }
  .bv-priority-p3 { background: #5b6776; }

  .bv-card-title {
    flex: 1;
    min-width: 0;
    font-family: var(--ui);
    font-size: 12.75px;
    font-weight: 500;
    color: var(--t1);
    line-height: 1.4;
    overflow-wrap: anywhere;
    word-break: break-word;
  }
  .bv-review-badge {
    flex-shrink: 0;
    font-family: var(--ui);
    font-size: 9.5px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--acc) 18%, transparent);
    color: var(--acc);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .bv-card-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  /* Coloured label chip — fg/bg/border are set inline from
     tagColor() so each tag carries its own identity. We only define
     shape here. Capitalisation kept intentional (preserves user
     input casing) but letter-spacing tightens for the small size. */
  .bv-card-tag {
    font-family: var(--ui);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.02em;
    padding: 2px 7px;
    border-radius: 10px;
    border: 1px solid transparent;
    line-height: 1.4;
    white-space: nowrap;
  }
  .bv-card-foot {
    display: flex;
    align-items: center;
    gap: 6px;
    font-family: var(--ui);
    font-size: 10px;
    color: var(--t4);
    min-width: 0;
    flex-wrap: wrap;
  }
  .bv-card-foot > * { min-width: 0; }
  .bv-card-actor-agent {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--acc);
    font-weight: 500;
  }
  .bv-card-time { color: var(--t4); }
  /* Comment count — quiet chip with the speech-bubble glyph. Shown
     only when count > 0 so cards stay clean. */
  .bv-card-comments {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--t3);
    font-family: var(--ui);
    font-weight: 500;
  }
  .bv-card-comments:hover { color: var(--t1); }
  /* Persona chip on the card foot — avatar + name, accent-tinted so
     it reads "this is X's work" without dominating the card. */
  .bv-card-creator {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--acc);
    font-weight: 600;
  }
  .bv-card-actor-avatar {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
  }
  /* Initials fallback — mirrors the Sidebar Avatar pattern when the
     signed-in user has no avatarUrl (e.g. email-only login). Two
     uppercase letters on an accent-tinted disc. */
  .bv-card-actor-initials {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--acc);
    color: #fff;
    font-size: 7px;
    font-weight: 700;
    line-height: 1;
    font-family: var(--ui);
    flex-shrink: 0;
    letter-spacing: 0;
  }
  .bv-card-actor-anon {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--t3);
  }
  /* Source chip — pushed to the right edge of the foot. The two
     repo flavours get a faint brand-tinted background so users can
     glance and tell GitHub from GitLab; local stays muted. */
  .bv-card-source {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-family: var(--ui);
    font-size: 9.5px;
    font-weight: 600;
    letter-spacing: 0.02em;
    padding: 2px 6px;
    border-radius: 9px;
    border: 1px solid var(--b1);
    text-decoration: none;
    color: var(--t3);
    background: transparent;
    line-height: 1.4;
    white-space: nowrap;
  }
  .bv-card-source:hover { color: var(--t1); border-color: var(--b2); }
  /* PR chip — sits at the right end of the card foot, ahead of the
     issue-source chip if both exist. Accent-tinted so it reads as
     "code in flight" at a glance, distinct from the GitHub/GitLab
     issue chips which are quieter. */
  .bv-card-pr {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-family: var(--ui);
    font-size: 9.5px;
    font-weight: 600;
    letter-spacing: 0.02em;
    padding: 2px 6px;
    border-radius: 9px;
    border: 1px solid color-mix(in srgb, var(--acc) 45%, transparent);
    text-decoration: none;
    color: var(--acc);
    background: color-mix(in srgb, var(--acc) 16%, transparent);
    line-height: 1.4;
    white-space: nowrap;
    transition: background 0.12s, border-color 0.12s;
  }
  .bv-card-pr:hover {
    background: color-mix(in srgb, var(--acc) 28%, transparent);
    border-color: var(--acc);
  }
  /* When both PR + issue source chips render, PR takes the auto-margin
     and the source follows with a small gap. */
  .bv-card-pr + .bv-card-source { margin-left: 4px; }
  .bv-card-source-github {
    color: #d8dee9;
    background: var(--surface-hover);
    border-color: var(--b1);
  }
  .bv-card-source-github:hover { background: var(--surface-card); }
  .bv-card-source-gitlab {
    color: #ffb27a;
    background: rgba(252, 109, 38, 0.10);
    border-color: rgba(252, 109, 38, 0.32);
  }
  .bv-card-source-gitlab:hover { background: rgba(252, 109, 38, 0.18); }
  .bv-card-source-external { color: var(--t3); }
  .bv-card-source-local {
    color: var(--t4);
    border-style: dashed;
    cursor: help;
  }
  /* Unread = an agent edited this card since the user last opened it.
     Subtle left-edge accent + a leading dot in the title row so the
     state is visible at a glance without dominating the card. The dot
     clears when the user opens the drawer (markCardSeen). */
  .bv-card-unread {
    border-left: 2px solid var(--acc);
    padding-left: 8px; /* compensate for the +1px border vs default */
  }
  .bv-card-unread-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--acc);
    flex-shrink: 0;
    margin-top: 5px;
    box-shadow: 0 0 6px color-mix(in srgb, var(--acc) 60%, transparent);
  }

  /* In-flight = an @-mention is currently running an agent CLI for
     this card. The whole card gets a faint accent border so the user
     sees activity even when scrolled past the foot. */
  .bv-card-inflight {
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--acc) 35%, transparent),
                0 0 14px color-mix(in srgb, var(--acc) 16%, transparent);
  }
  .bv-card-inflight-chip {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-family: var(--ui);
    font-size: 9.5px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 9px;
    border: 1px solid;
    line-height: 1.4;
    white-space: nowrap;
  }
  /* Pulse the icon so the indicator feels alive even when the rest of
     the card sits still — also signals "this is something happening
     right now," not a stale flag. */
  .bv-card-inflight-pulse {
    display: inline-flex;
    animation: bv-pulse 1.4s ease-in-out infinite;
  }
  @keyframes bv-pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50%      { opacity: 0.55; transform: scale(0.9); }
  }
  /* When BOTH inflight and source chips would render, the source chip
     loses its `margin-left: auto` claim — push it back. */
  .bv-card-inflight-chip + .bv-card-source { margin-left: 4px; }

  .bv-col-add {
    padding: 8px;
    border-top: 1px solid var(--b1);
    display: flex;
    gap: 6px;
    align-items: stretch;
  }
  .bv-add-more {
    flex-shrink: 0;
    width: 30px;
    border: 1px dashed var(--b1);
    border-radius: 6px;
    background: transparent;
    color: var(--t3);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: border-color 0.12s, color 0.12s, background 0.12s;
  }
  .bv-add-more:hover {
    border-color: var(--acc);
    border-style: solid;
    color: var(--acc);
    background: var(--surface-hover);
  }
  .bv-add-input {
    flex: 1;
    min-width: 0;
    border: 1px dashed var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 11.5px;
    padding: 6px 9px;
    border-radius: 6px;
    outline: none;
    transition: border-color 0.12s, background 0.12s;
  }
  .bv-add-input:focus {
    border-color: var(--acc);
    border-style: solid;
    background: var(--surface-hover);
    color: var(--t1);
  }
  .bv-add-input::placeholder { color: var(--t4); }
</style>

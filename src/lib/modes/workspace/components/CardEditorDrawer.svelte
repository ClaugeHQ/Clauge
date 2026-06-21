<script lang="ts">
  // Card drawer (rewritten 2026-05-09 for the claim/work-stream model).
  //
  // Three things changed from the v1 drawer:
  //
  //   1. Auto-save replaces Save/Cancel.
  //      Title + description debounce 600ms after change; priority +
  //      tags save instantly. No footer buttons.
  //
  //   2. Chat goes through a single backend endpoint.
  //      `workspaceCardDrawerChat` enforces claim semantics — it
  //      creates / reuses a hidden session, posts the user comment,
  //      runs the agent, posts the reply. No more @-mention parsing,
  //      no dirty-tree gate, no "Post-only vs Post & @claude" choice.
  //
  //   3. A claim banner makes the active state legible.
  //      Three states: unclaimed (silent — just chat), drawer-owned
  //      (chip + Start work / End work-stream), manual-claimed (big
  //      banner + chat disabled + "Open in Agent" / End buttons).
  //
  // Live refresh: the drawer subscribes to `workspace:card-updated`
  // (emitted by the backend after every comment / claim / release)
  // and refetches comments + claim if the cardId matches.

  import { onMount, onDestroy } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';

  // Teleport the drawer subtree to <body> so it renders relative to the
  // viewport, not a parent stacking context (.app-workspace). Without
  // this the overlay is trapped inside the workspace container and the
  // kanban behind it stays visible/clickable through the scrim.
  function teleportToBody(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentElement === document.body) node.remove();
      },
    };
  }

  import {
    workspaceCardUpdate,
    workspaceCardCommentList,
    workspaceCardPushToRepo,
    workspaceCardRaisePr,
    workspaceCardGetClaim,
    workspaceCardDrawerChat,
    workspaceCardAddComment,
    workspaceCardFetchTicketComments,
    workspaceCardPostTicketComment,
    workspaceCardEditTicketComment,
    workspaceCardDeleteTicketComment,
    workspaceCardRelease,
    type CardClaimState,
  } from '../commands';
  import { currentUserActor, describeActor, formatAttribution } from '../attribution';
  import { cardSourceBadge } from '../cardSource';
  import {
    markCardSeen, coworkers, loadCoworkers,
    markMentionStart, markMentionEnd, inflightMentions,
  } from '../stores';
  import type { Workspace, WorkspaceBoardCard, WorkspaceCardComment, WorkspaceCoworker } from '../types';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import GhNotInstalledModal from './GhNotInstalledModal.svelte';
  import GlabNotInstalledModal from './GlabNotInstalledModal.svelte';

  // Detect the "<tool> is not installed or not on PATH" message that
  // workspace::cli_errors::CliError::NotInstalled formats. If we recognise
  // it we swap the toast for an install-guide modal — same pattern the
  // agent mode uses for missing claude/codex/gemini/opencode binaries.
  function detectMissingCli(errMsg: string): 'gh' | 'glab' | null {
    const m = errMsg.match(/^(gh|glab) is not installed/);
    return m ? (m[1] as 'gh' | 'glab') : null;
  }
  import TagInput from './TagInput.svelte';
  import CardThread from './CardThread.svelte';
  import CoworkerAvatar from './CoworkerAvatar.svelte';
  import CoworkerMentionPopover from './CoworkerMentionPopover.svelte';
  import { renderCardMarkdown, handleImageToggleClick } from '../cardMarkdown';
  import { imagePaste, insertAtCaret, pickImageFile, imageMarkdown } from '../imageAttach';
  import { tick } from 'svelte';

  interface Props {
    card: WorkspaceBoardCard;
    workspace?: Workspace | null;
    onclose?: () => void;
    onsave?: () => void;
  }

  let { card, workspace = null, onclose, onsave }: Props = $props();

  // ── Editable mirrors (auto-saved) ────────────────────────────────
  let title = $state(card.title);
  let description = $state(card.description);
  let priority = $state<string | null>(card.priority);
  let tags = $state<string[]>((() => {
    try { return JSON.parse(card.tags) as string[]; } catch { return []; }
  })());

  // ── Claim + thread state (loaded on mount, refreshed on event) ──
  let claim = $state<CardClaimState>({
    claimedSessionId: card.claimedSessionId,
    claimedCoworkerId: card.claimedCoworkerId,
    session: null,
    coworker: null,
    drawerOwns: false,
  });
  let comments = $state<WorkspaceCardComment[]>([]);
  /** Reference to the chat textarea — needed by the @-mention
   *  popover for caret-tracking + selection rewrites. */
  let chatTextareaEl = $state<HTMLTextAreaElement | null>(null);
  let mentionPopover: CoworkerMentionPopover | null = null;

  // ── UI state ────────────────────────────────────────────────────
  // Two clear sections: the real ticket (description + ticket comments)
  // and the local AI coworker chat.
  let tab = $state<'ticket' | 'coworker'>('ticket');
  // Edit-in-place description: rendered (with lazy images) by default,
  // click to edit.
  let descEditing = $state(false);
  const renderedDesc = $derived(renderCardMarkdown(description));
  // Ticket-comment composer (separate from the coworker chat composer).
  let ticketDraft = $state('');
  let ticketReplyTo = $state<WorkspaceCardComment | null>(null);
  let postingTicket = $state(false);
  let ticketSyncing = $state(false);
  let ticketSyncedAt = $state<number | null>(null);
  let chatDraft = $state('');
  let chatting = $state(false);
  /** While a mention is in-flight, hold a reference to the coworker
   *  we're sending TO so the Send button label stays "Sending to
   *  @alex…" — `taggedCoworker` re-derives from chatDraft which we
   *  clear immediately on send for an empty input. */
  let chattingTo = $state<WorkspaceCoworker | null>(null);
  let pushing = $state(false);
  let showPushConfirm = $state(false);
  let showReleaseConfirm = $state(false);
  let releaseDeleteWorktree = $state(false);
  /** Coworker-switch confirm: when the user @-mentions a different
   *  coworker than the one currently claiming, we pop a confirm so
   *  the switch isn't surprising. The pending body lives in
   *  switchPendingBody until the user confirms. */
  let showSwitchConfirm = $state(false);
  let switchPendingBody = $state('');
  let switchToCoworker = $state<WorkspaceCoworker | null>(null);
  let switchFromName = $state<string>('');

  // ── Drawer width — resizable + persisted ─────────────────────────
  // Default 720px (was 600). User-draggable left edge sets a custom
  // width that survives reload via localStorage. Min 480 (avatars +
  // input still fit), max 1100 (don't eat the entire window).
  const DRAWER_WIDTH_KEY = 'clauge.workspace.drawer.width';
  const DRAWER_MIN = 480;
  const DRAWER_MAX = 1100;
  function loadWidth(): number {
    try {
      const raw = localStorage.getItem(DRAWER_WIDTH_KEY);
      const n = raw ? parseInt(raw, 10) : NaN;
      if (Number.isFinite(n) && n >= DRAWER_MIN && n <= DRAWER_MAX) return n;
    } catch { /* ignore */ }
    return 720;
  }
  let drawerWidth = $state<number>(loadWidth());
  let resizing = $state(false);

  function startResize(e: MouseEvent) {
    e.preventDefault();
    resizing = true;
    const startX = e.clientX;
    const startW = drawerWidth;
    const onMove = (ev: MouseEvent) => {
      // Drawer hugs the right edge → drag LEFT widens it.
      const next = Math.min(DRAWER_MAX, Math.max(DRAWER_MIN, startW + (startX - ev.clientX)));
      drawerWidth = next;
    };
    const onUp = () => {
      resizing = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      try { localStorage.setItem(DRAWER_WIDTH_KEY, String(drawerWidth)); } catch { /* ignore */ }
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }

  let unlisten: UnlistenFn | null = null;
  let titleSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let descSaveTimer: ReturnType<typeof setTimeout> | null = null;
  /** Reference to the scrollable body so we can auto-scroll the
   *  thread to the bottom whenever a new bubble lands. Otherwise
   *  the user has to manually scroll down to see the agent's
   *  reply, which is a terrible chat experience. */
  let bodyEl: HTMLDivElement | null = $state(null);

  /** Keep the bottom of the conversation in view. */
  let firstScroll = true;
  $effect(() => {
    void comments.length;
    // Only auto-scroll the conversational Coworker section — on the
    // Ticket section the description sits at the top and must stay
    // visible, so we never yank it out of view.
    if (tab !== 'coworker') return;
    if (!bodyEl) return;
    const el = bodyEl;
    requestAnimationFrame(() => {
      el.scrollTo({
        top: el.scrollHeight,
        behavior: firstScroll ? 'auto' : 'smooth',
      });
      firstScroll = false;
    });
  });

  /** Re-inject the thinking bubble whenever the global inflight
   *  store has an entry for THIS card and our local comments are
   *  missing it. Solves the "close drawer mid-flight, reopen → no
   *  thinking indicator" bug: the chat is still running in the
   *  store, so we render its bubble from the store data. Cleanup
   *  is automatic — when the agent responds, sendChat (or the live
   *  event refresh) clears the store entry and the synthetic
   *  bubble is filtered out next render. */
  $effect(() => {
    if (!card) return;
    const flight = $inflightMentions.get(card.id);
    const hasThinkingLocally = comments.some((c) => c.pending === 'thinking');
    if (flight && !hasThinkingLocally) {
      // Synthesise the bubble from store data so a reopened drawer
      // shows the same indicator the original did.
      comments = [
        ...comments,
        {
          id: `thinking-rehydrated-${card.id}`,
          cardId: card.id,
          actor: flight.coworkerName,
          coworkerId: flight.coworkerId,
          body: '',
          parentId: null,
          createdAt: flight.startedAt,
          channel: 'coworker',
          externalId: null,
          externalAuthor: null,
          pending: 'thinking',
        },
      ];
    } else if (!flight && hasThinkingLocally) {
      // Store was cleared (agent responded, or chat errored from a
      // path we didn't see) — drop our local thinking bubble too.
      comments = comments.filter((c) => c.pending !== 'thinking');
    }
  });

  const editor = $derived(describeActor(card.updatedBy));
  const source = $derived(cardSourceBadge(card));

  // Comments split by drawer section. Ticket = real issue discussion
  // (local or, for linked cards, mirrored from GitHub/GitLab). Coworker
  // = the local AI persona chat.
  const ticketComments = $derived(comments.filter((c) => c.channel !== 'coworker'));
  const coworkerComments = $derived(comments.filter((c) => c.channel === 'coworker'));
  /** A card linked to a real GitHub/GitLab issue. Drives the ticket
   *  composer hint + (Phase 3) routes comments to the issue. */
  const isCloudCard = $derived(!!card.externalId);

  // ── Lifecycle ribbon ────────────────────────────────────────────
  // Compact one-line summary that orients the user: when the card
  // was created, whether it's been promoted to an Issue, whether
  // there's a worktree branch, whether a PR is open. Each segment is
  // a link when it points to a host URL so it doubles as a jump.
  function formatLifecycleDate(iso: string | null | undefined): string {
    if (!iso) return '';
    const d = new Date(iso);
    if (isNaN(d.getTime())) return '';
    const sameYear = d.getFullYear() === new Date().getFullYear();
    return d.toLocaleDateString(undefined, sameYear
      ? { month: 'short', day: 'numeric' }
      : { month: 'short', day: 'numeric', year: 'numeric' });
  }
  function extractPrNumber(url: string): string {
    const m = url.match(/(?:pull|merge_requests)\/(\d+)/);
    return m ? `#${m[1]}` : '';
  }
  type LifecycleSeg = { label: string; href?: string; title?: string };
  const lifecycleSegments = $derived.by<LifecycleSeg[]>(() => {
    const out: LifecycleSeg[] = [];
    const created = formatLifecycleDate(card.createdAt);
    if (created) out.push({ label: `Created ${created}`, title: `Created ${card.createdAt}` });
    if (card.externalId && card.externalUrl) {
      out.push({ label: `Issue ${card.externalId}`, href: card.externalUrl, title: 'Open issue on host' });
    }
    const branch = claim.session?.worktreeBranch;
    if (branch) out.push({ label: `Branch ${branch}`, title: `Worktree branch · ${branch}` });
    if (card.prUrl) {
      const num = extractPrNumber(card.prUrl);
      out.push({ label: num ? `PR ${num}` : 'PR open', href: card.prUrl, title: 'Open PR on host' });
    }
    return out;
  });

  // ── Derived: source / push state ────────────────────────────────
  const repoUrl = $derived(workspace?.repoUrl ?? null);
  const canPush = $derived(source.kind === 'local' && !!repoUrl);
  const repoLabel = $derived.by(() => {
    const u = (repoUrl ?? '').toLowerCase();
    if (u.includes('github.com')) return 'GitHub';
    if (u.includes('gitlab')) return 'GitLab';
    return 'repo';
  });

  // ── Derived: claim states ───────────────────────────────────────
  /** What the chat input + banner should look like.
   *   - 'unclaimed'      → no banner, chat enabled
   *   - 'drawer-owned'   → chip + Start work / End work-stream, chat enabled
   *   - 'manual-conflict'→ banner + chat disabled
   */
  type ClaimMode = 'unclaimed' | 'drawer-owned' | 'manual-conflict';
  const claimMode = $derived.by<ClaimMode>(() => {
    if (!claim.claimedSessionId) return 'unclaimed';
    return claim.drawerOwns ? 'drawer-owned' : 'manual-conflict';
  });

  /** Scan the chat draft for the FIRST `@<coworkername>` matching a
   *  known coworker. Resolves to a coworker object when present,
   *  null otherwise. Drives:
   *    • Send button label ("Add comment" vs "Send to @alex")
   *    • Send routing (plain comment vs drawer_chat agent run)
   *    • Hint text below the textarea
   *  Word-boundary matching so `@claude` works but `email@host.com` doesn't. */
  const taggedCoworker = $derived.by<WorkspaceCoworker | null>(() => {
    if (!chatDraft.trim()) return null;
    for (const cw of $coworkers) {
      // Escape the name for regex safety (rarely needed but cheap).
      const safe = cw.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      const re = new RegExp(`(^|[^\\w])@${safe}\\b`, 'i');
      if (re.test(chatDraft)) return cw;
    }
    return null;
  });

  /** True when the drawer's session has a worktree (created by the
   *  agent calling cards_start_work mid-chat). Drives the chip copy. */
  const drawerHasWorktree = $derived(
    claimMode === 'drawer-owned' && !!claim.session?.worktreePath,
  );

  /** Two separate gates so the textarea can stay enabled even with
   *  an empty draft (otherwise users can't type the first character). */
  /** Workspace must have a project_path bound for chat to work — the
   *  agent needs a cwd. Surfaces a clear banner instead of a deferred
   *  "send fails" error. */
  const hasProject = $derived(!!workspace?.projectPath?.trim());
  /** Can the user TYPE in the input? Plain comments work even without
   *  a project; only mention-based chat needs one. So we keep typing
   *  open and only block sending of @-tagged messages when no project. */
  const canType = $derived(claimMode !== 'manual-conflict' && !chatting);
  /** Can the user SEND? Same as canType plus "has text" + (when
   *  tagging a coworker) workspace has a project. */
  const canChat = $derived(
    canType && chatDraft.trim().length > 0 && (!taggedCoworker || hasProject),
  );

  // ── Boot + live refresh ─────────────────────────────────────────
  onMount(async () => {
    markCardSeen(card.id, card.updatedAt);
    // Coworkers are needed for the picker + bubble rendering.
    await loadCoworkers();
    await refreshClaimAndComments();
    // Land on the Ticket section — the real issue (description + its
    // comment thread). The Coworker section is the local AI chat.
    tab = 'ticket';
    // For a GitHub/GitLab-linked card, pull the real issue comments.
    if (isCloudCard) syncTicketComments();
    try {
      unlisten = await listen<{ cardId: string }>('workspace:card-updated', async (e) => {
        if (e.payload?.cardId === card.id) await refreshClaimAndComments();
      });
    } catch (e) { console.warn('card-updated listen failed:', e); }
  });

  onDestroy(() => {
    unlisten?.();
    if (titleSaveTimer) { clearTimeout(titleSaveTimer); flushTitleSave(); }
    if (descSaveTimer)  { clearTimeout(descSaveTimer);  flushDescSave();  }
  });

  async function refreshClaimAndComments() {
    try {
      const [c, cs] = await Promise.all([
        workspaceCardGetClaim(card.id),
        workspaceCardCommentList(card.id),
      ]);
      claim = c;
      // Preserve only the thinking / error bubbles — those are local
      // state the server doesn't know about. The optimistic user
      // bubble (id `pending-XXX`) is intentionally NOT preserved: the
      // server's canonical user comment is already in `cs`, so keeping
      // the optimistic too would render the user's message twice
      // (briefly, until the in-flight await reconciles by id and
      // Svelte's keyed each dedupes). Drop the optimistic; trust the
      // canonical from the server.
      const localOnly = comments.filter((local) => !!local.pending);
      comments = [...cs, ...localOnly];
    } catch (e) { console.warn('refresh failed:', e); }
  }

  // ── Auto-save plumbing ──────────────────────────────────────────
  // Title + description debounce — they fire per-keystroke.
  // Priority + tags save immediately — they're discrete picks.

  /** Resize the description textarea to fit its content. CSS handles
   *  the floor (min-height) + ceiling (max-height + scroll); this
   *  just snaps the height to scrollHeight on every input so an
   *  empty card stays compact and a long one grows up to the cap. */
  function autoGrow(el: HTMLTextAreaElement) {
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight}px`;
  }

  function scheduleTitleSave() {
    if (titleSaveTimer) clearTimeout(titleSaveTimer);
    titleSaveTimer = setTimeout(flushTitleSave, 600);
  }
  function scheduleDescSave() {
    if (descSaveTimer) clearTimeout(descSaveTimer);
    descSaveTimer = setTimeout(flushDescSave, 800);
  }
  async function flushTitleSave() {
    titleSaveTimer = null;
    await persistCard();
  }
  async function flushDescSave() {
    descSaveTimer = null;
    await persistCard();
  }
  async function persistCard() {
    // Guard against the "drawer closed mid-debounce" race — when a
    // pending timer fires after the parent has unmounted us / nulled
    // the card prop, dereferencing card.id throws. Silently bail.
    if (!card) return;
    try {
      await workspaceCardUpdate({
        id: card.id,
        title: title.trim() || 'Untitled',
        description,
        priority: priority || null,
        tags,
        reviewChecklist: card.reviewChecklist,
        actor: currentUserActor(),
      });
      onsave?.();
    } catch (e) { errorToast('Save failed', e); }
  }
  function onPriorityChange() { if (card) persistCard(); }
  function onTagsChange()     { if (card) persistCard(); }

  // ── Chat / claim actions ────────────────────────────────────────

  async function sendChat() {
    const body = chatDraft.trim();
    if (!body || !canChat) return;
    const cw = taggedCoworker; // may be null → plain comment

    if (cw && claim.coworker && claim.coworker.id !== cw.id) {
      switchPendingBody = body;
      switchToCoworker = cw;
      switchFromName = claim.coworker.name;
      showSwitchConfirm = true;
      return;
    }

    // Capture every prop / store value we'll need across the await
    // BEFORE the await suspends. The drawer can be closed mid-flight
    // (parent sets editingCard=null), in which case `card` becomes
    // null/undefined and accessing `card.id` in the finally block
    // would throw — leaking the inflight tracker (kanban "thinking"
    // chip stuck forever, null-is-not-an-object errors). With a local
    // snapshot we're immune to component teardown.
    const cardIdSnap = card.id;
    chatting = true;
    chattingTo = cw;

    const stamp = new Date().toISOString();
    const optimisticId = `pending-${Date.now()}`;
    const optimisticUser: WorkspaceCardComment = {
      id: optimisticId,
      cardId: cardIdSnap,
      actor: currentUserActor(),
      coworkerId: cw?.id ?? null,
      body,
      parentId: null,
      createdAt: stamp,
      channel: 'coworker',
      externalId: null,
      externalAuthor: null,
    };
    comments = [...comments, optimisticUser];
    chatDraft = '';
    tab = 'coworker';

    if (!cw) {
      // Plain comment path — no agent, no thinking bubble.
      try {
        const created = await workspaceCardAddComment(cardIdSnap, body, currentUserActor(), 'coworker');
        // Component may be unmounted by now; guard the local mutation.
        if (card) {
          comments = comments.map((c) => (c.id === optimisticId ? created : c));
        }
        onsave?.();
      } catch (e) {
        if (card) {
          comments = comments.filter((c) => c.id !== optimisticId);
          chatDraft = body;
        }
        errorToast('Comment failed', e);
      } finally {
        if (card) {
          chatting = false;
          chattingTo = null;
        }
      }
      return;
    }

    // Mention path. Push thinking state to the GLOBAL store so the
    // kanban-tile pulse + a reopened-drawer thinking bubble both
    // see it — this means closing the drawer mid-flight no longer
    // loses the indicator.
    markMentionStart(cardIdSnap, {
      provider: cw.provider || 'claude',
      coworkerId: cw.id,
      coworkerName: cw.name,
      startedAt: new Date().toISOString(),
    });
    const thinkingId = `thinking-${Date.now()}`;
    comments = [
      ...comments,
      {
        id: thinkingId,
        cardId: cardIdSnap,
        actor: cw.name,
        coworkerId: cw.id,
        body: '',
        parentId: null,
        createdAt: new Date().toISOString(),
        channel: 'coworker',
        externalId: null,
        externalAuthor: null,
        pending: 'thinking',
      },
    ];

    try {
      const res = await workspaceCardDrawerChat(cardIdSnap, cw.id, body, currentUserActor());
      // All mutations guarded — the drawer may have been unmounted
      // while the agent was running.
      if (card) {
        comments = comments.map((c) => (c.id === optimisticId ? res.userComment : c));
        if (res.replyComment) {
          comments = comments.map((c) => (c.id === thinkingId ? res.replyComment! : c));
        } else if (res.agentError) {
          comments = comments.map((c) => (c.id === thinkingId
            ? { ...c, body: res.agentError ?? 'Agent failed', pending: 'error' as const }
            : c));
        } else {
          comments = comments.filter((c) => c.id !== thinkingId);
        }
        try { claim = await workspaceCardGetClaim(cardIdSnap); } catch { /* ignore */ }
      }
      onsave?.();
    } catch (e) {
      if (card) {
        comments = comments.filter((c) => c.id !== optimisticId && c.id !== thinkingId);
        chatDraft = body;
      }
      errorToast('Send message', e);
    } finally {
      // Critical: use the snapshotted cardId, not card.id — `card`
      // may be null by now if the user closed the drawer. If the
      // store ever gets stuck with a stale entry, this is the cleanup
      // path that misfired.
      markMentionEnd(cardIdSnap);
      if (card) {
        chatting = false;
        chattingTo = null;
      }
    }
  }

  /** User confirmed the coworker switch — proceed with the
   *  pending send. Backend handles the actual release+claim. */
  function confirmSwitchAndSend() {
    showSwitchConfirm = false;
    if (!switchToCoworker || !switchPendingBody) return;
    // Restore the draft so sendChat picks it back up via taggedCoworker.
    // The send call will now find no pending switch (we cleared it
    // above) and proceed past the guard.
    chatDraft = switchPendingBody;
    switchPendingBody = '';
    // Pretend the claim has already moved so the guard doesn't re-trigger.
    // The backend will reconcile on the actual server roundtrip.
    claim = { ...claim, coworker: switchToCoworker, claimedCoworkerId: switchToCoworker.id };
    switchToCoworker = null;
    sendChat();
  }
  function cancelSwitch() {
    showSwitchConfirm = false;
    switchPendingBody = '';
    switchToCoworker = null;
  }

  function onChatKey(e: KeyboardEvent) {
    if (mentionPopover?.handleKey(e)) {
      e.preventDefault();
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      sendChat();
    }
  }

  function onChatInput() {
    mentionPopover?.refresh();
  }

  // ── Ticket comment composer (the real issue discussion) ──────────
  function startTicketReply(c: WorkspaceCardComment) {
    ticketReplyTo = c;
    tab = 'ticket';
  }

  /** Fetch the linked issue's comments (GitHub) and merge the canonical
   *  ticket thread in, preserving coworker chat + any pending bubbles. */
  async function syncTicketComments() {
    if (!isCloudCard || ticketSyncing) return;
    ticketSyncing = true;
    try {
      const fetched = await workspaceCardFetchTicketComments(card.id);
      if (!card) return;
      const keep = comments.filter((c) => c.channel === 'coworker' || c.pending);
      comments = [...keep, ...fetched];
      ticketSyncedAt = Date.now();
    } catch (e) {
      showToast(`Couldn't sync ${repoLabel} comments — ${friendlyError(e)}`, 'error');
    } finally {
      if (card) ticketSyncing = false;
    }
  }

  async function editTicketComment(c: WorkspaceCardComment, body: string) {
    try {
      await workspaceCardEditTicketComment(c.id, body);
      if (card) comments = comments.map((x) => (x.id === c.id ? { ...x, body } : x));
      onsave?.();
    } catch (e) { errorToast('Edit failed', e); }
  }
  async function deleteTicketComment(c: WorkspaceCardComment) {
    try {
      await workspaceCardDeleteTicketComment(c.id);
      if (card) comments = comments.filter((x) => x.id !== c.id);
      onsave?.();
    } catch (e) { errorToast('Delete failed', e); }
  }

  function syncedAgo(): string {
    if (!ticketSyncedAt) return '';
    const s = Math.max(0, Math.round((Date.now() - ticketSyncedAt) / 1000));
    if (s < 5) return 'just now';
    if (s < 60) return `${s}s ago`;
    return `${Math.round(s / 60)}m ago`;
  }

  async function addTicketComment() {
    const raw = ticketDraft.trim();
    if (!raw || postingTicket) return;
    const cardIdSnap = card.id;
    let body = raw;
    if (ticketReplyTo) {
      const who = ticketReplyTo.externalAuthor || describeActor(ticketReplyTo.actor).label;
      const quoted = ticketReplyTo.body.split('\n').map((l) => `> ${l}`).join('\n');
      body = `> **@${who}**\n${quoted}\n\n${raw}`;
    }
    postingTicket = true;
    const optimisticId = `pending-ticket-${Date.now()}`;
    const replyId = ticketReplyTo?.id ?? null;
    const optimistic: WorkspaceCardComment = {
      id: optimisticId,
      cardId: cardIdSnap,
      actor: currentUserActor(),
      coworkerId: null,
      body,
      parentId: replyId,
      createdAt: new Date().toISOString(),
      channel: 'ticket',
      externalId: null,
      externalAuthor: null,
    };
    comments = [...comments, optimistic];
    ticketDraft = '';
    ticketReplyTo = null;
    try {
      const created = isCloudCard
        ? await workspaceCardPostTicketComment(cardIdSnap, body, currentUserActor())
        : await workspaceCardAddComment(cardIdSnap, body, currentUserActor(), 'ticket', replyId);
      if (card) comments = comments.map((c) => (c.id === optimisticId ? created : c));
      onsave?.();
    } catch (e) {
      if (card) { comments = comments.filter((c) => c.id !== optimisticId); ticketDraft = raw; }
      errorToast('Comment failed', e);
    } finally {
      if (card) postingTicket = false;
    }
  }

  function onTicketKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      addTicketComment();
    }
  }

  // ── Image attachment ─────────────────────────────────────────────
  let descEditEl = $state<HTMLTextAreaElement | null>(null);
  let ticketComposerEl = $state<HTMLTextAreaElement | null>(null);
  async function addDescImage() {
    if (!descEditing) { descEditing = true; await tick(); }
    const f = await pickImageFile();
    if (!f || !descEditEl) return;
    try {
      const md = await imageMarkdown(f);
      if (md) { description = insertAtCaret(descEditEl, description, md); scheduleDescSave(); }
    } catch (e) { showToast(friendlyError(e), 'error'); }
  }
  async function addTicketImage() {
    const f = await pickImageFile();
    if (!f || !ticketComposerEl) return;
    try {
      const md = await imageMarkdown(f);
      if (md) ticketDraft = insertAtCaret(ticketComposerEl, ticketDraft, md);
    } catch (e) { showToast(friendlyError(e), 'error'); }
  }

  // ── Description edit-in-place ────────────────────────────────────
  function onDescViewClick(e: MouseEvent) {
    // Image chips reveal/collapse without entering edit mode.
    if (handleImageToggleClick(e)) return;
    descEditing = true;
  }
  function onDescBlur() {
    descEditing = false;
    flushDescSave();
  }

  async function doRelease() {
    try {
      await workspaceCardRelease(card.id, currentUserActor(), releaseDeleteWorktree);
      showToast('Work-stream released', 'success');
      claim = await workspaceCardGetClaim(card.id);
      releaseDeleteWorktree = false;
    } catch (e) { errorToast('Release failed', e); }
  }

  let showGhNotInstalled = $state(false);
  let showGlabNotInstalled = $state(false);

  async function doPush() {
    if (pushing) return;
    pushing = true;
    try {
      const r = await workspaceCardPushToRepo(card.id, currentUserActor());
      showToast(`Created issue ${r.externalId}`, 'success');
      onsave?.();
    } catch (e) {
      const msg = `${e}`;
      const missing = detectMissingCli(msg);
      if (missing === 'gh') showGhNotInstalled = true;
      else if (missing === 'glab') showGlabNotInstalled = true;
      else showToast(`Issue creation failed: ${msg}`, 'error');
    }
    finally { pushing = false; }
  }

  // ── Raise PR ───────────────────────────────────────────────────
  // Idempotent: when the card already has a `prUrl`, this just pushes
  // new commits to the existing PR's branch. UI flips the button copy
  // accordingly so the user knows what's happening before clicking.
  let raisingPr = $state(false);
  /** Visible only when the card actually has a worktree + branch —
   *  there's nothing to PR otherwise. Drawer-owned claim implies the
   *  session row has them set after a `cards_start_work` call. */
  const canRaisePr = $derived(
    !!claim.session?.worktreePath &&
    !!claim.session?.worktreeBranch &&
    !chatting,
  );
  const hasPrAlready = $derived(!!card.prUrl);

  async function doRaisePr(titleOverride?: string, bodyOverride?: string) {
    if (raisingPr || !canRaisePr) return;
    raisingPr = true;
    try {
      const r = await workspaceCardRaisePr(
        card.id,
        currentUserActor(),
        titleOverride?.trim() || undefined,
        bodyOverride?.trim() || undefined,
      );
      showToast(
        r.alreadyExisted
          ? `Pushed update to PR on ${r.branch}`
          : `PR opened on ${r.branch}`,
        'success',
      );
      onsave?.();
    } catch (e) {
      const msg = `${e}`;
      const missing = detectMissingCli(msg);
      if (missing === 'gh') showGhNotInstalled = true;
      else if (missing === 'glab') showGlabNotInstalled = true;
      else showToast(msg, 'error');
    } finally {
      raisingPr = false;
    }
  }

  // ── PR preview dialog ──────────────────────────────────────────
  // Opens a small modal letting the user review/edit the PR title +
  // body before the actual `gh pr create` shells out. Only shown for
  // FRESH PRs — for an existing PR (`hasPrAlready`) `raise_or_update_pr`
  // ignores title/body anyway (it just pushes new commits), so the
  // preview would be pointless friction.
  let showPrPreview = $state(false);
  let prTitleDraft = $state('');
  let prBodyDraft = $state('');

  function openPrPreviewOrPush() {
    if (!canRaisePr) return;
    if (hasPrAlready) {
      doRaisePr();
      return;
    }
    const branch = claim.session?.worktreeBranch ?? '';
    prTitleDraft = card.title;
    prBodyDraft = `Card branch \`${branch}\` — see card thread for context.`;
    showPrPreview = true;
  }

  function confirmPrFromPreview() {
    const t = prTitleDraft;
    const b = prBodyDraft;
    showPrPreview = false;
    doRaisePr(t, b);
  }

  function handleKey(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); onclose?.(); }
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && document.activeElement?.tagName === 'TEXTAREA') {
      // ⌘↵ in chat input → send
      const target = e.target as HTMLTextAreaElement;
      if (target.classList.contains('cd-chat-input')) {
        e.preventDefault();
        sendChat();
      }
    }
  }
</script>

<svelte:window onkeydown={handleKey} />

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="cd-overlay" use:teleportToBody onclick={onclose}>
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="cd-drawer" class:cd-drawer-resizing={resizing} style="width: {drawerWidth}px;" onclick={(e) => e.stopPropagation()}>
    <!-- Drag-handle hugs the left edge — wider hit zone (8px) so you
         don't need millimetre precision to grab it. Cursor switches
         to ew-resize on hover. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="cd-resize" onmousedown={startResize} title="Drag to resize"></div>
    <!-- ─────────── Header ─────────── -->
    <div class="cd-head">
      <input
        class="cd-title"
        bind:value={title}
        oninput={scheduleTitleSave}
        onblur={flushTitleSave}
        placeholder="Untitled"
        spellcheck="false"
      />
      <button class="cd-close" onclick={onclose} title="Close">×</button>
    </div>

    <!-- ─────────── Meta row ─────────── -->
    <div class="cd-meta">
      <label class="cd-meta-cell">
        <span class="cd-meta-key">Priority</span>
        <select class="cd-input" bind:value={priority} onchange={onPriorityChange}>
          <option value={null}>—</option>
          <option value="P0">P0</option>
          <option value="P1">P1</option>
          <option value="P2">P2</option>
          <option value="P3">P3</option>
        </select>
      </label>
      <div class="cd-meta-cell cd-meta-cell-grow">
        <span class="cd-meta-key">Tags</span>
        <TagInput bind:value={tags} onchange={onTagsChange} />
      </div>
    </div>

    <!-- ─────────── Lifecycle ribbon ─────────── -->
    {#if lifecycleSegments.length > 0}
      <div class="cd-lifecycle" role="status" aria-label="Card lifecycle">
        {#each lifecycleSegments as seg, i (seg.label)}
          {#if i > 0}<span class="cd-lifecycle-sep" aria-hidden="true">·</span>{/if}
          {#if seg.href}
            <a
              class="cd-lifecycle-seg cd-lifecycle-link"
              href={seg.href}
              target="_blank"
              rel="noreferrer noopener"
              title={seg.title}
            >{seg.label}</a>
          {:else}
            <span class="cd-lifecycle-seg" title={seg.title}>{seg.label}</span>
          {/if}
        {/each}
      </div>
    {/if}

    <!-- ─────────── Body (Ticket / Coworker) ─────────── -->
    <div class="cd-tabs">
      <button class="cd-tab" class:cd-tab-on={tab === 'ticket'} onclick={() => (tab = 'ticket')}>
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v3a2 2 0 0 0 0 4v3a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-3a2 2 0 0 0 0-4z"/><line x1="12" y1="7" x2="12" y2="17" stroke-dasharray="2 3"/></svg>
        Ticket
        {#if ticketComments.length > 0}<span class="cd-tab-count">{ticketComments.length}</span>{/if}
      </button>
      <button class="cd-tab" class:cd-tab-on={tab === 'coworker'} onclick={() => (tab = 'coworker')}>
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
        Coworker
        {#if coworkerComments.length > 0}<span class="cd-tab-count">{coworkerComments.length}</span>{/if}
      </button>
    </div>

    <div class="cd-body" bind:this={bodyEl}>
      {#if tab === 'ticket'}
        <!-- Editable description — rendered with lazy images, click to edit. -->
        <div class="cd-desc-section">
          <div class="cd-comments-head">
            <span class="cd-section-key">Description</span>
            <button class="cd-img-btn" onclick={addDescImage} title="Attach image (or paste a screenshot)">
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="M21 15l-5-5L5 21"/></svg>
              Image
            </button>
          </div>
          {#if descEditing}
            <!-- svelte-ignore a11y_autofocus -->
            <textarea
              class="cd-desc"
              bind:value={description}
              bind:this={descEditEl}
              use:imagePaste={(md) => { if (descEditEl) { description = insertAtCaret(descEditEl, description, md); scheduleDescSave(); } }}
              autofocus
              oninput={(e) => { autoGrow(e.currentTarget); scheduleDescSave(); }}
              onfocus={(e) => autoGrow(e.currentTarget)}
              onblur={onDescBlur}
              placeholder="Describe this ticket. Markdown supported. Auto-saves as you type."
            ></textarea>
          {:else if renderedDesc}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <div class="cd-desc-view" onclick={onDescViewClick} title="Click to edit">
              <!-- eslint-disable-next-line svelte/no-at-html-tags -->
              {@html renderedDesc}
            </div>
          {:else}
            <button class="cd-desc-empty" onclick={() => (descEditing = true)}>+ Add a description</button>
          {/if}
        </div>

        {#if card.reviewChecklist}
          <div class="cd-checklist">
            <div class="cd-checklist-key">Review checklist <span class="cd-dim">(set by {editor.label})</span></div>
            <pre>{card.reviewChecklist}</pre>
          </div>
        {/if}

        <div class="cd-comments-head">
          <span class="cd-section-key">Comments</span>
          {#if isCloudCard}
            <span class="cd-sync-state">
              {#if ticketSyncing}Syncing…{:else if ticketSyncedAt}Synced {syncedAgo()}{/if}
            </span>
            <button class="cd-sync-btn" onclick={syncTicketComments} disabled={ticketSyncing} title="Refresh comments from {repoLabel}">
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
            </button>
          {/if}
        </div>
        <CardThread comments={ticketComments} emptyText="No comments yet — start the discussion below." onreply={startTicketReply} onedit={editTicketComment} ondelete={deleteTicketComment} />
      {:else}
        <CardThread comments={coworkerComments} emptyText="No messages yet — @-mention a coworker below to start a chat." />
      {/if}
    </div>

    <!-- ─────────── Composer (Ticket comment / Coworker chat) ─────────── -->
    <div class="cd-chat">
      {#if tab === 'ticket'}
        {#if ticketReplyTo}
          <div class="cd-reply-chip">
            <span class="cd-reply-ico">↳</span>
            <span class="cd-reply-text">Replying to <strong>@{ticketReplyTo.externalAuthor || describeActor(ticketReplyTo.actor).label}</strong></span>
            <button class="cd-reply-x" onclick={() => (ticketReplyTo = null)} title="Cancel reply">×</button>
          </div>
        {/if}
        <div class="cd-chat-row">
          <textarea
            class="cd-chat-input"
            bind:value={ticketDraft}
            bind:this={ticketComposerEl}
            use:imagePaste={(md) => { if (ticketComposerEl) ticketDraft = insertAtCaret(ticketComposerEl, ticketDraft, md); }}
            placeholder={isCloudCard ? `Comment on ${repoLabel} issue ${card.externalId}…` : 'Add a comment to this ticket…'}
            onkeydown={onTicketKey}
            rows="3"
          ></textarea>
          <div class="cd-chat-foot">
            <span class="cd-chat-hint">
              {isCloudCard ? `Posts to the ${repoLabel} issue` : 'Ticket comment'} · <kbd>⌘↵</kbd> to send
            </span>
            <span class="cd-chat-foot-spacer"></span>
            <button class="cd-img-btn cd-img-btn-inline" onclick={addTicketImage} title="Attach image (or paste a screenshot)">
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="M21 15l-5-5L5 21"/></svg>
            </button>
            <button class="cd-chat-send" onclick={addTicketComment} disabled={!ticketDraft.trim() || postingTicket}>
              {postingTicket ? 'Posting…' : ticketReplyTo ? 'Post reply' : 'Comment'}
            </button>
          </div>
        </div>
      {:else}
      {#if claimMode === 'manual-conflict'}
        <div class="cd-claim cd-claim-conflict">
          <span class="cd-claim-ico" style="color: var(--err, #f87171);">⚠</span>
          <div class="cd-claim-body">
            <div class="cd-claim-title">
              Active in terminal session <strong>{claim.session?.title ?? 'unknown'}</strong>
            </div>
            <div class="cd-claim-sub">
              You can read + leave plain comments here, but to chat with a coworker you need to switch to that session — or End the work-stream below.
            </div>
          </div>
          <button class="cd-claim-btn cd-claim-btn-warn" onclick={() => (showReleaseConfirm = true)}>
            End
          </button>
        </div>
      {/if}

      {#if taggedCoworker && !hasProject}
        <!-- @-mentioning a coworker without a workspace project_path
             would fail server-side ("workspace has no project bound").
             Surface the precondition up front so the user knows what
             to fix instead of seeing a cryptic error after Send. -->
        <div class="cd-chat-warn">
          <span class="cd-chat-warn-ico">⚠</span>
          <div>
            Bind a project to this workspace before tagging a coworker —
            agents need a working directory. Plain comments still work without one.
          </div>
        </div>
      {/if}

      <!-- Unified chat input. Plain comments work without a coworker;
           typing `@` opens the inline picker; tagging a coworker
           routes to the agent. -->
      <div class="cd-chat-row">
        <textarea
          bind:this={chatTextareaEl}
          class="cd-chat-input"
          bind:value={chatDraft}
          placeholder={claimMode === 'manual-conflict'
            ? 'Chat is locked — End the work-stream to comment or chat here.'
            : 'Add a note, or type @ to mention a coworker and trigger them…'}
          disabled={!canType}
          oninput={onChatInput}
          onkeydown={onChatKey}
          rows="3"
        ></textarea>
        <CoworkerMentionPopover
          bind:this={mentionPopover}
          bind:text={chatDraft}
          textareaEl={chatTextareaEl}
        />
        <div class="cd-chat-foot">
          <!-- Static hint on the left. Spells out the contract: a
               plain comment is just a note; the coworker only acts
               when explicitly @-tagged. Removes ambiguity about why
               nothing happened after a non-tagged message. -->
          <span class="cd-chat-hint">
            Coworkers respond only when <strong>@-tagged</strong> · <kbd>⌘↵</kbd> to send
          </span>
          <span class="cd-chat-foot-spacer"></span>
          <!-- Right cluster: active-coworker badge sits immediately
               left of the Send button so the user reads
               "[live indicator] [send to that person]" as one unit. -->
          {#if claimMode === 'drawer-owned' && claim.coworker}
            {@const cw = claim.coworker}
            <span class="cd-active-badge" title={drawerHasWorktree ? `Working on ${claim.session?.worktreeBranch}` : `${cw.name} is the active coworker`}>
              <span class="cd-active-dot" aria-hidden="true"></span>
              <CoworkerAvatar seed={cw.avatarSeed} style={cw.avatarStyle} size={14} />
              <span class="cd-active-name">@{cw.name}</span>
            </span>
          {/if}
          <button
            class="cd-chat-send"
            class:cd-chat-send-mention={!!taggedCoworker}
            onclick={sendChat}
            disabled={!canChat}
          >
            {#if chatting}
              {chattingTo ? `Sending to @${chattingTo.name}…` : 'Posting…'}
            {:else if taggedCoworker}
              Send to @{taggedCoworker.name}
            {:else}
              Add comment
            {/if}
          </button>
        </div>
      </div>
      {/if}
    </div>

    <!-- ─────────── Footer (push + meta) ─────────── -->
    <div class="cd-foot">
      {#if source.kind === 'local'}
        <button
          class="cd-foot-btn"
          onclick={() => (showPushConfirm = true)}
          disabled={!canPush || pushing}
          title={canPush ? `Create a real ${repoLabel} issue from this card` : 'Set the workspace repo URL first'}
        >
          {pushing ? 'Creating…' : `Create issue on ${repoLabel}`}
        </button>
      {:else if source.url}
        <a class="cd-foot-link" href={source.url} target="_blank" rel="noreferrer noopener">
          {source.label} ↗
        </a>
      {/if}
      <!-- Raise / update PR. Only renders when the card has an active
           worktree+branch (otherwise there's no branch to PR). Button
           label flips between three states:
             • no PR yet   → "Raise PR"
             • has PR      → "Push update to PR" + a separate "View PR ↗" link
             • busy        → spinner copy -->
      {#if canRaisePr || hasPrAlready}
        {#if hasPrAlready && card.prUrl}
          <a class="cd-foot-link" href={card.prUrl} target="_blank" rel="noreferrer noopener" title="Open PR on the host">
            View PR ↗
          </a>
        {/if}
        {#if canRaisePr}
          <button
            class="cd-foot-btn"
            onclick={openPrPreviewOrPush}
            disabled={raisingPr}
            title={hasPrAlready
              ? 'Push new commits to the existing PR (no new PR is opened)'
              : 'Review and open a PR — pushes the branch and opens it on the host'}
          >
            {#if raisingPr}
              {hasPrAlready ? 'Pushing…' : 'Opening…'}
            {:else if hasPrAlready}
              Push update to PR
            {:else}
              Open PR…
            {/if}
          </button>
        {/if}
      {/if}
      <span class="cd-foot-spacer"></span>
      <span class="cd-foot-meta">
        Updated {formatAttribution(card.updatedBy, card.updatedAt)}
      </span>
    </div>
  </div>
</div>

<ConfirmDialog
  bind:show={showPushConfirm}
  title="Create issue on {repoLabel}?"
  message={`Creates a new issue on ${repoLabel} with this card's title and description. The issue will be public if the repo is public.`}
  confirmText={`Create on ${repoLabel}`}
  confirmColor="var(--acc)"
  onconfirm={doPush}
/>

<ConfirmDialog
  bind:show={showSwitchConfirm}
  title="Switch coworker?"
  message={switchToCoworker
    ? `This card is currently with @${switchFromName}. Sending to @${switchToCoworker.name} will hand the card off — @${switchFromName}'s thread stays in the conversation, but @${switchToCoworker.name} becomes the active coworker.`
    : ''}
  confirmText={switchToCoworker ? `Switch to @${switchToCoworker.name}` : 'Switch'}
  confirmColor="var(--acc)"
  onconfirm={confirmSwitchAndSend}
  oncancel={cancelSwitch}
/>

<ConfirmDialog
  bind:show={showReleaseConfirm}
  title="End the work-stream?"
  message={drawerHasWorktree
    ? 'The card will be unclaimed. You can choose to keep the worktree (the branch stays around for reference) or delete it now.'
    : 'The card will be unclaimed. Anyone can start a new chat on it after this.'}
  confirmText={releaseDeleteWorktree ? 'End & delete worktree' : 'End work-stream'}
  confirmColor="var(--err, #f87171)"
  onconfirm={doRelease}
  discardText={drawerHasWorktree ? (releaseDeleteWorktree ? 'Keep worktree' : 'Delete worktree') : undefined}
  ondiscard={drawerHasWorktree ? () => { releaseDeleteWorktree = !releaseDeleteWorktree; showReleaseConfirm = true; } : undefined}
/>

<GhNotInstalledModal bind:show={showGhNotInstalled} />
<GlabNotInstalledModal bind:show={showGlabNotInstalled} />

<Modal bind:show={showPrPreview} title="Open PR" width="520px">
  <div class="cd-pr-preview">
    <div class="cd-pr-field">
      <span class="cd-pr-label">Branch</span>
      <code class="cd-pr-branch">{claim.session?.worktreeBranch ?? ''}</code>
    </div>
    <div class="cd-pr-field">
      <span class="cd-pr-label">Title</span>
      <input
        class="cd-pr-input"
        type="text"
        bind:value={prTitleDraft}
        placeholder="PR title"
        spellcheck="false"
      />
    </div>
    <div class="cd-pr-field">
      <span class="cd-pr-label">Body</span>
      <textarea
        class="cd-pr-textarea"
        bind:value={prBodyDraft}
        rows="6"
        placeholder="PR description (markdown supported)"
        spellcheck="false"
      ></textarea>
    </div>
    <div class="cd-pr-actions">
      <button class="cd-pr-btn" type="button" onclick={() => (showPrPreview = false)}>Cancel</button>
      <button
        class="cd-pr-btn primary"
        type="button"
        onclick={confirmPrFromPreview}
        disabled={!prTitleDraft.trim() || raisingPr}
      >
        {raisingPr ? 'Opening…' : 'Open PR'}
      </button>
    </div>
  </div>
</Modal>

<style>
  .cd-overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    z-index: var(--z-drawer);
    display: flex;
    justify-content: flex-end;
    animation: fadeIn 0.15s ease;
  }
  @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }

  .cd-drawer {
    /* width is set inline (resizable) — see drawerWidth state */
    max-width: 100%;
    height: 100%;
    /* Use --modal-bg (always opaque enough to contain text). On the
       glass theme the parent overlay already provides the scrim; the
       drawer itself sits as a real card on top, not a translucent
       pane. */
    background: var(--modal-bg, #0d1117);
    border-left: 1px solid var(--b1);
    box-shadow: -10px 0 30px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
    animation: slideIn 0.18s ease;
    min-width: 0;
    position: relative; /* for the absolute resize handle */
  }
  :global(body.glass-mode) .cd-drawer {
    backdrop-filter: blur(30px) saturate(180%);
    -webkit-backdrop-filter: blur(30px) saturate(180%);
  }
  .cd-drawer-resizing {
    /* Disable the slide-in animation + child transitions while
       dragging so the resize feels instant. */
    animation: none;
    transition: none;
    user-select: none;
  }
  /* 8px hit zone on the left edge — invisible by default, accent
     line on hover. Cursor change tells the user it's draggable. */
  .cd-resize {
    position: absolute;
    top: 0; left: 0; bottom: 0;
    width: 8px;
    cursor: ew-resize;
    z-index: 10;
    transition: background 0.15s;
  }
  .cd-resize:hover {
    background: linear-gradient(to right, var(--acc) 0, var(--acc) 2px, transparent 2px);
  }
  .cd-drawer-resizing .cd-resize {
    background: linear-gradient(to right, var(--acc) 0, var(--acc) 2px, transparent 2px);
  }
  @keyframes slideIn { from { transform: translateX(20px); opacity: 0.6; } to { transform: none; opacity: 1; } }

  /* ── Header ───────────────────────────── */
  .cd-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px 8px;
    border-bottom: 1px solid var(--b1);
    flex-shrink: 0;
  }
  .cd-title {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 16px;
    font-weight: 600;
    padding: 4px 6px;
    border-radius: 5px;
    outline: none;
    transition: background 0.12s;
  }
  .cd-title:hover { background: var(--surface-hover); }
  .cd-title:focus { background: var(--surface-hover); }
  .cd-close {
    width: 26px; height: 26px;
    border: none; background: transparent;
    color: var(--t3); font-size: 18px; line-height: 1;
    cursor: default; border-radius: 5px;
  }
  .cd-close:hover { background: var(--surface-hover); color: var(--t1); }

  /* ── Meta row ───────────────────────────── */
  .cd-meta {
    display: flex;
    gap: 12px;
    padding: 8px 14px 10px;
    border-bottom: 1px solid var(--b1);
    flex-shrink: 0;
  }
  .cd-meta-cell { display: flex; flex-direction: column; gap: 4px; }
  .cd-meta-cell-grow { flex: 1; min-width: 0; }
  .cd-meta-key {
    font-family: var(--ui);
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: var(--t4);
    text-transform: uppercase;
  }
  .cd-input {
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 5px 8px;
    color: var(--t1);
    font-family: var(--mono);
    font-size: 11.5px;
    outline: none;
    min-width: 70px;
  }
  .cd-input:focus { border-color: var(--acc); }
  /* Override the native <select> chrome for the priority dropdown. Without
     this, macOS Light mode renders the 3D silver bevel + native up/down
     arrows, which screams against the dark drawer. Same SVG chevron + style
     pattern as `.stg-select` in SettingsModal. Scoped to `select.cd-input`
     so plain text/number inputs that share the class are unaffected. */
  select.cd-input {
    -webkit-appearance: none;
    appearance: none;
    padding-right: 24px;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23b0b0c8' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'><polyline points='3 5 6 8 9 5'/></svg>");
    background-repeat: no-repeat;
    background-position: right 7px center;
    background-size: 10px 10px;
  }
  .cd-input option { background: var(--n); color: var(--t1); }

  /* ── Tabs + body ───────────────────────────── */
  .cd-tabs {
    display: flex;
    gap: 2px;
    padding: 0 14px;
    border-bottom: 1px solid var(--b1);
    flex-shrink: 0;
  }
  .cd-tab {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 11.5px;
    font-weight: 600;
    padding: 8px 10px;
    cursor: default;
    margin-bottom: -1px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    transition: color 0.12s, border-color 0.12s;
  }
  .cd-tab:hover { color: var(--t1); }
  .cd-tab-on { color: var(--t1); border-bottom-color: var(--acc); }
  .cd-tab-count {
    font-family: var(--mono);
    font-size: 10px;
    background: var(--surface-card);
    color: var(--t2);
    padding: 1px 6px;
    border-radius: 8px;
    line-height: 1.4;
  }
  .cd-tab-on .cd-tab-count {
    background: color-mix(in srgb, var(--acc) 18%, transparent);
    color: var(--acc);
  }

  .cd-body {
    flex: 1;
    overflow-y: auto;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
  }
  .cd-desc {
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 10px 12px;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 12.5px;
    line-height: 1.6;
    outline: none;
    /* Auto-grow with content (driven by inline style.height in the
     * onInput handler). Floor at ~6 lines so empty/short descriptions
     * still read as a real input area, not a vestigial slit; ceiling
     * so a long doc doesn't push the chat input off-screen — overflow
     * scrolls inside the textarea past the cap. */
    min-height: 140px;
    max-height: 360px;
    resize: none;
    overflow-y: auto;
    transition: border-color 0.12s;
    box-sizing: border-box;
    width: 100%;
  }
  .cd-desc:focus { border-color: var(--acc); }

  /* ── Ticket section: description + comments ─────────────── */
  .cd-desc-section { display: flex; flex-direction: column; gap: 6px; }
  .cd-section-key {
    font-family: var(--ui);
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: var(--t4);
    text-transform: uppercase;
  }
  .cd-comments-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .cd-sync-state { font-family: var(--ui); font-size: 10px; color: var(--t4); }
  .cd-sync-btn {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px; height: 20px;
    border: 1px solid var(--b1);
    border-radius: 5px;
    background: transparent;
    color: var(--t3);
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s;
  }
  .cd-sync-btn:hover:not(:disabled) { color: var(--t1); border-color: var(--acc); }
  .cd-sync-btn:disabled { opacity: 0.5; }
  .cd-img-btn {
    margin-left: auto;
    display: inline-flex; align-items: center; gap: 4px;
    border: 1px solid var(--b1); background: transparent; color: var(--t3);
    font-family: var(--ui); font-size: 10px; padding: 2px 7px; border-radius: 5px; cursor: pointer;
    transition: color 0.12s, border-color 0.12s;
  }
  .cd-img-btn:hover { color: var(--t1); border-color: var(--acc); }
  .cd-img-btn-inline { margin-left: 0; padding: 4px 7px; }
  .cd-desc-empty {
    text-align: left;
    border: 1px dashed var(--b1);
    border-radius: 6px;
    padding: 10px 12px;
    background: transparent;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 12px;
    cursor: pointer;
    transition: border-color 0.12s, color 0.12s;
  }
  .cd-desc-empty:hover { border-color: var(--acc); color: var(--t1); }
  .cd-desc-view {
    font-family: var(--ui);
    font-size: 12.5px;
    color: var(--t1);
    line-height: 1.65;
    padding: 10px 12px;
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 6px;
    overflow-wrap: anywhere;
    word-break: break-word;
    box-sizing: border-box;
    max-width: 100%;
    cursor: text;
    transition: border-color 0.12s;
  }
  .cd-desc-view:hover { border-color: var(--b2); }
  .cd-desc-view :global(pre), .cd-desc-view :global(img) { max-width: 100%; }
  .cd-desc-view :global(p) { margin: 0 0 8px; }
  .cd-desc-view :global(p:last-child) { margin-bottom: 0; }
  .cd-desc-view :global(code) {
    font-family: var(--mono); font-size: 11.5px;
    background: rgba(0,0,0,0.22); padding: 1px 4px; border-radius: 3px;
  }
  .cd-desc-view :global(pre) {
    background: rgba(0,0,0,0.25); border: 1px solid var(--b1);
    border-radius: 5px; padding: 8px 10px; overflow-x: auto; font-size: 11px;
  }
  .cd-desc-view :global(a) { color: var(--acc); text-decoration: none; }
  .cd-desc-view :global(a:hover) { text-decoration: underline; }
  .cd-desc-view :global(ul), .cd-desc-view :global(ol) { margin: 6px 0; padding-left: 22px; }
  .cd-desc-view :global(li) { margin: 2px 0; }
  .cd-desc-view :global(blockquote) { border-left: 2px solid var(--b2); padding-left: 10px; margin: 6px 0; color: var(--t3); }
  .cd-desc-view :global(h1), .cd-desc-view :global(h2), .cd-desc-view :global(h3) { margin: 8px 0 4px; font-weight: 600; color: var(--t1); }
  .cd-desc-view :global(h1) { font-size: 14px; }
  .cd-desc-view :global(h2) { font-size: 13px; }
  .cd-desc-view :global(h3) { font-size: 12.5px; }
  .cd-desc-view :global(.th-img-toggle) {
    display: inline-flex; align-items: center; gap: 4px;
    padding: 4px 8px; margin: 4px 0;
    border: 1px dashed var(--b2); border-radius: 5px;
    background: var(--surface-hover); color: var(--t2);
    font-family: var(--ui); font-size: 11px; cursor: pointer;
  }
  .cd-desc-view :global(.th-img-toggle:hover) { border-color: var(--acc); color: var(--t1); }
  .cd-desc-view :global(.th-img-hint) { color: var(--t4); font-size: 10px; }
  .cd-desc-view :global(.th-img-revealed) { display: block; position: relative; margin: 6px 0; }
  .cd-desc-view :global(.th-img-revealed img) { max-width: 100%; border-radius: 6px; cursor: pointer; display: block; }
  .cd-desc-view :global(.th-img-collapse) {
    position: absolute; top: 4px; right: 4px;
    background: rgba(0,0,0,0.6); color: #fff;
    font-family: var(--ui); font-size: 10px;
    padding: 2px 6px; border-radius: 3px; pointer-events: none;
    opacity: 0; transition: opacity 0.15s;
  }
  .cd-desc-view :global(.th-img-revealed:hover .th-img-collapse) { opacity: 1; }

  /* Reply chip above the ticket composer. */
  .cd-reply-chip {
    display: flex; align-items: center; gap: 6px;
    padding: 4px 8px; border-radius: 5px;
    background: color-mix(in srgb, var(--acc) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--acc) 24%, transparent);
    font-family: var(--ui); font-size: 11px; color: var(--t2);
  }
  .cd-reply-ico { color: var(--acc); }
  .cd-reply-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cd-reply-text strong { color: var(--acc); font-weight: 600; }
  .cd-reply-x { border: none; background: transparent; color: var(--t3); font-size: 14px; cursor: pointer; padding: 0 2px; }
  .cd-reply-x:hover { color: var(--err, #f87171); }

  .cd-checklist {
    border: 1px solid color-mix(in srgb, #a78bfa 30%, transparent);
    background: rgba(167, 139, 250, 0.08);
    border-radius: 6px;
    padding: 10px 12px;
  }
  .cd-checklist-key {
    font-family: var(--ui);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: var(--t4);
    text-transform: uppercase;
    margin-bottom: 6px;
  }
  .cd-checklist pre { margin: 0; white-space: pre-wrap; color: var(--t1); font-family: var(--mono); font-size: 11.5px; line-height: 1.6; }
  .cd-dim { color: var(--t4); text-transform: none; letter-spacing: 0; font-weight: 500; }

  /* ── Claim banner + chat input ───────────────────────────── */
  .cd-chat {
    border-top: 1px solid var(--b1);
    padding: 10px 14px 12px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .cd-claim {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 6px;
    border: 1px solid;
  }
  /* Slim active-coworker chip — single 24px line just above the
     chat input. Avatar + name · role + an End link. Way less visual
     weight than the two-line banner; the bubbles in the thread
     already tell the user who's chatting. */
  .cd-claim-slim {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--acc) 7%, transparent);
    border: 1px solid color-mix(in srgb, var(--acc) 22%, transparent);
    font-family: var(--ui);
    font-size: 11px;
    color: var(--t2);
    line-height: 1.4;
  }
  .cd-claim-slim-text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cd-claim-slim-text strong { color: var(--acc); font-weight: 600; }
  .cd-claim-slim-text code {
    font-family: var(--mono);
    background: var(--surface-hover);
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 10px;
  }
  .cd-claim-slim-end {
    border: none;
    background: transparent;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 10.5px;
    padding: 0 6px;
    cursor: pointer;
  }
  .cd-claim-slim-end:hover { color: var(--err, #f87171); }
  .cd-claim-mine {
    border-color: color-mix(in srgb, var(--acc) 32%, transparent);
    background: color-mix(in srgb, var(--acc) 7%, transparent);
  }
  .cd-claim-conflict {
    border-color: color-mix(in srgb, var(--err, #f87171) 35%, transparent);
    background: color-mix(in srgb, var(--err, #f87171) 8%, transparent);
  }
  .cd-claim-ico {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    font-size: 14px;
  }
  .cd-claim-body { flex: 1; min-width: 0; }
  .cd-claim-title {
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 600;
    color: var(--t1);
  }
  .cd-claim-title code {
    font-family: var(--mono);
    background: var(--surface-card);
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 11px;
  }
  .cd-claim-sub {
    font-family: var(--ui);
    font-size: 10.5px;
    color: var(--t3);
    margin-top: 2px;
    line-height: 1.4;
  }
  .cd-claim-btn {
    height: 26px;
    padding: 0 10px;
    border-radius: 5px;
    font-family: var(--ui);
    font-size: 11px;
    font-weight: 600;
    cursor: default;
    border: 1px solid var(--b2);
    background: transparent;
    color: var(--t2);
    transition: opacity 0.12s, border-color 0.12s, color 0.12s;
  }
  .cd-claim-btn:hover:not(:disabled) { color: var(--t1); border-color: var(--acc); }
  .cd-claim-btn:disabled { opacity: 0.45; }
  .cd-claim-btn-primary {
    border: none;
    background: var(--acc);
    color: #fff;
  }
  .cd-claim-btn-primary:hover:not(:disabled) { opacity: 0.9; color: #fff; }
  .cd-claim-btn-warn:hover { color: var(--err, #f87171); border-color: var(--err, #f87171); }

  .cd-chat-row {
    display: flex; flex-direction: column; gap: 6px;
    position: relative; /* anchor for the @-mention popover */
  }
  .cd-chat-warn {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 7px 10px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--err, #f87171) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--err, #f87171) 30%, transparent);
    font-family: var(--ui);
    font-size: 11px;
    color: var(--t1);
    line-height: 1.45;
  }
  .cd-chat-warn-ico { color: var(--err, #f87171); font-size: 13px; line-height: 1; flex-shrink: 0; }
  .cd-hint-active {
    color: var(--acc);
    font-weight: 500;
  }
  .cd-hint-active strong { color: var(--acc); }
  /* When @ is detected, the Send button morphs into accent so the
     routing change is visible. */
  .cd-chat-send-mention {
    /* already accent — but raise the prominence with a subtle glow */
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--acc) 22%, transparent);
  }
  /* Compact "active coworker" badge in the chat-foot row. Lives
     between the hint (left) and the Send button (right). Pulsing
     green dot signals "live" — the claim is held + ready. */
  .cd-active-badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 8px 3px 6px;
    border-radius: 11px;
    background: color-mix(in srgb, var(--acc) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--acc) 26%, transparent);
    font-family: var(--ui);
    font-size: 10.5px;
    color: var(--t2);
    line-height: 1.4;
    white-space: nowrap;
  }
  .cd-active-name { font-weight: 600; color: var(--t1); }
  .cd-active-dot {
    width: 7px; height: 7px;
    border-radius: 50%;
    background: #2ee08a;
    box-shadow: 0 0 6px rgba(46, 224, 138, 0.6);
    animation: cd-active-pulse 1.6s ease-in-out infinite;
    flex-shrink: 0;
  }
  @keyframes cd-active-pulse {
    0%, 100% { opacity: 1;   transform: scale(1); }
    50%      { opacity: 0.4; transform: scale(0.78); }
  }

  .cd-chat-hint kbd {
    font-family: var(--mono);
    background: var(--surface-card);
    padding: 0 4px;
    border-radius: 3px;
    font-size: 9.5px;
  }
  .cd-chat-input {
    width: 100%;
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 8px 10px;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 12px;
    line-height: 1.5;
    outline: none;
    resize: vertical;
    transition: border-color 0.12s, opacity 0.12s;
    box-sizing: border-box;
  }
  .cd-chat-input:focus { border-color: var(--acc); }
  .cd-chat-input:disabled { opacity: 0.5; cursor: not-allowed; }
  .cd-chat-foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .cd-chat-foot-spacer { flex: 1; }
  .cd-chat-hint { font-family: var(--ui); font-size: 10.5px; color: var(--t4); }
  .cd-chat-send {
    height: 28px;
    padding: 0 14px;
    border-radius: 5px;
    border: none;
    background: var(--acc);
    color: #fff;
    font-family: var(--ui);
    font-size: 11.5px;
    font-weight: 600;
    cursor: default;
    transition: opacity 0.12s;
  }
  .cd-chat-send:hover:not(:disabled) { opacity: 0.9; }
  .cd-chat-send:disabled { opacity: 0.4; }

  /* ── Footer ───────────────────────────── */
  .cd-foot {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    border-top: 1px solid var(--b1);
    flex-shrink: 0;
    font-family: var(--ui);
    font-size: 10.5px;
    color: var(--t4);
  }
  .cd-foot-btn {
    height: 26px;
    padding: 0 12px;
    border-radius: 5px;
    border: 1px solid var(--b2);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 11px;
    font-weight: 600;
    cursor: default;
    transition: opacity 0.12s, border-color 0.12s, color 0.12s;
  }
  .cd-foot-btn:hover:not(:disabled) { color: var(--t1); border-color: var(--acc); }
  .cd-foot-btn:disabled { opacity: 0.4; }
  .cd-foot-link {
    font-family: var(--ui);
    font-size: 11px;
    color: var(--t3);
    text-decoration: none;
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 5px 10px;
  }
  .cd-foot-link:hover { color: var(--t1); border-color: var(--b2); }
  .cd-foot-spacer { flex: 1; }
  .cd-foot-meta { color: var(--t4); }

  /* ── PR preview dialog ───────────────────────────────────────── */
  .cd-pr-preview {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .cd-pr-field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .cd-pr-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--t2);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .cd-pr-branch {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--t1);
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 6px 10px;
    width: fit-content;
    word-break: break-all;
  }
  .cd-pr-input,
  .cd-pr-textarea {
    background: var(--surface-hover);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 8px 10px;
    font-family: var(--ui);
    font-size: 13px;
    color: var(--t1);
    outline: none;
    transition: border-color 0.12s;
  }
  .cd-pr-textarea {
    resize: vertical;
    min-height: 110px;
    line-height: 1.5;
  }
  .cd-pr-input:focus,
  .cd-pr-textarea:focus {
    border-color: var(--acc);
  }
  .cd-pr-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 2px;
  }
  .cd-pr-btn {
    padding: 7px 14px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12.5px;
    cursor: default;
    transition: background 0.12s, border-color 0.12s, color 0.12s, filter 0.12s;
  }
  .cd-pr-btn:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--t1);
    border-color: var(--b2);
  }
  .cd-pr-btn.primary {
    background: var(--acc);
    border-color: var(--acc);
    color: #fff;
  }
  .cd-pr-btn.primary:hover:not(:disabled) {
    filter: brightness(1.1);
  }
  .cd-pr-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ── Lifecycle ribbon ────────────────────────────────────────── */
  .cd-lifecycle {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 7px 14px 8px;
    border-bottom: 1px solid var(--b-subtle);
    font-family: var(--ui);
    font-size: 11px;
    color: var(--t3);
    min-width: 0;
    flex-shrink: 0;
    line-height: 1.4;
  }
  .cd-lifecycle-seg {
    display: inline-flex;
    align-items: center;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cd-lifecycle-link {
    color: var(--t2);
    text-decoration: none;
    border-bottom: 1px dotted transparent;
    transition: color 0.12s, border-color 0.12s;
  }
  .cd-lifecycle-link:hover {
    color: var(--acc);
    border-bottom-color: currentColor;
  }
  .cd-lifecycle-sep {
    color: var(--t4);
    user-select: none;
  }
</style>

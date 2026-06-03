<script lang="ts">
  import { onDestroy } from 'svelte';
  import MilkdownEditor from './MilkdownEditor.svelte';
  import TagInput from './TagInput.svelte';
  import CoworkerAvatar from './CoworkerAvatar.svelte';
  import { activeWorkspace } from '../stores';
  import { workspaceNoteGet, workspaceNoteUpdate, workspaceNoteExportToFile } from '../commands';
  import { marked } from 'marked';
  import { describeActor, formatAttribution, currentUserActor } from '../attribution';
  import type { WorkspaceNote } from '../types';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import { agentSessions, activeAgentSession } from '$lib/modes/agent/stores';
  import { setMode } from '$lib/stores/app';
  import { activateTabAcrossMode } from '$lib/utils/tabActivation';
  import { tabs as sharedTabs, addTab, activateTab, updateTab } from '$lib/shared/stores/tabs';
  import { getPurposeColor } from '$lib/modes/agent/ai/prompt';
  import { get } from 'svelte/store';

  interface Props {
    noteId: string;
  }

  let { noteId }: Props = $props();

  let note = $state<WorkspaceNote | null>(null);
  let title = $state('');
  let tags = $state<string[]>([]);
  let currentContent = $state('');
  let saving = $state(false);
  let dirty = $state(false);
  let saveTimeout: ReturnType<typeof setTimeout> | null = null;
  // Baseline = markdown that's currently persisted (or, on first emit,
  // the Crepe-normalized version of what we loaded). Crepe fires
  // `markdownUpdated` on initial parse and on certain cursor-placement
  // ops even when the user didn't edit anything — without this guard
  // those phantom updates trip the autosave and stamp the current user
  // as the note's updater, which is what you saw when "just scrolling"
  // re-attributed the note.
  let baseline = $state<string | null>(null);

  const linkedSession = $derived.by(() => {
    if (!note?.linkedSessionId) return null;
    return get(agentSessions).find(s => s.id === note!.linkedSessionId) ?? null;
  });

  /** Load the note. Re-runs when noteId changes (user switching tabs).
   *  If the note isn't already linked AND there's an active agent session
   *  for the same project, auto-link silently — saves the user a click on
   *  the "Link active session" button. The project-match check avoids
   *  cross-wiring an unrelated session that just happens to be active. */
  async function bootstrap(id: string) {
    note = null;
    baseline = null; // reset so the next editor mount can adopt its own first emit
    try {
      const fetched = await workspaceNoteGet(id);
      note = fetched;
      title = fetched.title;
      try { tags = JSON.parse(fetched.tags); } catch { tags = []; }
      currentContent = fetched.content;

      if (!fetched.linkedSessionId) {
        const active = get(activeAgentSession);
        const ws = get(activeWorkspace);
        const projectMatches =
          !!active &&
          (!ws?.projectPath || active.projectPath === ws.projectPath);
        if (active && projectMatches) {
          // Auto-link is in-memory only — if we persist it here we'd
          // stamp the current user as the note's updater on every open,
          // even when they only viewed it. The link rides along on the
          // next genuine edit's save instead.
          note = { ...fetched, linkedSessionId: active.id };
        }
      }
    } catch (e) {
      errorToast('Failed to load note', e);
    }
  }

  function onContentChange(markdown: string) {
    if (baseline === null) {
      // First emit after mount — Crepe's normalized parse of the loaded
      // content. Adopt it as the persisted baseline; not a real edit.
      baseline = markdown;
      currentContent = markdown;
      return;
    }
    if (markdown === baseline) {
      // Spurious re-emit (cursor placement, plugin DOM ops). The
      // markdown didn't actually change — don't autosave, don't bump
      // attribution.
      return;
    }
    currentContent = markdown;
    dirty = true;
    scheduleSave();
  }

  function scheduleSave() {
    if (saveTimeout) clearTimeout(saveTimeout);
    saveTimeout = setTimeout(saveNow, 600);
  }

  async function saveNow() {
    if (!note || saving) return;
    saving = true;
    try {
      await workspaceNoteUpdate({
        id: note.id,
        title: title.trim() || 'Untitled',
        content: currentContent,
        tags,
        linkedSessionId: note.linkedSessionId,
        actor: currentUserActor(),
      });
      dirty = false;
      baseline = currentContent; // persisted content is now the new baseline
      // Local refresh — keep editor mounted, just refresh metadata.
      const refreshed = await workspaceNoteGet(note.id);
      note = { ...refreshed, content: currentContent };
      const myTab = get(sharedTabs).find(t => t.mode === 'workspace' && t.key === `note:${refreshed.id}`);
      if (myTab && myTab.label !== refreshed.title) {
        updateTab(myTab.id, { label: refreshed.title || 'Untitled' });
      }
    } catch (e) {
      errorToast('Save failed', e);
    } finally {
      saving = false;
    }
  }

  function onTitleBlur() {
    if (!dirty && title === note?.title) return;
    dirty = true;
    saveNow();
  }
  function onTagsChange(_next: string[]) {
    dirty = true;
    saveNow();
  }

  async function attachToActiveSession() {
    if (!note) return;
    const s = get(activeAgentSession);
    if (!s) {
      showToast('No active agent session — open one in Agent mode first', 'error');
      return;
    }
    note.linkedSessionId = s.id;
    await saveNow();
    showToast(`Linked to "${s.title}"`, 'success');
  }

  async function detachSession() {
    if (!note) return;
    note.linkedSessionId = null;
    await saveNow();
  }

  function openLinkedSession() {
    if (!linkedSession) return;
    // Activate / open the agent tab for this session.
    const allTabs = get(sharedTabs);
    const existing = allTabs.find(t => t.mode === 'agent' && t.key === linkedSession.id);
    if (existing) {
      activateTabAcrossMode(existing.id);
    } else {
      const tab = addTab(linkedSession.title, 'agent', linkedSession.id, getPurposeColor(linkedSession.purpose));
      activateTab(tab.id);
      activeAgentSession.set(linkedSession);
      void setMode('agent');
    }
  }

  $effect(() => { bootstrap(noteId); });

  onDestroy(() => {
    if (saveTimeout) clearTimeout(saveTimeout);
    if (dirty) saveNow();
  });

  // ── Export ─────────────────────────────────────────────────────────
  let exportMenuOpen = $state(false);

  /** Wrap raw markdown HTML in a stand-alone document with enough inline
   *  styling that the exported file looks decent on its own (any browser,
   *  no Clauge tokens to inherit). Kept intentionally lean — headings,
   *  code, blockquote, links, images. */
  function buildHtmlDocument(noteTitle: string, markdown: string): string {
    const body = marked.parse(markdown ?? '', { async: false }) as string;
    const safeTitle = (noteTitle || 'Untitled').replace(/[<>&]/g, (c) =>
      c === '<' ? '&lt;' : c === '>' ? '&gt;' : '&amp;');
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>${safeTitle}</title>
<style>
  body { font: 15px/1.65 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; max-width: 760px; margin: 40px auto; padding: 0 24px; color: #1a1a1a; }
  h1, h2, h3 { line-height: 1.25; margin: 1.6em 0 0.5em; }
  h1 { font-size: 2em; border-bottom: 1px solid #e5e7eb; padding-bottom: 0.3em; }
  h2 { font-size: 1.5em; }
  h3 { font-size: 1.2em; }
  p { margin: 0.8em 0; }
  a { color: #2563eb; }
  code { background: #f3f4f6; padding: 1px 5px; border-radius: 4px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.92em; }
  pre { background: #f3f4f6; padding: 14px 16px; border-radius: 6px; overflow-x: auto; }
  pre code { background: transparent; padding: 0; }
  blockquote { border-left: 3px solid #d1d5db; color: #4b5563; padding: 4px 14px; margin: 1em 0; }
  img { max-width: 100%; height: auto; border-radius: 6px; }
  hr { border: none; border-top: 1px solid #e5e7eb; margin: 2em 0; }
  table { border-collapse: collapse; }
  th, td { border: 1px solid #e5e7eb; padding: 6px 10px; }
</style>
</head>
<body>
<h1>${safeTitle}</h1>
${body}
</body>
</html>`;
  }

  async function exportAs(kind: 'md' | 'html') {
    exportMenuOpen = false;
    if (!note) return;
    try {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const safeName = (note.title || 'note').replace(/[/\\?%*:|"<>]/g, '-');
      const ext = kind === 'md' ? 'md' : 'html';
      const dest = await save({
        defaultPath: `${safeName}.${ext}`,
        title: `Export ${note.title || 'note'}`,
        filters: [{ name: kind === 'md' ? 'Markdown' : 'HTML', extensions: [ext] }],
      });
      if (typeof dest !== 'string' || !dest) return;
      const content = kind === 'md' ? (note.content ?? '') : buildHtmlDocument(note.title, note.content ?? '');
      await workspaceNoteExportToFile(dest, content);
      showToast(`Exported to ${dest}`, 'success');
    } catch (e: any) {
      errorToast('Export failed', e);
    }
  }
</script>

<svelte:window onclick={() => (exportMenuOpen = false)} />

{#if !note}
  <div class="nv-loading">Loading…</div>
{:else}
  {@const editor_info = describeActor(note.updatedBy)}
  <div class="nv">
    <div class="nv-meta">
      <span class="nv-crumb">{$activeWorkspace?.name ?? 'workspace'}</span>
      <span class="nv-sep">/</span>
      <span class="nv-crumb-active">{note.title || 'untitled'}</span>
      <span style="flex:1"></span>
      {#if saving}
        <span class="nv-saving">saving…</span>
      {:else if dirty}
        <span class="nv-dirty">unsaved</span>
      {:else}
        <span class="nv-saved">saved</span>
      {/if}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="nv-export-wrap" onclick={(e) => e.stopPropagation()}>
        <button
          class="nv-export-btn"
          title="Export note"
          aria-label="Export note"
          onclick={() => (exportMenuOpen = !exportMenuOpen)}
        >
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
          Export
        </button>
        {#if exportMenuOpen}
          <div class="nv-export-menu" role="menu">
            <button class="nv-export-item" onclick={() => exportAs('md')}>
              <span class="nv-export-item-label">Markdown</span>
              <span class="nv-export-item-ext">.md</span>
            </button>
            <button class="nv-export-item" onclick={() => exportAs('html')}>
              <span class="nv-export-item-label">HTML</span>
              <span class="nv-export-item-ext">.html</span>
            </button>
          </div>
        {/if}
      </div>
    </div>

    <input
      class="nv-title"
      bind:value={title}
      onblur={onTitleBlur}
      placeholder="Untitled"
      spellcheck="false"
    />

    <div class="nv-props">
      <div class="nv-prop-key">PROJECT</div>
      <div class="nv-prop-val">
        {#if $activeWorkspace?.projectName}
          <span class="nv-pill">
            <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>
            {$activeWorkspace.projectName}
          </span>
        {:else}
          <span class="nv-prop-empty">none</span>
        {/if}
      </div>

      <div class="nv-prop-key">TAGS</div>
      <div class="nv-prop-val">
        <TagInput bind:value={tags} onchange={onTagsChange} />
      </div>

      <div class="nv-prop-key">LINKED SESSION</div>
      <div class="nv-prop-val">
        {#if linkedSession}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span class="nv-pill nv-pill-clickable" onclick={openLinkedSession}>
            <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l1.6 4.8L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.2L12 3z"/></svg>
            {linkedSession.title}
            <span class="nv-pill-dim">· {linkedSession.purpose}</span>
          </span>
          <button class="nv-mini-btn" onclick={detachSession} title="Unlink">×</button>
        {:else if $activeAgentSession && (!$activeWorkspace?.projectPath || $activeAgentSession.projectPath !== $activeWorkspace.projectPath)}
          <!-- Active session is for a different project — manual opt-in only. -->
          <button class="nv-mini-btn" onclick={attachToActiveSession} title="The active agent session is for a different project — link anyway?">
            Link {$activeAgentSession.title} (different project)
          </button>
        {:else}
          <span class="nv-prop-empty">no session linked</span>
        {/if}
      </div>

      <div class="nv-prop-key">UPDATED</div>
      <div class="nv-prop-val">
        <span class="nv-attr">
          {#if editor_info.kind === 'coworker'}
            <span class="nv-attr-badge nv-attr-user" title="Edited by @{editor_info.label}">
              <CoworkerAvatar seed={editor_info.coworkerSeed ?? editor_info.label} style={editor_info.coworkerStyle ?? 'personas'} size={14} />
              <span>@{editor_info.label}</span>
            </span>
          {:else if editor_info.kind === 'agent'}
            <span class="nv-attr-badge nv-attr-agent" title="Edited by {editor_info.label}">
              <svg viewBox="0 0 24 24" width="9" height="9" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l1.6 4.8L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.2L12 3z"/></svg>
              {editor_info.label}
            </span>
          {:else if editor_info.kind === 'user'}
            <span class="nv-attr-badge nv-attr-user" title="Edited by @{editor_info.label}">
              {#if editor_info.avatarUrl}
                <img class="nv-attr-avatar" src={editor_info.avatarUrl} alt="" width="14" height="14"/>
              {:else}
                <span class="nv-attr-initials">{editor_info.label.slice(0, 2).toUpperCase()}</span>
              {/if}
              <span>@{editor_info.label}</span>
            </span>
          {:else}
            <span class="nv-attr-badge nv-attr-anon" title="Edited by you">
              <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 21a8 8 0 10-16 0"/><circle cx="12" cy="7" r="4"/></svg>
              {editor_info.label}
            </span>
          {/if}
          <span class="nv-attr-time">· {formatAttribution(note.updatedBy, note.updatedAt).split('· ')[1] ?? ''}</span>
        </span>
      </div>
    </div>

    <div class="nv-editor">
      <MilkdownEditor noteId={note.id} onChange={onContentChange} />
    </div>
  </div>
{/if}

<style>
  .nv-loading {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 12.5px;
  }
  .nv {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    padding: 16px 28px 0;
  }
  .nv-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 14px;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t4);
  }
  .nv-crumb { color: var(--t3); }
  .nv-crumb-active { color: var(--t2); }
  .nv-sep { color: var(--t4); }
  .nv-saving { color: var(--warn, #f5a623); font-style: italic; }
  .nv-dirty { color: var(--t4); font-style: italic; }

  .nv-export-wrap {
    position: relative;
    margin-left: 8px;
  }
  .nv-export-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 9px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-family: var(--ui);
    font-size: 11px;
    cursor: default;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }
  .nv-export-btn:hover {
    background: var(--surface-hover);
    color: var(--t1);
    border-color: var(--b2);
  }
  .nv-export-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    min-width: 160px;
    background: var(--modal-bg, #101016);
    border: 1px solid var(--b1);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
    padding: 4px;
    z-index: 50;
    animation: nv-export-in 0.12s ease;
  }
  @keyframes nv-export-in {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: none; }
  }
  .nv-export-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    padding: 7px 10px;
    border: none;
    background: transparent;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 12px;
    border-radius: 5px;
    cursor: default;
    text-align: left;
  }
  .nv-export-item:hover {
    background: var(--surface-hover);
  }
  .nv-export-item-ext {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t4);
  }
  .nv-saved { color: var(--state-saved); }

  .nv-title {
    border: none;
    background: transparent;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.01em;
    outline: none;
    padding: 0;
    margin: 0 0 14px;
    width: 100%;
  }
  .nv-title::placeholder { color: var(--t4); }

  .nv-props {
    display: grid;
    grid-template-columns: 110px 1fr;
    row-gap: 8px;
    column-gap: 14px;
    align-items: center;
    margin-bottom: 18px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--b1);
    max-width: 780px;
  }
  .nv-prop-key {
    font-family: var(--ui);
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--t4);
  }
  .nv-prop-val {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-family: var(--ui);
    font-size: 12px;
    color: var(--t2);
    min-width: 0;
  }
  .nv-prop-empty { color: var(--t4); font-style: italic; }
  .nv-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    border-radius: 12px;
    border: 1px solid var(--b1);
    background: var(--surface-hover);
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--t1);
  }
  .nv-pill-clickable { cursor: default; }
  .nv-pill-clickable:hover { border-color: var(--acc); color: var(--acc); }
  .nv-pill-dim { color: var(--t4); }


  .nv-mini-btn {
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 11px;
    padding: 3px 8px;
    border-radius: 5px;
    cursor: default;
    transition: border-color 0.12s, color 0.12s;
  }
  .nv-mini-btn:hover { border-color: var(--acc); color: var(--t1); }

  .nv-attr {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: var(--ui);
    font-size: 11.5px;
    color: var(--t2);
  }
  .nv-attr-avatar { border-radius: 50%; object-fit: cover; }
  /* Two-letter fallback when a logged-in user has no avatarUrl. */
  .nv-attr-initials {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--acc);
    color: #fff;
    font-size: 8px;
    font-weight: 700;
    line-height: 1;
    font-family: var(--ui);
    flex-shrink: 0;
  }
  .nv-attr-time { color: var(--t4); }
  .nv-attr-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 7px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--acc) 15%, transparent);
    color: var(--acc);
    font-size: 10px;
    font-weight: 500;
  }
  /* Anonymous (not signed in) — neutral chip, no accent fill. */
  .nv-attr-badge.nv-attr-anon {
    background: var(--surface-hover);
    color: var(--t3);
  }

  .nv-editor {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    margin: 0 -28px;
    padding: 0 28px;
  }
</style>

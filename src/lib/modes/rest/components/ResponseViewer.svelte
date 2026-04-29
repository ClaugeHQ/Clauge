<script lang="ts">
  import type { HttpResponse } from '$lib/types';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { showToast } from '$lib/components/shared/toast';
  import { highlightJSON } from '$lib/shared/utils/json-highlight';
  import ResponseHeaders from './ResponseHeaders.svelte';
  import { onMount, onDestroy } from 'svelte';

  interface Props {
    response: HttpResponse | null;
    loading: boolean;
  }

  let { response, loading }: Props = $props();

  type TabId = 'pretty' | 'raw' | 'preview' | 'headers';
  const TABS: { id: TabId; label: string }[] = [
    { id: 'pretty', label: 'Pretty' },
    { id: 'raw', label: 'Raw' },
    { id: 'preview', label: 'Preview' },
    { id: 'headers', label: 'Headers' },
  ];

  let activeTab = $state<TabId>('pretty');

  // Search state
  let showSearch = $state(false);
  let searchQuery = $state('');
  let searchInputRef = $state<HTMLInputElement | null>(null);
  let currentMatchIndex = $state(0);
  let viewerRef = $state<HTMLDivElement | null>(null);

  function escapeRegex(s: string): string {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  const matchCount = $derived.by(() => {
    if (!searchQuery || !response) return 0;
    const body = activeTab === 'pretty' ? prettyBody : (activeTab === 'raw' ? response.body : '');
    if (!body) return 0;
    const regex = new RegExp(escapeRegex(searchQuery), 'gi');
    const matches = body.match(regex);
    return matches ? matches.length : 0;
  });

  const searchHighlightedBody = $derived.by(() => {
    if (!searchQuery || !response) return '';
    const body = activeTab === 'pretty' ? prettyBody : (activeTab === 'raw' ? response.body : '');
    if (!body) return '';
    // For pretty tab, apply JSON highlighting first, then search highlight
    if (activeTab === 'pretty') {
      const highlighted = highlightJSON(body);
      if (!searchQuery) return highlighted;
      // We need to highlight matches in the visible text, not in HTML tags
      return highlightSearchInHtml(highlighted, searchQuery, currentMatchIndex);
    }
    // For raw tab, escape HTML then highlight search
    return highlightSearchInText(body, searchQuery, currentMatchIndex);
  });

  function escapeHtml(str: string): string {
    return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }

  function highlightSearchInText(text: string, query: string, activeIndex: number): string {
    const escaped = escapeHtml(text);
    const regex = new RegExp(escapeRegex(escapeHtml(query)), 'gi');
    let matchIdx = 0;
    return escaped.replace(regex, (match) => {
      const cls = matchIdx === activeIndex ? 'search-match active-match' : 'search-match';
      matchIdx++;
      return `<mark class="${cls}">${match}</mark>`;
    });
  }

  function highlightSearchInHtml(html: string, query: string, activeIndex: number): string {
    // Split HTML into tags and text segments, only highlight in text segments
    const parts = html.split(/(<[^>]*>)/);
    const escapedQuery = escapeRegex(query);
    const regex = new RegExp(escapedQuery, 'gi');
    let matchIdx = 0;

    return parts.map(part => {
      if (part.startsWith('<')) return part;
      return part.replace(regex, (match) => {
        const cls = matchIdx === activeIndex ? 'search-match active-match' : 'search-match';
        matchIdx++;
        return `<mark class="${cls}">${match}</mark>`;
      });
    }).join('');
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  }

  function formatBody(body: string): string {
    try {
      return JSON.stringify(JSON.parse(body), null, 2);
    } catch {
      return body;
    }
  }

  const isSuccess = $derived(response ? response.status >= 200 && response.status < 300 : false);

  const prettyBody = $derived(response ? formatBody(response.body) : '');
  const highlightedBody = $derived(prettyBody ? highlightJSON(prettyBody) : '');

  async function copyResponse() {
    if (!response) return;
    try {
      await writeText(response.body);
      showToast('Response copied', 'success');
    } catch {
      showToast('Failed to copy', 'error');
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'f' && response) {
      e.preventDefault();
      showSearch = true;
      setTimeout(() => searchInputRef?.focus(), 0);
    }
  }

  function handleSearchKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      closeSearch();
    } else if (e.key === 'Enter') {
      if (e.shiftKey) {
        prevMatch();
      } else {
        nextMatch();
      }
    }
  }

  function nextMatch() {
    if (matchCount === 0) return;
    currentMatchIndex = (currentMatchIndex + 1) % matchCount;
    scrollToMatch();
  }

  function prevMatch() {
    if (matchCount === 0) return;
    currentMatchIndex = (currentMatchIndex - 1 + matchCount) % matchCount;
    scrollToMatch();
  }

  function scrollToMatch() {
    setTimeout(() => {
      const active = viewerRef?.querySelector('.active-match');
      if (active) {
        active.scrollIntoView({ block: 'center', behavior: 'smooth' });
      }
    }, 10);
  }

  function closeSearch() {
    showSearch = false;
    searchQuery = '';
    currentMatchIndex = 0;
  }

  // Reset match index when query changes
  $effect(() => {
    if (searchQuery) {
      currentMatchIndex = 0;
    }
  });

  onMount(() => {
    document.addEventListener('keydown', handleKeydown);
  });

  onDestroy(() => {
    document.removeEventListener('keydown', handleKeydown);
  });
</script>

<div class="response-viewer">
  {#if !response && !loading}
    <!-- Empty state -->
    <div class="empty">
      <div class="empty-icon">
        <svg viewBox="0 0 24 24">
          <circle cx="12" cy="12" r="10" />
          <path d="M8 12l3 3 5-5" />
        </svg>
      </div>
      <div class="empty-text">
        Press <span class="kbd">&#8984;Enter</span> or click <span class="kbd">Send</span> to send request<br />
        <span class="kbd">&#8984;L</span> to toggle AI Assistant
      </div>
    </div>
  {:else if loading}
    <!-- Loading state -->
    <div class="empty">
      <div class="loading-text">Sending request<span class="loading-dots"></span></div>
    </div>
  {:else if response}
    <!-- Response header bar -->
    <div class="resp-hdr">
      <span class="status-pill" class:s-ok={isSuccess} class:s-err={!isSuccess}>
        {response.status} {response.status_text}
      </span>
      <span class="resp-meta">{response.duration_ms}ms</span>
      <span class="resp-meta">{formatSize(response.size_bytes)}</span>
      <div class="resp-spacer"></div>
      <button class="resp-copy" onclick={copyResponse} title="Copy response body">
        <svg viewBox="0 0 24 24" width="13" height="13">
          <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
          <path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" />
        </svg>
      </button>
    </div>

    <!-- Search bar -->
    {#if showSearch}
      <div class="search-bar">
        <div class="search-icon">
          <svg viewBox="0 0 24 24" width="13" height="13"><circle cx="11" cy="11" r="8" stroke="currentColor" fill="none" stroke-width="2"/><path d="M21 21l-4.35-4.35" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round"/></svg>
        </div>
        <input
          bind:this={searchInputRef}
          class="search-input"
          type="text"
          placeholder="Search response..."
          bind:value={searchQuery}
          onkeydown={handleSearchKeydown}
        />
        {#if searchQuery}
          <span class="search-count">{matchCount > 0 ? `${currentMatchIndex + 1} of ${matchCount}` : 'No matches'}</span>
        {/if}
        <button class="search-nav-btn" onclick={prevMatch} title="Previous (Shift+Enter)" disabled={matchCount === 0}>
          <svg viewBox="0 0 24 24" width="12" height="12"><path d="M18 15l-6-6-6 6" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round"/></svg>
        </button>
        <button class="search-nav-btn" onclick={nextMatch} title="Next (Enter)" disabled={matchCount === 0}>
          <svg viewBox="0 0 24 24" width="12" height="12"><path d="M6 9l6 6 6-6" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round"/></svg>
        </button>
        <button class="search-nav-btn" onclick={closeSearch} title="Close (Esc)">
          <svg viewBox="0 0 24 24" width="12" height="12"><path d="M18 6L6 18M6 6l12 12" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round"/></svg>
        </button>
      </div>
    {/if}

    <!-- Tab bar -->
    <div class="ph">
      {#each TABS as tab (tab.id)}
        <button
          class="pht"
          class:on={activeTab === tab.id}
          onclick={() => { activeTab = tab.id; }}
        >
          {tab.label}
        </button>
      {/each}
    </div>

    <!-- Tab content -->
    {#if activeTab === 'pretty'}
      <div class="viewer" bind:this={viewerRef}>
        {#if searchQuery}
          {@html searchHighlightedBody}
        {:else}
          {@html highlightedBody}
        {/if}
      </div>
    {:else if activeTab === 'raw'}
      <div class="viewer raw" bind:this={viewerRef}>
        {#if searchQuery}
          {@html searchHighlightedBody}
        {:else}
          {response.body}
        {/if}
      </div>
    {:else if activeTab === 'preview'}
      <div class="viewer preview-msg">
        <span>Preview not available for JSON responses</span>
      </div>
    {:else if activeTab === 'headers'}
      <ResponseHeaders headers={response.headers} />
    {/if}
  {/if}
</div>

<style>
  .response-viewer {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  /* ── Empty state ── */
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    background: transparent;
  }

  .empty-icon svg {
    width: 36px;
    height: 36px;
    stroke: var(--t4);
    fill: none;
    stroke-width: 1.2;
    stroke-linecap: round;
  }

  .empty-text {
    font-size: 12px;
    color: var(--t3);
    text-align: center;
    line-height: 1.6;
  }

  .kbd {
    display: inline-block;
    padding: 1px 5px;
    border-radius: 3px;
    border: 1px solid var(--b1);
    font-size: 10px;
    font-family: var(--mono);
    color: var(--t3);
    background: var(--n);
  }

  /* ── Loading ── */
  .loading-text {
    font-size: 12px;
    color: var(--t3);
    font-family: var(--mono);
  }

  .loading-dots::after {
    content: '';
    animation: dots 1.4s steps(4, end) infinite;
  }

  @keyframes dots {
    0%   { content: ''; }
    25%  { content: '.'; }
    50%  { content: '..'; }
    75%  { content: '...'; }
    100% { content: ''; }
  }

  /* ── Response header bar ── */
  .resp-hdr {
    height: 32px;
    flex-shrink: 0;
    background: transparent;
    border-bottom: 1px solid var(--b1);
    padding: 0 12px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .status-pill {
    font-size: 12px;
    font-weight: 600;
    padding: 2px 9px;
    border-radius: 10px;
    font-family: var(--mono);
  }

  .s-ok {
    background: rgba(29,200,128,0.1);
    color: var(--ok);
  }

  .s-err {
    background: rgba(240,68,68,0.1);
    color: var(--err);
  }

  .resp-meta {
    font-size: 11px;
    color: var(--t3);
    font-family: var(--mono);
  }

  .resp-spacer {
    flex: 1;
  }

  .resp-copy {
    width: 24px;
    height: 24px;
    border: none;
    background: transparent;
    color: var(--t3);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    transition: color 0.1s, background 0.1s;
  }
  .resp-copy:hover {
    color: var(--t1);
    background: var(--b1);
  }
  .resp-copy svg {
    stroke: currentColor;
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* ── Search bar ── */
  .search-bar {
    height: 34px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    padding: 0 10px;
    display: flex;
    align-items: center;
    gap: 6px;
    animation: slideDown 0.15s ease;
  }
  @keyframes slideDown {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: none; }
  }
  .search-icon {
    color: var(--t3);
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }
  .search-input {
    flex: 1;
    background: transparent;
    border: 1px solid var(--b1);
    border-radius: 4px;
    padding: 7px 11px;
    font-size: 12px;
    font-family: var(--mono);
    color: var(--t1);
    outline: none;
    height: 24px;
    transition: border-color 0.15s;
  }
  .search-input:focus {
    border-color: var(--acc);
  }
  .search-input::placeholder {
    color: var(--t3);
  }
  .search-count {
    font-size: 10.5px;
    color: var(--t3);
    font-family: var(--mono);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .search-nav-btn {
    width: 22px;
    height: 22px;
    border: none;
    background: transparent;
    color: var(--t3);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    flex-shrink: 0;
    transition: color 0.1s, background 0.1s;
  }
  .search-nav-btn:hover:not(:disabled) {
    color: var(--t1);
    background: var(--b1);
  }
  .search-nav-btn:disabled {
    opacity: 0.3;
    cursor: default;
  }

  /* ── Tab bar ── */
  .ph {
    height: 34px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    padding: 0 12px;
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .pht {
    font-size: 12px;
    color: var(--t3);
    cursor: pointer;
    font-family: var(--mono);
    padding-bottom: 2px;
    border: none;
    background: transparent;
    border-bottom: 1.5px solid transparent;
    transition: color 0.1s;
  }
  .pht.on {
    color: var(--t1);
    border-bottom-color: var(--acc);
  }
  .pht:hover:not(.on) {
    color: var(--t2);
  }

  /* ── Viewer ── */
  .viewer {
    flex: 1;
    min-height: 0;
    background: transparent;
    padding: 12px 14px;
    overflow-y: auto;
    overflow-x: hidden;
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 1.75;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--t1);
  }
  .viewer::-webkit-scrollbar { width: 4px; }
  .viewer::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .raw {
    white-space: pre-wrap;
    word-break: break-all;
  }

  .preview-msg {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t3);
    font-size: 12px;
    white-space: normal;
  }

  /* Search match highlighting */
  :global(.search-match) {
    background: rgba(245, 166, 35, 0.25);
    color: inherit;
    border-radius: 2px;
    padding: 0 1px;
  }
  :global(.active-match) {
    background: rgba(245, 166, 35, 0.55);
    outline: 1px solid rgba(245, 166, 35, 0.6);
  }
</style>

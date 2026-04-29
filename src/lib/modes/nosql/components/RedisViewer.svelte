<script lang="ts">
  import { showToast } from '$lib/components/shared/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { redisListKeys, redisGetKey, redisSetKey, redisDeleteKey, redisExecute } from '../commands';
  import { highlightJSON } from '$lib/utils/json-highlight';
  import type { RedisKeyInfo, RedisValue } from '../types';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';

  interface Props {
    connectionId: string;
  }

  let { connectionId }: Props = $props();

  type ViewTab = 'keys' | 'console';
  let activeTab = $state<ViewTab>('keys');

  // Keys
  let keys = $state<RedisKeyInfo[]>([]);
  let searchPattern = $state('*');
  let loading = $state(false);
  let cursor = $state(0);
  let hasMore = $state(false);
  let selectedKey = $state<RedisValue | null>(null);
  let loadingValue = $state(false);

  // Set key
  let showSetKey = $state(false);
  let newKeyName = $state('');
  let newKeyValue = $state('');
  let newKeyTtl = $state(-1);

  // Console
  let consoleInput = $state('');
  let consoleHistory = $state<{ command: string; result: string; isError: boolean }[]>([]);
  let commandHistory = $state<string[]>([]);
  let historyIdx = $state(-1);
  let consoleEl: HTMLDivElement | undefined = $state();

  const TYPE_COLORS: Record<string, string> = {
    string: '#e0a370',
    list: '#9ccfd8',
    set: '#f6c177',
    hash: '#c4a7e7',
    zset: '#eb6f92',
    stream: '#c4a7e7',
  };

  const TYPE_BG: Record<string, string> = {
    string: 'rgba(224,163,112,0.12)',
    list: 'rgba(156,207,216,0.12)',
    set: 'rgba(246,193,119,0.12)',
    hash: 'rgba(196,167,231,0.12)',
    zset: 'rgba(235,111,146,0.12)',
    stream: 'rgba(196,167,231,0.12)',
  };

  let prevConnId = $state('');
  $effect(() => {
    if (connectionId && connectionId !== prevConnId) {
      prevConnId = connectionId;
      queueMicrotask(() => loadKeys());
    }
  });

  async function loadKeys() {
    loading = true;
    try {
      const result = await redisListKeys(connectionId, searchPattern, 0, 100);
      keys = result.keys;
      cursor = result.nextCursor;
      hasMore = result.nextCursor !== 0;
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
      keys = [];
    } finally {
      loading = false;
    }
  }

  async function loadMore() {
    if (!hasMore) return;
    loading = true;
    try {
      const result = await redisListKeys(connectionId, searchPattern, cursor, 100);
      keys = [...keys, ...result.keys];
      cursor = result.nextCursor;
      hasMore = result.nextCursor !== 0;
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      loading = false;
    }
  }

  async function selectKey(keyInfo: RedisKeyInfo) {
    loadingValue = true;
    try {
      selectedKey = await redisGetKey(connectionId, keyInfo.key);
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      loadingValue = false;
    }
  }

  async function handleDeleteKey(key: string) {
    try {
      await redisDeleteKey(connectionId, key);
      showToast(`Key "${key}" deleted`, 'success');
      keys = keys.filter((k) => k.key !== key);
      if (selectedKey?.key === key) selectedKey = null;
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    }
  }

  async function handleSetKey() {
    try {
      const ttl = newKeyTtl > 0 ? newKeyTtl : undefined;
      await redisSetKey(connectionId, newKeyName, newKeyValue, ttl);
      showToast(`Key "${newKeyName}" set`, 'success');
      showSetKey = false;
      newKeyName = '';
      newKeyValue = '';
      newKeyTtl = -1;
      loadKeys();
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    }
  }

  async function copyValue() {
    if (!selectedKey) return;
    try {
      const val = typeof selectedKey.value === 'string' ? selectedKey.value : JSON.stringify(selectedKey.value, null, 2);
      await writeText(val);
      showToast('Value copied', 'success');
    } catch {
      showToast('Failed to copy', 'error');
    }
  }

  async function executeCommand() {
    if (!consoleInput.trim()) return;
    const cmd = consoleInput.trim();
    consoleInput = '';
    commandHistory = [cmd, ...commandHistory.slice(0, 50)];
    historyIdx = -1;
    try {
      const result = await redisExecute(connectionId, cmd);
      consoleHistory = [...consoleHistory, { command: cmd, result, isError: false }];
    } catch (e: any) {
      consoleHistory = [...consoleHistory, { command: cmd, result: String(e), isError: true }];
    }
    requestAnimationFrame(() => {
      if (consoleEl) consoleEl.scrollTop = consoleEl.scrollHeight;
    });
  }

  function handleConsoleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      executeCommand();
    } else if (e.key === 'ArrowUp' && commandHistory.length > 0) {
      e.preventDefault();
      historyIdx = Math.min(historyIdx + 1, commandHistory.length - 1);
      consoleInput = commandHistory[historyIdx];
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (historyIdx > 0) {
        historyIdx -= 1;
        consoleInput = commandHistory[historyIdx];
      } else {
        historyIdx = -1;
        consoleInput = '';
      }
    }
  }

  function formatValue(val: any): string {
    if (typeof val === 'string') return val;
    try {
      return JSON.stringify(val, null, 2);
    } catch {
      return String(val);
    }
  }

  function isJsonValue(val: any): boolean {
    if (typeof val !== 'string') return typeof val === 'object';
    try { JSON.parse(val); return true; } catch { return false; }
  }

  function highlightValue(val: any): string {
    const str = formatValue(val);
    if (isJsonValue(val)) return highlightJSON(str);
    return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }

  function formatTtl(ttl: number): string {
    if (ttl === -1) return 'No expiry';
    if (ttl === -2) return 'Expired';
    if (ttl < 60) return `${ttl}s`;
    if (ttl < 3600) return `${Math.floor(ttl / 60)}m ${ttl % 60}s`;
    return `${Math.floor(ttl / 3600)}h ${Math.floor((ttl % 3600) / 60)}m`;
  }
</script>

<div class="rv-wrap">
  <!-- Tabs -->
  <div class="rv-tabs">
    <button class="rv-tab" class:on={activeTab === 'keys'} onclick={() => activeTab = 'keys'}>
      <svg viewBox="0 0 24 24" width="12" height="12"><path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 11-7.778 7.778 5.5 5.5 0 017.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/></svg>
      Keys
    </button>
    <button class="rv-tab" class:on={activeTab === 'console'} onclick={() => activeTab = 'console'}>
      <svg viewBox="0 0 24 24" width="12" height="12"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>
      Console
    </button>
    <div class="rv-tab-spacer"></div>
    {#if activeTab === 'keys'}
      <span class="rv-meta">{keys.length} keys</span>
    {/if}
  </div>

  {#if activeTab === 'keys'}
    <!-- Search bar -->
    <div class="rv-search-bar">
      <svg class="rv-search-icon" viewBox="0 0 24 24" width="12" height="12"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
      <input
        class="rv-search-input"
        type="text"
        bind:value={searchPattern}
        placeholder="Pattern (e.g. user:*)"
        spellcheck="false"
        onkeydown={(e) => { if (e.key === 'Enter') loadKeys(); }}
      />
      <button class="rv-action-btn rv-scan-btn" onclick={loadKeys}>Scan</button>
      <button class="rv-action-btn rv-set-btn" onclick={() => showSetKey = true}>+ Set Key</button>
    </div>

    <div class="rv-content">
      <!-- Key list -->
      <div class="rv-key-list">
        {#if loading && keys.length === 0}
          <div class="rv-empty-state">
            <svg viewBox="0 0 24 24" width="28" height="28"><path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 11-7.778 7.778 5.5 5.5 0 017.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/></svg>
            <span>Scanning<span class="rv-dots"></span></span>
          </div>
        {:else if keys.length === 0}
          <div class="rv-empty-state">
            <svg viewBox="0 0 24 24" width="28" height="28"><path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 11-7.778 7.778 5.5 5.5 0 017.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/></svg>
            <span>No keys found</span>
          </div>
        {:else}
          {#each keys as keyInfo (keyInfo.key)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="rv-key-item"
              class:selected={selectedKey?.key === keyInfo.key}
              onclick={() => selectKey(keyInfo)}
            >
              <span class="rv-key-type" style="color: {TYPE_COLORS[keyInfo.keyType] ?? 'var(--t3)'}; background: {TYPE_BG[keyInfo.keyType] ?? 'rgba(255,255,255,0.05)'}">
                {keyInfo.keyType}
              </span>
              <span class="rv-key-name">{keyInfo.key}</span>
              <span class="rv-key-ttl">{formatTtl(keyInfo.ttl)}</span>
              <button class="rv-key-del" title="Delete" onclick={(e) => { e.stopPropagation(); handleDeleteKey(keyInfo.key); }}>
                <svg viewBox="0 0 24 24" width="11" height="11"><path d="M18 6L6 18M6 6l12 12"/></svg>
              </button>
            </div>
          {/each}
          {#if hasMore}
            <button class="rv-load-more" onclick={loadMore} disabled={loading}>
              {loading ? 'Loading...' : 'Load more keys'}
            </button>
          {/if}
        {/if}
      </div>

      <!-- Value viewer -->
      <div class="rv-value-pane">
        {#if loadingValue}
          <div class="rv-empty-state">
            <span>Loading<span class="rv-dots"></span></span>
          </div>
        {:else if selectedKey}
          <div class="rv-value-header">
            <span class="rv-value-key">{selectedKey.key}</span>
            <span class="rv-value-type" style="color: {TYPE_COLORS[selectedKey.keyType] ?? 'var(--t3)'}; background: {TYPE_BG[selectedKey.keyType] ?? 'rgba(255,255,255,0.05)'}">
              {selectedKey.keyType}
            </span>
            <span class="rv-value-ttl">{formatTtl(selectedKey.ttl)}</span>
            <button class="rv-value-copy" onclick={copyValue} title="Copy value">
              <svg viewBox="0 0 24 24" width="12" height="12"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>
            </button>
          </div>
          <div class="rv-value-body">{@html highlightValue(selectedKey.value)}</div>
        {:else}
          <div class="rv-empty-state">
            <svg viewBox="0 0 24 24" width="28" height="28"><polyline points="15 18 9 12 15 6"/></svg>
            <span>Select a key to view its value</span>
          </div>
        {/if}
      </div>
    </div>

    <!-- Set Key modal -->
    {#if showSetKey}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="rv-modal-overlay" onclick={(e) => { if (e.target === e.currentTarget) showSetKey = false; }}>
        <div class="rv-modal">
          <div class="rv-modal-hdr">
            <span>Set Key</span>
            <button class="rv-modal-close" onclick={() => showSetKey = false}>&times;</button>
          </div>
          <div class="rv-modal-body">
            <label class="rv-field">
              <span class="rv-field-label">Key</span>
              <input class="rv-field-input" type="text" bind:value={newKeyName} placeholder="mykey" />
            </label>
            <label class="rv-field">
              <span class="rv-field-label">Value</span>
              <textarea class="rv-field-ta" bind:value={newKeyValue} placeholder="value" spellcheck="false"></textarea>
            </label>
            <label class="rv-field">
              <span class="rv-field-label">TTL (seconds)</span>
              <input class="rv-field-input" type="number" bind:value={newKeyTtl} placeholder="-1 for no expiry" />
            </label>
          </div>
          <div class="rv-modal-actions">
            <button class="rv-modal-cancel" onclick={() => showSetKey = false}>Cancel</button>
            <button class="rv-modal-primary" onclick={handleSetKey}>Set</button>
          </div>
        </div>
      </div>
    {/if}
  {:else}
    <!-- Console -->
    <div class="rv-console" bind:this={consoleEl}>
      {#if consoleHistory.length === 0}
        <div class="rv-console-welcome">
          <span class="rv-console-welcome-title">Redis Console</span>
          <span class="rv-console-welcome-hint">Type commands below. Use Up/Down arrows for history.</span>
          <span class="rv-console-welcome-hint">Try: PING, INFO server, KEYS *, GET key</span>
        </div>
      {/if}
      {#each consoleHistory as entry}
        <div class="rv-console-entry">
          <div class="rv-console-cmd"><span class="rv-console-prompt-inline">&gt;</span> {entry.command}</div>
          <div class="rv-console-result" class:error={entry.isError}>{entry.result}</div>
        </div>
      {/each}
    </div>
    <div class="rv-console-input-bar">
      <span class="rv-console-prompt">&gt;</span>
      <input
        class="rv-console-input"
        type="text"
        bind:value={consoleInput}
        placeholder="Enter Redis command..."
        onkeydown={handleConsoleKeydown}
        spellcheck="false"
      />
      <button class="rv-action-btn" onclick={executeCommand}>Send</button>
    </div>
  {/if}
</div>

<style>
  .rv-wrap {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }

  /* ── Tabs ── */
  .rv-tabs {
    height: 34px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    padding: 0 12px;
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .rv-tab {
    font-size: 11.5px;
    color: var(--t3);
    cursor: default;
    font-family: var(--mono);
    padding: 4px 10px 6px;
    border: none;
    background: transparent;
    border-bottom: 1.5px solid transparent;
    transition: color 0.1s;
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .rv-tab svg {
    stroke: currentColor;
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.6;
  }
  .rv-tab.on { color: var(--t1); border-bottom-color: var(--acc); }
  .rv-tab.on svg { opacity: 1; }
  .rv-tab:hover:not(.on) { color: var(--t2); }
  .rv-tab-spacer { flex: 1; }
  .rv-meta { font-size: 10.5px; color: var(--t4); font-family: var(--mono); }

  /* ── Search bar ── */
  .rv-search-bar {
    height: 38px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    padding: 0 10px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .rv-search-icon {
    stroke: var(--t4);
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex-shrink: 0;
  }
  .rv-search-input {
    flex: 1;
    height: 26px;
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 0 8px;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--t1);
    outline: none;
    transition: border-color 0.15s;
  }
  .rv-search-input:focus {
    border-color: var(--acc);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--acc) 10%, transparent);
  }
  .rv-search-input::placeholder { color: var(--t4); }
  .rv-action-btn {
    height: 26px;
    padding: 0 10px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11px;
    font-family: var(--ui);
    cursor: default;
    flex-shrink: 0;
    transition: border-color 0.1s, color 0.1s, background 0.1s;
  }
  .rv-action-btn:hover { border-color: var(--b2); color: var(--t1); }
  .rv-scan-btn { border-color: var(--acc); color: var(--acc); }
  .rv-scan-btn:hover { background: var(--acc); color: #fff; }
  .rv-set-btn { color: var(--acc); border-color: color-mix(in srgb, var(--acc) 50%, transparent); }
  .rv-set-btn:hover { background: var(--acc); color: #fff; border-color: var(--acc); }

  /* ── Content split ── */
  .rv-content { flex: 1; min-height: 0; display: flex; overflow: hidden; }
  .rv-key-list {
    width: 45%;
    overflow-y: auto;
    border-right: 1px solid var(--b1);
  }
  .rv-key-list::-webkit-scrollbar { width: 4px; }
  .rv-key-list::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .rv-empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 12px;
    color: var(--t4);
    font-size: 12px;
    font-family: var(--mono);
  }
  .rv-empty-state svg {
    stroke: var(--t4);
    fill: none;
    stroke-width: 1.2;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.5;
  }
  .rv-dots::after { content: ''; animation: rvdots 1.4s steps(4, end) infinite; }
  @keyframes rvdots { 0% { content: ''; } 25% { content: '.'; } 50% { content: '..'; } 75% { content: '...'; } }

  .rv-key-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-bottom: 1px solid var(--b1);
    cursor: default;
    transition: background 0.08s;
    border-left: 2px solid transparent;
  }
  .rv-key-item:hover { background: color-mix(in srgb, var(--acc) 3%, transparent); }
  .rv-key-item.selected {
    background: color-mix(in srgb, var(--acc) 6%, transparent);
    border-left-color: var(--acc);
  }
  .rv-key-type {
    font-size: 9px;
    font-weight: 700;
    font-family: var(--mono);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    min-width: 40px;
    text-align: center;
    padding: 2px 5px;
    border-radius: 4px;
    flex-shrink: 0;
  }
  .rv-key-name {
    font-size: 12px;
    color: var(--t1);
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .rv-key-ttl { font-size: 10px; color: var(--t4); font-family: var(--mono); flex-shrink: 0; }
  .rv-key-del {
    width: 20px; height: 20px;
    border: none; background: transparent; color: var(--t4);
    cursor: default; display: flex; align-items: center; justify-content: center;
    border-radius: 4px; opacity: 0; transition: opacity 0.1s, color 0.1s;
  }
  .rv-key-item:hover .rv-key-del { opacity: 1; }
  .rv-key-del svg { stroke: currentColor; fill: none; stroke-width: 2; stroke-linecap: round; }
  .rv-key-del:hover { color: var(--err); background: color-mix(in srgb, var(--err) 10%, transparent); }
  .rv-load-more {
    width: 100%; padding: 10px; border: none; background: transparent;
    color: var(--acc); font-size: 11px; font-family: var(--mono);
    cursor: default; transition: background 0.1s;
  }
  .rv-load-more:hover { background: color-mix(in srgb, var(--acc) 5%, transparent); }
  .rv-load-more:disabled { opacity: 0.4; }

  /* ── Value pane ── */
  .rv-value-pane { width: 55%; display: flex; flex-direction: column; overflow: hidden; }
  .rv-value-header {
    height: 34px; flex-shrink: 0; background: var(--n2);
    border-bottom: 1px solid var(--b1); padding: 0 12px;
    display: flex; align-items: center; gap: 8px;
  }
  .rv-value-key {
    font-size: 12px; color: var(--t1); font-family: var(--mono); font-weight: 600;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .rv-value-type {
    font-size: 9px; font-weight: 700; font-family: var(--mono);
    text-transform: uppercase; padding: 2px 5px; border-radius: 4px; flex-shrink: 0;
  }
  .rv-value-ttl { font-size: 10px; color: var(--t4); font-family: var(--mono); margin-left: auto; flex-shrink: 0; }
  .rv-value-copy {
    width: 24px; height: 24px; border: none; background: transparent;
    color: var(--t3); cursor: default; display: flex; align-items: center;
    justify-content: center; border-radius: 5px; flex-shrink: 0;
    transition: color 0.1s, background 0.1s;
  }
  .rv-value-copy svg { stroke: currentColor; fill: none; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .rv-value-copy:hover { color: var(--t1); background: var(--b1); }
  .rv-value-body {
    flex: 1; min-height: 0; padding: 12px 14px;
    font-family: var(--mono); font-size: 11.5px; line-height: 1.65;
    white-space: pre-wrap; word-break: break-word; color: var(--t1);
    overflow-y: auto; -webkit-user-select: text; user-select: text; cursor: text;
  }
  .rv-value-body::-webkit-scrollbar { width: 4px; }
  .rv-value-body::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }
  /* JSON highlighting in value pane */
  .rv-value-body :global(.key)  { color: #c4a7e7; }
  .rv-value-body :global(.str)  { color: #9ccfd8; }
  .rv-value-body :global(.num)  { color: #f6c177; }
  .rv-value-body :global(.boo)  { color: #eb6f92; }
  .rv-value-body :global(.pu)   { color: var(--t4); }

  /* ── Console ── */
  .rv-console {
    flex: 1; overflow-y: auto;
    background: color-mix(in srgb, var(--n) 80%, black);
    padding: 14px;
  }
  .rv-console::-webkit-scrollbar { width: 4px; }
  .rv-console::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }
  .rv-console-welcome {
    display: flex; flex-direction: column; gap: 4px; padding-bottom: 14px;
    border-bottom: 1px solid var(--b1); margin-bottom: 14px;
  }
  .rv-console-welcome-title { font-family: var(--mono); font-size: 13px; color: var(--acc); font-weight: 600; }
  .rv-console-welcome-hint { font-family: var(--mono); font-size: 11px; color: var(--t4); }
  .rv-console-entry { margin-bottom: 10px; }
  .rv-console-cmd { font-family: var(--mono); font-size: 12px; color: var(--t1); }
  .rv-console-prompt-inline { color: var(--acc); font-weight: 700; }
  .rv-console-result {
    font-family: var(--mono); font-size: 12px; color: var(--t2);
    white-space: pre-wrap; word-break: break-word; padding-left: 18px; margin-top: 3px;
    -webkit-user-select: text; user-select: text; cursor: text;
  }
  .rv-console-result.error { color: var(--err); }
  .rv-console-input-bar {
    height: 38px; flex-shrink: 0; background: var(--n2);
    border-top: 1px solid var(--b1); padding: 0 10px;
    display: flex; align-items: center; gap: 6px;
  }
  .rv-console-prompt { color: var(--acc); font-family: var(--mono); font-size: 14px; font-weight: 700; flex-shrink: 0; }
  .rv-console-input {
    flex: 1; height: 26px; background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1); border-radius: 5px; padding: 0 8px;
    font-family: var(--mono); font-size: 12px; color: var(--t1); outline: none;
  }
  .rv-console-input:focus { border-color: var(--acc); }
  .rv-console-input::placeholder { color: var(--t4); }

  /* ── Modal ── */
  .rv-modal-overlay {
    position: fixed; top: 0; left: 0; right: 0; bottom: 0;
    background: rgba(0,0,0,0.45); backdrop-filter: blur(8px);
    z-index: 1000; display: flex; align-items: center; justify-content: center;
    animation: rvFadeIn 0.12s ease;
  }
  @keyframes rvFadeIn { from { opacity: 0; } to { opacity: 1; } }
  .rv-modal {
    background: var(--modal-bg, var(--n)); border: 1px solid var(--b1);
    border-radius: var(--radius-xl); box-shadow: 0 24px 48px rgba(0,0,0,0.45);
    width: 480px; max-height: 80vh; display: flex; flex-direction: column; overflow: hidden;
    animation: rvSlideIn 0.15s ease;
  }
  @keyframes rvSlideIn { from { opacity: 0; transform: translateY(-8px) scale(0.98); } to { opacity: 1; transform: translateY(0) scale(1); } }
  .rv-modal-hdr {
    display: flex; align-items: center; padding: 14px 18px;
    border-bottom: 1px solid var(--b1); background: var(--n2);
    font-size: 14px; font-weight: 600; color: var(--t1); font-family: var(--ui);
  }
  .rv-modal-close {
    margin-left: auto; width: 28px; height: 28px; border-radius: 6px;
    border: 1px solid var(--b1); background: transparent; cursor: default;
    color: var(--t3); font-size: 16px; display: flex; align-items: center;
    justify-content: center; transition: background 0.1s, color 0.1s;
  }
  .rv-modal-close:hover { background: var(--c); color: var(--t1); }
  .rv-modal-body { padding: 18px; display: flex; flex-direction: column; gap: 12px; }
  .rv-modal-actions {
    display: flex; justify-content: flex-end; gap: 8px;
    padding: 14px 18px; border-top: 1px solid var(--b1);
  }
  .rv-field { display: flex; flex-direction: column; gap: 4px; }
  .rv-field-label {
    font-size: 11px; font-weight: 600; color: var(--t2); font-family: var(--ui);
    text-transform: uppercase; letter-spacing: 0.05em;
  }
  .rv-field-input {
    height: 32px; background: rgba(255,255,255,0.04); border: 1px solid var(--b1);
    border-radius: var(--radius-md); padding: 0 10px; font-size: 12px;
    color: var(--t1); font-family: var(--mono); outline: none;
  }
  .rv-field-input:focus { border-color: var(--acc); }
  .rv-field-ta {
    min-height: 80px; background: rgba(255,255,255,0.04); border: 1px solid var(--b1);
    border-radius: var(--radius-md); padding: 8px 10px; font-size: 12px;
    color: var(--t1); font-family: var(--mono); outline: none; resize: none;
  }
  .rv-field-ta:focus { border-color: var(--acc); }
  .rv-modal-cancel {
    height: 30px; padding: 0 14px; border-radius: 6px; border: 1px solid var(--b1);
    background: transparent; color: var(--t2); font-size: 11.5px; font-family: var(--ui);
    cursor: default; transition: border-color 0.12s, color 0.12s;
  }
  .rv-modal-cancel:hover { border-color: var(--b2); color: var(--t1); }
  .rv-modal-primary {
    height: 30px; padding: 0 16px; border-radius: 6px; border: none;
    background: var(--acc); color: #fff; font-size: 11.5px; font-weight: 600;
    font-family: var(--ui); cursor: default; transition: opacity 0.12s;
  }
  .rv-modal-primary:hover { opacity: 0.85; }
</style>

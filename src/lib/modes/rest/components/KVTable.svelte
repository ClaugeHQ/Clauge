<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { KVInput } from '$lib/types';
  import EnvInput from '$lib/components/shared/EnvInput.svelte';
  import { KV_DEBOUNCE_MS, SUGGEST_CLOSE_MS } from '$lib/shared/constants/timings';

  let { items = [], label = 'Add item', showKeySuggestions = false, onchange }: {
    items: KVInput[];
    label?: string;
    showKeySuggestions?: boolean;
    onchange: (items: KVInput[]) => void;
  } = $props();

  let localItems: KVInput[] = $state([]);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let localChange = false;

  onDestroy(() => {
    if (debounceTimer) clearTimeout(debounceTimer);
  });

  // Header name suggestions
  const COMMON_HEADERS = [
    'Accept', 'Accept-Charset', 'Accept-Encoding', 'Accept-Language',
    'Authorization', 'Cache-Control', 'Connection', 'Content-Disposition',
    'Content-Encoding', 'Content-Language', 'Content-Length', 'Content-Type',
    'Cookie', 'Date', 'ETag', 'Expect', 'Forwarded', 'From', 'Host',
    'If-Match', 'If-Modified-Since', 'If-None-Match', 'If-Range',
    'If-Unmodified-Since', 'Keep-Alive', 'Origin', 'Pragma', 'Proxy-Authorization',
    'Range', 'Referer', 'TE', 'Trailer', 'Transfer-Encoding', 'Upgrade',
    'User-Agent', 'Via', 'Warning', 'X-Forwarded-For', 'X-Forwarded-Host',
    'X-Forwarded-Proto', 'X-Request-ID', 'X-Requested-With',
  ];

  let suggestIdx = $state(-1);
  let suggestOpen = $state(false);
  let suggestFilter = $state('');
  let suggestRowIdx = $state(-1);

  const suggestItems = $derived(
    suggestFilter
      ? COMMON_HEADERS.filter(h => h.toLowerCase().includes(suggestFilter.toLowerCase())).slice(0, 8)
      : []
  );

  $effect(() => {
    if (localChange) {
      localChange = false;
      return;
    }
    localItems = items.length > 0
      ? items.map(i => ({ ...i }))
      : [{ key: '', value: '', enabled: 0 }];
  });

  function debounceSave() {
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      localChange = true;
      const toSave = localItems.filter(i => i.key.trim() !== '' || i.value.trim() !== '');
      onchange(toSave);
    }, KV_DEBOUNCE_MS);
  }

  function updateKey(index: number, value: string) {
    localItems[index].key = value;
    if (showKeySuggestions && value.length > 0) {
      suggestFilter = value;
      suggestRowIdx = index;
      suggestIdx = 0;
      suggestOpen = suggestItems.length > 0;
    } else {
      suggestOpen = false;
    }
    debounceSave();
  }

  function updateValue(index: number, value: string) {
    localItems[index].value = value;
    debounceSave();
  }

  function toggleEnabled(index: number) {
    localItems[index].enabled = localItems[index].enabled ? 0 : 1;
    debounceSave();
  }

  function removeRow(index: number) {
    localItems.splice(index, 1);
    if (localItems.length === 0) {
      localItems.push({ key: '', value: '', enabled: 0 });
    }
    suggestOpen = false;
    debounceSave();
  }

  function addRow() {
    localItems.push({ key: '', value: '', enabled: 0 });
  }

  function selectSuggestion(header: string) {
    if (suggestRowIdx >= 0 && suggestRowIdx < localItems.length) {
      localItems[suggestRowIdx].key = header;
      localItems[suggestRowIdx].enabled = 1;
      suggestOpen = false;
      debounceSave();
    }
  }

  function handleKeyKeydown(e: KeyboardEvent, index: number) {
    if (!suggestOpen || suggestItems.length === 0) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      suggestIdx = Math.min(suggestIdx + 1, suggestItems.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      suggestIdx = Math.max(suggestIdx - 1, 0);
    } else if (e.key === 'Enter' && suggestOpen) {
      e.preventDefault();
      selectSuggestion(suggestItems[suggestIdx]);
    } else if (e.key === 'Escape') {
      suggestOpen = false;
    }
  }

  function handleKeyBlur() {
    setTimeout(() => { suggestOpen = false; }, SUGGEST_CLOSE_MS);
  }
</script>

<div class="kv-editor">
  <div class="kv-rows">
    {#each localItems as item, i (i)}
      <div class="kv-row" class:disabled={!item.enabled}>
        <input
          type="checkbox"
          class="kv-check"
          class:kv-check-dim={item.key.trim() === '' && item.value.trim() === ''}
          checked={!!item.enabled}
          onchange={() => toggleEnabled(i)}
        />
        <div class="kv-key-wrap">
          <input
            class="kv-input kv-key"
            type="text"
            placeholder="Key"
            value={item.key}
            oninput={(e) => updateKey(i, (e.target as HTMLInputElement).value)}
            onkeydown={(e) => handleKeyKeydown(e, i)}
            onblur={handleKeyBlur}
            spellcheck="false"
          />
          {#if suggestOpen && suggestRowIdx === i && suggestItems.length > 0}
            <div class="kv-suggest">
              {#each suggestItems as header, si (header)}
                <button
                  class="kv-suggest-item"
                  class:active={si === suggestIdx}
                  onmousedown={(e) => { e.preventDefault(); selectSuggestion(header); }}
                  onmouseenter={() => { suggestIdx = si; }}
                >
                  {header}
                </button>
              {/each}
            </div>
          {/if}
        </div>
        <div class="kv-val">
          <EnvInput
            value={item.value}
            placeholder="Value"
            onchange={(v) => updateValue(i, v)}
          />
        </div>
        <button class="kv-del" onclick={() => removeRow(i)} title="Remove">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>
    {/each}
  </div>
  <button class="kv-add" onclick={addRow}>+ {label}</button>
</div>

<style>
  .kv-editor {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow-y: auto;
    padding: 8px 10px;
  }
  .kv-editor::-webkit-scrollbar { width: 3px; }
  .kv-editor::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .kv-rows {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .kv-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 30px;
  }
  .kv-row.disabled .kv-input,
  .kv-row.disabled :global(.env-editor) {
    opacity: 0.4;
  }

  .kv-check {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    accent-color: var(--acc);
    cursor: pointer;
  }
  .kv-check-dim { opacity: 0.25; }

  .kv-key-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
  }

  .kv-input {
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--b1);
    border-radius: 5px;
    background: transparent;
    color: var(--t1);
    font-size: 12px;
    font-family: var(--mono);
    outline: none;
    transition: border-color 0.15s;
    box-sizing: border-box;
  }
  .kv-input:focus { border-color: var(--acc); }
  .kv-input::placeholder { color: var(--t4); }

  .kv-val {
    flex: 2;
    min-width: 0;
  }

  .kv-del {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--t4);
    border-radius: 4px;
    cursor: pointer;
    flex-shrink: 0;
    transition: color 0.1s, background 0.1s;
  }
  .kv-del:hover { color: var(--err); background: rgba(255,255,255,0.05); }

  .kv-add {
    margin-top: 6px;
    align-self: flex-start;
    padding: 4px 10px;
    border: 1px dashed var(--b1);
    border-radius: 5px;
    background: transparent;
    color: var(--t3);
    font-size: 11px;
    font-family: var(--ui);
    cursor: pointer;
    transition: color 0.1s, border-color 0.1s;
  }
  .kv-add:hover { color: var(--t1); border-color: var(--b2); }

  /* Suggestion dropdown */
  .kv-suggest {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    margin-top: 2px;
    background: var(--n);
    border: 1px solid var(--b1);
    border-radius: 5px;
    z-index: 500;
    max-height: 180px;
    overflow-y: auto;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.4);
  }
  .kv-suggest-item {
    width: 100%;
    padding: 6px 10px;
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--t2);
    border: none;
    background: transparent;
    text-align: left;
    cursor: default;
    transition: background 0.08s;
  }
  .kv-suggest-item:hover,
  .kv-suggest-item.active {
    background: var(--c);
    color: var(--t1);
  }
</style>

<script lang="ts">
  import { activeRequest, requestEnvOverrides } from '$lib/modes/rest/stores';
  import { activeEnvId, getEffectiveEnvId } from '$lib/modes/rest/stores';
  import { getEnvVariablesForResolution } from '$lib/modes/rest/commands';
  import { METHOD_COLORS, METHOD_COLORS_LIGHT } from '$lib/utils/theme';
  import { appearance } from '$lib/stores/settings';
  import { get } from 'svelte/store';
  import { activeTabId, tabs, markDirty, setDraft, getDraft, updateTab } from '$lib/shared/stores/tabs';
  import { parseCurl } from '$lib/modes/rest/utils/curl-parser';
  import { showToast } from '$lib/components/shared/toast';
  import ReqEnvPill from './ReqEnvPill.svelte';
  import { tick } from 'svelte';

  interface Props {
    onsend?: () => void;
    onmethodchange?: (method: string) => void;
  }

  let { onsend, onmethodchange }: Props = $props();

  const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'] as const;

  let envVars = $state<Record<string, string>>({});
  let envVarEntries = $state<{ key: string; value: string }[]>([]);
  let acOpen = $state(false);
  let acFilter = $state('');
  let acSelectedIdx = $state(0);
  let editorEl = $state<HTMLDivElement | null>(null);
  let urlFocused = $state(false);
  let suppressRender = false;
  let justSelected = false;

  let localMethod = $state('GET');
  let localUrl = $state('');
  let prevTabId = -1;
  let prevReqId = '';

  // Reset local state on tab switch or when active request changes
  $effect(() => {
    const tabId = $activeTabId;
    const req = $activeRequest;
    const reqId = req?.id || '';
    const isTabSwitch = tabId !== prevTabId;
    const isReqChange = reqId !== prevReqId && reqId !== '';

    if (isTabSwitch || isReqChange) {
      prevTabId = tabId;
      prevReqId = reqId;
      if (req) {
        localMethod = req.method;
        localUrl = req.url;
      } else {
        // Unsaved tab — restore from draft or reset
        const draft = getDraft(tabId);
        localMethod = draft?.method || 'GET';
        localUrl = draft?.url || '';
      }
      // Re-render editor with correct URL
      if (editorEl) {
        suppressRender = true;
        renderToEditor(req?.url ?? localUrl);
        requestAnimationFrame(() => { suppressRender = false; });
      }
    }
  });

  const method = $derived($activeRequest?.method ?? localMethod);
  const url = $derived($activeRequest?.url ?? localUrl);
  const activeMethodColors = $derived($appearance?.theme === 'light' ? METHOD_COLORS_LIGHT : METHOD_COLORS);
  const methodColor = $derived(activeMethodColors[method] ?? activeMethodColors.GET);

  const overrideKey = $derived($activeRequest?.id ?? String($activeTabId));
  const effectiveEnvId = $derived(
    getEffectiveEnvId(overrideKey, $requestEnvOverrides, $activeEnvId)
  );

  let envFetchVersion = 0;
  $effect(() => {
    const envId = effectiveEnvId;
    const version = ++envFetchVersion;
    if (envId) {
      getEnvVariablesForResolution(envId).then(vars => {
        if (version === envFetchVersion) {
          envVars = vars;
          envVarEntries = Object.entries(vars).map(([key, value]) => ({ key, value }));
        }
      }).catch(() => {
        if (version === envFetchVersion) {
          envVars = {};
          envVarEntries = [];
        }
      });
    } else {
      envVars = {};
      envVarEntries = [];
    }
  });

  const acItems = $derived(
    acFilter
      ? envVarEntries.filter(v => v.key.toLowerCase().includes(acFilter.toLowerCase()))
      : envVarEntries
  );

  // Render URL into editor with syntax coloring + variable chips
  function renderToEditor(value: string) {
    if (!editorEl) return;
    if (!value) {
      editorEl.innerHTML = '';
      return;
    }

    let html = '';
    // Split by {{var}} patterns
    const parts = value.split(/(\{\{[^}]*\}\})/g);
    for (const part of parts) {
      if (part.startsWith('{{') && part.endsWith('}}')) {
        const varName = part.slice(2, -2);
        html += `<span class="url-var" contenteditable="false" data-var="${esc(varName)}">${esc(varName)}</span>`;
      } else {
        html += colorUrlSegment(part);
      }
    }
    editorEl.innerHTML = html;

    // Only add zero-width spaces around chips (needed for cursor positioning near chips)
    const chips = editorEl.querySelectorAll('.url-var');
    if (chips.length > 0) {
      for (const chip of chips) {
        const next = chip.nextSibling;
        if (!next || next.nodeType !== Node.TEXT_NODE) {
          chip.after(document.createTextNode('\u200B'));
        }
      }
      const last = editorEl.lastChild;
      if (!last || last.nodeType !== Node.TEXT_NODE) {
        editorEl.appendChild(document.createTextNode('\u200B'));
      }
    }
  }

  function esc(s: string): string {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  function colorUrlSegment(text: string): string {
    if (!text) return '';
    let result = '';
    let rest = text;

    // Protocol
    const pm = rest.match(/^(https?:\/\/)/);
    if (pm) {
      result += `<span class="url-proto">${esc(pm[1])}</span>`;
      rest = rest.slice(pm[1].length);
    }

    // Split at query string
    const qi = rest.indexOf('?');
    const before = qi >= 0 ? rest.slice(0, qi) : rest;
    const query = qi >= 0 ? rest.slice(qi) : '';

    // Host + path
    const si = before.indexOf('/');
    if (si >= 0) {
      result += `<span class="url-host">${esc(before.slice(0, si))}</span>`;
      result += `<span class="url-path">${esc(before.slice(si))}</span>`;
    } else {
      result += `<span class="url-host">${esc(before)}</span>`;
    }

    // Query
    if (query) {
      result += `<span class="url-qmark">?</span>`;
      const pairs = query.slice(1).split('&');
      for (let i = 0; i < pairs.length; i++) {
        if (i > 0) result += `<span class="url-qmark">&amp;</span>`;
        const ei = pairs[i].indexOf('=');
        if (ei >= 0) {
          result += `<span class="url-qkey">${esc(pairs[i].slice(0, ei))}</span>`;
          result += `<span class="url-qmark">=</span>`;
          result += `<span class="url-qval">${esc(pairs[i].slice(ei + 1))}</span>`;
        } else {
          result += `<span class="url-qkey">${esc(pairs[i])}</span>`;
        }
      }
    }

    return result;
  }

  // Extract plain text from editor (chips → {{varName}})
  function extractValue(): string {
    if (!editorEl) return '';
    return walkNodes(editorEl).replace(/\u200B/g, '');
  }

  function walkNodes(parent: Node): string {
    let result = '';
    for (const node of parent.childNodes) {
      if (node.nodeType === Node.TEXT_NODE) {
        result += node.textContent ?? '';
      } else if (node instanceof HTMLElement) {
        if (node.dataset.var !== undefined) {
          // Variable chip — convert back to {{varName}}
          result += `{{${node.dataset.var}}}`;
        } else {
          // Color span or other wrapper — recurse into children
          result += walkNodes(node);
        }
      }
    }
    return result;
  }

  // Sync value changes into the editor (only for saved requests when URL changes externally)
  $effect(() => {
    const v = url;
    const el = editorEl;
    const req = $activeRequest;
    // Only re-render for saved requests — unsaved tabs are handled by handleInput
    if (!suppressRender && el && req) {
      renderToEditor(v);
    }
  });

  // Get cursor offset in plain text (treating chips as {{var}})
  function getCursorOffset(): number {
    if (!editorEl) return -1;
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return -1;
    const range = sel.getRangeAt(0);

    let offset = 0;
    const walker = document.createTreeWalker(editorEl, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
    let node: Node | null;
    while ((node = walker.nextNode())) {
      if (node instanceof HTMLElement && node.dataset.var !== undefined) {
        if (node.contains(range.startContainer) || node === range.startContainer) {
          return offset + node.dataset.var.length + 4; // after {{var}}
        }
        offset += node.dataset.var.length + 4; // {{var}}
        // Skip children of chip
        walker.nextNode(); // skip the text node inside chip
        continue;
      }
      if (node.nodeType === Node.TEXT_NODE) {
        if (node === range.startContainer) {
          return offset + range.startOffset;
        }
        offset += (node.textContent ?? '').length;
      }
    }
    return offset;
  }

  // Restore cursor to a plain-text offset
  function restoreCursor(targetOffset: number) {
    if (!editorEl || targetOffset < 0) return;
    const sel = window.getSelection();
    if (!sel) return;

    let offset = 0;
    const walker = document.createTreeWalker(editorEl, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
    let node: Node | null;
    while ((node = walker.nextNode())) {
      if (node instanceof HTMLElement && node.dataset.var !== undefined) {
        const chipLen = node.dataset.var.length + 4;
        if (targetOffset <= offset + chipLen) {
          // Place cursor after chip
          const parent = node.parentNode!;
          const idx = Array.from(parent.childNodes).indexOf(node as ChildNode);
          const range = document.createRange();
          range.setStart(parent, idx + 1);
          range.collapse(true);
          sel.removeAllRanges();
          sel.addRange(range);
          return;
        }
        offset += chipLen;
        walker.nextNode(); // skip text inside chip
        continue;
      }
      if (node.nodeType === Node.TEXT_NODE) {
        const len = (node.textContent ?? '').length;
        if (targetOffset <= offset + len) {
          const range = document.createRange();
          range.setStart(node, targetOffset - offset);
          range.collapse(true);
          sel.removeAllRanges();
          sel.addRange(range);
          return;
        }
        offset += len;
      }
    }
    // Fallback: place at end
    const range = document.createRange();
    range.selectNodeContents(editorEl);
    range.collapse(false);
    sel.removeAllRanges();
    sel.addRange(range);
  }

  function handleInput() {
    suppressRender = true;
    const cursorPos = getCursorOffset();
    const value = extractValue();

    const tabId = $activeTabId;
    if ($activeRequest) {
      activeRequest.update(r => r ? { ...r, url: value } : r);
      setDraft(tabId, { url: value });
      markDirty(tabId);
    } else {
      localUrl = value;
      setDraft(tabId, { url: value });
      markDirty(tabId);
    }

    // Check for autocomplete trigger
    const textBefore = value.slice(0, cursorPos);
    const varMatch = textBefore.match(/\{\{(\w*)$/);
    if (varMatch) {
      acFilter = varMatch[1];
      acSelectedIdx = 0;
      acOpen = true;
    } else {
      acOpen = false;
    }

    // Re-render with colors and restore cursor
    if (!acOpen) {
      renderToEditor(value);
      restoreCursor(cursorPos);
    }

    requestAnimationFrame(() => { suppressRender = false; });
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (acOpen && acItems.length > 0) {
        insertAutocomplete(acItems[acSelectedIdx].key);
      } else if (!e.metaKey && !e.ctrlKey) {
        // Plain Enter sends. Cmd+Enter is handled by RestPanel's global shortcut.
        onsend?.();
      }
      return;
    }

    if (!acOpen) return;

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      acSelectedIdx = Math.min(acSelectedIdx + 1, acItems.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      acSelectedIdx = Math.max(acSelectedIdx - 1, 0);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      acOpen = false;
    }
  }

  function handleFocus() {
    urlFocused = true;
  }

  function handleBlur() {
    setTimeout(() => {
      urlFocused = false;
      acOpen = false;
      // If insertAutocomplete just ran, don't re-render (it already rendered correctly)
      if (justSelected) {
        justSelected = false;
        return;
      }
      suppressRender = false;
      renderToEditor(url);
    }, 150);
  }

  function insertAutocomplete(varName: string) {
    if (!editorEl) return;
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return;

    // Find and remove the partial {{ before cursor
    const range = sel.getRangeAt(0);
    const textNode = range.startContainer;
    if (textNode.nodeType !== Node.TEXT_NODE) return;

    const text = textNode.textContent ?? '';
    const offset = range.startOffset;
    const before = text.slice(0, offset);
    const braceStart = before.lastIndexOf('{{');
    if (braceStart === -1) return;

    // Split: keep text before {{, remove {{ and partial
    const keepBefore = text.slice(0, braceStart);
    const keepAfter = text.slice(offset);

    // Create the chip
    const chip = document.createElement('span');
    chip.className = 'url-var';
    chip.contentEditable = 'false';
    chip.dataset.var = varName;
    chip.textContent = varName;

    // Replace text node
    const parent = textNode.parentNode!;
    const beforeNode = document.createTextNode(keepBefore);
    const afterNode = document.createTextNode(keepAfter);
    parent.insertBefore(beforeNode, textNode);
    parent.insertBefore(chip, textNode);
    parent.insertBefore(afterNode, textNode);
    parent.removeChild(textNode);

    acOpen = false;
    justSelected = true;

    // Extract value and figure out cursor position
    suppressRender = true;
    const newValue = extractValue();
    // Find where {{varName}} ends in the full plain text
    const chipPattern = `{{${varName}}}`;
    const chipIdx = newValue.lastIndexOf(chipPattern);
    const cursorTarget = chipIdx >= 0 ? chipIdx + chipPattern.length : newValue.length;

    const tabId = $activeTabId;
    if ($activeRequest) {
      activeRequest.update(r => r ? { ...r, url: newValue } : r);
      setDraft(tabId, { url: newValue });
      markDirty(tabId);
    } else {
      localUrl = newValue;
      setDraft(tabId, { url: newValue });
      markDirty(tabId);
    }

    renderToEditor(newValue);
    restoreCursor(cursorTarget);
    suppressRender = false;
  }

  function handlePaste(e: ClipboardEvent) {
    const text = e.clipboardData?.getData('text');
    if (!text) return;

    const trimmed = text.trim();
    if (trimmed.startsWith('curl ') || trimmed.startsWith('curl\t')) {
      e.preventDefault();
      const parsed = parseCurl(trimmed);
      if (!parsed) return;

      const tabId = $activeTabId;

      if ($activeRequest) {
        activeRequest.update(r => r ? {
          ...r,
          url: parsed.url,
          method: parsed.method,
          body: parsed.body,
          bodyType: parsed.bodyType,
          authType: parsed.authType,
          authData: parsed.authData,
          headers: parsed.headers.map((h, i) => ({ id: '', requestId: r.id, key: h.key, value: h.value, enabled: h.enabled, sortOrder: i })),
        } : r);
      }
      localUrl = parsed.url;
      localMethod = parsed.method;

      // Full replace — clear all fields, then set what the cURL provides
      setDraft(tabId, {
        method: parsed.method,
        url: parsed.url,
        headers: parsed.headers,
        body: parsed.body,
        bodyType: parsed.bodyType,
        authType: parsed.authType,
        authData: parsed.authData,
        params: [],
      });
      markDirty(tabId);
      onmethodchange?.(parsed.method);

      const _mc = get(appearance)?.theme === 'light' ? METHOD_COLORS_LIGHT : METHOD_COLORS;
      const newColor = _mc[parsed.method] ?? _mc.GET;
      const name = $activeRequest?.name ?? (parsed.url || 'New Request');
      updateTab(tabId, { dot: newColor.color, label: parsed.method + ' ' + name });

      // Render the URL into the editor and place cursor at end
      renderToEditor(parsed.url);
      if (editorEl) {
        const sel = window.getSelection();
        if (sel) {
          const range = document.createRange();
          range.selectNodeContents(editorEl);
          range.collapse(false);
          sel.removeAllRanges();
          sel.addRange(range);
        }
      }

      showToast('Imported from cURL', 'success');
      return;
    }

    // Normal paste — insert as plain text
    e.preventDefault();
    document.execCommand('insertText', false, text);
  }

  function handleMethodChange(e: Event) {
    const target = e.target as HTMLSelectElement;
    const newMethod = target.value;
    const tabId = $activeTabId;
    if ($activeRequest) {
      activeRequest.update(r => r ? { ...r, method: newMethod } : r);
      setDraft(tabId, { method: newMethod });
      markDirty(tabId);
    } else {
      localMethod = newMethod;
      setDraft(tabId, { method: newMethod });
      markDirty(tabId);
    }
    const _mc2 = get(appearance)?.theme === 'light' ? METHOD_COLORS_LIGHT : METHOD_COLORS;
    const newColor = _mc2[newMethod] ?? _mc2.GET;
    const name = $activeRequest?.name ?? (localUrl || 'New Request');
    updateTab(tabId, { dot: newColor.color, label: newMethod + ' ' + name });
    onmethodchange?.(newMethod);
  }

  function handleSend() {
    onsend?.();
  }
</script>

<div class="req-bar">
  <select
    class="method-sel"
    value={method}
    onchange={handleMethodChange}
    style:color={methodColor.color}
    style:background={methodColor.bg}
    style:border-color="var(--b1)"
  >
    {#each METHODS as m}
      <option value={m}>{m}</option>
    {/each}
  </select>

  <div class="url-wrap">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      bind:this={editorEl}
      class="url-editor"
      class:url-focused={urlFocused}
      contenteditable="true"
      role="textbox"
      tabindex="0"
      data-placeholder="Enter URL or &#123;&#123;variable&#125;&#125;"
      oninput={handleInput}
      onkeydown={handleKeydown}
      onpaste={handlePaste}
      onfocus={handleFocus}
      onblur={handleBlur}
      spellcheck="false"
    ></div>

    {#if acOpen && acItems.length > 0}
      <div class="ac-dropdown">
        {#each acItems as item, i (item.key)}
          <button
            class="ac-item"
            class:active={i === acSelectedIdx}
            onmousedown={(e: MouseEvent) => { e.preventDefault(); insertAutocomplete(item.key); }}
            onmouseenter={() => { acSelectedIdx = i; }}
          >
            <span class="ac-item-name">{item.key}</span>
            <span class="ac-item-val">{item.value}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <ReqEnvPill />

  <button class="send-btn" onclick={handleSend}>
    Send &#9654;
  </button>
</div>

<style>
  .req-bar {
    min-height: 44px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    padding: 9px 14px;
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }

  .method-sel {
    height: 32px;
    padding: 0 8px;
    border-radius: 5px;
    border: 1px solid;
    font-family: var(--mono);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.04em;
    cursor: default;
    outline: none;
    flex-shrink: 0;
  }

  .url-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
  }

  .url-editor {
    min-height: 32px;
    max-height: 32px;
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md);
    padding: 8px 14px;
    font-family: var(--mono);
    font-size: 13px;
    line-height: 18px;
    color: var(--t1);
    outline: none;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    box-sizing: border-box;
    transition: border-color 0.2s ease, box-shadow 0.2s ease, background 0.2s ease;
    cursor: text;
  }
  .url-editor:empty::before {
    content: attr(data-placeholder);
    color: var(--t3);
    pointer-events: none;
  }
  .url-editor.url-focused {
    white-space: pre-wrap;
    word-break: break-all;
    text-overflow: clip;
    max-height: 108px;
    overflow-y: auto;
    border-color: var(--acc);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 15%, transparent);
    background: rgba(255,255,255,0.06);
  }

  /* URL syntax colors */
  :global(.url-proto) { color: var(--t3); }
  :global(.url-host) { color: var(--t1); font-weight: 500; }
  :global(.url-path) { color: #60a5fa; }
  :global(.url-qmark) { color: var(--t3); }
  :global(.url-qkey) { color: #a78bfa; }
  :global(.url-qval) { color: #4ade80; }

  /* Variable chip */
  :global(.url-var) {
    display: inline-block;
    background: color-mix(in srgb, var(--acc) 18%, transparent);
    color: var(--acc);
    border-radius: 3px;
    padding: 0 5px;
    font-size: 11px;
    font-weight: 600;
    line-height: 17px;
    vertical-align: baseline;
    cursor: default;
    user-select: all;
  }

  .ac-dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    background: var(--n);
    border: 1px solid var(--b1);
    border-radius: 6px;
    margin-top: 4px;
    z-index: 500;
    max-height: 200px;
    overflow-y: auto;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  }

  .ac-item {
    width: 100%;
    padding: 7px 12px;
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: default;
    transition: background 0.08s;
    font-family: var(--mono);
    font-size: 12.5px;
    border: none;
    background: transparent;
    text-align: left;
  }
  .ac-item:hover,
  .ac-item.active {
    background: var(--c);
  }
  .ac-item-name {
    color: var(--t1);
  }
  .ac-item-val {
    color: var(--t3);
    font-size: 11.5px;
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 200px;
  }

  .send-btn {
    height: 32px;
    padding: 0 20px;
    border-radius: 8px;
    border: none;
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 600;
    cursor: default;
    transition: opacity 0.12s;
    flex-shrink: 0;
    color: #fff;
    background: var(--acc);
  }
  .send-btn:hover {
    opacity: 0.85;
  }
  .send-btn:active {
    opacity: 0.75;
  }
</style>

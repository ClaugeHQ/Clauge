<script lang="ts">
  import { activeEnvId, getEffectiveEnvId } from '$lib/modes/rest/stores';
  import { activeRequest, requestEnvOverrides } from '$lib/modes/rest/stores';
  import { activeTabId } from '$lib/shared/stores/tabs';
  import { getEnvVariablesForResolution } from '$lib/modes/rest/commands';

  let { value = '', placeholder = '', type = 'text', onchange }: {
    value: string;
    placeholder?: string;
    type?: 'text' | 'password';
    onchange: (value: string) => void;
  } = $props();

  let editorEl = $state<HTMLDivElement | null>(null);
  let acOpen = $state(false);
  let acFilter = $state('');
  let acIdx = $state(0);
  let envVarEntries = $state<{ key: string; value: string }[]>([]);
  let suppressRender = false;
  let justSelected = false;
  let dropdownStyle = $state('');

  const overrideKey = $derived($activeRequest?.id ?? String($activeTabId));
  const effectiveEnvId = $derived(
    getEffectiveEnvId(overrideKey, $requestEnvOverrides, $activeEnvId)
  );

  let fetchVersion = 0;
  $effect(() => {
    const envId = effectiveEnvId;
    const version = ++fetchVersion;
    if (envId) {
      getEnvVariablesForResolution(envId).then(vars => {
        // Only apply if this is still the latest fetch (prevents stale overwrites)
        if (version === fetchVersion) {
          envVarEntries = Object.entries(vars).map(([key, value]) => ({ key, value }));
        }
      }).catch(() => {
        if (version === fetchVersion) envVarEntries = [];
      });
    } else {
      envVarEntries = [];
    }
  });

  const acItems = $derived(
    acFilter
      ? envVarEntries.filter(v => v.key.toLowerCase().includes(acFilter.toLowerCase())).slice(0, 8)
      : envVarEntries.slice(0, 8)
  );

  function esc(s: string): string {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  function renderToEditor(val: string) {
    if (!editorEl) return;
    if (!val) { editorEl.innerHTML = ''; return; }

    let html = '';
    const parts = val.split(/(\{\{[^}]*\}\})/g);
    for (const part of parts) {
      if (part.startsWith('{{') && part.endsWith('}}')) {
        const varName = part.slice(2, -2);
        html += `<span class="url-var" contenteditable="false" data-var="${esc(varName)}">${esc(varName)}</span>`;
      } else {
        if (type === 'password') {
          html += esc('\u2022'.repeat(part.length));
        } else {
          html += esc(part);
        }
      }
    }
    editorEl.innerHTML = html;

    // Only add zero-width spaces around chips (needed for cursor positioning near chips)
    const chips = editorEl.querySelectorAll('.url-var');
    if (chips.length > 0) {
      for (const chip of chips) {
        if (!chip.nextSibling || chip.nextSibling.nodeType !== Node.TEXT_NODE) {
          chip.after(document.createTextNode('\u200B'));
        }
      }
      const last = editorEl.lastChild;
      if (!last || last.nodeType !== Node.TEXT_NODE) {
        editorEl.appendChild(document.createTextNode('\u200B'));
      }
    }
  }

  function walkNodes(parent: Node): string {
    let result = '';
    for (const node of parent.childNodes) {
      if (node.nodeType === Node.TEXT_NODE) {
        result += node.textContent ?? '';
      } else if (node instanceof HTMLElement) {
        if (node.dataset.var !== undefined) {
          result += `{{${node.dataset.var}}}`;
        } else {
          result += walkNodes(node);
        }
      }
    }
    return result;
  }

  function extractValue(): string {
    if (!editorEl) return '';
    return walkNodes(editorEl).replace(/\u200B/g, '');
  }

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
          return offset + node.dataset.var.length + 4;
        }
        offset += node.dataset.var.length + 4;
        walker.nextNode();
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
        walker.nextNode();
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
    const range = document.createRange();
    range.selectNodeContents(editorEl);
    range.collapse(false);
    sel.removeAllRanges();
    sel.addRange(range);
  }

  $effect(() => {
    const v = value;
    const el = editorEl;
    if (!suppressRender && el) {
      renderToEditor(v);
    }
  });

  function handleInput() {
    suppressRender = true;
    const cursorPos = getCursorOffset();
    const val = extractValue();
    onchange(val);

    const textBefore = val.slice(0, cursorPos);
    const varMatch = textBefore.match(/\{\{(\w*)$/);
    if (varMatch) {
      acFilter = varMatch[1];
      acIdx = 0;
      acOpen = true;
      // Position dropdown using fixed coordinates to escape overflow clipping
      if (editorEl) {
        const rect = editorEl.getBoundingClientRect();
        dropdownStyle = `position:fixed;top:${rect.bottom + 2}px;left:${rect.left}px;width:${rect.width}px;`;
      }
    } else {
      acOpen = false;
    }

    if (!acOpen) {
      renderToEditor(val);
      restoreCursor(cursorPos);
    }

    requestAnimationFrame(() => { suppressRender = false; });
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!acOpen || acItems.length === 0) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      acIdx = Math.min(acIdx + 1, acItems.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      acIdx = Math.max(acIdx - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      selectItem(acItems[acIdx].key);
    } else if (e.key === 'Escape') {
      acOpen = false;
    }
  }

  function selectItem(varName: string) {
    if (!editorEl) return;
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return;

    const range = sel.getRangeAt(0);
    const textNode = range.startContainer;
    if (textNode.nodeType !== Node.TEXT_NODE) return;

    const text = textNode.textContent ?? '';
    const offset = range.startOffset;
    const before = text.slice(0, offset);
    const braceStart = before.lastIndexOf('{{');
    if (braceStart === -1) return;

    const keepBefore = text.slice(0, braceStart);
    const keepAfter = text.slice(offset);

    const chip = document.createElement('span');
    chip.className = 'url-var';
    chip.contentEditable = 'false';
    chip.dataset.var = varName;
    chip.textContent = varName;

    const parent = textNode.parentNode!;
    const beforeNode = document.createTextNode(keepBefore);
    const afterNode = document.createTextNode(keepAfter);
    parent.insertBefore(beforeNode, textNode);
    parent.insertBefore(chip, textNode);
    parent.insertBefore(afterNode, textNode);
    parent.removeChild(textNode);

    acOpen = false;
    justSelected = true;

    suppressRender = true;
    const newValue = extractValue();
    const chipPattern = `{{${varName}}}`;
    const chipEndIdx = newValue.lastIndexOf(chipPattern);
    const cursorTarget = chipEndIdx >= 0 ? chipEndIdx + chipPattern.length : newValue.length;

    onchange(newValue);
    renderToEditor(newValue);
    restoreCursor(cursorTarget);
    suppressRender = false;
  }

  function handleBlur() {
    setTimeout(() => {
      acOpen = false;
      // If selectItem just ran, don't re-render (it already rendered with correct value)
      if (justSelected) {
        justSelected = false;
        return;
      }
      suppressRender = false;
      renderToEditor(value);
    }, 150);
  }

  function handlePaste(e: ClipboardEvent) {
    e.preventDefault();
    const text = e.clipboardData?.getData('text') ?? '';
    document.execCommand('insertText', false, text);
  }
</script>

<div class="env-input-wrap">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    bind:this={editorEl}
    class="env-editor"
    contenteditable="true"
    role="textbox"
    tabindex="0"
    data-placeholder={placeholder}
    oninput={handleInput}
    onkeydown={handleKeydown}
    onblur={handleBlur}
    onpaste={handlePaste}
    spellcheck="false"
  ></div>
  {#if acOpen && acItems.length > 0}
    <div class="env-ac-dropdown" style={dropdownStyle}>
      {#each acItems as item, i (item.key)}
        <button
          class="env-ac-item"
          class:active={i === acIdx}
          onmousedown={(e) => { e.preventDefault(); selectItem(item.key); }}
          onmouseenter={() => { acIdx = i; }}
        >
          <span class="env-ac-name">{item.key}</span>
          <span class="env-ac-val">{item.value}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .env-input-wrap {
    position: relative;
    width: 100%;
  }
  .env-editor {
    width: 100%;
    min-height: 28px;
    background: var(--n2);
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 5px 10px;
    color: var(--t1);
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 17px;
    outline: none;
    transition: border-color 0.15s;
    box-sizing: border-box;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: text;
  }
  .env-editor:focus {
    border-color: var(--acc);
    white-space: pre-wrap;
    word-break: break-all;
    text-overflow: clip;
    max-height: 95px;
    overflow-y: auto;
  }
  .env-editor:empty::before {
    content: attr(data-placeholder);
    color: var(--t3);
    pointer-events: none;
  }
  .env-ac-dropdown {
    background: var(--n);
    border: 1px solid var(--b1);
    border-radius: 5px;
    z-index: 9999;
    max-height: 180px;
    overflow-y: auto;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.4);
  }
  .env-ac-item {
    width: 100%;
    padding: 6px 10px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--t2);
    border: none;
    background: transparent;
    text-align: left;
    cursor: default;
    transition: background 0.08s;
  }
  .env-ac-item:hover,
  .env-ac-item.active {
    background: var(--c);
    color: var(--t1);
  }
  .env-ac-name { color: var(--t1); }
  .env-ac-val {
    color: var(--t3);
    font-size: 10px;
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 150px;
  }
</style>

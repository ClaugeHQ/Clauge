<script lang="ts">
  import { activeEnvId, getEffectiveEnvId, environments, createEnvironment, setActiveEnv, setEnvVariable } from '$lib/modes/rest/stores';
  import { activeRequest, requestEnvOverrides } from '$lib/modes/rest/stores';
  import { activeTabId } from '$lib/shared/stores/tabs';
  import { getEnvVariablesForResolution } from '$lib/modes/rest/commands';
  import { get } from 'svelte/store';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';

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

  // Manual undo/redo. EnvInput is a contenteditable that rewrites innerHTML
  // on every keystroke (variable chip rendering), which destroys the
  // browser's native undo stack — same root cause that broke Cmd+Z on the
  // URL bar. We keep our own ring of {value, cursor} snapshots, coalesce
  // bursts of typing into one entry, and intercept the keyboard shortcuts.
  // Fixes undo across every consumer (KVTable header/param values,
  // AuthEditor token / username / password / api-key value).
  const UNDO_MAX = 100;
  const UNDO_COALESCE_MS = 300;
  type Snap = { value: string; cursor: number };
  let undoStack: Snap[] = [];
  let redoStack: Snap[] = [];
  let lastSnapAt = 0;
  let lastSnapValue = '';
  // Reset the stack when the consumer field changes identity (different tab
  // / request / auth slot). Tracked outside the effect so that prop-driven
  // updates from our OWN onchange don't trigger a reset on every keystroke.
  let lastSeenTabId = -999;
  let lastSeenReqId = '__init__';
  $effect(() => {
    const tabId = $activeTabId;
    const reqId = $activeRequest?.id ?? '';
    if (tabId === lastSeenTabId && reqId === lastSeenReqId) return;
    lastSeenTabId = tabId;
    lastSeenReqId = reqId;
    undoStack = [{ value, cursor: value.length }];
    redoStack = [];
    lastSnapAt = 0;
    lastSnapValue = value;
  });

  function pushSnapshot(val: string, cursor: number) {
    if (val === lastSnapValue) return;
    const now = Date.now();
    const coalesce = now - lastSnapAt < UNDO_COALESCE_MS && undoStack.length > 0;
    if (coalesce) {
      undoStack[undoStack.length - 1] = { value: val, cursor };
    } else {
      undoStack.push({ value: val, cursor });
      if (undoStack.length > UNDO_MAX) undoStack.shift();
    }
    redoStack.length = 0;
    lastSnapAt = now;
    lastSnapValue = val;
  }

  function applySnapshot(snap: Snap) {
    suppressRender = true;
    onchange(snap.value);
    renderToEditor(snap.value);
    restoreCursor(snap.cursor);
    lastSnapValue = snap.value;
    lastSnapAt = 0; // next edit starts a new coalesce window
    requestAnimationFrame(() => { suppressRender = false; });
  }

  function doUndo() {
    if (undoStack.length <= 1) return;
    const current = undoStack.pop()!;
    redoStack.push(current);
    applySnapshot(undoStack[undoStack.length - 1]);
  }

  function doRedo() {
    if (redoStack.length === 0) return;
    const next = redoStack.pop()!;
    undoStack.push(next);
    applySnapshot(next);
  }

  // WKWebView (Tauri's renderer on macOS) sometimes routes Cmd+Z through
  // beforeinput with inputType `historyUndo` before keydown can intercept.
  function handleBeforeInput(e: InputEvent) {
    if (e.inputType === 'historyUndo') {
      e.preventDefault();
      doUndo();
    } else if (e.inputType === 'historyRedo') {
      e.preventDefault();
      doRedo();
    }
  }

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

  // Inline create: when the user types {{name and no var with that EXACT
  // key exists yet, offer a "+ Create" row at the bottom of the dropdown.
  // Validates a basic identifier shape so we don't try to create a var
  // named with weird characters that won't round-trip through resolution.
  const VAR_NAME_RE = /^[A-Za-z_][A-Za-z0-9_-]*$/;
  const exactMatch = $derived(envVarEntries.some(v => v.key === acFilter));
  const canCreate = $derived(acFilter.length > 0 && !exactMatch && VAR_NAME_RE.test(acFilter));

  async function createInline(name: string) {
    if (!VAR_NAME_RE.test(name)) {
      showToast('Variable names use letters, digits, _ and -', 'error');
      return;
    }
    try {
      // Resolve which env to write to. Falls back to creating a "Default"
      // env when the user hasn't set one up yet — without this, typing
      // {{foo}} in any field with no env configured is a dead-end.
      let envId = get(activeEnvId);
      if (!envId) {
        const envs = get(environments);
        if (envs.length > 0) {
          envId = envs[0].id;
          await setActiveEnv(envId);
        } else {
          const created = await createEnvironment('Default', 'var(--acc)');
          envId = created.id;
          await setActiveEnv(envId);
        }
      }
      await setEnvVariable(envId, name, '', 0);

      // Refresh local list so the chip renders with the new var present
      // and future {{ autocomplete sees it.
      const vars = await getEnvVariablesForResolution(envId);
      envVarEntries = Object.entries(vars).map(([key, value]) => ({ key, value }));

      // Finally drop the chip into the field.
      selectItem(name);
      showToast(`Added {{${name}}} to environment`, 'success');
    } catch (e) {
      errorToast('Failed to create variable', e);
    }
  }

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
    pushSnapshot(val, cursorPos);
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
    // Manual undo/redo — intercept before the browser fights us.
    const isMod = e.metaKey || e.ctrlKey;
    if (isMod && (e.key === 'z' || e.key === 'Z')) {
      e.preventDefault();
      if (e.shiftKey) doRedo();
      else doUndo();
      return;
    }
    if (isMod && (e.key === 'y' || e.key === 'Y')) {
      e.preventDefault();
      doRedo();
      return;
    }

    if (!acOpen) return;
    if (acItems.length === 0 && !canCreate) return;

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      acIdx = Math.min(acIdx + 1, Math.max(0, acItems.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      acIdx = Math.max(acIdx - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (acItems.length > 0) {
        selectItem(acItems[acIdx].key);
      } else if (canCreate) {
        createInline(acFilter);
      }
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

    // Distinct undo entry for the chip insertion so Cmd+Z reverts it.
    lastSnapAt = 0;
    pushSnapshot(newValue, cursorTarget);
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
    onbeforeinput={handleBeforeInput}
    onkeydown={handleKeydown}
    onblur={handleBlur}
    onpaste={handlePaste}
    spellcheck="false"
  ></div>
  {#if acOpen && (acItems.length > 0 || canCreate)}
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
      {#if canCreate}
        <button
          class="env-ac-item env-ac-create"
          onmousedown={(e) => { e.preventDefault(); createInline(acFilter); }}
        >
          <span class="env-ac-create-icon">+</span>
          <span class="env-ac-name">Create {`{{${acFilter}}}`}</span>
          <span class="env-ac-val">
            {#if !$activeEnvId}new env "Default"{:else}in current env{/if}
          </span>
        </button>
      {/if}
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
    z-index: var(--z-drawer);
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
  .env-ac-create {
    border-top: 1px solid var(--b1);
  }
  .env-ac-create-icon {
    display: inline-flex;
    width: 14px;
    height: 14px;
    align-items: center;
    justify-content: center;
    border-radius: 3px;
    background: var(--acc);
    color: #fff;
    font-weight: 700;
    font-size: 12px;
    line-height: 1;
  }
</style>

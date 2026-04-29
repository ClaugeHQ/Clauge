<script lang="ts">
  import { highlightJSON } from '$lib/shared/utils/json-highlight';
  import { showToast } from '$lib/components/shared/toast';
  import FormKVEditor from './FormKVEditor.svelte';
  import MultipartEditor from './MultipartEditor.svelte';
  import BinaryPicker from './BinaryPicker.svelte';
  import { BODY_DEBOUNCE_MS } from '$lib/shared/constants/timings';

  let { body = '', bodyType = 'json', onchange }: {
    body: string;
    bodyType: string;
    onchange: (body: string, bodyType: string) => void;
  } = $props();

  let localBody = $state(body);
  let localType = $state(bodyType);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let isFocused = $state(false);
  let textareaRef = $state<HTMLTextAreaElement | null>(null);
  let scrollTop = $state(0);

  const isTextType = $derived(['json', 'text', 'xml'].includes(localType));

  const highlightedBody = $derived(
    localType === 'json' && localBody ? highlightJSON(localBody) : ''
  );

  const lineCount = $derived(isTextType && localBody ? localBody.split('\n').length : 1);

  $effect(() => {
    localBody = body;
  });

  $effect(() => {
    localType = bodyType;
  });

  function handleInput(e: Event) {
    const target = e.target as HTMLTextAreaElement;
    localBody = target.value;
    debounceSave();
  }

  function handleTypeChange(e: Event) {
    const target = e.target as HTMLSelectElement;
    const newType = target.value;
    // Clear body when switching between incompatible types
    const wasStructured = ['urlencoded', 'multipart', 'binary'].includes(localType);
    const isStructured = ['urlencoded', 'multipart', 'binary'].includes(newType);
    if (wasStructured !== isStructured || (wasStructured && isStructured && localType !== newType)) {
      localBody = '';
    }
    localType = newType;
    onchange(localBody, localType);
  }

  function debounceSave() {
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      onchange(localBody, localType);
    }, BODY_DEBOUNCE_MS);
  }

  function formatJson() {
    if (localType !== 'json') return;
    try {
      const parsed = JSON.parse(localBody);
      localBody = JSON.stringify(parsed, null, 2);
      onchange(localBody, localType);
      showToast('JSON formatted', 'success');
    } catch {
      showToast('Invalid JSON', 'error');
    }
  }

  function handleScroll(e: Event) {
    scrollTop = (e.target as HTMLTextAreaElement).scrollTop;
  }

  function handleStructuredChange(newBody: string) {
    localBody = newBody;
    onchange(newBody, localType);
  }
</script>

<div class="body-editor">
  <div class="body-toolbar">
    <select class="body-type-sel" value={localType} onchange={handleTypeChange}>
      <option value="json">JSON</option>
      <option value="text">Text</option>
      <option value="xml">XML</option>
      <option value="urlencoded">Form URL-Encoded</option>
      <option value="multipart">Multipart Form</option>
      <option value="binary">Binary</option>
      <option value="none">None</option>
    </select>
    {#if localType === 'json'}
      <button class="ph-btn" onclick={formatJson} title="Format JSON (pretty-print)">
        <svg viewBox="0 0 24 24" width="11" height="11"><path d="M4 7h16M4 12h10M4 17h6" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round"/></svg>
        Format
      </button>
    {/if}
  </div>

  {#if localType === 'none'}
    <div class="body-empty">
      <span class="body-empty-msg">No body for this request</span>
    </div>
  {:else if localType === 'urlencoded'}
    <FormKVEditor body={localBody} onchange={handleStructuredChange} />
  {:else if localType === 'multipart'}
    <MultipartEditor body={localBody} onchange={handleStructuredChange} />
  {:else if localType === 'binary'}
    <BinaryPicker body={localBody} onchange={handleStructuredChange} />
  {:else}
    <!-- JSON / Text / XML -->
    <div class="editor-wrap">
      <div class="line-gutter" style="transform: translateY({-scrollTop}px);">
        {#each Array(lineCount) as _, i}
          <span class="line-num">{i + 1}</span>
        {/each}
      </div>
      <div class="editor-area">
        {#if localType === 'json' && !isFocused && localBody}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="highlight-overlay"
            onclick={() => { isFocused = true; setTimeout(() => textareaRef?.focus(), 0); }}
          >{@html highlightedBody}</div>
        {:else}
          <textarea
            bind:this={textareaRef}
            class="editor"
            spellcheck="false"
            placeholder={localType === 'json' ? '{\n  "key": "value"\n}' : localType === 'xml' ? '<root>\n  <element>value</element>\n</root>' : 'Enter request body...'}
            value={localBody}
            oninput={handleInput}
            onscroll={handleScroll}
            onfocus={() => { isFocused = true; }}
            onblur={() => { isFocused = false; }}
          ></textarea>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .body-editor {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }
  .body-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: transparent;
    border-bottom: 1px solid var(--b1);
  }
  .body-type-sel {
    height: 20px;
    padding: 0 6px;
    border-radius: 4px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    cursor: pointer;
    outline: none;
    transition: border-color 0.1s, color 0.1s;
  }
  .body-type-sel:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .body-type-sel:focus {
    border-color: var(--acc);
  }

  .editor-wrap {
    flex: 1;
    display: flex;
    overflow: hidden;
    background: transparent;
  }
  .line-gutter {
    width: 40px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    padding-top: 12px;
    background: transparent;
    border-right: 1px solid var(--b1);
    user-select: none;
    overflow: hidden;
  }
  .line-num {
    display: block;
    text-align: right;
    padding-right: 8px;
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 1.75;
    color: var(--t3);
    height: calc(12.5px * 1.75);
  }

  .editor-area {
    flex: 1;
    position: relative;
    overflow: hidden;
  }

  .editor {
    width: 100%;
    height: 100%;
    background: transparent;
    border: none;
    outline: none;
    resize: none;
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 1.75;
    color: var(--t1);
    padding: 12px 14px;
    overflow-y: auto;
    white-space: pre;
    tab-size: 2;
  }
  .editor::placeholder {
    color: var(--t3);
  }
  .editor::-webkit-scrollbar {
    width: 4px;
  }
  .editor::-webkit-scrollbar-thumb {
    background: var(--b1);
    border-radius: 2px;
  }

  .highlight-overlay {
    width: 100%;
    height: 100%;
    padding: 12px 14px;
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 1.75;
    white-space: pre;
    tab-size: 2;
    color: var(--t1);
    overflow-y: auto;
    cursor: text;
  }
  .highlight-overlay::-webkit-scrollbar {
    width: 4px;
  }
  .highlight-overlay::-webkit-scrollbar-thumb {
    background: var(--b1);
    border-radius: 2px;
  }

  .body-empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
  }
  .body-empty-msg {
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    font-style: italic;
  }
  .ph-btn {
    height: 20px;
    padding: 0 8px;
    border-radius: 4px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    cursor: pointer;
    transition: border-color 0.1s, color 0.1s;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .ph-btn:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
</style>

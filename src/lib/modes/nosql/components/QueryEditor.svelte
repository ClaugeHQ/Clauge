<script lang="ts">
  import { mod } from '$lib/utils/platform';
  const m = mod();

  interface Props {
    value: string;
    placeholder?: string;
    onexecute?: (value: string) => void;
    label?: string;
  }

  let { value = $bindable(), placeholder = '{}', onexecute, label = 'Query' }: Props = $props();

  let lineCount = $derived(Math.max(1, (value || '').split('\n').length));

  function handleKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      onexecute?.(value);
    }
    // Tab inserts 2 spaces
    if (e.key === 'Tab') {
      e.preventDefault();
      const ta = e.target as HTMLTextAreaElement;
      const start = ta.selectionStart;
      const end = ta.selectionEnd;
      value = value.substring(0, start) + '  ' + value.substring(end);
      requestAnimationFrame(() => {
        ta.selectionStart = ta.selectionEnd = start + 2;
      });
    }
  }

  function formatJson() {
    try {
      const parsed = JSON.parse(value);
      value = JSON.stringify(parsed, null, 2);
    } catch {
      // ignore parse errors
    }
  }
</script>

<div class="qe-wrap">
  <div class="qe-header">
    <span class="qe-label">{label}</span>
    <span class="qe-hint">{m}+Enter to run</span>
    <button class="qe-btn" onclick={formatJson}>
      <svg viewBox="0 0 24 24" width="10" height="10"><polyline points="4 7 4 4 20 4 20 7"/><line x1="9" y1="20" x2="15" y2="20"/><line x1="12" y1="4" x2="12" y2="20"/></svg>
      Format
    </button>
    <button class="qe-btn qe-run" onclick={() => onexecute?.(value)}>
      Run &#9654;
    </button>
  </div>
  <div class="qe-editor">
    <div class="qe-gutter">
      {#each Array(lineCount) as _, i}
        <span class="qe-line-num">{i + 1}</span>
      {/each}
    </div>
    <textarea
      class="qe-textarea"
      bind:value
      {placeholder}
      onkeydown={handleKeydown}
      spellcheck="false"
    ></textarea>
  </div>
</div>

<style>
  .qe-wrap {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .qe-header {
    height: 32px;
    flex-shrink: 0;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    padding: 0 12px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .qe-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--t2);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .qe-hint {
    font-size: 10px;
    color: var(--t4);
    font-family: var(--mono);
    margin-left: 4px;
    margin-right: auto;
  }
  .qe-btn {
    height: 22px;
    padding: 0 8px;
    border-radius: 4px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t3);
    font-size: 10.5px;
    font-family: var(--ui);
    cursor: default;
    transition: border-color 0.1s, color 0.1s;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .qe-btn svg {
    stroke: currentColor;
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .qe-btn:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .qe-run {
    border-color: var(--acc);
    color: var(--acc);
  }
  .qe-run:hover {
    background: var(--acc);
    color: #fff;
  }

  .qe-editor {
    flex: 1;
    display: flex;
    min-height: 0;
    overflow: hidden;
  }
  .qe-gutter {
    width: 36px;
    flex-shrink: 0;
    background: var(--n2);
    border-right: 1px solid var(--b1);
    padding: 12px 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    user-select: none;
  }
  .qe-line-num {
    font-family: var(--mono);
    font-size: 10.5px;
    line-height: 1.6;
    color: var(--t4);
    text-align: right;
    padding-right: 8px;
    min-height: calc(12.5px * 1.6);
  }
  .qe-textarea {
    flex: 1;
    background: transparent;
    border: none;
    padding: 12px 14px;
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 1.6;
    color: var(--t1);
    resize: none;
    outline: none;
    tab-size: 2;
  }
  .qe-textarea::placeholder {
    color: var(--t4);
  }
  .qe-textarea::-webkit-scrollbar { width: 4px; }
  .qe-textarea::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }
</style>

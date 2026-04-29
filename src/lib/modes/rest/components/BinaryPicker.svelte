<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';

  let { body = '', onchange }: {
    body: string;
    onchange: (body: string) => void;
  } = $props();

  let filePath = $state('');
  let fileName = $state('');
  let localChange = false;

  $effect(() => {
    if (localChange) {
      localChange = false;
      return;
    }
    try {
      const parsed = JSON.parse(body);
      filePath = parsed.filePath ?? '';
      fileName = parsed.fileName ?? '';
    } catch {
      filePath = '';
      fileName = '';
    }
  });

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  async function pickFile() {
    const selected = await open({ multiple: false });
    if (selected) {
      const path = String(selected);
      const name = path.split('/').pop() ?? 'file';
      filePath = path;
      fileName = name;
      localChange = true;
      onchange(JSON.stringify({ filePath: path, fileName: name }));
    }
  }

  function clearFile() {
    filePath = '';
    fileName = '';
    localChange = true;
    onchange('');
  }
</script>

<div class="bin-picker">
  {#if filePath}
    <div class="bin-selected">
      <svg class="bin-icon" viewBox="0 0 24 24" width="20" height="20">
        <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" stroke="currentColor" fill="none" stroke-width="1.5"/>
        <polyline points="14 2 14 8 20 8" stroke="currentColor" fill="none" stroke-width="1.5"/>
      </svg>
      <div class="bin-info">
        <span class="bin-name">{fileName}</span>
        <span class="bin-path" title={filePath}>{filePath}</span>
      </div>
      <button class="bin-change" onclick={pickFile}>Change</button>
      <button class="bin-clear" onclick={clearFile} title="Remove file">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
      </button>
    </div>
  {:else}
    <div class="bin-empty">
      <button class="bin-choose" onclick={pickFile}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
          <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/>
          <polyline points="17 8 12 3 7 8"/>
          <line x1="12" y1="3" x2="12" y2="15"/>
        </svg>
        Choose File
      </button>
      <span class="bin-hint">Select a file to upload as the request body</span>
    </div>
  {/if}
</div>

<style>
  .bin-picker {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
  }

  .bin-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }

  .bin-choose {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 24px;
    border: 1px dashed var(--b2);
    border-radius: 8px;
    background: transparent;
    color: var(--t2);
    font-size: 13px;
    font-family: var(--ui);
    font-weight: 500;
    cursor: pointer;
    transition: border-color 0.15s, color 0.15s, background 0.15s;
  }
  .bin-choose:hover {
    border-color: var(--acc);
    color: var(--acc);
    background: color-mix(in srgb, var(--acc) 5%, transparent);
  }

  .bin-hint {
    font-size: 11px;
    color: var(--t4);
    font-family: var(--ui);
  }

  .bin-selected {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border: 1px solid var(--b1);
    border-radius: 8px;
    background: rgba(255,255,255,0.02);
    width: 100%;
    max-width: 500px;
  }

  .bin-icon { color: var(--acc); flex-shrink: 0; }

  .bin-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .bin-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .bin-path {
    font-size: 10px;
    color: var(--t4);
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .bin-change {
    padding: 4px 10px;
    border: 1px solid var(--b1);
    border-radius: 5px;
    background: transparent;
    color: var(--t3);
    font-size: 11px;
    font-family: var(--ui);
    cursor: pointer;
    flex-shrink: 0;
    transition: color 0.1s, border-color 0.1s;
  }
  .bin-change:hover { color: var(--t1); border-color: var(--b2); }

  .bin-clear {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--t4);
    border-radius: 4px;
    cursor: pointer;
    flex-shrink: 0;
    transition: color 0.1s;
  }
  .bin-clear:hover { color: var(--err); }
</style>

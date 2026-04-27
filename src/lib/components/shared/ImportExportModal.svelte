<script lang="ts">
  import Modal from './Modal.svelte';
  import { showToast } from './toast';
  import { friendlyError } from '$lib/utils/errors';
  import { loadCollections, collections } from '$lib/stores/collections';
  import * as cmd from '$lib/commands';
  import type { Collection } from '$lib/types';

  interface Props {
    show: boolean;
  }

  let { show = $bindable() }: Props = $props();

  let activeTab: 'import' | 'export' = $state('import');

  // Import state
  let importFormat: 'auto' | 'clauge' | 'postman' | 'curl' = $state('auto');
  let importText = $state('');
  let importFile: File | null = $state(null);
  let importing = $state(false);
  let fileInputEl: HTMLInputElement | undefined = $state();

  // Export state
  let exportCollectionId = $state('__all__');
  let exportFormat: 'clauge' | 'curl' = $state('clauge');
  let exporting = $state(false);

  const collectionList = $derived($collections);

  // Reset to JSON format when "All Collections" is selected (cURL not available for all)
  $effect(() => {
    if (exportCollectionId === '__all__' && exportFormat === 'curl') {
      exportFormat = 'clauge';
    }
  });

  function detectFormat(text: string): 'clauge' | 'postman' | 'curl' {
    const trimmed = text.trim();
    if (trimmed.startsWith('curl ') || trimmed.startsWith('curl\t')) {
      return 'curl';
    }
    try {
      const obj = JSON.parse(trimmed);
      if (obj.format && typeof obj.format === 'string' && obj.format.startsWith('clauge/')) {
        return 'clauge';
      }
      if (obj.info && obj.item) {
        return 'postman';
      }
    } catch {
      // Not JSON
    }
    return 'curl';
  }

  function handleFileChange(e: Event) {
    const input = e.target as HTMLInputElement;
    if (input.files && input.files.length > 0) {
      importFile = input.files[0];
    }
  }

  async function handleImport() {
    importing = true;
    try {
      let content = importText.trim();

      // Read file if selected
      if (importFile) {
        content = await importFile.text();
      }

      if (!content) {
        showToast('Please provide content to import', 'error');
        importing = false;
        return;
      }

      const format = importFormat === 'auto' ? detectFormat(content) : importFormat;

      if (format === 'curl') {
        await cmd.importCurl(content);
        showToast('Imported cURL as request', 'success');
      } else if (format === 'postman') {
        const result = await cmd.importPostman(content);
        showToast(result.message, 'success');
      } else {
        const result = await cmd.importClauge(content);
        showToast(result.message, 'success');
      }

      await loadCollections();
      importText = '';
      importFile = null;
      if (fileInputEl) fileInputEl.value = '';
      show = false;
    } catch (err: any) {
      showToast(friendlyError(err), 'error');
    } finally {
      importing = false;
    }
  }

  async function handleExport() {
    exporting = true;
    try {
      if (exportFormat === 'curl') {
        // Export as cURL: get collection JSON, parse requests, generate cURL for each
        const json = await cmd.exportCollection(exportCollectionId);
        const data = JSON.parse(json);
        const requests = data.requests || [];
        if (requests.length === 0) {
          showToast('No requests to export', 'error');
          exporting = false;
          return;
        }
        const curlCommands: string[] = [];
        for (const req of requests) {
          let curlCmd = `curl -X ${req.method} '${req.url}'`;
          if (req.headers) {
            for (const h of req.headers) {
              if (h.enabled !== 0) {
                curlCmd += ` \\\n  -H '${h.key}: ${h.value}'`;
              }
            }
          }
          if (req.body && ['POST', 'PUT', 'PATCH'].includes(req.method)) {
            curlCmd += ` \\\n  -d '${(req.body || '').replace(/'/g, "'\\''")}'`;
          }
          curlCommands.push(`# ${req.name || req.url}\n${curlCmd}`);
        }
        const content = curlCommands.join('\n\n');
        downloadFile(content, getCollectionName(exportCollectionId), 'sh', 'text/x-sh');
        showToast('Export downloaded as cURL', 'success');
      } else {
        let json: string;
        if (exportCollectionId === '__all__') {
          json = await cmd.exportAllCollections();
        } else {
          json = await cmd.exportCollection(exportCollectionId);
        }
        downloadFile(json, exportCollectionId === '__all__' ? 'clauge-export' : getCollectionName(exportCollectionId), 'json', 'application/json');
        showToast('Export downloaded', 'success');
      }
    } catch (err: any) {
      showToast(friendlyError(err), 'error');
    } finally {
      exporting = false;
    }
  }

  function getCollectionName(id: string): string {
    const coll = collectionList.find((c: Collection) => c.id === id);
    return coll ? coll.name.replace(/\s+/g, '-').toLowerCase() : 'collection';
  }

  function downloadFile(content: string, filename: string, ext: string = 'json', mimeType: string = 'application/json') {
    const blob = new Blob([content], { type: mimeType });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${filename}.${ext}`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }
</script>

<Modal bind:show title="Import / Export" width="540px">
  <div class="ie-tabs">
    <button
      class="ie-tab"
      class:active={activeTab === 'import'}
      onclick={() => activeTab = 'import'}
    >Import</button>
    <button
      class="ie-tab"
      class:active={activeTab === 'export'}
      onclick={() => activeTab = 'export'}
    >Export</button>
  </div>

  {#if activeTab === 'import'}
    <div class="ie-section">
      <label class="ie-label">Format</label>
      <select class="ie-select" bind:value={importFormat}>
        <option value="auto">Auto-detect</option>
        <option value="clauge">Clauge JSON</option>
        <option value="postman">Postman Collection</option>
        <option value="curl">cURL Command</option>
      </select>

      <label class="ie-label">Choose File</label>
      <input
        bind:this={fileInputEl}
        type="file"
        accept=".json,.txt"
        class="ie-file"
        onchange={handleFileChange}
      />

      <label class="ie-label">Or paste content</label>
      <textarea
        class="ie-textarea"
        placeholder="Paste JSON or cURL command here..."
        rows="8"
        bind:value={importText}
      ></textarea>

      <button
        class="ie-btn ie-btn-primary"
        onclick={handleImport}
        disabled={importing || (!importText.trim() && !importFile)}
      >
        {importing ? 'Importing...' : 'Import'}
      </button>
    </div>
  {:else}
    <div class="ie-section">
      <label class="ie-label">Collection</label>
      <select class="ie-select" bind:value={exportCollectionId}>
        <option value="__all__">All Collections</option>
        {#each collectionList as coll}
          <option value={coll.id}>{coll.name}</option>
        {/each}
      </select>

      <label class="ie-label">Format</label>
      <select class="ie-select" bind:value={exportFormat}>
        <option value="clauge">Clauge JSON</option>
        {#if exportCollectionId !== '__all__'}
          <option value="curl">cURL (.sh)</option>
        {/if}
      </select>

      {#if exportFormat === 'curl'}
        <p class="ie-note">Exports each request as a cURL command in a shell script.</p>
      {/if}

      <button
        class="ie-btn ie-btn-primary"
        onclick={handleExport}
        disabled={exporting}
      >
        {exporting ? 'Exporting...' : exportFormat === 'curl' ? 'Export as cURL' : 'Export as JSON'}
      </button>
    </div>
  {/if}
</Modal>

<style>
  .ie-tabs {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--b1);
    margin-bottom: 16px;
  }
  .ie-tab {
    flex: 1;
    padding: 8px 0;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--t3);
    font-size: 12px;
    font-weight: 600;
    font-family: var(--ui);
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s;
  }
  .ie-tab:hover {
    color: var(--t2);
  }
  .ie-tab.active {
    color: var(--acc);
    border-bottom-color: var(--acc);
  }
  .ie-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .ie-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--t3);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-family: var(--ui);
  }
  .ie-select {
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 12.5px;
    color: var(--t1);
    font-family: var(--mono);
    outline: none;
  }
  .ie-select:focus {
    border-color: var(--acc);
  }
  .ie-file {
    font-size: 11px;
    color: var(--t2);
    font-family: var(--mono);
  }
  .ie-file::file-selector-button {
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 5px;
    padding: 5px 12px;
    font-size: 11px;
    color: var(--t2);
    font-family: var(--ui);
    cursor: pointer;
    margin-right: 8px;
    transition: background 0.12s;
  }
  .ie-file::file-selector-button:hover {
    background: var(--c);
  }
  .ie-textarea {
    background: var(--e);
    border: 1px solid var(--b1);
    border-radius: 6px;
    padding: 10px;
    font-size: 11.5px;
    color: var(--t1);
    font-family: var(--mono);
    resize: vertical;
    outline: none;
    min-height: 100px;
  }
  .ie-textarea::placeholder {
    color: var(--t4);
  }
  .ie-textarea:focus {
    border-color: var(--acc);
  }
  .ie-btn {
    padding: 8px 16px;
    border-radius: 8px;
    border: 1px solid var(--b1);
    font-size: 12px;
    font-weight: 600;
    font-family: var(--ui);
    cursor: pointer;
    transition: background 0.12s, opacity 0.12s;
    margin-top: 4px;
  }
  .ie-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .ie-btn-primary {
    background: var(--acc);
    color: #fff;
    border-color: var(--acc);
  }
  .ie-btn-primary:hover:not(:disabled) {
    opacity: 0.9;
  }
  .ie-note {
    font-size: 11px;
    color: var(--t3);
    font-family: var(--ui);
    margin: 0;
    line-height: 1.4;
  }
</style>

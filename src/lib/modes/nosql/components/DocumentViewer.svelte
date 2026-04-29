<script lang="ts">
  import { showToast } from '$lib/components/shared/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { nosqlFindDocuments, nosqlInsertDocument, nosqlUpdateDocument, nosqlDeleteDocument, nosqlCountDocuments } from '../commands';
  import { parseCsv, parseJsonDocs } from '$lib/utils/import-parser';
  import QueryEditor from './QueryEditor.svelte';
  import { highlightJSON } from '$lib/utils/json-highlight';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';

  interface Props {
    connectionId: string;
    database: string;
    collection: string;
    initialFilter?: string;
  }

  let { connectionId, database, collection, initialFilter }: Props = $props();

  type ViewTab = 'documents' | 'query' | 'aggregation';
  let activeTab = $state<ViewTab>('documents');

  // Document listing
  let documents = $state<any[]>([]);
  let totalCount = $state(0);
  let durationMs = $state(0);
  let loading = $state(false);
  let limit = $state(25);
  let skip = $state(0);
  // Committed filter/sort (used for actual queries)
  let filterQuery = $state('{}');
  let sortQuery = $state('{}');
  // Input values (local state, committed on Enter/Find)
  let filterInput = $state('{}');
  let sortInput = $state('{}');
  let showOptions = $state(false);
  let expandedDocs = $state<Set<number>>(new Set());
  let editingIdx = $state<number | null>(null);
  let editingValue = $state('');

  // Search within documents
  let showSearch = $state(false);
  let searchQuery = $state('');
  let searchInputRef = $state<HTMLInputElement | null>(null);
  let currentMatchIndex = $state(0);
  let docListRef = $state<HTMLDivElement | null>(null);

  // Query / Aggregation
  let queryValue = $state('{}');
  let aggPipeline = $state('[]');
  let queryResult = $state<any[] | null>(null);
  let queryDuration = $state(0);

  // Add data dropdown
  let showAddMenu = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);

  // Close add menu on click outside
  function handleWindowClick() { if (showAddMenu) showAddMenu = false; }

  // Generate random ObjectId (24 hex chars)
  function generateObjectId(): string {
    const hex = '0123456789abcdef';
    let id = '';
    for (let i = 0; i < 24; i++) id += hex[Math.floor(Math.random() * 16)];
    return id;
  }

  function makeInsertTemplate(): string {
    return `{\n  "_id": {\n    "$oid": "${generateObjectId()}"\n  }\n}`;
  }

  // Count documents in editor (single object or array)
  function countInsertDocs(text: string): number {
    try {
      const parsed = JSON.parse(text);
      if (Array.isArray(parsed)) return parsed.length;
      return 1;
    } catch { return 0; }
  }

  // Insert/Update/Delete modals
  let showInsert = $state(false);
  let insertDoc = $state(makeInsertTemplate());
  let insertValid = $state(true);
  let insertError = $state('');
  let insertErrorLine = $state(-1);
  let insertDocCount = $derived(insertValid ? countInsertDocs(insertDoc) : 0);
  let showUpdateModal = $state(false);
  let updateFilter = $state('');
  let updateDoc = $state('');
  let updateFilterValid = $state(true);
  let updateDocValid = $state(true);
  let updateFilterError = $state('');
  let updateDocError = $state('');
  let updateFilterErrorLine = $state(-1);
  let updateDocErrorLine = $state(-1);
  let showDeleteModal = $state(false);
  let deleteFilter = $state('');
  let deleteFilterValid = $state(true);
  let deleteFilterError = $state('');
  let deleteFilterErrorLine = $state(-1);

  // Track previous collection to detect actual changes
  let prevKey = $state('');
  // Track the last AI filter we applied so we don't re-apply it after user reset
  let appliedAiFilter = $state('');

  // Reload when collection changes
  $effect(() => {
    const key = `${connectionId}:${database}:${collection}`;
    if (connectionId && database && collection && key !== prevKey) {
      prevKey = key;
      skip = 0;
      limit = 25;
      const filter = initialFilter && initialFilter !== '{}' ? initialFilter : '{}';
      filterQuery = filter;
      filterInput = filter;
      appliedAiFilter = initialFilter ?? '';
      sortQuery = '{}';
      sortInput = '{}';
      expandedDocs = new Set();
      queueMicrotask(() => loadDocuments());
    }
  });

  // Apply AI-injected filter changes — only when a NEW filter arrives from AI
  $effect(() => {
    if (initialFilter && initialFilter !== '{}' && initialFilter !== appliedAiFilter) {
      appliedAiFilter = initialFilter;
      filterQuery = initialFilter;
      filterInput = initialFilter;
      skip = 0;
      queueMicrotask(() => loadDocuments());
    }
  });

  function resetFilters() {
    filterQuery = '{}';
    filterInput = '{}';
    sortQuery = '{}';
    sortInput = '{}';
    limit = 25;
    skip = 0;
    queueMicrotask(() => loadDocuments());
  }

  async function loadDocuments() {
    loading = true;
    try {
      const result = await nosqlFindDocuments(connectionId, database, collection, filterQuery, sortQuery, limit, skip);
      documents = result.documents;
      durationMs = result.durationMs;
      totalCount = result.totalCount ?? 0;
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
      documents = [];
    } finally {
      loading = false;
    }
  }

  function executeFilter() {
    filterQuery = filterInput.trim() || '{}';
    sortQuery = sortInput.trim() || '{}';
    skip = 0;
    expandedDocs = new Set();
    loadDocuments();
  }

  function clearFilter() {
    filterInput = '{}';
    sortInput = '{}';
    filterQuery = '{}';
    sortQuery = '{}';
    appliedAiFilter = '';
    limit = 25;
    skip = 0;
    expandedDocs = new Set();
    loadDocuments();
  }

  const hasActiveFilter = $derived(filterInput !== '{}' || sortInput !== '{}');

  // Search helpers
  function escapeRegex(s: string) { return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); }

  const matchCount = $derived.by(() => {
    if (!searchQuery || documents.length === 0) return 0;
    const regex = new RegExp(escapeRegex(searchQuery), 'gi');
    let count = 0;
    for (let i = 0; i < documents.length; i++) {
      if (!expandedDocs.has(i)) continue;
      const text = JSON.stringify(documents[i], null, 2);
      const matches = text.match(regex);
      if (matches) count += matches.length;
    }
    return count;
  });

  function highlightSearchInDoc(json: string): string {
    if (!searchQuery) return highlightJSON(json);
    // First apply JSON syntax highlighting, then overlay search matches
    const highlighted = highlightJSON(json);
    const regex = new RegExp(`(${escapeRegex(searchQuery)})`, 'gi');
    let matchIdx = -1;
    return highlighted.replace(/>([^<]*)</g, (full, text) => {
      return '>' + text.replace(regex, (_m: string) => {
        matchIdx++;
        const cls = matchIdx === currentMatchIndex ? 'dv-search-match dv-active-match' : 'dv-search-match';
        return `<span class="${cls}">${_m}</span>`;
      }) + '<';
    });
  }

  function handleDocKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'f') {
      e.preventDefault();
      showSearch = true;
      setTimeout(() => searchInputRef?.focus(), 0);
    }
  }

  function handleSearchKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      closeSearch();
    } else if (e.key === 'Enter' && matchCount > 0) {
      if (e.shiftKey) {
        currentMatchIndex = (currentMatchIndex - 1 + matchCount) % matchCount;
      } else {
        currentMatchIndex = (currentMatchIndex + 1) % matchCount;
      }
      scrollToSearchMatch();
    }
  }

  function scrollToSearchMatch() {
    setTimeout(() => {
      const active = docListRef?.querySelector('.dv-active-match');
      if (active) active.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }, 10);
  }

  function closeSearch() {
    showSearch = false;
    searchQuery = '';
    currentMatchIndex = 0;
  }

  $effect(() => {
    if (searchQuery) currentMatchIndex = 0;
  });

  function nextPage() {
    skip += limit;
    expandedDocs = new Set();
    loadDocuments();
  }

  function prevPage() {
    skip = Math.max(0, skip - limit);
    expandedDocs = new Set();
    loadDocuments();
  }

  function toggleExpand(i: number) {
    const next = new Set(expandedDocs);
    if (next.has(i)) next.delete(i); else next.add(i);
    expandedDocs = next;
  }

  async function copyDocument(doc: any) {
    try {
      await writeText(JSON.stringify(doc, null, 2));
      showToast('Document copied', 'success');
    } catch {
      showToast('Failed to copy', 'error');
    }
  }

  function editDocument(doc: any) {
    const id = doc._id;
    updateFilter = JSON.stringify({ _id: id }, null, 2);
    // Exclude _id from $set — MongoDB throws if you try to update _id
    const { _id, ...rest } = doc;
    updateDoc = JSON.stringify({ $set: rest }, null, 2);
    showUpdateModal = true;
  }

  function validateJson(value: string): { valid: boolean; error: string; errorLine: number } {
    try {
      JSON.parse(value);
      return { valid: true, error: '', errorLine: -1 };
    } catch (err: any) {
      const msg = err.message ?? 'Invalid JSON';
      let line = -1;
      const m = msg.match(/position (\d+)/);
      if (m) {
        line = value.slice(0, parseInt(m[1])).split('\n').length;
      }
      return { valid: false, error: msg, errorLine: line };
    }
  }

  function handleInsertInput(e: Event) {
    insertDoc = (e.target as HTMLTextAreaElement).value;
    const r = validateJson(insertDoc);
    insertValid = r.valid;
    insertError = r.error;
    insertErrorLine = r.errorLine;
  }

  function handleUpdateFilterInput(e: Event) {
    updateFilter = (e.target as HTMLTextAreaElement).value;
    const r = validateJson(updateFilter);
    updateFilterValid = r.valid;
    updateFilterError = r.error;
    updateFilterErrorLine = r.errorLine;
  }

  function handleUpdateDocInput(e: Event) {
    updateDoc = (e.target as HTMLTextAreaElement).value;
    const r = validateJson(updateDoc);
    updateDocValid = r.valid;
    updateDocError = r.error;
    updateDocErrorLine = r.errorLine;
  }

  function handleDeleteFilterInput(e: Event) {
    deleteFilter = (e.target as HTMLTextAreaElement).value;
    const r = validateJson(deleteFilter);
    deleteFilterValid = r.valid;
    deleteFilterError = r.error;
    deleteFilterErrorLine = r.errorLine;
  }

  function handleEditInput(e: Event) {
    editingValue = (e.target as HTMLTextAreaElement).value;
    validateEditJson();
  }

  let editJsonValid = $state(true);
  let editErrorLine = $state(-1);
  let editErrorMsg = $state('');

  // Compute textarea height from line count (19px per line + 16px padding)
  function taHeight(value: string): string {
    if (!value) return 'auto';
    return (value.split('\n').length * 19 + 16) + 'px';
  }
  const editTaHeight = $derived(taHeight(editingValue));

  function startInlineEdit(idx: number, doc: any) {
    editingIdx = idx;
    editingValue = JSON.stringify(doc, null, 2);
    validateEditJson();
  }

  function cancelInlineEdit() {
    editingIdx = null;
    editingValue = '';
    editJsonValid = true;
    editErrorLine = -1;
    editErrorMsg = '';
  }

  function validateEditJson() {
    try {
      JSON.parse(editingValue);
      editJsonValid = true;
      editErrorLine = -1;
      editErrorMsg = '';
    } catch (err: any) {
      editJsonValid = false;
      editErrorMsg = err.message ?? 'Invalid JSON';
      // Try to extract line number from error message
      const lineMatch = editErrorMsg.match(/position (\d+)/);
      if (lineMatch) {
        const pos = parseInt(lineMatch[1]);
        const before = editingValue.slice(0, pos);
        editErrorLine = before.split('\n').length;
      } else {
        editErrorLine = -1;
      }
    }
  }

  async function saveInlineEdit(originalDoc: any) {
    if (editingIdx === null || !editJsonValid) return;
    try {
      const updatedDoc = JSON.parse(editingValue);
      const id = originalDoc._id;
      const filter = JSON.stringify({ _id: id });
      // Exclude _id from $set
      const { _id, ...rest } = updatedDoc;
      const update = JSON.stringify({ $set: rest });
      await nosqlUpdateDocument(connectionId, database, collection, filter, update);
      showToast('Document updated', 'success');
      cancelInlineEdit();
      loadDocuments();
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    }
  }

  // Generate highlighted lines with line numbers for edit view
  function getEditLines(value: string): { num: number; html: string; hasError: boolean }[] {
    const rawLines = value.split('\n');
    return rawLines.map((line, i) => ({
      num: i + 1,
      html: highlightJSONLine(line),
      hasError: editErrorLine === i + 1,
    }));
  }

  function highlightJSONLine(line: string): string {
    const escaped = line.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    return escaped.replace(
      /("(?:[^"\\]|\\.)*")\s*(:)|("(?:[^"\\]|\\.)*")|((?:-?\d+\.?\d*(?:[eE][+-]?\d+)?))|(\btrue\b|\bfalse\b|\bnull\b)|([{}[\]:,])/g,
      (_m, key, colon, str, num, boo, pu) => {
        if (key !== undefined && colon !== undefined) return `<span class="key">${key}</span><span class="pu">${colon}</span>`;
        if (str !== undefined) return `<span class="str">${str}</span>`;
        if (num !== undefined) return `<span class="num">${num}</span>`;
        if (boo !== undefined) return `<span class="boo">${boo}</span>`;
        if (pu !== undefined) return `<span class="pu">${pu}</span>`;
        return _m;
      }
    );
  }

  const editLines = $derived(editingValue ? getEditLines(editingValue) : []);

  function deleteDocument(doc: any) {
    const id = doc._id;
    deleteFilter = JSON.stringify({ _id: id }, null, 2);
    showDeleteModal = true;
  }

  async function handleInsert() {
    try {
      const parsed = JSON.parse(insertDoc);
      const docs = Array.isArray(parsed) ? parsed : [parsed];
      for (const doc of docs) {
        await nosqlInsertDocument(connectionId, database, collection, JSON.stringify(doc));
      }
      showToast(`${docs.length} document${docs.length > 1 ? 's' : ''} inserted`, 'success');
      showInsert = false;
      insertDoc = makeInsertTemplate();
      loadDocuments();
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    }
  }

  function handleInsertKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && insertValid) {
      e.preventDefault();
      handleInsert();
    }
  }

  function formatInsertDoc() {
    try {
      const parsed = JSON.parse(insertDoc);
      insertDoc = JSON.stringify(parsed, null, 2);
      const r = validateJson(insertDoc);
      insertValid = r.valid;
      insertError = r.error;
      insertErrorLine = r.errorLine;
    } catch { /* already invalid, do nothing */ }
  }

  async function handleFileImport(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    input.value = '';

    try {
      const text = await file.text();
      let docs: any[];

      if (file.name.endsWith('.csv')) {
        docs = parseCsv(text);
      } else {
        docs = parseJsonDocs(text);
      }

      if (docs.length === 0) { showToast('No documents found in file', 'error'); return; }

      let inserted = 0;
      for (const doc of docs) {
        await nosqlInsertDocument(connectionId, database, collection, JSON.stringify(doc));
        inserted++;
      }
      showToast(`Imported ${inserted} document${inserted > 1 ? 's' : ''}`, 'success');
      loadDocuments();
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    }
  }

  async function handleUpdate() {
    try {
      const count = await nosqlUpdateDocument(connectionId, database, collection, updateFilter, updateDoc);
      showToast(`${count} document(s) updated`, 'success');
      showUpdateModal = false;
      loadDocuments();
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    }
  }

  async function handleDelete() {
    try {
      const count = await nosqlDeleteDocument(connectionId, database, collection, deleteFilter);
      showToast(`${count} document(s) deleted`, 'success');
      showDeleteModal = false;
      loadDocuments();
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    }
  }

  async function handleQueryExecute(val: string) {
    queryValue = val;
    loading = true;
    try {
      const result = await nosqlFindDocuments(connectionId, database, collection, val, sortQuery, 100, 0);
      queryResult = result.documents;
      queryDuration = result.durationMs;
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      loading = false;
    }
  }

  async function handleAggExecute(val: string) {
    aggPipeline = val;
    loading = true;
    try {
      const { nosqlAggregate } = await import('../commands');
      const result = await nosqlAggregate(connectionId, database, collection, val);
      queryResult = result.documents;
      queryDuration = result.durationMs;
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
    } finally {
      loading = false;
    }
  }

  function displayId(id: any): string {
    if (!id) return '';
    if (typeof id === 'string') return id;
    if (typeof id === 'object' && id.$oid) return id.$oid;
    return JSON.stringify(id);
  }

  function formatDoc(doc: any): string {
    try {
      return JSON.stringify(doc, null, 2);
    } catch {
      return String(doc);
    }
  }

  function docPreview(doc: any): string {
    const keys = Object.keys(doc).filter(k => k !== '_id').slice(0, 4);
    const parts = keys.map(k => {
      const v = doc[k];
      const val = typeof v === 'string' ? `"${v.length > 30 ? v.slice(0, 30) + '...' : v}"` :
                  v === null ? 'null' :
                  typeof v === 'object' ? (Array.isArray(v) ? `[${v.length}]` : '{...}') :
                  String(v);
      return `${k}: ${val}`;
    });
    return parts.join('  ·  ') + (Object.keys(doc).length > keys.length + 1 ? '  ...' : '');
  }

  const currentPage = $derived(Math.floor(skip / limit) + 1);
  const totalPages = $derived(Math.max(1, Math.ceil(totalCount / limit)));
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<svelte:window onclick={handleWindowClick} />
<div class="dv-wrap">
  <!-- Mode tabs + actions -->
  <div class="dv-tabs">
    <button class="dv-tab" class:on={activeTab === 'documents'} onclick={() => activeTab = 'documents'}>
      <svg viewBox="0 0 24 24" width="12" height="12"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
      Documents
    </button>
    <button class="dv-tab" class:on={activeTab === 'query'} onclick={() => activeTab = 'query'}>
      <svg viewBox="0 0 24 24" width="12" height="12"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
      Query
    </button>
    <button class="dv-tab" class:on={activeTab === 'aggregation'} onclick={() => activeTab = 'aggregation'}>
      <svg viewBox="0 0 24 24" width="12" height="12"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
      Aggregation
    </button>
    <div class="dv-tab-spacer"></div>
    {#if activeTab === 'documents'}
      <span class="dv-meta">{totalCount.toLocaleString()} docs</span>
      <span class="dv-meta dv-meta-sep">·</span>
      <span class="dv-meta">{durationMs}ms</span>
      <div class="dv-add-wrap">
        <button class="dv-toolbar-btn" onclick={(e) => { e.stopPropagation(); showAddMenu = !showAddMenu; }} title="Add Data">
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
          Add Data
          <svg viewBox="0 0 24 24" width="9" height="9" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M6 9l6 6 6-6"/></svg>
        </button>
        {#if showAddMenu}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="dv-add-menu" onclick={() => showAddMenu = false}>
            <button class="dv-add-menu-item" onclick={() => { showAddMenu = false; insertDoc = makeInsertTemplate(); insertValid = true; insertError = ''; insertErrorLine = -1; showInsert = true; }}>
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><path d="M12 11v6M9 14h6"/></svg>
              Insert Document
            </button>
            <button class="dv-add-menu-item" onclick={() => { showAddMenu = false; fileInput?.click(); }}>
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
              Import JSON or CSV File
            </button>
          </div>
        {/if}
        <input bind:this={fileInput} type="file" accept=".json,.csv" style="display:none" onchange={handleFileImport} />
      </div>
      <button class="dv-toolbar-btn" onclick={() => loadDocuments()} title="Refresh">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 11-2.12-9.36L23 10"/></svg>
      </button>
    {/if}
  </div>

  {#if activeTab === 'documents'}
    <!-- Filter bar -->
    <div class="dv-filter-bar">
      <textarea
        class="dv-filter-input"
        bind:value={filterInput}
        placeholder={'{"field": "value"}'}
        spellcheck="false"
        rows="1"
        oninput={(e) => {
          const t = e.currentTarget;
          t.style.height = 'auto';
          t.style.height = Math.min(t.scrollHeight, 200) + 'px';
        }}
        onkeydown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            // Cmd+Enter always executes the filter
            e.preventDefault();
            executeFilter();
          }
        }}
      ></textarea>
      <div class="dv-bar-actions">
        <button class="dv-bar-btn dv-find-btn" onclick={executeFilter}>Find</button>
        <button class="dv-bar-btn dv-reset-btn" onclick={clearFilter}>Reset</button>
        <button class="dv-bar-btn dv-options-btn" class:active={showOptions} onclick={() => showOptions = !showOptions}>
          Options
          <svg viewBox="0 0 24 24" width="9" height="9" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d={showOptions ? 'M18 15l-6-6-6 6' : 'M6 9l6 6 6-6'}/></svg>
        </button>
        <select class="dv-limit-sel" bind:value={limit} onchange={() => { skip = 0; loadDocuments(); }} title="Per page">
          <option value={10}>10</option>
          <option value={25}>25</option>
          <option value={50}>50</option>
          <option value={100}>100</option>
        </select>
        <button class="dv-page-btn" onclick={prevPage} disabled={skip === 0}>
          <svg viewBox="0 0 24 24" width="10" height="10"><polyline points="15 18 9 12 15 6"/></svg>
        </button>
        <span class="dv-page-info">{currentPage} / {totalPages}</span>
        <button class="dv-page-btn" onclick={nextPage} disabled={skip + limit >= totalCount}>
          <svg viewBox="0 0 24 24" width="10" height="10"><polyline points="9 18 15 12 9 6"/></svg>
        </button>
      </div>
    </div>

    {#if showOptions}
      <div class="dv-options-row">
        <span class="dv-option-label">Sort</span>
        <input
          class="dv-option-input"
          type="text"
          bind:value={sortInput}
          placeholder={'{"_id": -1}'}
          spellcheck="false"
          onkeydown={(e) => { if (e.key === 'Enter') executeFilter(); }}
        />
      </div>
    {/if}

    <!-- Search bar -->
    {#if showSearch}
      <div class="dv-search-bar">
        <svg viewBox="0 0 24 24" width="13" height="13"><circle cx="11" cy="11" r="8" stroke="currentColor" fill="none" stroke-width="2"/><path d="M21 21l-4.35-4.35" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round"/></svg>
        <input
          bind:this={searchInputRef}
          class="dv-search-input"
          type="text"
          placeholder="Search in documents..."
          bind:value={searchQuery}
          onkeydown={handleSearchKeydown}
        />
        {#if searchQuery}
          <span class="dv-search-count">{matchCount > 0 ? `${currentMatchIndex + 1} of ${matchCount}` : 'No matches'}</span>
        {/if}
        <button class="dv-search-nav" onclick={() => { currentMatchIndex = (currentMatchIndex - 1 + matchCount) % matchCount; scrollToSearchMatch(); }} disabled={matchCount === 0}>
          <svg viewBox="0 0 24 24" width="11" height="11"><path d="M18 15l-6-6-6 6" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round"/></svg>
        </button>
        <button class="dv-search-nav" onclick={() => { currentMatchIndex = (currentMatchIndex + 1) % matchCount; scrollToSearchMatch(); }} disabled={matchCount === 0}>
          <svg viewBox="0 0 24 24" width="11" height="11"><path d="M6 9l6 6 6-6" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round"/></svg>
        </button>
        <button class="dv-search-nav" onclick={closeSearch}>
          <svg viewBox="0 0 24 24" width="11" height="11"><path d="M18 6L6 18M6 6l12 12" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round"/></svg>
        </button>
      </div>
    {/if}

    <!-- Document list -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dv-list" bind:this={docListRef} onkeydown={handleDocKeydown} tabindex="-1">
      {#if loading}
        <div class="dv-empty-state">
          <svg viewBox="0 0 24 24" width="32" height="32"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>
          <span>Loading<span class="dv-dots"></span></span>
        </div>
      {:else if documents.length === 0}
        <div class="dv-empty-state">
          <svg viewBox="0 0 24 24" width="32" height="32"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
          <span>No documents found</span>
        </div>
      {:else}
        {#each documents as doc, i (i)}
          {@const isExpanded = expandedDocs.has(i)}
          <div class="dv-doc" class:expanded={isExpanded}>
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="dv-doc-header" onclick={() => toggleExpand(i)}>
              <span class="dv-doc-chevron" class:open={isExpanded}>
                <svg viewBox="0 0 24 24" width="10" height="10"><polyline points="9 18 15 12 9 6"/></svg>
              </span>
              <span class="dv-doc-idx">#{skip + i + 1}</span>
              <span class="dv-doc-id">{displayId(doc._id)}</span>
              <div class="dv-doc-actions">
                <button class="dv-doc-btn" title="Copy" onclick={(e) => { e.stopPropagation(); copyDocument(doc); }}>
                  <svg viewBox="0 0 24 24" width="12" height="12"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>
                </button>
                <button class="dv-doc-btn" title="Edit" onclick={(e) => { e.stopPropagation(); editDocument(doc); }}>
                  <svg viewBox="0 0 24 24" width="12" height="12"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>
                </button>
                <button class="dv-doc-btn danger" title="Delete" onclick={(e) => { e.stopPropagation(); deleteDocument(doc); }}>
                  <svg viewBox="0 0 24 24" width="12" height="12"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
                </button>
              </div>
            </div>
            {#if isExpanded}
              {#if editingIdx === i}
                <div class="dv-doc-edit-wrap">
                  <div class="dv-edit-editor">
                    <div class="dv-edit-gutter">
                      {#each editingValue.split('\n') as _, ln}
                        <span class="dv-edit-ln" class:dv-ln-err={editErrorLine === ln + 1}>{ln + 1}</span>
                      {/each}
                    </div>
                    <textarea
                      class="dv-edit-ta"
                      style="height:{editTaHeight}"
                      value={editingValue}
                      oninput={handleEditInput}
                      spellcheck="false"
                    ></textarea>
                  </div>
                  {#if !editJsonValid}
                    <div class="dv-edit-error">
                      <svg viewBox="0 0 24 24" width="12" height="12"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
                      {editErrorMsg}
                    </div>
                  {/if}
                  <div class="dv-doc-edit-actions">
                    <button class="dv-edit-cancel" onclick={cancelInlineEdit}>Cancel</button>
                    <button class="dv-edit-save" disabled={!editJsonValid} onclick={() => saveInlineEdit(doc)}>Update</button>
                  </div>
                </div>
              {:else}
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div class="dv-doc-body" ondblclick={() => startInlineEdit(i, doc)}>
                  {@html searchQuery ? highlightSearchInDoc(formatDoc(doc)) : highlightJSON(formatDoc(doc))}
                </div>
              {/if}
            {:else}
              <div class="dv-doc-preview">{docPreview(doc)}</div>
            {/if}
          </div>
        {/each}
      {/if}
    </div>


    <!-- Insert Document modal (Compass-style) -->
    {#if showInsert}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="dv-modal-overlay" onclick={(e) => { if (e.target === e.currentTarget) showInsert = false; }}>
        <div class="dv-modal dv-insert-modal">
          <div class="dv-insert-header">
            <div>
              <div class="dv-insert-title">Insert Document</div>
              <div class="dv-insert-subtitle">To collection <strong>{database}</strong>.<strong>{collection}</strong></div>
            </div>
            <button class="dv-modal-close" onclick={() => showInsert = false}>
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
            </button>
          </div>
          <div class="dv-insert-toolbar">
            <button class="dv-insert-tool-btn" onclick={formatInsertDoc} title="Format JSON">
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
              Format
            </button>
            {#if insertDocCount > 0}
              <span class="dv-insert-doc-count">{insertDocCount} document{insertDocCount > 1 ? 's' : ''}</span>
            {/if}
          </div>
          <div class="dv-insert-editor-area">
            <div class="dv-insert-editor-container">
              <div class="dv-edit-editor dv-insert-editor">
                <div class="dv-edit-gutter">
                  {#each insertDoc.split('\n') as _, ln}
                    <span class="dv-edit-ln" class:dv-ln-err={insertErrorLine === ln + 1}>{ln + 1}</span>
                  {/each}
                </div>
                <textarea
                  class="dv-edit-ta"
                  style="min-height:300px"
                  value={insertDoc}
                  oninput={handleInsertInput}
                  onkeydown={handleInsertKeydown}
                  spellcheck="false"
                  placeholder="Paste one or more documents here"
                ></textarea>
              </div>
              <div class="dv-insert-highlight" aria-hidden="true">
                {@html highlightJSON(insertDoc)}
              </div>
            </div>
            {#if !insertValid}
              <div class="dv-edit-error">
                <svg viewBox="0 0 24 24" width="12" height="12"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
                {insertError}
              </div>
            {/if}
          </div>
          <div class="dv-insert-footer">
            <span class="dv-insert-hint">
              <kbd>&#8984;Enter</kbd> to insert
            </span>
            <button class="dv-modal-cancel" onclick={() => showInsert = false}>Cancel</button>
            <button class="dv-modal-primary" disabled={!insertValid} onclick={handleInsert}>
              Insert{insertDocCount > 1 ? ` ${insertDocCount} Docs` : ''}
            </button>
          </div>
        </div>
      </div>
    {/if}

    <!-- Update modal -->
    {#if showUpdateModal}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="dv-modal-overlay" onclick={(e) => { if (e.target === e.currentTarget) showUpdateModal = false; }}>
        <div class="dv-modal dv-modal-lg">
          <div class="dv-modal-hdr">
            <span>Update Document</span>
            <button class="dv-modal-close" onclick={() => showUpdateModal = false}>&times;</button>
          </div>
          <div class="dv-modal-editor-wrap">
            <span class="dv-modal-label">Filter</span>
            <div class="dv-edit-editor dv-modal-editor-sm">
              <div class="dv-edit-gutter">
                {#each updateFilter.split('\n') as _, ln}
                  <span class="dv-edit-ln" class:dv-ln-err={updateFilterErrorLine === ln + 1}>{ln + 1}</span>
                {/each}
              </div>
              <textarea class="dv-edit-ta" style="height:{taHeight(updateFilter)}" value={updateFilter} oninput={handleUpdateFilterInput} spellcheck="false"></textarea>
            </div>
            {#if !updateFilterValid}
              <div class="dv-edit-error">
                <svg viewBox="0 0 24 24" width="12" height="12"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
                {updateFilterError}
              </div>
            {/if}
          </div>
          <div class="dv-modal-editor-wrap">
            <span class="dv-modal-label">Update</span>
            <div class="dv-edit-editor dv-modal-editor">
              <div class="dv-edit-gutter">
                {#each updateDoc.split('\n') as _, ln}
                  <span class="dv-edit-ln" class:dv-ln-err={updateDocErrorLine === ln + 1}>{ln + 1}</span>
                {/each}
              </div>
              <textarea class="dv-edit-ta" style="height:{taHeight(updateDoc)}" value={updateDoc} oninput={handleUpdateDocInput} spellcheck="false"></textarea>
            </div>
            {#if !updateDocValid}
              <div class="dv-edit-error">
                <svg viewBox="0 0 24 24" width="12" height="12"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
                {updateDocError}
              </div>
            {/if}
          </div>
          <div class="dv-modal-actions">
            <button class="dv-modal-cancel" onclick={() => showUpdateModal = false}>Cancel</button>
            <button class="dv-modal-primary" disabled={!updateFilterValid || !updateDocValid} onclick={handleUpdate}>Update</button>
          </div>
        </div>
      </div>
    {/if}

    <!-- Delete modal -->
    {#if showDeleteModal}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="dv-modal-overlay" onclick={(e) => { if (e.target === e.currentTarget) showDeleteModal = false; }}>
        <div class="dv-modal dv-modal-lg">
          <div class="dv-modal-hdr">
            <span>Delete Document</span>
            <button class="dv-modal-close" onclick={() => showDeleteModal = false}>&times;</button>
          </div>
          <div class="dv-modal-editor-wrap">
            <span class="dv-modal-label">Filter</span>
            <div class="dv-edit-editor dv-modal-editor-sm">
              <div class="dv-edit-gutter">
                {#each deleteFilter.split('\n') as _, ln}
                  <span class="dv-edit-ln" class:dv-ln-err={deleteFilterErrorLine === ln + 1}>{ln + 1}</span>
                {/each}
              </div>
              <textarea class="dv-edit-ta" style="height:{taHeight(deleteFilter)}" value={deleteFilter} oninput={handleDeleteFilterInput} spellcheck="false"></textarea>
            </div>
            {#if !deleteFilterValid}
              <div class="dv-edit-error">
                <svg viewBox="0 0 24 24" width="12" height="12"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
                {deleteFilterError}
              </div>
            {/if}
          </div>
          <div class="dv-modal-actions">
            <button class="dv-modal-cancel" onclick={() => showDeleteModal = false}>Cancel</button>
            <button class="dv-modal-primary danger" disabled={!deleteFilterValid} onclick={handleDelete}>Delete</button>
          </div>
        </div>
      </div>
    {/if}
  {:else if activeTab === 'query'}
    <div class="dv-query-area">
      <QueryEditor bind:value={queryValue} label="Find Query" placeholder={'{"field": "value"}'} onexecute={handleQueryExecute} />
    </div>
    {#if queryResult}
      <div class="dv-query-result">
        <div class="dv-query-meta">
          <span>{queryResult.length} results</span>
          <span class="dv-meta-sep">·</span>
          <span>{queryDuration}ms</span>
        </div>
        <div class="dv-query-body">{@html highlightJSON(JSON.stringify(queryResult, null, 2))}</div>
      </div>
    {:else}
      <div class="dv-empty-state">
        <svg viewBox="0 0 24 24" width="32" height="32"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
        <span>Write a query and press Run</span>
      </div>
    {/if}
  {:else if activeTab === 'aggregation'}
    <div class="dv-query-area">
      <QueryEditor bind:value={aggPipeline} label="Aggregation Pipeline" placeholder={'[{"$match": {}}]'} onexecute={handleAggExecute} />
    </div>
    {#if queryResult}
      <div class="dv-query-result">
        <div class="dv-query-meta">
          <span>{queryResult.length} results</span>
          <span class="dv-meta-sep">·</span>
          <span>{queryDuration}ms</span>
        </div>
        <div class="dv-query-body">{@html highlightJSON(JSON.stringify(queryResult, null, 2))}</div>
      </div>
    {:else}
      <div class="dv-empty-state">
        <svg viewBox="0 0 24 24" width="32" height="32"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
        <span>Write a pipeline and press Run</span>
      </div>
    {/if}
  {/if}
</div>

<style>
  @import './DocumentViewer.svelte.css';
</style>

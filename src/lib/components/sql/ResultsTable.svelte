<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { SqlQueryResult } from '$lib/types/sql';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { showToast } from '$lib/components/shared/toast';
  import { setSqlPending, sqlRowLimit } from '$lib/stores/sql';
  import { sqlExecuteQuery } from '$lib/commands/sql_client';
  import { friendlyError } from '$lib/utils/errors';

  interface Props {
    result: SqlQueryResult | null;
    error: string | null;
    loading: boolean;
    tabId?: number;
    query?: string;
    liveConnectionId?: string;
    databaseName?: string;
  }

  let { result, error, loading, tabId = -1, query = '', liveConnectionId = '', databaseName = '' }: Props = $props();

  let limitInputValue = $state(String($sqlRowLimit));
  // Keep input in sync when store changes externally
  $effect(() => {
    limitInputValue = String($sqlRowLimit);
  });

  function handleLimitChange() {
    const parsed = parseInt(limitInputValue, 10);
    if (isNaN(parsed) || parsed < 0) {
      limitInputValue = String($sqlRowLimit);
      return;
    }
    sqlRowLimit.set(parsed);
  }

  const isLimitHit = $derived(
    result && $sqlRowLimit > 0 && result.rows.length >= $sqlRowLimit
  );

  // Save confirmation modal
  let showSaveModal = $state(false);
  let saveStatements = $state<string[]>([]);
  let savingChanges = $state(false);

  // Column widths
  let columnWidths = $state<number[]>([]);
  let resizingCol = $state<number | null>(null);
  let resizeStartX = $state(0);
  let resizeStartWidth = $state(0);

  // Inline editing
  let editingCell = $state<{ row: number; col: number } | null>(null);
  let editValue = $state('');
  let dirtyRows = $state<Map<number, Map<number, string>>>(new Map());
  let deletedRows = $state<Set<number>>(new Set());

  // Right-click menu
  let contextMenu = $state<{ x: number; y: number; rowIdx: number; colIdx: number } | null>(null);

  const hasPendingChanges = $derived(dirtyRows.size > 0 || deletedRows.size > 0);

  // Sync pending state to store so Topbar can check before closing tab
  $effect(() => {
    if (tabId >= 0) {
      setSqlPending(tabId, hasPendingChanges);
    }
  });

  // Reset state when result changes
  $effect(() => {
    if (result) {
      columnWidths = result.columns.map((col, i) => computeColumnWidth(col, i));
      editingCell = null;
      dirtyRows = new Map();
      deletedRows = new Set();
    }
  });

  // Listen for Cmd+S save event
  function handleGlobalSave() {
    if (hasPendingChanges) saveChanges();
  }

  onMount(() => {
    window.addEventListener('clauge:sql-save', handleGlobalSave);
  });

  onDestroy(() => {
    window.removeEventListener('clauge:sql-save', handleGlobalSave);
  });

  function computeColumnWidth(colName: string, colIdx: number): number {
    if (!result || result.rows.length === 0) return 150;
    // Sample first 20 rows to estimate width
    const samples = result.rows.slice(0, 20);
    const headerLen = colName.length;
    let maxLen = headerLen;
    for (const row of samples) {
      const val = formatValue(row[colIdx]);
      maxLen = Math.max(maxLen, Math.min(val.length, 50));
    }
    // ~7.5px per char + 24px padding
    const width = maxLen * 7.5 + 24;
    return Math.max(80, Math.min(400, width));
  }

  function formatValue(val: unknown): string {
    if (val === null || val === undefined) return 'NULL';
    if (typeof val === 'object') return JSON.stringify(val);
    return String(val);
  }

  function getValueType(val: unknown): string {
    if (val === null || val === undefined) return 'null';
    if (typeof val === 'boolean') return 'bool';
    if (typeof val === 'number') return 'num';
    if (typeof val === 'object') return 'json';
    const str = String(val);
    // UUID pattern
    if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(str)) return 'uuid';
    // Date/timestamp pattern
    if (/^\d{4}-\d{2}-\d{2}/.test(str)) return 'date';
    return 'str';
  }

  // Inline editing
  function getCellValue(rowIdx: number, colIdx: number): string {
    const dirty = dirtyRows.get(rowIdx);
    if (dirty?.has(colIdx)) return dirty.get(colIdx)!;
    return formatValue(result?.rows[rowIdx]?.[colIdx]);
  }

  function isCellDirty(rowIdx: number, colIdx: number): boolean {
    return dirtyRows.get(rowIdx)?.has(colIdx) ?? false;
  }

  let pendingDblClick: { row: number; col: number } | null = null;

  function handleCellDblClick(rowIdx: number, colIdx: number) {
    if (deletedRows.has(rowIdx)) return;
    if (editingCell) {
      pendingDblClick = { row: rowIdx, col: colIdx };
      commitEdit();
      return;
    }
    editingCell = { row: rowIdx, col: colIdx };
    editValue = getCellValue(rowIdx, colIdx);
  }

  function commitEdit() {
    if (!editingCell || !result) return;
    const { row, col } = editingCell;
    const originalValue = formatValue(result.rows[row]?.[col]);
    if (editValue !== originalValue) {
      const rowDirty = new Map(dirtyRows.get(row) ?? []);
      rowDirty.set(col, editValue);
      dirtyRows = new Map(dirtyRows);
      dirtyRows.set(row, rowDirty);
    }
    editingCell = null;

    // Open pending double-click cell after commit
    if (pendingDblClick) {
      const { row: r, col: c } = pendingDblClick;
      pendingDblClick = null;
      requestAnimationFrame(() => {
        editingCell = { row: r, col: c };
        editValue = getCellValue(r, c);
      });
    }
  }

  function cancelEdit() {
    editingCell = null;
    pendingDblClick = null;
  }

  function handleEditKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitEdit();
    } else if (e.key === 'Escape') {
      cancelEdit();
    } else if (e.key === 'Tab') {
      // Tab to next cell
      e.preventDefault();
      commitEdit();
      if (result && editingCell) {
        const nextCol = editingCell.col + 1;
        if (nextCol < result.columns.length) {
          requestAnimationFrame(() => handleCellDblClick(editingCell!.row, nextCol));
        }
      }
    }
  }

  function handleEditBlur() {
    // Delay commit to allow double-click on another cell to register first
    setTimeout(() => {
      if (editingCell && !pendingDblClick) {
        commitEdit();
      }
    }, 150);
  }

  function discardChanges() {
    dirtyRows = new Map();
    deletedRows = new Set();
    showToast('Changes discarded', 'info');
  }

  function parseTableName(sql: string): string | null {
    const match = sql.match(/\bFROM\s+([`"']?[\w.]+[`"']?)/i);
    return match ? match[1].replace(/[`"']/g, '') : null;
  }

  function escapeVal(val: string): string {
    if (val === 'NULL') return 'NULL';
    return `'${val.replace(/'/g, "''")}'`;
  }

  function buildWhereClause(row: unknown[], cols: string[]): string {
    // Use first column (usually PK) for WHERE
    const pk = cols[0];
    const val = formatValue(row[0]);
    if (val === 'NULL') return `${pk} IS NULL`;
    return `${pk} = ${escapeVal(val)}`;
  }

  function saveChanges() {
    if (!result || !hasPendingChanges) return;

    const tableName = parseTableName(query);
    if (!tableName) {
      showToast('Could not determine table name from query', 'error');
      return;
    }

    const cols = result.columns;
    const stmts: string[] = [];

    for (const [rowIdx, changes] of dirtyRows) {
      const row = result.rows[rowIdx];
      const setClauses: string[] = [];
      for (const [colIdx, newVal] of changes) {
        setClauses.push(`${cols[colIdx]} = ${escapeVal(newVal)}`);
      }
      const where = buildWhereClause(row, cols);
      stmts.push(`UPDATE ${tableName} SET ${setClauses.join(', ')} WHERE ${where};`);
    }

    for (const rowIdx of deletedRows) {
      const row = result.rows[rowIdx];
      const where = buildWhereClause(row, cols);
      stmts.push(`DELETE FROM ${tableName} WHERE ${where};`);
    }

    saveStatements = stmts;
    showSaveModal = true;
  }

  async function executeSave() {
    if (!liveConnectionId || saveStatements.length === 0) return;
    savingChanges = true;
    let successCount = 0;
    let errorCount = 0;

    for (const sql of saveStatements) {
      try {
        await sqlExecuteQuery(liveConnectionId, sql);
        successCount++;
      } catch (e: any) {
        errorCount++;
        showToast(`Failed: ${e}`, 'error');
      }
    }

    savingChanges = false;
    showSaveModal = false;

    if (successCount > 0) {
      showToast(`${successCount} change(s) saved`, 'success');

      // Apply edits in-place (like DBeaver) — no re-query, row stays in position
      if (result) {
        const updatedRows = [...result.rows];
        // Apply dirty cell changes
        for (const [rowIdx, changes] of dirtyRows) {
          updatedRows[rowIdx] = [...updatedRows[rowIdx]];
          for (const [colIdx, newVal] of changes) {
            updatedRows[rowIdx][colIdx] = newVal === 'NULL' ? null : newVal;
          }
        }
        // Remove deleted rows (iterate in reverse to keep indices valid)
        const sortedDeleted = [...deletedRows].sort((a, b) => b - a);
        for (const rowIdx of sortedDeleted) {
          updatedRows.splice(rowIdx, 1);
        }
        result = { ...result, rows: updatedRows };
      }

      dirtyRows = new Map();
      deletedRows = new Set();
      saveStatements = [];
    }
  }

  function handleDeleteRow(rowIdx: number) {
    deletedRows = new Set([...deletedRows, rowIdx]);
  }

  function handleUndoDelete(rowIdx: number) {
    deletedRows = new Set([...deletedRows].filter(r => r !== rowIdx));
  }

  // Column resize
  function handleResizeStart(e: MouseEvent, colIdx: number) {
    e.preventDefault();
    e.stopPropagation();
    resizingCol = colIdx;
    resizeStartX = e.clientX;
    resizeStartWidth = columnWidths[colIdx];

    const onMousemove = (ev: MouseEvent) => {
      if (resizingCol === null) return;
      const diff = ev.clientX - resizeStartX;
      columnWidths = columnWidths.map((w, i) => i === resizingCol ? Math.max(60, resizeStartWidth + diff) : w);
    };

    const onMouseup = () => {
      resizingCol = null;
      window.removeEventListener('mousemove', onMousemove);
      window.removeEventListener('mouseup', onMouseup);
    };

    window.addEventListener('mousemove', onMousemove);
    window.addEventListener('mouseup', onMouseup);
  }

  // Context menu
  function handleCellContextMenu(e: MouseEvent, rowIdx: number, colIdx: number) {
    e.preventDefault();
    contextMenu = { x: e.clientX, y: e.clientY, rowIdx, colIdx };
  }

  function closeContextMenu() {
    contextMenu = null;
  }

  async function copyToClipboard(text: string) {
    try {
      await writeText(text);
      showToast('Copied', 'success');
    } catch {
      showToast('Failed to copy', 'error');
    }
  }

  async function copyCellValue() {
    if (!contextMenu || !result) return;
    const val = result.rows[contextMenu.rowIdx]?.[contextMenu.colIdx];
    const text = typeof val === 'object' && val !== null ? JSON.stringify(val, null, 2) : formatValue(val);
    await copyToClipboard(text);
    closeContextMenu();
  }

  async function copyRow() {
    if (!contextMenu || !result) return;
    const row = result.rows[contextMenu.rowIdx];
    const obj: Record<string, unknown> = {};
    result.columns.forEach((col, i) => { obj[col] = row[i]; });
    await copyToClipboard(JSON.stringify(obj, null, 2));
    closeContextMenu();
  }

  async function copyAllResults() {
    if (!result) return;
    const header = result.columns.join('\t');
    const rows = result.rows.map(row => row.map(formatValue).join('\t')).join('\n');
    await copyToClipboard(header + '\n' + rows);
  }

  async function exportCsv() {
    if (!result) return;
    const escape = (v: string) => {
      if (v.includes(',') || v.includes('"') || v.includes('\n')) return '"' + v.replace(/"/g, '""') + '"';
      return v;
    };
    const header = result.columns.map(escape).join(',');
    const rows = result.rows.map(row => row.map(cell => escape(formatValue(cell))).join(',')).join('\n');
    await copyToClipboard(header + '\n' + rows);
  }

  const totalWidth = $derived(
    40 + (columnWidths.length > 0 ? columnWidths.reduce((a, b) => a + b, 0) : 0)
  );
</script>

<svelte:window onclick={closeContextMenu} />

<div class="results-table">
  {#if loading}
    <div class="rt-empty">
      <span class="rt-loading">Executing query<span class="loading-dots"></span></span>
    </div>
  {:else if error}
    <div class="rt-error">
      <div class="rt-error-icon">
        <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M12 8v4M12 16h.01"/></svg>
      </div>
      <div class="rt-error-msg">{friendlyError(error)}</div>
    </div>
  {:else if !result}
    <div class="rt-empty">
      <div class="rt-empty-icon">
        <svg viewBox="0 0 24 24">
          <ellipse cx="12" cy="5" rx="9" ry="3"/>
          <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/>
          <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
        </svg>
      </div>
      <div class="rt-empty-text">Execute a query to see results</div>
    </div>
  {:else if result.columns.length === 0}
    <div class="rt-empty">
      <div class="rt-affected">
        {result.affectedRows} row{result.affectedRows !== 1 ? 's' : ''} affected
      </div>
      <div class="rt-duration">{result.durationMs}ms</div>
    </div>
  {:else}
    <div class="rt-scroll" class:resizing={resizingCol !== null}>
      <table class="rt-table" style="min-width:{totalWidth}px">
        <thead>
          <tr>
            <th class="rt-row-num">#</th>
            {#each result.columns as col, colIdx}
              <th style="width:{columnWidths[colIdx]}px;min-width:{columnWidths[colIdx]}px">
                <span class="rt-th-text">{col}</span>
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                  class="rt-resize-handle"
                  onmousedown={(e) => handleResizeStart(e, colIdx)}
                ></div>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each result.rows as row, rowIdx}
            <tr class:rt-deleted={deletedRows.has(rowIdx)}>
              <td class="rt-row-num">{rowIdx + 1}</td>
              {#each row as cell, colIdx}
                {@const vtype = getValueType(cell)}
                {@const editing = editingCell?.row === rowIdx && editingCell?.col === colIdx}
                {@const dirty = isCellDirty(rowIdx, colIdx)}
                {#if editing}
                  <td class="rt-editing" style="width:{columnWidths[colIdx]}px">
                    <textarea
                      class="rt-edit-input"
                      value={editValue}
                      oninput={(e) => editValue = (e.target as HTMLTextAreaElement).value}
                      onblur={handleEditBlur}
                      onkeydown={handleEditKeydown}
                      rows="1"
                      autofocus
                    ></textarea>
                  </td>
                {:else}
                  <td
                    class="rt-cell rt-type-{vtype}"
                    class:rt-dirty={dirty}
                    style="width:{columnWidths[colIdx]}px"
                    ondblclick={() => handleCellDblClick(rowIdx, colIdx)}
                    oncontextmenu={(e) => handleCellContextMenu(e, rowIdx, colIdx)}
                  >
                    <span class="rt-cell-text">{dirty ? getCellValue(rowIdx, colIdx) : formatValue(cell)}</span>
                  </td>
                {/if}
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <div class="rt-footer">
      <span class="rt-footer-item">{result.rows.length} row{result.rows.length !== 1 ? 's' : ''}</span>
      {#if isLimitHit}
        <span class="rt-footer-truncated" title="Result was limited to {$sqlRowLimit} rows">limit reached</span>
      {/if}
      <span class="rt-footer-sep">&middot;</span>
      <span class="rt-footer-item">{result.columns.length} col{result.columns.length !== 1 ? 's' : ''}</span>
      <span class="rt-footer-sep">&middot;</span>
      <span class="rt-footer-item">{result.durationMs}ms</span>
      {#if result.affectedRows > 0}
        <span class="rt-footer-sep">&middot;</span>
        <span class="rt-footer-item">{result.affectedRows} affected</span>
      {/if}
      {#if hasPendingChanges}
        <span class="rt-footer-sep">&middot;</span>
        <span class="rt-footer-dirty">{dirtyRows.size + deletedRows.size} pending</span>
        <button class="rt-footer-btn rt-save-btn" onclick={saveChanges} title="Generate SQL (Cmd+S)">
          Save
        </button>
        <button class="rt-footer-btn rt-discard-btn" onclick={discardChanges} title="Discard changes">
          Discard
        </button>
      {/if}
      <div class="rt-footer-spacer"></div>
      <label class="rt-limit-label" title="Max rows to fetch (0 = no limit)">
        Limit
        <input
          class="rt-limit-input"
          type="number"
          min="0"
          step="100"
          bind:value={limitInputValue}
          onblur={handleLimitChange}
          onkeydown={(e) => { if (e.key === 'Enter') { e.currentTarget.blur(); } }}
        />
      </label>
      <span class="rt-footer-sep">&middot;</span>
      <button class="rt-footer-btn" onclick={exportCsv} title="Export CSV">
        <svg viewBox="0 0 24 24"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
        CSV
      </button>
      <button class="rt-footer-btn" onclick={copyAllResults} title="Copy all">
        <svg viewBox="0 0 24 24"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>
        Copy
      </button>
    </div>
  {/if}
</div>

<!-- Context Menu -->
{#if contextMenu}
  <div class="rt-ctx" style="left:{contextMenu.x}px;top:{contextMenu.y}px">
    <button class="rt-ctx-item" onclick={copyCellValue}>Copy Cell</button>
    <button class="rt-ctx-item" onclick={copyRow}>Copy Row as JSON</button>
    <div class="rt-ctx-sep"></div>
    {#if deletedRows.has(contextMenu.rowIdx)}
      <button class="rt-ctx-item" onclick={() => { handleUndoDelete(contextMenu!.rowIdx); closeContextMenu(); }}>Undo Delete</button>
    {:else}
      <button class="rt-ctx-item rt-ctx-danger" onclick={() => { handleDeleteRow(contextMenu!.rowIdx); closeContextMenu(); }}>Delete Row</button>
    {/if}
    <div class="rt-ctx-sep"></div>
    <button class="rt-ctx-item" onclick={() => { copyAllResults(); closeContextMenu(); }}>Copy All</button>
    <button class="rt-ctx-item" onclick={() => { exportCsv(); closeContextMenu(); }}>Export CSV</button>
  </div>
{/if}

<!-- Save Confirmation Modal -->
{#if showSaveModal}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="rt-save-overlay" onclick={(e) => { if (e.target === e.currentTarget) showSaveModal = false; }}>
    <div class="rt-save-modal">
      <div class="rt-save-header">
        <span class="rt-save-title">Review Changes</span>
        <span class="rt-save-db">{databaseName || 'Unknown database'}</span>
        <button class="rt-save-close" onclick={() => showSaveModal = false}>&times;</button>
      </div>
      <div class="rt-save-body">
        <div class="rt-save-count">{saveStatements.length} statement{saveStatements.length !== 1 ? 's' : ''} will be executed:</div>
        <div class="rt-save-sql">
          {#each saveStatements as stmt, i}
            <div class="rt-save-stmt">
              <span class="rt-save-ln">{i + 1}</span>
              <span class="rt-save-code">{stmt}</span>
            </div>
          {/each}
        </div>
      </div>
      <div class="rt-save-footer">
        <button class="rt-save-cancel" onclick={() => showSaveModal = false}>Cancel</button>
        <button class="rt-save-execute" disabled={savingChanges} onclick={executeSave}>
          {savingChanges ? 'Executing...' : 'Execute'}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .results-table {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }

  .rt-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }
  .rt-empty-icon svg {
    width: 36px; height: 36px;
    stroke: var(--t4); fill: none; stroke-width: 1.2; stroke-linecap: round;
  }
  .rt-empty-text { font-size: 12px; color: var(--t3); font-family: var(--mono); }
  .rt-loading { font-size: 12px; color: var(--t3); font-family: var(--mono); }

  .loading-dots::after { content: ''; animation: dots 1.4s steps(4, end) infinite; }
  @keyframes dots { 0% { content: ''; } 25% { content: '.'; } 50% { content: '..'; } 75% { content: '...'; } }

  .rt-error {
    flex: 1; display: flex; flex-direction: column;
    align-items: center; justify-content: center; gap: 10px; padding: 24px;
  }
  .rt-error-icon svg { width: 28px; height: 28px; stroke: var(--err); fill: none; stroke-width: 1.5; stroke-linecap: round; }
  .rt-error-msg { font-size: 12px; color: var(--err); font-family: var(--mono); text-align: center; max-width: 400px; line-height: 1.6; word-break: break-word; font-weight: 500; }

  .rt-affected { font-size: 14px; color: var(--acc); font-family: var(--mono); font-weight: 600; }
  .rt-duration { font-size: 11px; color: var(--t3); font-family: var(--mono); }

  /* Scroll area */
  .rt-scroll { flex: 1; overflow: auto; }
  .rt-scroll.resizing { user-select: none; }
  .rt-scroll::-webkit-scrollbar { width: 4px; height: 4px; }
  .rt-scroll::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  /* Table */
  .rt-table { border-collapse: collapse; font-family: var(--mono); font-size: 12px; table-layout: fixed; }

  .rt-table thead { position: sticky; top: 0; z-index: 2; }

  .rt-table th {
    background: var(--n2);
    color: var(--t2);
    font-weight: 600;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 8px 12px;
    border-bottom: 2px solid var(--b1);
    border-right: 1px solid color-mix(in srgb, var(--b1) 50%, transparent);
    text-align: left;
    white-space: nowrap;
    user-select: none;
    position: relative;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rt-th-text { overflow: hidden; text-overflow: ellipsis; }

  .rt-resize-handle {
    position: absolute; top: 0; right: 0; width: 5px; height: 100%;
    cursor: col-resize; background: transparent; transition: background 0.1s;
  }
  .rt-resize-handle:hover { background: var(--acc); }

  /* Cells — max 3 lines, then clip */
  .rt-cell {
    padding: 7px 12px;
    border-bottom: 1px solid var(--b1);
    border-right: 1px solid color-mix(in srgb, var(--b1) 50%, transparent);
    color: var(--t1);
    cursor: default;
    max-height: 57px;
    overflow: hidden;
  }
  .rt-cell:last-child { border-right: none; }
  .rt-cell-text {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    word-break: break-word;
    line-height: 19px;
  }

  /* Type-specific styling */
  .rt-type-null { color: var(--t4) !important; font-style: italic; }
  .rt-type-null .rt-cell-text { color: var(--t4); }
  .rt-type-bool .rt-cell-text { color: #eb6f92; font-weight: 500; }
  .rt-type-num .rt-cell-text { color: #f6c177; }
  .rt-type-json .rt-cell-text { color: var(--t3); cursor: pointer; }
  .rt-type-uuid .rt-cell-text { color: var(--t3); font-size: 11px; }
  .rt-type-date .rt-cell-text { color: #9ccfd8; }
  .rt-type-str .rt-cell-text { color: var(--t1); }

  .rt-table tr:hover td { background: color-mix(in srgb, var(--acc) 3%, transparent); }

  .rt-dirty {
    background: rgba(245, 166, 35, 0.1) !important;
    border-left: 2px solid var(--warn) !important;
  }
  .rt-deleted td {
    text-decoration: line-through;
    opacity: 0.4;
    background: rgba(240, 68, 68, 0.06) !important;
  }

  .rt-editing { padding: 0 !important; }
  .rt-edit-input {
    width: 100%;
    min-height: 28px;
    max-height: 95px;
    padding: 5px 12px;
    border: 2px solid var(--acc);
    border-radius: 0;
    background: color-mix(in srgb, var(--acc) 8%, transparent);
    font-family: var(--mono); font-size: 11.5px;
    line-height: 19px;
    color: var(--t1); outline: none;
    box-sizing: border-box;
    resize: none;
    overflow-y: auto;
    white-space: pre-wrap;
    word-break: break-word;
    field-sizing: content;
  }

  .rt-row-num {
    color: var(--t4) !important;
    font-size: 10px;
    width: 40px; min-width: 40px;
    text-align: center;
    user-select: none;
    border-right: 1px solid var(--b1);
  }

  /* Footer */
  .rt-footer {
    height: 32px; flex-shrink: 0;
    background: var(--n2); border-top: 1px solid var(--b1);
    padding: 0 12px; display: flex; align-items: center; gap: 6px;
  }
  .rt-footer-item { font-size: 10.5px; color: var(--t3); font-family: var(--mono); }
  .rt-footer-sep { color: var(--t4); font-size: 10px; }
  .rt-footer-spacer { flex: 1; }
  .rt-footer-btn {
    height: 22px; padding: 0 8px; border-radius: 4px;
    border: 1px solid var(--b1); background: transparent;
    color: var(--t3); font-size: 10px; font-family: var(--ui);
    cursor: default; display: flex; align-items: center; gap: 4px;
    transition: color 0.1s, border-color 0.1s, background 0.1s;
  }
  .rt-footer-btn:hover { color: var(--t1); border-color: var(--b2); background: rgba(255,255,255,0.03); }
  .rt-save-btn { color: var(--acc); border-color: color-mix(in srgb, var(--acc) 40%, transparent); font-weight: 600; }
  .rt-save-btn:hover { color: #fff; border-color: var(--acc); background: var(--acc); }
  .rt-discard-btn { color: var(--err); border-color: color-mix(in srgb, var(--err) 30%, transparent); }
  .rt-discard-btn:hover { color: var(--err); border-color: var(--err); background: color-mix(in srgb, var(--err) 8%, transparent); }
  .rt-footer-dirty { font-size: 10.5px; color: var(--warn); font-family: var(--mono); font-weight: 600; }
  .rt-footer-truncated {
    font-size: 9.5px; color: var(--warn); font-family: var(--ui); font-weight: 600;
    background: color-mix(in srgb, var(--warn) 12%, transparent);
    padding: 1px 5px; border-radius: 3px; margin-left: 2px;
  }
  .rt-limit-label {
    display: flex; align-items: center; gap: 4px;
    font-size: 10.5px; color: var(--t3); font-family: var(--ui);
  }
  .rt-limit-input {
    width: 52px; height: 20px; padding: 0 4px;
    border: 1px solid var(--b1); border-radius: 4px;
    background: transparent; color: var(--t2);
    font-size: 10.5px; font-family: var(--mono);
    text-align: center; outline: none;
    transition: border-color 0.15s;
    -moz-appearance: textfield;
  }
  .rt-limit-input:focus { border-color: var(--acc); }
  .rt-limit-input::-webkit-inner-spin-button,
  .rt-limit-input::-webkit-outer-spin-button { -webkit-appearance: none; margin: 0; }
  .rt-footer-btn svg { width: 11px; height: 11px; stroke: currentColor; fill: none; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }

  /* Context menu */
  .rt-ctx {
    position: fixed; background: var(--n); border: 1px solid var(--b1);
    border-radius: 6px; padding: 4px 0; min-width: 140px; z-index: 1000;
    box-shadow: 0 8px 24px rgba(0,0,0,0.4); animation: fadeIn 0.1s ease;
  }
  @keyframes fadeIn { from { opacity: 0; transform: scale(0.96); } to { opacity: 1; } }
  .rt-ctx-item {
    width: 100%; padding: 6px 14px; border: none; background: transparent;
    color: var(--t2); font-size: 12px; font-family: var(--ui); cursor: pointer;
    text-align: left; transition: background 0.08s, color 0.08s;
  }
  .rt-ctx-item:hover { background: var(--c); color: var(--t1); }
  .rt-ctx-danger:hover { background: rgba(240,68,68,0.1); color: var(--err); }
  .rt-ctx-sep { height: 1px; background: var(--b1); margin: 4px 0; }

  /* Save confirmation modal */
  .rt-save-overlay {
    position: fixed; top: 0; left: 0; right: 0; bottom: 0;
    background: rgba(0,0,0,0.45); backdrop-filter: blur(6px);
    z-index: 1000; display: flex; align-items: center; justify-content: center;
    animation: fadeIn 0.12s ease;
  }
  .rt-save-modal {
    background: var(--modal-bg, var(--n)); border: 1px solid var(--b1);
    border-radius: 12px; box-shadow: 0 24px 48px rgba(0,0,0,0.5);
    width: 560px; max-height: 80vh; display: flex; flex-direction: column;
    overflow: hidden; animation: slideIn 0.15s ease;
  }
  @keyframes slideIn { from { opacity: 0; transform: translateY(-8px) scale(0.98); } to { opacity: 1; } }
  .rt-save-header {
    display: flex; align-items: center; gap: 10px;
    padding: 14px 18px; border-bottom: 1px solid var(--b1); background: var(--n2);
  }
  .rt-save-title { font-size: 14px; font-weight: 600; color: var(--t1); font-family: var(--ui); }
  .rt-save-db {
    font-size: 10px; font-family: var(--mono); color: var(--acc);
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    padding: 2px 8px; border-radius: 4px; font-weight: 600;
  }
  .rt-save-close {
    margin-left: auto; width: 28px; height: 28px; border-radius: 6px;
    border: 1px solid var(--b1); background: transparent; color: var(--t3);
    font-size: 16px; cursor: default; display: flex; align-items: center;
    justify-content: center; transition: background 0.1s;
  }
  .rt-save-close:hover { background: var(--c); color: var(--t1); }
  .rt-save-body { padding: 16px 18px; overflow-y: auto; flex: 1; }
  .rt-save-count { font-size: 12px; color: var(--t2); font-family: var(--ui); margin-bottom: 10px; }
  .rt-save-sql {
    background: rgba(255,255,255,0.02); border: 1px solid var(--b1);
    border-radius: 6px; overflow: hidden;
  }
  .rt-save-stmt {
    display: flex; padding: 6px 0; border-bottom: 1px solid color-mix(in srgb, var(--b1) 50%, transparent);
  }
  .rt-save-stmt:last-child { border-bottom: none; }
  .rt-save-ln {
    width: 32px; text-align: right; padding-right: 10px;
    font-size: 10px; color: var(--t4); font-family: var(--mono);
    flex-shrink: 0; opacity: 0.5; line-height: 19px;
  }
  .rt-save-code {
    font-size: 12px; font-family: var(--mono); color: var(--t1);
    line-height: 19px; word-break: break-word; padding-right: 12px;
  }
  .rt-save-footer {
    display: flex; justify-content: flex-end; gap: 8px;
    padding: 14px 18px; border-top: 1px solid var(--b1);
  }
  .rt-save-cancel {
    height: 30px; padding: 0 14px; border-radius: 6px;
    border: 1px solid var(--b1); background: transparent;
    color: var(--t2); font-size: 12px; font-family: var(--ui); cursor: default;
    transition: border-color 0.1s, color 0.1s;
  }
  .rt-save-cancel:hover { border-color: var(--b2); color: var(--t1); }
  .rt-save-execute {
    height: 30px; padding: 0 18px; border-radius: 6px;
    border: none; background: var(--acc); color: #fff;
    font-size: 12px; font-weight: 600; font-family: var(--ui); cursor: default;
    transition: opacity 0.1s;
  }
  .rt-save-execute:hover:not(:disabled) { opacity: 0.85; }
  .rt-save-execute:disabled { opacity: 0.4; cursor: not-allowed; }
</style>

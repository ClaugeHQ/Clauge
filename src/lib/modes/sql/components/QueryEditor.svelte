<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { EditorView, keymap } from '@codemirror/view';
  import { sql, PostgreSQL, MySQL, SQLite, keywordCompletionSource, schemaCompletionSource } from '@codemirror/lang-sql';
  import { autocompletion, type Completion, type CompletionContext, type CompletionResult, type CompletionSource } from '@codemirror/autocomplete';
  import { format as formatSql } from 'sql-formatter';
  import { activeConnection } from '../stores';
  import { mode } from '$lib/stores/app';
  import type { TableInfo } from '../types';
  import { parserProfileFor } from '../dialects';
  import { splitSqlStatements, splitSqlStatementsWithPositions } from '../utils/splitter';
  import { showToast } from '$lib/shared/primitives/toast';
  import {
    attachSqlEditor,
    detachSqlEditor,
    getSqlEditorEntry,
  } from '../services/sqlEditorReparent';

  interface Props {
    /** Numeric topbar tab id. The reparent registry keys EditorViews
     *  by this id so the same view survives canvas reparent and
     *  cross-mount (panel ↔ tile) swaps. */
    tabId: number;
    query: string;
    tables?: TableInfo[];
    columnMap?: Record<string, string[]>;
    /** True while column metadata is being fetched in the background.
     *  Surfaces a small "Loading schema…" hint so users know that empty
     *  autocomplete suggestions are temporary, not broken. */
    schemaLoading?: boolean;
    /** Resolved default schema for unqualified table completion. Comes
     *  from `current_schema()` on Postgres connections; undefined for
     *  engines that have no schema concept. */
    defaultSchema?: string;
    /** True while a query is in flight OR while the pool is connecting.
     *  Cmd/Ctrl+Enter no-ops; the Run button is already disabled in the
     *  parent. Prevents queue-piling on the same tab. */
    disabled?: boolean;
    onexecute?: (query: string) => void;
    onexecutemulti?: (queries: string[]) => void;
  }

  let { tabId, query, tables = [], columnMap = {}, schemaLoading = false, defaultSchema, disabled = false, onexecute, onexecutemulti }: Props = $props();

  let editorHost: HTMLDivElement | undefined = $state();
  let suppressExternalSync = false;

  const CM_DIALECTS: Record<string, typeof PostgreSQL> = {
    PostgreSQL,
    MySQL,
    SQLite,
  };

  const SQL_FORMATTER_DIALECTS: Record<string, 'postgresql' | 'mysql' | 'sqlite' | 'sql'> = {
    PostgreSQL: 'postgresql',
    MySQL: 'mysql',
    SQLite: 'sqlite',
  };

  const dialect = $derived(
    CM_DIALECTS[parserProfileFor($activeConnection?.driver ?? '')] ?? PostgreSQL
  );

  // CodeMirror's SQLNamespace is a *nested* shape — keys are single
  // name segments and CodeMirror does NOT split them on `.`. Writing
  // `{ "public.users": [...] }` would register a literal table named
  // "public.users", not schema "public" containing table "users".
  //
  // For Postgres (rows carry `schema`), produce a nested map:
  //   { public: { users: ['id', 'name'] }, analytics: { events: [...] } }
  // and rely on `defaultSchema: 'public'` (passed in the sql() config)
  // so unqualified `users` completes from the default schema for free.
  //
  // For engines without schemas (MySQL/SQLite/ClickHouse/D1) the output
  // is flat: `{ tableName: columns }` — the previous behaviour.
  function buildSchema(): Record<string, any> {
    if (!tables.length) return {};

    const hasSchemas = tables.some((t) => !!t.schema);
    if (!hasSchemas) {
      const out: Record<string, string[]> = {};
      for (const t of tables) {
        if (!t.name) continue;
        out[t.name] = columnMap[t.name] ?? [];
      }
      return out;
    }

    const out: Record<string, Record<string, string[]> | string[]> = {};
    for (const t of tables) {
      if (!t.name) continue;
      const schemaName = t.schema || '_unschemed';
      const colKey = t.schema ? `${t.schema}.${t.name}` : t.name;
      const cols = columnMap[colKey] ?? columnMap[t.name] ?? [];

      let bucket = out[schemaName];
      if (!bucket || Array.isArray(bucket)) {
        bucket = {};
        out[schemaName] = bucket;
      }
      (bucket as Record<string, string[]>)[t.name] = cols;
    }

    return out;
  }

  const defaultSchemaForDialect = $derived(
    defaultSchema ??
    (parserProfileFor($activeConnection?.driver ?? '') === 'PostgreSQL' ? 'public' : undefined)
  );

  interface FromScope {
    refs: string[];
    aliases: Map<string, string>;
  }

  function analyzeFromScope(buffer: string, cursorPos: number): FromScope {
    const stmts = splitSqlStatementsWithPositions(buffer);
    let target = stmts.find((s) => cursorPos >= s.from && cursorPos <= s.to + 1);
    if (!target) {
      for (let i = stmts.length - 1; i >= 0; i--) {
        if (stmts[i].to < cursorPos) { target = stmts[i]; break; }
      }
    }
    if (!target) return { refs: [], aliases: new Map() };

    const text = target.text;
    const refs = new Set<string>();
    const aliases = new Map<string, string>();
    const KW_RE = /^(?:FROM|JOIN|UPDATE|INTO)\b/i;
    const NOT_AN_ALIAS = /^(?:WHERE|GROUP|ORDER|HAVING|LIMIT|JOIN|LEFT|RIGHT|INNER|OUTER|CROSS|FULL|ON|USING|UNION|INTERSECT|EXCEPT|FOR|FETCH|LATERAL|NATURAL|SET|VALUES|RETURNING|WINDOW|OFFSET)$/i;

    let i = 0;
    const len = text.length;
    while (i < len) {
      const ch = text[i];
      const next = i + 1 < len ? text[i + 1] : '';

      if (ch === '-' && next === '-') {
        const eol = text.indexOf('\n', i);
        i = eol === -1 ? len : eol + 1;
        continue;
      }
      if (ch === '/' && next === '*') {
        const end = text.indexOf('*/', i + 2);
        i = end === -1 ? len : end + 2;
        continue;
      }
      if (ch === "'") {
        i++;
        while (i < len) {
          if (text[i] === "'" && i + 1 < len && text[i + 1] === "'") { i += 2; }
          else if (text[i] === "'") { i++; break; }
          else { i++; }
        }
        continue;
      }
      if (ch === '$') {
        const tagEnd = text.indexOf('$', i + 1);
        if (tagEnd !== -1) {
          const tag = text.slice(i, tagEnd + 1);
          const tagContent = tag.slice(1, -1);
          if (tagContent === '' || /^[a-zA-Z_][a-zA-Z0-9_]*$/.test(tagContent)) {
            const closeIdx = text.indexOf(tag, tagEnd + 1);
            if (closeIdx !== -1) { i = closeIdx + tag.length; continue; }
          }
        }
        i++;
        continue;
      }

      const isWordChar = /[A-Za-z_]/.test(ch);
      const prevCh = i > 0 ? text[i - 1] : ' ';
      const atWordStart = !/[A-Za-z_0-9]/.test(prevCh) && isWordChar;
      if (atWordStart) {
        const kw = text.slice(i).match(KW_RE);
        if (kw) {
          let j = i + kw[0].length;
          while (j < len && /\s/.test(text[j])) j++;
          const tableRef = readIdentifier(text, j);
          if (tableRef) {
            refs.add(tableRef.name);
            j = tableRef.endPos;
            while (j < len && /\s/.test(text[j])) j++;
            const asMatch = text.slice(j).match(/^AS\b/i);
            if (asMatch) {
              j += asMatch[0].length;
              while (j < len && /\s/.test(text[j])) j++;
            }
            const aliasMatch = text.slice(j).match(/^[A-Za-z_]\w*/);
            if (aliasMatch && !NOT_AN_ALIAS.test(aliasMatch[0])) {
              aliases.set(aliasMatch[0], tableRef.name);
              j += aliasMatch[0].length;
            }
            i = j;
            continue;
          }
        }
      }
      i++;
    }

    return { refs: Array.from(refs), aliases };
  }

  function readIdentifier(text: string, start: number): { name: string; endPos: number } | null {
    function readPart(pos: number): { name: string; endPos: number } | null {
      if (pos >= text.length) return null;
      const c = text[pos];
      if (c === '"' || c === '`') {
        const close = text.indexOf(c, pos + 1);
        if (close === -1) return null;
        return { name: text.slice(pos + 1, close), endPos: close + 1 };
      }
      const m = text.slice(pos).match(/^[A-Za-z_]\w*/);
      if (!m) return null;
      return { name: m[0], endPos: pos + m[0].length };
    }
    const first = readPart(start);
    if (!first) return null;
    if (text[first.endPos] !== '.') return first;
    const second = readPart(first.endPos + 1);
    if (!second) return first;
    return { name: `${first.name}.${second.name}`, endPos: second.endPos };
  }

  function buildColumnSource(currentColumnMap: Record<string, string[]>): CompletionSource {
    return (context: CompletionContext): CompletionResult | null => {
      const word = context.matchBefore(/[A-Za-z_][A-Za-z0-9_]*/);
      if (!word || (word.from === word.to && !context.explicit)) return null;
      const charBefore = word.from > 0
        ? context.state.doc.sliceString(word.from - 1, word.from)
        : '';

      const buffer = context.state.doc.toString();
      const scope = analyzeFromScope(buffer, context.pos);

      if (charBefore === '.') {
        const prefixMatch = context.state.doc
          .sliceString(0, word.from - 1)
          .match(/([A-Za-z_]\w*)$/);
        if (!prefixMatch) return null;
        const aliased = scope.aliases.get(prefixMatch[1]);
        if (!aliased) return null;
        const bare = aliased.split('.').pop() ?? aliased;
        const cols = currentColumnMap[aliased] ?? currentColumnMap[bare];
        if (!cols) return null;
        return {
          from: word.from,
          options: cols.map((c) => ({
            label: c,
            type: 'property',
            detail: aliased,
            boost: 10,
          })),
          validFor: /^[A-Za-z_][A-Za-z0-9_]*$/,
        };
      }

      if (scope.refs.length === 0) return null;
      const options: Completion[] = [];
      const seen = new Set<string>();
      for (const ref of scope.refs) {
        const bare = ref.split('.').pop() ?? ref;
        const cols = currentColumnMap[ref] ?? currentColumnMap[bare];
        if (!cols) continue;
        for (const col of cols) {
          const key = `${col}|${ref}`;
          if (seen.has(key)) continue;
          seen.add(key);
          options.push({
            label: col,
            type: 'property',
            detail: ref,
            boost: 5,
          });
        }
      }
      if (options.length === 0) return null;
      return {
        from: word.from,
        options,
        validFor: /^[A-Za-z_][A-Za-z0-9_]*$/,
      };
    };
  }

  // Reconfigure SQL extension when tables, columns, or dialect changes.
  //
  // The view + sqlCompartment live in the singleton registry; we look
  // them up by tabId here and dispatch through the singleton.
  //
  // CRITICAL: read every reactive dep BEFORE the entry guard.
  // Svelte 5 tracks only reads that happen during the synchronous effect
  // run. If we read the deps inside `if (entry)` they would not be
  // tracked when the entry hasn't been created yet (first mount race),
  // and the effect would never re-fire when tables/columnMap arrive.
  $effect(() => {
    const tablesDep = tables;
    const columnsDep = columnMap;
    const dialectDep = dialect;
    const defaultSchemaDep = defaultSchemaForDialect;
    void tablesDep;
    const idDep = tabId;
    const entry = getSqlEditorEntry(idDep);
    if (!entry) return;
    const schema = buildSchema();
    const sqlConfig = { dialect: dialectDep, schema, defaultSchema: defaultSchemaDep, upperCaseKeywords: true };
    entry.view.dispatch({
      effects: entry.sqlCompartment.reconfigure([
        sql(sqlConfig),
        autocompletion({
          activateOnTyping: true,
          maxRenderedOptions: 25,
          override: [
            buildColumnSource(columnsDep),
            schemaCompletionSource(sqlConfig),
            keywordCompletionSource(dialectDep, true),
          ],
        }),
      ]),
    });
  });

  $effect(() => {
    const onExecDep = onexecute;
    const onExecMultiDep = onexecutemulti;
    const disabledDep = disabled;
    void onExecDep;
    void onExecMultiDep;
    void disabledDep;
    const entry = getSqlEditorEntry(tabId);
    if (!entry) return;
    entry.view.dispatch({
      effects: entry.execKeymapCompartment.reconfigure(
        keymap.of([
          {
            key: 'Mod-Enter',
            run: (v) => {
              if (disabledDep) {
                showToast('Query already running — cancel to start a new one', 'info');
                return true;
              }
              executeFromCursor(v);
              return true;
            },
            preventDefault: true,
          },
          { key: 'Mod-Shift-f', run: formatBuffer, preventDefault: true },
        ]),
      ),
    });
  });

  onMount(() => {
    if (editorHost) {
      attachSqlEditor(tabId, editorHost);
    }
  });

  // Both SqlPanel and Atlas's CanvasTileBody stay mounted across mode
  // switches (panels toggle via visibility, not unmount). The singleton's
  // CodeMirror DOM can only live in one slot at a time, so when Atlas
  // claims it the home host goes empty — and vice versa. Re-claim on
  // every $mode change so whichever side is visible wins.
  $effect(() => {
    if ($mode === 'sql' && editorHost) {
      attachSqlEditor(tabId, editorHost);
    }
  });

  onDestroy(() => {
    if (editorHost) {
      detachSqlEditor(tabId, editorHost);
    }
  });

  // Sync external query changes into the singleton editor (e.g. insert
  // from nav, AI-driven binding swap). The registry's updateListener
  // writes user typing back into sqlTabState; the suppress flag breaks
  // that loop so dispatch-from-store doesn't bounce back as a "doc
  // change" that re-writes the same value.
  $effect(() => {
    const q = query;
    const entry = getSqlEditorEntry(tabId);
    if (!entry) return;
    if (suppressExternalSync) return;
    const current = entry.view.state.doc.toString();
    if (q === current) return;
    suppressExternalSync = true;
    entry.view.dispatch({
      changes: { from: 0, to: entry.view.state.doc.length, insert: q ?? '' },
    });
    queueMicrotask(() => { suppressExternalSync = false; });
  });

  function isExecutableStatement(text: string): boolean {
    return text.trim().length > 0;
  }

  function formatBuffer(view: EditorView): boolean {
    const profile = parserProfileFor($activeConnection?.driver ?? '');
    const language = SQL_FORMATTER_DIALECTS[profile] ?? 'sql';
    const sel = view.state.selection.main;
    const source = sel.empty
      ? view.state.doc.toString()
      : view.state.sliceDoc(sel.from, sel.to);
    if (!source.trim()) return true;
    try {
      const formatted = formatSql(source, { language, keywordCase: 'upper' });
      if (formatted === source) return true;
      if (sel.empty) {
        view.dispatch({
          changes: { from: 0, to: view.state.doc.length, insert: formatted },
        });
      } else {
        view.dispatch({
          changes: { from: sel.from, to: sel.to, insert: formatted },
        });
      }
    } catch (e: any) {
      showToast(`Format failed: ${e?.message ?? String(e)}`, 'error');
    }
    return true;
  }

  function executeFromCursor(view: EditorView) {
    const sel = view.state.selection.main;

    if (!sel.empty) {
      const selected = view.state.sliceDoc(sel.from, sel.to).trim();
      if (!selected) return;

      const stmts = splitSqlStatements(selected);
      if (stmts.length > 1 && onexecutemulti) {
        onexecutemulti(stmts);
        return;
      }

      if (stmts.length === 1 && isExecutableStatement(stmts[0])) {
        onexecute?.(stmts[0]);
      }
      return;
    }

    const fullText = view.state.doc.toString();
    const cursorPos = sel.head;
    const statements = splitSqlStatementsWithPositions(fullText);
    if (statements.length === 0) return;

    let stmt = statements.find(s => cursorPos >= s.from && cursorPos <= s.to + 1);

    if (!stmt) {
      for (let i = statements.length - 1; i >= 0; i--) {
        if (statements[i].to < cursorPos) { stmt = statements[i]; break; }
      }
    }

    if (stmt && isExecutableStatement(stmt.text)) {
      onexecute?.(stmt.text);
    }
  }

  /** Called by the Execute button — requires selection */
  export function handleExecute() {
    const entry = getSqlEditorEntry(tabId);
    if (!entry) return;
    if (disabled) {
      showToast('Query already running — cancel to start a new one', 'info');
      return;
    }
    const sel = entry.view.state.selection.main;

    if (!sel.empty) {
      executeFromCursor(entry.view);
      return;
    }

    showToast('Select the query you want to execute', 'info');
  }
</script>

<div class="query-editor">
  <div class="qe-editor" bind:this={editorHost}></div>
  {#if schemaLoading}
    <div class="qe-schema-loading" title="Fetching column metadata for autocomplete. Suggestions improve as this completes.">
      <span class="qe-spinner"></span>
      <span>Loading schema for autocomplete…</span>
    </div>
  {/if}
</div>

<style>
  .query-editor {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
    position: relative;
  }
  .qe-editor {
    flex: 1;
    overflow: hidden;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .qe-editor :global(.sql-editor-reparent-container .cm-editor) {
    height: 100%;
  }
  .qe-schema-loading {
    position: absolute;
    bottom: 6px;
    right: 10px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--surface, #1a1a1a) 85%, transparent);
    border: 1px solid var(--b1, #2a2a2a);
    color: var(--t3, #8a8a8a);
    font-family: var(--ui);
    font-size: 10.5px;
    pointer-events: none;
    user-select: none;
  }
  .qe-spinner {
    width: 9px;
    height: 9px;
    border: 1.5px solid var(--b1, #2a2a2a);
    border-top-color: var(--acc, #7c5cf8);
    border-radius: 50%;
    animation: qe-spin 0.8s linear infinite;
  }
  @keyframes qe-spin {
    to { transform: rotate(360deg); }
  }
</style>

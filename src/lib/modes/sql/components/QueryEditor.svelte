<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { EditorView, keymap, placeholder as cmPlaceholder, lineNumbers } from '@codemirror/view';
  import { EditorState, Compartment } from '@codemirror/state';
  import { sql, PostgreSQL, MySQL, SQLite } from '@codemirror/lang-sql';
  import { autocompletion } from '@codemirror/autocomplete';
  import { oneDarkHighlightStyle } from '@codemirror/theme-one-dark';
  import { syntaxHighlighting } from '@codemirror/language';
  import { defaultKeymap, historyKeymap, indentWithTab } from '@codemirror/commands';
  import { history } from '@codemirror/commands';
  import { activeConnection } from '../stores';
  import type { TableInfo } from '../types';
  import { parserProfileFor } from '../dialects';
  import { splitSqlStatements } from '../utils/splitter';
  import { showToast } from '$lib/shared/primitives/toast';
  import { mod } from '$lib/utils/platform';

  interface Props {
    query: string;
    tables?: TableInfo[];
    columnMap?: Record<string, string[]>;
    onquerychange?: (query: string) => void;
    onexecute?: (query: string) => void;
    onexecutemulti?: (queries: string[]) => void;
  }

  let { query, tables = [], columnMap = {}, onquerychange, onexecute, onexecutemulti }: Props = $props();

  let editorContainer: HTMLDivElement | undefined = $state();
  let editorView: EditorView | undefined;
  let sqlCompartment = new Compartment();
  let suppressExternalSync = false;

  // CodeMirror dialect objects are imported per-driver; the registry only
  // carries the profile name, so we map name -> object here. Unknown drivers
  // fall through to PostgreSQL (matches the legacy default).
  const CM_DIALECTS: Record<string, typeof PostgreSQL> = {
    PostgreSQL,
    MySQL,
    SQLite,
  };

  const dialect = $derived(
    CM_DIALECTS[parserProfileFor($activeConnection?.driver ?? '')] ?? PostgreSQL
  );

  function buildSchema() {
    if (!tables.length) return {};
    return tables.reduce((acc, t) => {
      acc[t.name] = columnMap[t.name] ?? [];
      return acc;
    }, {} as Record<string, string[]>);
  }

  // Reconfigure SQL extension when tables, columns, or dialect changes
  $effect(() => {
    if (editorView) {
      const _cm = columnMap; // track dependency
      const schema = buildSchema();
      editorView.dispatch({
        effects: sqlCompartment.reconfigure(
          sql({ dialect, schema, upperCaseKeywords: true })
        ),
      });
    }
  });

  const editorTheme = EditorView.theme({
    '&': { backgroundColor: 'transparent', fontSize: '12.5px', height: '100%' },
    '.cm-content': { fontFamily: 'var(--mono)', caretColor: 'var(--acc)', padding: '12px 4px' },
    '.cm-cursor': { borderLeftColor: 'var(--acc)' },
    '.cm-gutters': {
      backgroundColor: 'transparent', borderRight: '1px solid var(--b1)',
      color: 'var(--t4)', fontSize: '11px', fontFamily: 'var(--mono)', minWidth: '36px',
    },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: 'var(--t2)' },
    '.cm-activeLine': { backgroundColor: 'rgba(255,255,255,0.03)' },
    '.cm-selectionBackground': { backgroundColor: 'rgba(124,92,248,0.2) !important' },
    '&.cm-focused .cm-selectionBackground': { backgroundColor: 'rgba(124,92,248,0.3) !important' },
    '.cm-tooltip-autocomplete': {
      backgroundColor: 'var(--n2)', border: '1px solid var(--b1)',
      borderRadius: '6px', overflow: 'hidden',
    },
    '.cm-tooltip-autocomplete ul li': { fontFamily: 'var(--mono)', fontSize: '12px', padding: '4px 8px' },
    '.cm-tooltip-autocomplete ul li[aria-selected]': { backgroundColor: 'var(--acc)', color: '#fff' },
    '.cm-scroller': { overflow: 'auto' },
    '.cm-scroller::-webkit-scrollbar': { width: '4px' },
    '.cm-scroller::-webkit-scrollbar-thumb': { background: 'var(--b1)', borderRadius: '2px' },
    '.cm-placeholder': { color: 'var(--t4)' },
  });

  function createEditor(container: HTMLDivElement, initialDoc: string) {
    const state = EditorState.create({
      doc: initialDoc,
      extensions: [
        lineNumbers(),
        history(),
        keymap.of([
          { key: 'Mod-Enter', run: (view) => { executeFromCursor(view); return true; }, preventDefault: true },
          ...defaultKeymap,
          ...historyKeymap,
          indentWithTab,
        ]),
        sqlCompartment.of(sql({ dialect: PostgreSQL, schema: buildSchema(), upperCaseKeywords: true })),
        autocompletion({ activateOnTyping: true }),
        syntaxHighlighting(oneDarkHighlightStyle),
        editorTheme,
        cmPlaceholder(`Write your SQL query here -- ${mod()}+Enter to execute`),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            suppressExternalSync = true;
            const value = update.state.doc.toString();
            onquerychange?.(value);
            queueMicrotask(() => { suppressExternalSync = false; });
          }
        }),
      ],
    });

    return new EditorView({ state, parent: container });
  }

  onMount(() => {
    if (editorContainer) {
      editorView = createEditor(editorContainer, query ?? '');
    }
  });

  onDestroy(() => {
    editorView?.destroy();
  });

  // Sync external query changes into editor (e.g. tab switch, insert from nav)
  $effect(() => {
    const q = query;
    if (editorView && !suppressExternalSync && q !== editorView.state.doc.toString()) {
      editorView.dispatch({
        changes: { from: 0, to: editorView.state.doc.length, insert: q ?? '' },
      });
    }
  });

  // Executable-iff-non-empty: trust the database engine to validate syntax
  // and surface its own error message. node-sql-parser's grammar coverage
  // is incomplete (it falls back to PostgreSQL for ClickHouse, doesn't
  // recognize `SETTINGS` / `PREWHERE` / `FINAL` / `ARRAY JOIN` / extension
  // SQL on any dialect, and silently drops single-keyword statements like
  // `BEGIN` / `COMMIT` / `VACUUM`). Using it as a gate produced a class of
  // bug where pressing Run did nothing, with no toast or feedback. The
  // server is the source of truth; let it answer.
  function isExecutableStatement(text: string): boolean {
    return text.trim().length > 0;
  }

  function executeFromCursor(view: EditorView) {
    const sel = view.state.selection.main;

    // If there's a selection, execute the selected text
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

    // No selection — find the single statement at cursor position
    const fullText = view.state.doc.toString();
    const cursorPos = sel.head;
    let start = 0;
    const statements: { from: number; to: number; text: string }[] = [];
    const parts = fullText.split(';');
    for (const part of parts) {
      const end = start + part.length;
      const trimmed = part.trim();
      if (trimmed) {
        statements.push({ from: start, to: end, text: trimmed });
      }
      start = end + 1;
    }

    const stmt = statements.find(s => cursorPos >= s.from && cursorPos <= s.to + 1);
    if (stmt && isExecutableStatement(stmt.text)) {
      onexecute?.(stmt.text);
    }
  }

  /** Called by the Execute button — requires selection */
  export function handleExecute() {
    if (!editorView) return;
    const sel = editorView.state.selection.main;

    if (!sel.empty) {
      executeFromCursor(editorView);
      return;
    }

    showToast('Select the query you want to execute', 'info');
  }
</script>

<div class="query-editor">
  <div class="qe-editor" bind:this={editorContainer}></div>
</div>

<style>
  .query-editor {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }
  .qe-editor {
    flex: 1;
    overflow: hidden;
    min-height: 0;
  }
  .qe-editor :global(.cm-editor) {
    height: 100%;
  }
</style>

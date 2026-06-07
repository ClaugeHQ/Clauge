import { EditorView, keymap, placeholder as cmPlaceholder, lineNumbers } from '@codemirror/view';
import { EditorState, Compartment, Prec } from '@codemirror/state';
import { sql, PostgreSQL } from '@codemirror/lang-sql';
import { autocompletion } from '@codemirror/autocomplete';
import { oneDarkHighlightStyle } from '@codemirror/theme-one-dark';
import { syntaxHighlighting } from '@codemirror/language';
import { defaultKeymap, historyKeymap, indentWithTab, history } from '@codemirror/commands';
import { search, searchKeymap } from '@codemirror/search';
import { mount, unmount } from 'svelte';
import { mod } from '$lib/utils/platform';
import { getSqlTabData, setSqlTabData } from '../stores';
import { splitSqlStatements, splitSqlStatementsWithPositions } from '../utils/splitter';
import { showToast } from '$lib/shared/primitives/toast';
import SqlTileHeader from '../components/SqlTileHeader.svelte';

/**
 * Per-tab execute hooks published by QueryEditor (via setSqlTabExecutor)
 * and consumed by the Mod-Enter binding inside the singleton EditorView.
 *
 * Why this lives here: when Mod-Enter was registered from QueryEditor's
 * `$effect`, there was a window between editor creation and the effect
 * firing where Cmd+Enter fell through to the browser's contenteditable
 * default (insert newline / replace selection). Binding it inside the
 * registry with Prec.highest() guarantees the key is captured from the
 * very first dispatch.
 */
export interface SqlTabExecutor {
  onexecute?: (query: string) => void;
  onexecutemulti?: (queries: string[]) => void;
  disabled?: boolean;
}

const tabExecutors = new Map<number, SqlTabExecutor>();

export function setSqlTabExecutor(tabId: number, exec: SqlTabExecutor): void {
  tabExecutors.set(tabId, exec);
}

export function clearSqlTabExecutor(tabId: number): void {
  tabExecutors.delete(tabId);
}

/**
 * Run the same cursor-aware execution logic QueryEditor.executeFromCursor
 * uses, looking up the per-tab onexecute / onexecutemulti closures via
 * the executor map. Returns true to consume the key event.
 */
function executeAtCursorForTab(tabId: number, view: EditorView): boolean {
  const exec = tabExecutors.get(tabId);
  if (!exec) return false;
  if (exec.disabled) {
    showToast('Query already running — cancel to start a new one', 'info');
    return true;
  }
  const sel = view.state.selection.main;

  if (!sel.empty) {
    const selected = view.state.sliceDoc(sel.from, sel.to).trim();
    if (!selected) return true;
    const stmts = splitSqlStatements(selected);
    if (stmts.length > 1 && exec.onexecutemulti) {
      exec.onexecutemulti(stmts);
      return true;
    }
    if (stmts.length === 1 && stmts[0].trim().length > 0) {
      exec.onexecute?.(stmts[0]);
    }
    return true;
  }

  const fullText = view.state.doc.toString();
  const cursorPos = sel.head;
  const statements = splitSqlStatementsWithPositions(fullText);
  if (statements.length === 0) return true;

  let stmt = statements.find((s) => cursorPos >= s.from && cursorPos <= s.to + 1);
  if (!stmt) {
    for (let i = statements.length - 1; i >= 0; i--) {
      if (statements[i].to < cursorPos) {
        stmt = statements[i];
        break;
      }
    }
  }
  if (stmt && stmt.text.trim().length > 0) {
    exec.onexecute?.(stmt.text);
  }
  return true;
}

/**
 * Singleton registry of CodeMirror EditorViews keyed by SQL tab id.
 *
 * The view lives outside any Svelte component lifecycle so its DOM can
 * be reparented between SqlPanel and Canvas tile slots without losing
 * cursor position, selection, scroll, or undo history. QueryEditor.svelte
 * is reduced to a thin host: it calls attach on mount, detach on
 * destroy, and reconfigures the per-tab compartments (dialect/schema,
 * execute keymap) on the singleton view via its existing $effects.
 *
 * The updateListener installed at creation time writes every doc change
 * back to sqlTabState so SqlPanel's autosave $effect and any other
 * reader (AI prompts, canvas tile co-existence) see the latest text.
 */

export interface SqlEditorEntry {
  view: EditorView;
  /** Outer wrapper that gets reparented between hosts. Contains
   *  `headerSlot` and `editorSlot`. */
  container: HTMLDivElement;
  /** Thin strip above the editor. Hidden by default; the canvas tile
   *  toggles it on via `setHeaderVisible(tabId, true)` so the
   *  in-tile connection picker only renders in tiles, not in the
   *  home-mode SqlPanel (which has its own action-bar picker). */
  headerSlot: HTMLDivElement;
  /** Hosts CodeMirror's view.dom. */
  editorSlot: HTMLDivElement;
  /** Mounted SqlTileHeader Svelte component. Created once per entry
   *  and stays alive across reparents, so its dropdown state and
   *  derived subscriptions persist. */
  header: ReturnType<typeof mount> | null;
  /** Owns the sql() + autocompletion() config. Reconfigured by
   *  QueryEditor when tables / columnMap / dialect change. */
  sqlCompartment: Compartment;
  /** Owns the Mod-Enter / Mod-Shift-f keymap. Reconfigured by
   *  QueryEditor when its onexecute / onexecutemulti / disabled props
   *  change, so the binding always closes over the live callbacks. */
  execKeymapCompartment: Compartment;
}

const editors = new Map<number, SqlEditorEntry>();

const editorTheme = EditorView.theme({
  '&': { backgroundColor: 'transparent', fontSize: '12.5px', height: '100%' },
  '.cm-content': { fontFamily: 'var(--mono)', caretColor: 'var(--acc)', padding: '12px 4px' },
  '.cm-cursor': { borderLeftColor: 'var(--acc)' },
  '.cm-gutters': {
    backgroundColor: 'transparent', borderRight: '1px solid var(--b1)',
    color: 'var(--t4)', fontSize: '11px', fontFamily: 'var(--mono)', minWidth: '36px',
  },
  '.cm-activeLineGutter': { backgroundColor: 'transparent', color: 'var(--t2)' },
  '.cm-activeLine': { backgroundColor: 'var(--surface-hover)' },
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
  '.cm-panels': {
    backgroundColor: 'var(--n2)',
    color: 'var(--t1)',
    borderTop: '1px solid var(--b1)',
    borderBottom: '1px solid var(--b1)',
    fontFamily: 'var(--ui)',
  },
  '.cm-panels.cm-panels-top': { borderBottom: '1px solid var(--b1)', borderTop: 'none' },
  '.cm-panels.cm-panels-bottom': { borderTop: '1px solid var(--b1)', borderBottom: 'none' },
  '.cm-search': {
    padding: '6px 8px',
    display: 'flex',
    alignItems: 'center',
    gap: '6px',
    flexWrap: 'wrap',
    fontSize: '12px',
  },
  '.cm-search label': {
    display: 'inline-flex',
    alignItems: 'center',
    gap: '4px',
    color: 'var(--t3)',
    fontSize: '11.5px',
    whiteSpace: 'nowrap',
  },
  '.cm-search input[type="checkbox"]': {
    accentColor: 'var(--acc)',
    margin: '0',
  },
  '.cm-textfield': {
    backgroundColor: 'var(--surface, #111)',
    color: 'var(--t1)',
    border: '1px solid var(--b1)',
    borderRadius: '4px',
    padding: '4px 8px',
    fontSize: '12px',
    fontFamily: 'var(--mono)',
    minWidth: '180px',
    outline: 'none',
  },
  '.cm-textfield:focus': {
    borderColor: 'var(--acc)',
    boxShadow: '0 0 0 1px var(--acc)',
  },
  '.cm-button': {
    backgroundColor: 'var(--surface, #111)',
    color: 'var(--t2)',
    border: '1px solid var(--b1)',
    borderRadius: '4px',
    padding: '3px 10px',
    fontSize: '11.5px',
    fontFamily: 'var(--ui)',
    cursor: 'pointer',
    backgroundImage: 'none',
    textShadow: 'none',
  },
  '.cm-button:hover': {
    backgroundColor: 'var(--surface-hover, #1a1a1a)',
    color: 'var(--t1)',
    borderColor: 'var(--b2, #333)',
  },
  '.cm-button:active': {
    backgroundColor: 'var(--n3, #0c0c0c)',
  },
  '.cm-button[name="close"], button[name="close"].cm-panel-close': {
    backgroundColor: 'transparent',
    border: 'none',
    color: 'var(--t3)',
    fontSize: '14px',
    padding: '2px 6px',
    marginLeft: 'auto',
  },
  '.cm-button[name="close"]:hover': { color: 'var(--t1)' },
  '.cm-searchMatch': {
    backgroundColor: 'rgba(245,166,35,0.25)',
    outline: '1px solid rgba(245,166,35,0.5)',
  },
  '.cm-searchMatch-selected': {
    backgroundColor: 'rgba(245,166,35,0.5)',
    outline: '1px solid rgba(245,166,35,0.8)',
  },
});

function createEntry(tabId: number, initialDoc: string): SqlEditorEntry {
  const sqlCompartment = new Compartment();
  const execKeymapCompartment = new Compartment();

  const extensions = [
    lineNumbers(),
    history(),
    // Mod-Enter is wired at creation time with Prec.highest so the
    // execute binding always wins over default Enter handlers,
    // autocomplete keymaps, and the empty initial state of the per-
    // QueryEditor compartment. Closes over `tabId` and looks the live
    // executor up at run-time, so prop changes in QueryEditor flow
    // through without needing a re-dispatch.
    Prec.highest(
      keymap.of([
        {
          key: 'Mod-Enter',
          preventDefault: true,
          run: (view) => executeAtCursorForTab(tabId, view),
        },
      ]),
    ),
    keymap.of([
      ...defaultKeymap,
      ...historyKeymap,
      ...searchKeymap,
      indentWithTab,
    ]),
    execKeymapCompartment.of([]),
    sqlCompartment.of([
      sql({ dialect: PostgreSQL, upperCaseKeywords: true }),
      autocompletion({ activateOnTyping: true, maxRenderedOptions: 25 }),
    ]),
    search({ top: true }),
    syntaxHighlighting(oneDarkHighlightStyle),
    editorTheme,
    cmPlaceholder(`Write your SQL query here -- ${mod()}+Enter to execute`),
    EditorView.updateListener.of((update) => {
      if (!update.docChanged) return;
      const value = update.state.doc.toString();
      setSqlTabData(tabId, { query: value });
    }),
  ];

  const state = EditorState.create({ doc: initialDoc, extensions });

  const container = document.createElement('div');
  container.className = 'sql-editor-reparent-container';
  container.style.height = '100%';
  container.style.width = '100%';
  container.style.display = 'flex';
  container.style.flexDirection = 'column';
  container.style.minHeight = '0';
  container.style.overflow = 'hidden';

  const headerSlot = document.createElement('div');
  headerSlot.className = 'sql-editor-header-slot';
  headerSlot.style.display = 'none';
  headerSlot.style.flexShrink = '0';
  container.appendChild(headerSlot);

  const editorSlot = document.createElement('div');
  editorSlot.className = 'sql-editor-editor-slot';
  editorSlot.style.flex = '1';
  editorSlot.style.minHeight = '0';
  editorSlot.style.display = 'flex';
  editorSlot.style.flexDirection = 'column';
  editorSlot.style.overflow = 'hidden';
  container.appendChild(editorSlot);

  const view = new EditorView({ state, parent: editorSlot });
  return {
    view,
    container,
    headerSlot,
    editorSlot,
    header: null,
    sqlCompartment,
    execKeymapCompartment,
  };
}

export function getSqlEditorEntry(tabId: number): SqlEditorEntry | undefined {
  return editors.get(tabId);
}

export function getSqlEditorView(tabId: number): EditorView | undefined {
  return editors.get(tabId)?.view;
}

export interface AttachOptions {
  /** Show the in-tile connection picker strip above the editor. Pass
   *  `true` from canvas tile mounts; leave unset (or false) for the
   *  home-mode SqlPanel, which renders its own action-bar picker. */
  showHeader?: boolean;
}

/**
 * Mount the EditorView for `tabId` into `slot`. Lazily creates the view
 * on first attach using `sqlTabState[tabId].query` as the initial doc.
 * The native appendChild semantics auto-detach the container from any
 * previous parent, preserving CodeMirror's internal state. Calls
 * requestMeasure + focus so layout and focus settle into the new slot.
 */
export function attachSqlEditor(
  tabId: number,
  slot: HTMLElement,
  options: AttachOptions = {},
): void {
  let entry = editors.get(tabId);
  if (!entry) {
    const initialDoc = getSqlTabData(tabId).query ?? '';
    entry = createEntry(tabId, initialDoc);
    editors.set(tabId, entry);
  }

  if (options.showHeader) {
    if (!entry.header) {
      entry.header = mount(SqlTileHeader, {
        target: entry.headerSlot,
        props: { tabId },
      });
    }
    entry.headerSlot.style.display = '';
  } else {
    entry.headerSlot.style.display = 'none';
  }

  slot.appendChild(entry.container);

  try {
    entry.view.requestMeasure();
  } catch {
    // Slot may not have measurable dimensions yet on first mount.
  }
  const active = document.activeElement;
  if (!active || active === document.body) {
    entry.view.focus();
  }
}

/**
 * Remove the EditorView's container from `slot` without destroying the
 * view. Blurs contentDOM first so any active IME composition tears
 * down before the DOM moves, preventing dangling composition state.
 */
export function detachSqlEditor(tabId: number, slot: HTMLElement): void {
  const entry = editors.get(tabId);
  if (!entry) return;

  try {
    (entry.view.contentDOM as HTMLElement).blur();
  } catch {
    // contentDOM may not be focused; blur is best-effort.
  }

  if (entry.container.parentElement === slot) {
    slot.removeChild(entry.container);
  }
}

/**
 * Destroy the EditorView and drop the registry entry. Call when the
 * tab is permanently closed (not just unmounted for a reparent).
 */
export function destroySqlEditor(tabId: number): void {
  tabExecutors.delete(tabId);
  const entry = editors.get(tabId);
  if (!entry) return;
  if (entry.header) {
    try {
      unmount(entry.header);
    } catch {
      // best-effort teardown
    }
    entry.header = null;
  }
  try {
    entry.view.destroy();
  } catch {
    // best-effort teardown
  }
  if (entry.container.parentElement) {
    entry.container.parentElement.removeChild(entry.container);
  }
  editors.delete(tabId);
}

/** Tab ids that currently have a live EditorView in the registry. */
export function listOpenSqlEditors(): number[] {
  return [...editors.keys()];
}

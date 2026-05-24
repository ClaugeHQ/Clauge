<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { EditorView, keymap, placeholder as cmPlaceholder, lineNumbers } from '@codemirror/view';
  import { EditorState, Compartment } from '@codemirror/state';
  import { json } from '@codemirror/lang-json';
  import { xml } from '@codemirror/lang-xml';
  import { linter, lintGutter, type Diagnostic } from '@codemirror/lint';
  import { syntaxHighlighting } from '@codemirror/language';
  import { oneDarkHighlightStyle } from '@codemirror/theme-one-dark';
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
  import {
    autocompletion,
    type CompletionContext,
    type CompletionResult,
    type Completion,
  } from '@codemirror/autocomplete';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import FormKVEditor from './FormKVEditor.svelte';
  import MultipartEditor from './MultipartEditor.svelte';
  import BinaryPicker from './BinaryPicker.svelte';
  import { BODY_DEBOUNCE_MS } from '$lib/shared/constants/timings';
  import {
    activeEnvId,
    activeRequest,
    requestEnvOverrides,
    environments,
    getEffectiveEnvId,
    createEnvironment,
    setActiveEnv,
    setEnvVariable,
  } from '$lib/modes/rest/stores';
  import { activeTabId } from '$lib/shared/stores/tabs';
  import { getEnvVariablesForResolution } from '$lib/modes/rest/commands';
  import { get } from 'svelte/store';

  let { body = '', bodyType = 'json', onchange }: {
    body: string;
    bodyType: string;
    onchange: (body: string, bodyType: string) => void;
  } = $props();

  // localBody / localType are seeded from props by the first sync $effect
  // below. Initialising via $state(body) confuses svelte-check (the literal
  // prop reference looks like a one-shot capture) and obscures the actual
  // sync path through the effect.
  let localBody = $state('');
  let localType = $state('json');
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  let editorContainer: HTMLDivElement | undefined = $state();
  let editorView: EditorView | undefined;
  const langCompartment = new Compartment();
  const lintCompartment = new Compartment();
  const placeholderCompartment = new Compartment();
  let suppressExternalSync = false;

  const isTextType = $derived(['json', 'text', 'xml'].includes(localType));

  // ── Env var autocomplete for {{var}} ────────────────────────────────
  // CodeMirror has its own completion infrastructure (`@codemirror/autocomplete`)
  // so unlike EnvInput we don't hand-roll a dropdown — we register a
  // completion source that fires whenever the user types `{{`. Reads
  // current env vars into a module-level cache so the source function
  // (which CodeMirror calls synchronously) can return matches without
  // an await. The cache refreshes when the active env changes.
  let envVarCache: { key: string; value: string }[] = [];
  const VAR_NAME_RE = /^[A-Za-z_][A-Za-z0-9_-]*$/;

  const overrideKey = $derived($activeRequest?.id ?? String($activeTabId));
  const effectiveEnvId = $derived(
    getEffectiveEnvId(overrideKey, $requestEnvOverrides, $activeEnvId)
  );

  let envFetchVersion = 0;
  $effect(() => {
    const envId = effectiveEnvId;
    const version = ++envFetchVersion;
    if (envId) {
      getEnvVariablesForResolution(envId).then(vars => {
        if (version === envFetchVersion) {
          envVarCache = Object.entries(vars).map(([key, value]) => ({ key, value }));
        }
      }).catch(() => {
        if (version === envFetchVersion) envVarCache = [];
      });
    } else {
      envVarCache = [];
    }
  });

  async function createEnvVarInline(name: string) {
    if (!VAR_NAME_RE.test(name)) {
      showToast('Variable names use letters, digits, _ and -', 'error');
      return;
    }
    try {
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
      const vars = await getEnvVariablesForResolution(envId);
      envVarCache = Object.entries(vars).map(([key, value]) => ({ key, value }));
      showToast(`Added {{${name}}} to environment`, 'success');
    } catch (e) {
      errorToast('Failed to create variable', e);
    }
  }

  function envVarCompletion(ctx: CompletionContext): CompletionResult | null {
    // Match `{{` followed by an optional identifier prefix.
    // word: only the identifier part (so completions REPLACE the prefix
    // rather than appending to it).
    const match = ctx.matchBefore(/\{\{([A-Za-z_][A-Za-z0-9_-]*)?/);
    if (!match) return null;
    // Don't auto-open in the middle of `{{foo` without explicit trigger if
    // user is mid-typing — but CodeMirror handles trigger gating already
    // via `explicit` flag, so just always offer when we matched.
    const afterBraces = match.text.slice(2); // text after {{
    const from = match.from + 2; // replace only the identifier part, leave {{ alone

    const options: Completion[] = envVarCache.map(v => ({
      label: v.key,
      detail: v.value.length > 40 ? v.value.slice(0, 37) + '…' : v.value,
      type: 'variable',
      apply: `${v.key}}}`,
    }));

    const exact = envVarCache.some(v => v.key === afterBraces);
    if (afterBraces.length > 0 && !exact && VAR_NAME_RE.test(afterBraces)) {
      options.push({
        label: `Create {{${afterBraces}}}`,
        type: 'keyword',
        detail: get(activeEnvId) ? 'in current env' : 'new env "Default"',
        apply: (view, _completion, fromPos, toPos) => {
          // Insert chip immediately so the user doesn't wait on the async
          // create — the var is added in the background.
          view.dispatch({
            changes: { from: fromPos, to: toPos, insert: `${afterBraces}}}` },
            selection: { anchor: fromPos + afterBraces.length + 2 },
          });
          createEnvVarInline(afterBraces);
        },
      });
    }

    if (options.length === 0) return null;

    return {
      from,
      to: match.to,
      options,
      // Re-evaluate when the user keeps typing inside {{ — without this
      // the dropdown closes after the first match.
      validFor: /^[A-Za-z0-9_-]*$/,
    };
  }

  // ── Language extension per body type ────────────────────────────────
  // JSON/XML carry full grammar + highlighting; "text" gets no language
  // but still wraps + highlights generic content.
  function langExtFor(type: string) {
    if (type === 'json') return json();
    if (type === 'xml') return xml();
    return [];
  }

  // Hand-rolled JSON linter. We don't use `@codemirror/lang-json`'s
  // `jsonParseLinter()` because it greps the runtime's JSON.parse
  // error message for "position N" — and JavaScriptCore (Tauri's
  // WKWebView on macOS) omits that field entirely ("JSON Parse error:
  // Unexpected EOF"). Result: every error landed at offset 0 and the
  // gutter dot stuck to line 1 regardless of where the real fault was.
  //
  // This scanner walks the document once and reports the first error
  // it finds with the exact from/to offsets. Cases covered (matches
  // the failure modes a user actually hits while editing):
  //   • Unterminated string  → range of the offending string literal
  //   • Unclosed `{` / `[`   → the opening bracket
  //   • Unexpected `}` / `]` → that closing bracket
  //   • Mismatched bracket   → the mismatched closer
  //   • Trailing comma       → the comma
  //   • Empty / whitespace   → no diagnostics (don't yell at users
  //                            who haven't typed anything yet)
  function scanJsonError(text: string): Diagnostic | null {
    const n = text.length;
    let i = 0;
    const stack: { ch: '{' | '['; pos: number }[] = [];
    let expectValue = true;

    const skipWs = () => {
      while (i < n) {
        const c = text.charCodeAt(i);
        if (c === 32 || c === 9 || c === 10 || c === 13) i++;
        else break;
      }
    };

    while (i < n) {
      skipWs();
      if (i >= n) break;
      const c = text[i];

      // String
      if (c === '"') {
        const start = i;
        i++;
        let closed = false;
        while (i < n) {
          const ch = text[i];
          if (ch === '\\') { i += 2; continue; }
          if (ch === '\n') break; // strict JSON forbids raw newlines in strings
          if (ch === '"') { i++; closed = true; break; }
          i++;
        }
        if (!closed) {
          return {
            from: start,
            to: Math.min(n, i),
            severity: 'error',
            message: 'Unterminated string',
          };
        }
        expectValue = false;
        continue;
      }

      // Opening bracket
      if (c === '{' || c === '[') {
        stack.push({ ch: c as '{' | '[', pos: i });
        i++;
        expectValue = true;
        skipWs();
        // Empty object/array is fine — flip expectValue back off if next is the closer
        if (i < n && (text[i] === '}' || text[i] === ']')) expectValue = false;
        continue;
      }

      // Closing bracket
      if (c === '}' || c === ']') {
        const top = stack.pop();
        if (!top) {
          return { from: i, to: i + 1, severity: 'error', message: `Unexpected '${c}'` };
        }
        const expected = top.ch === '{' ? '}' : ']';
        if (c !== expected) {
          return {
            from: i,
            to: i + 1,
            severity: 'error',
            message: `Mismatched bracket — expected '${expected}'`,
          };
        }
        i++;
        expectValue = false;
        continue;
      }

      // Comma — flag trailing commas (next non-ws is a closer)
      if (c === ',') {
        const commaPos = i;
        i++;
        const saved = i;
        skipWs();
        if (i < n && (text[i] === '}' || text[i] === ']')) {
          return { from: commaPos, to: commaPos + 1, severity: 'error', message: 'Trailing comma' };
        }
        i = saved;
        expectValue = true;
        continue;
      }

      // Colon — just consume
      if (c === ':') {
        i++;
        expectValue = true;
        continue;
      }

      // Literals: true / false / null
      if (c === 't' || c === 'f' || c === 'n') {
        const rest = text.substring(i);
        if (rest.startsWith('true')) { i += 4; expectValue = false; continue; }
        if (rest.startsWith('false')) { i += 5; expectValue = false; continue; }
        if (rest.startsWith('null')) { i += 4; expectValue = false; continue; }
        return { from: i, to: i + 1, severity: 'error', message: `Unexpected token '${c}'` };
      }

      // Numbers
      if (c === '-' || (c >= '0' && c <= '9')) {
        const start = i;
        if (c === '-') i++;
        while (i < n && text[i] >= '0' && text[i] <= '9') i++;
        if (i < n && text[i] === '.') {
          i++;
          while (i < n && text[i] >= '0' && text[i] <= '9') i++;
        }
        if (i < n && (text[i] === 'e' || text[i] === 'E')) {
          i++;
          if (i < n && (text[i] === '+' || text[i] === '-')) i++;
          while (i < n && text[i] >= '0' && text[i] <= '9') i++;
        }
        if (i === start) {
          return { from: i, to: i + 1, severity: 'error', message: 'Invalid number' };
        }
        expectValue = false;
        continue;
      }

      // Anything else here is unexpected
      return { from: i, to: i + 1, severity: 'error', message: `Unexpected character '${c}'` };
    }

    // EOF — anything left unclosed?
    if (stack.length > 0) {
      const last = stack[stack.length - 1];
      return {
        from: last.pos,
        to: last.pos + 1,
        severity: 'error',
        message: `Unclosed '${last.ch}'`,
      };
    }

    void expectValue;
    return null;
  }

  function lintExtFor(type: string) {
    if (type !== 'json') return [];
    return linter((view) => {
      const text = view.state.doc.toString();
      if (!text.trim()) return [];
      const err = scanJsonError(text);
      return err ? [err] : [];
    });
  }

  function placeholderFor(type: string): string {
    if (type === 'json') return '{\n  "key": "value"\n}';
    if (type === 'xml') return '<root>\n  <element>value</element>\n</root>';
    return 'Enter request body...';
  }

  // ── Theme: matches SQL QueryEditor + adds wrap-aware tweaks ─────────
  const editorTheme = EditorView.theme({
    '&': { backgroundColor: 'transparent', fontSize: '12.5px', height: '100%' },
    '.cm-content': {
      fontFamily: 'var(--mono)',
      caretColor: 'var(--acc)',
      padding: '12px 4px',
      // Wrapped lines need to break inside long unspaced tokens (URLs,
      // tokens, hashes) — overflow-wrap alone isn't enough because
      // CodeMirror sets word-break: normal; word-wrap; on .cm-content.
      wordBreak: 'break-word',
    },
    '.cm-cursor': { borderLeftColor: 'var(--acc)' },
    '.cm-gutters': {
      backgroundColor: 'transparent',
      borderRight: '1px solid var(--b1)',
      color: 'var(--t4)',
      fontSize: '11px',
      fontFamily: 'var(--mono)',
      minWidth: '36px',
    },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: 'var(--t2)' },
    '.cm-activeLine': { backgroundColor: 'transparent' },
    '.cm-selectionBackground': { backgroundColor: 'rgba(124,92,248,0.2) !important' },
    '&.cm-focused .cm-selectionBackground': { backgroundColor: 'rgba(124,92,248,0.3) !important' },
    '.cm-scroller': { overflow: 'auto', fontFamily: 'var(--mono)' },
    '.cm-scroller::-webkit-scrollbar': { width: '4px', height: '4px' },
    '.cm-scroller::-webkit-scrollbar-thumb': { background: 'var(--b1)', borderRadius: '2px' },
    // Diagnostic styling — underline the exact character range and put
    // a coloured dot on the gutter line containing the error. CodeMirror
    // computes both positions from jsonParseLinter()'s `from`/`to`, so
    // the dot lands on the actual error line, not the top of the editor.
    '.cm-lintRange-error': {
      backgroundImage: 'none',
      textDecoration: 'underline wavy #ef4444',
      textUnderlineOffset: '3px',
    },
    '.cm-gutter-lint': { width: '14px' },
    '.cm-gutter-lint .cm-gutterElement': {
      padding: '0',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
    },
    '.cm-gutter-lint .cm-lint-marker-error': {
      content: '""',
      width: '7px',
      height: '7px',
      borderRadius: '50%',
      background: '#ef4444',
      boxShadow: '0 0 0 2px rgba(239, 68, 68, 0.18)',
    },
    // Hide the default SVG so our circle is the only thing showing.
    '.cm-gutter-lint .cm-lint-marker-error svg': { display: 'none' },
    '.cm-gutter-lint .cm-lint-marker-warning': {
      width: '7px',
      height: '7px',
      borderRadius: '50%',
      background: '#f59e0b',
    },
    '.cm-gutter-lint .cm-lint-marker-warning svg': { display: 'none' },
    // Env-var autocomplete popup — match the other dropdowns in the app
    // (EnvInput, RequestBar URL bar) so the body editor's {{ menu doesn't
    // look out of place.
    '.cm-tooltip-autocomplete': {
      background: 'var(--n)',
      border: '1px solid var(--b1)',
      borderRadius: '6px',
      overflow: 'hidden',
      boxShadow: '0 6px 20px rgba(0, 0, 0, 0.4)',
    },
    '.cm-tooltip-autocomplete ul': {
      fontFamily: 'var(--mono)',
      fontSize: '11.5px',
    },
    '.cm-tooltip-autocomplete ul li': {
      padding: '6px 10px',
      display: 'flex',
      alignItems: 'center',
      gap: '8px',
      color: 'var(--t2)',
    },
    '.cm-tooltip-autocomplete ul li[aria-selected]': {
      background: 'var(--c)',
      color: 'var(--t1)',
    },
    '.cm-completionLabel': { color: 'var(--t1)' },
    '.cm-completionDetail': {
      marginLeft: 'auto',
      color: 'var(--t3)',
      fontSize: '10px',
      maxWidth: '160px',
      overflow: 'hidden',
      textOverflow: 'ellipsis',
      whiteSpace: 'nowrap',
      fontStyle: 'normal',
    },
  });

  function setBody(next: string) {
    localBody = next;
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      onchange(localBody, localType);
    }, BODY_DEBOUNCE_MS);
  }

  function buildState(value: string, type: string) {
    return EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        history(),
        EditorView.lineWrapping,
        placeholderCompartment.of(cmPlaceholder(placeholderFor(type))),
        langCompartment.of(langExtFor(type)),
        lintCompartment.of(lintExtFor(type)),
        lintGutter(),
        syntaxHighlighting(oneDarkHighlightStyle),
        // `override` replaces the language's default sources entirely —
        // we only want {{var}} completion in body fields, not JSON
        // schema or keyword completion. activateOnTyping keeps it
        // popping open as the user types after `{{` without needing
        // Ctrl+Space.
        autocompletion({
          override: [envVarCompletion],
          activateOnTyping: true,
          closeOnBlur: true,
          maxRenderedOptions: 12,
        }),
        keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
        editorTheme,
        EditorView.updateListener.of((u) => {
          if (suppressExternalSync) return;
          if (u.docChanged) {
            setBody(u.state.doc.toString());
          }
        }),
      ],
    });
  }

  function mountEditor() {
    if (!editorContainer || editorView) return;
    if (!isTextType) return;
    editorView = new EditorView({
      state: buildState(localBody, localType),
      parent: editorContainer,
    });
  }

  function teardownEditor() {
    editorView?.destroy();
    editorView = undefined;
  }

  // Mount / unmount when toggling between text-types and structured types.
  $effect(() => {
    if (isTextType) {
      // Need to wait a tick for editorContainer to bind after the {#if} flips.
      queueMicrotask(() => mountEditor());
    } else {
      teardownEditor();
    }
  });

  // External body changes (tab switch, request load, format) → push to editor.
  $effect(() => {
    const incoming = body;
    if (!editorView) {
      localBody = incoming;
      return;
    }
    if (incoming === editorView.state.doc.toString()) {
      localBody = incoming;
      return;
    }
    suppressExternalSync = true;
    editorView.dispatch({
      changes: { from: 0, to: editorView.state.doc.length, insert: incoming },
    });
    localBody = incoming;
    queueMicrotask(() => { suppressExternalSync = false; });
  });

  // bodyType prop changes → swap language/linter/placeholder without
  // destroying the editor (preserves undo stack across format toggles).
  $effect(() => {
    const incomingType = bodyType;
    if (incomingType === localType) return;

    const wasStructured = ['urlencoded', 'multipart', 'binary'].includes(localType);
    const isStructured = ['urlencoded', 'multipart', 'binary'].includes(incomingType);
    localType = incomingType;

    // Body resets are handled in handleTypeChange when the user picks a
    // structured ↔ raw transition. External prop-driven changes just
    // reconfigure the editor language.
    if (!editorView) return;
    if (wasStructured || isStructured) return;
    editorView.dispatch({
      effects: [
        langCompartment.reconfigure(langExtFor(incomingType)),
        lintCompartment.reconfigure(lintExtFor(incomingType)),
        placeholderCompartment.reconfigure(cmPlaceholder(placeholderFor(incomingType))),
      ],
    });
  });

  onDestroy(() => {
    teardownEditor();
    if (debounceTimer) clearTimeout(debounceTimer);
  });

  function handleTypeChange(e: Event) {
    const target = e.target as HTMLSelectElement;
    const newType = target.value;
    const wasStructured = ['urlencoded', 'multipart', 'binary'].includes(localType);
    const isStructured = ['urlencoded', 'multipart', 'binary'].includes(newType);
    if (wasStructured !== isStructured || (wasStructured && isStructured && localType !== newType)) {
      localBody = '';
      if (editorView) {
        suppressExternalSync = true;
        editorView.dispatch({ changes: { from: 0, to: editorView.state.doc.length, insert: '' } });
        queueMicrotask(() => { suppressExternalSync = false; });
      }
    }
    localType = newType;
    if (editorView && !isStructured) {
      editorView.dispatch({
        effects: [
          langCompartment.reconfigure(langExtFor(newType)),
          lintCompartment.reconfigure(lintExtFor(newType)),
          placeholderCompartment.reconfigure(cmPlaceholder(placeholderFor(newType))),
        ],
      });
    }
    onchange(localBody, localType);
  }

  function formatJson() {
    if (localType !== 'json') return;
    invoke('telemetry_bump', { key: 'rest.format_json' }).catch(() => {});
    const current = editorView ? editorView.state.doc.toString() : localBody;
    try {
      const parsed = JSON.parse(current);
      const formatted = JSON.stringify(parsed, null, 2);
      if (editorView) {
        editorView.dispatch({
          changes: { from: 0, to: editorView.state.doc.length, insert: formatted },
        });
      } else {
        localBody = formatted;
      }
      onchange(formatted, localType);
      showToast('JSON formatted', 'success');
    } catch {
      showToast('Invalid JSON', 'error');
    }
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
    <div class="editor-wrap" bind:this={editorContainer}></div>
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
    padding-right: 22px;
    border-radius: 4px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    cursor: pointer;
    outline: none;
    transition: border-color 0.1s, color 0.1s;
    -webkit-appearance: none;
    appearance: none;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23b0b0c8' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'><polyline points='3 5 6 8 9 5'/></svg>");
    background-repeat: no-repeat;
    background-position: right 5px center;
    background-size: 8px 8px;
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
    overflow: hidden;
    background: transparent;
    min-height: 0;
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

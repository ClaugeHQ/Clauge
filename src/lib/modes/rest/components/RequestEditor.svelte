<script lang="ts">
  import { activeRequest } from '$lib/modes/rest/stores';
  import type { KVInput } from '$lib/types';
  import { activeTabId, markDirty, setDraft, getDraft, draftRequests } from '$lib/shared/stores/tabs';
  import BodyEditor from './BodyEditor.svelte';
  import KVTable from './KVTable.svelte';
  import AuthEditor from './AuthEditor.svelte';

  interface Props {
    currentMethod?: string;
  }

  let { currentMethod = 'GET' }: Props = $props();

  type Tab = 'body' | 'headers' | 'auth' | 'params' | 'pre-script';

  // Read from draft for unsaved tabs (reactive via $draftRequests)
  const draft = $derived($draftRequests.get($activeTabId));

  // Unified data source: activeRequest (saved) or draft (unsaved)
  const body = $derived($activeRequest?.body ?? draft?.body ?? '');
  const bodyType = $derived($activeRequest?.bodyType ?? draft?.bodyType ?? 'json');
  const headers = $derived(
    $activeRequest?.headers?.map(h => ({ key: h.key, value: h.value, enabled: h.enabled }))
    ?? draft?.headers
    ?? []
  );
  const savedParams = $derived(
    $activeRequest?.params?.map(p => ({ key: p.key, value: p.value, enabled: p.enabled }))
    ?? draft?.params
    ?? []
  );

  // Parse query params from URL and merge with saved params
  const urlForParams = $derived($activeRequest?.url ?? draft?.url ?? '');
  const params = $derived.by(() => {
    const qIdx = urlForParams.indexOf('?');
    if (qIdx === -1) return savedParams;
    const qs = urlForParams.substring(qIdx + 1);
    if (!qs) return savedParams;
    const urlParams: KVInput[] = qs.split('&').filter(Boolean).map(pair => {
      const eqIdx = pair.indexOf('=');
      if (eqIdx >= 0) {
        return { key: decodeURIComponent(pair.substring(0, eqIdx)), value: decodeURIComponent(pair.substring(eqIdx + 1)), enabled: 1 };
      }
      return { key: decodeURIComponent(pair), value: '', enabled: 1 };
    });
    // Merge: URL params take precedence, add any saved params not in URL
    const urlKeys = new Set(urlParams.map(p => p.key));
    const extra = savedParams.filter(p => !urlKeys.has(p.key));
    return [...urlParams, ...extra];
  });
  const authType = $derived($activeRequest?.authType ?? draft?.authType ?? 'none');
  const authData = $derived($activeRequest?.authData ?? draft?.authData ?? '{}');
  const preScriptVal = $derived($activeRequest?.preScript ?? draft?.preScript ?? '');

  const tabs: { id: Tab; label: string }[] = [
    { id: 'body', label: 'Body' },
    { id: 'headers', label: 'Headers' },
    { id: 'auth', label: 'Auth' },
    { id: 'params', label: 'Params' },
    { id: 'pre-script', label: 'Pre-script' },
  ];

  const BODY_METHODS = ['POST', 'PUT', 'PATCH', 'DELETE'];
  const method = $derived($activeRequest?.method?.toUpperCase() ?? currentMethod.toUpperCase());
  const hasBody = $derived(BODY_METHODS.includes(method));

  let activeTab: Tab = $state('headers');
  let prevMethod = '';

  // Auto-switch default tab when method changes
  $effect(() => {
    const m = method;
    if (m !== prevMethod) {
      prevMethod = m;
      if (BODY_METHODS.includes(m) && activeTab === 'params') {
        activeTab = 'body';
      } else if (!BODY_METHODS.includes(m) && activeTab === 'body') {
        activeTab = 'headers';
      }
    }
  });
  let preScript = $state('');
  let preScriptTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    preScript = preScriptVal;
  });

  function handleBodyChange(body: string, bodyType: string) {
    const tabId = $activeTabId;
    if ($activeRequest) {
      activeRequest.update(r => r ? { ...r, body, bodyType } : r);
    }
    setDraft(tabId, { body, bodyType });
    markDirty(tabId);
  }

  function handleHeadersChange(items: KVInput[]) {
    const tabId = $activeTabId;
    if ($activeRequest) {
      activeRequest.update(r => r ? { ...r, headers: items.map((h, i) => ({ id: '', requestId: r.id, key: h.key, value: h.value, enabled: h.enabled, sortOrder: i })) } : r);
    }
    setDraft(tabId, { headers: items });
    markDirty(tabId);
  }

  function handleParamsChange(items: KVInput[]) {
    const tabId = $activeTabId;

    // Update params in draft/request
    if ($activeRequest) {
      activeRequest.update(r => r ? { ...r, params: items.map((p, i) => ({ id: '', requestId: r.id, key: p.key, value: p.value, enabled: p.enabled, sortOrder: i })) } : r);
    }
    setDraft(tabId, { params: items });

    // Sync params back to URL
    const currentUrl = $activeRequest?.url ?? draft?.url ?? '';
    const baseUrl = currentUrl.split('?')[0];
    const enabledParams = items.filter(p => p.enabled && p.key.trim());
    if (enabledParams.length > 0) {
      const qs = enabledParams.map(p => `${encodeURIComponent(p.key)}=${encodeURIComponent(p.value)}`).join('&');
      const newUrl = `${baseUrl}?${qs}`;
      if ($activeRequest) {
        activeRequest.update(r => r ? { ...r, url: newUrl } : r);
      }
      setDraft(tabId, { url: newUrl });
    } else if (currentUrl.includes('?')) {
      // Remove query string if no params
      if ($activeRequest) {
        activeRequest.update(r => r ? { ...r, url: baseUrl } : r);
      }
      setDraft(tabId, { url: baseUrl });
    }

    markDirty(tabId);
  }

  function handleAuthChange(authType: string, authData: string) {
    const tabId = $activeTabId;
    if ($activeRequest) {
      activeRequest.update(r => r ? { ...r, authType, authData } : r);
    }
    setDraft(tabId, { authType, authData });
    markDirty(tabId);
  }

  function handlePreScriptInput(e: Event) {
    const target = e.target as HTMLTextAreaElement;
    preScript = target.value;
    const tabId = $activeTabId;
    if ($activeRequest) {
      activeRequest.update(r => r ? { ...r, preScript } : r);
    }
    setDraft(tabId, { preScript });
    markDirty(tabId);
  }
</script>

<div class="req-editor">
  <div class="ph">
    {#each tabs as tab (tab.id)}
      {#if tab.id !== 'body' || hasBody}
      <button
        class="ph-tab"
        class:active={activeTab === tab.id}
        onclick={() => activeTab = tab.id}
      >
        {tab.label}
        {#if tab.id === 'headers' && headers.length > 0}
          <span class="ph-badge">{headers.length}</span>
        {/if}
        {#if tab.id === 'params' && params.length > 0}
          <span class="ph-badge">{params.length}</span>
        {/if}
      </button>
      {/if}
    {/each}
  </div>

  <div class="ph-content">
    {#if activeTab === 'body'}
      <BodyEditor
        body={body}
        bodyType={bodyType}
        onchange={handleBodyChange}
      />
    {:else if activeTab === 'headers'}
      <KVTable
        items={headers}
        label="Add header"
        showKeySuggestions={true}
        onchange={handleHeadersChange}
      />
    {:else if activeTab === 'auth'}
      <AuthEditor
        authType={authType}
        authData={authData}
        onchange={handleAuthChange}
      />
    {:else if activeTab === 'params'}
      <KVTable
        items={params}
        label="Add param"
        onchange={handleParamsChange}
      />
    {:else if activeTab === 'pre-script'}
      <textarea
        class="editor"
        spellcheck="false"
        placeholder="// Pre-request script (not executed yet)"
        value={preScript}
        oninput={handlePreScriptInput}
      ></textarea>
    {/if}
  </div>
</div>

<style>
  .req-editor {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }
  .ph {
    height: 38px;
    background: var(--n2);
    border-bottom: 1px solid var(--b1);
    display: flex;
    align-items: center;
    padding: 0 12px;
    gap: 12px;
    flex-shrink: 0;
  }
  .ph-tab {
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    cursor: pointer;
    transition: color 0.1s;
    border-bottom: 1.5px solid transparent;
    padding-bottom: 2px;
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .ph-tab:hover {
    color: var(--t2);
  }
  .ph-tab.active {
    color: var(--t1);
    border-bottom-color: var(--acc);
  }
  .ph-badge {
    font-size: 10px;
    background: var(--b1);
    color: var(--t2);
    padding: 1px 5px;
    border-radius: 8px;
    font-family: var(--mono);
  }
  .ph-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .ph-empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
  }
  .ph-empty-msg {
    color: var(--t3);
    font-size: 12px;
    font-family: var(--mono);
    font-style: italic;
  }
  .editor {
    flex: 1;
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
</style>

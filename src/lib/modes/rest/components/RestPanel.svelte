<script lang="ts">
  import RequestBar from './RequestBar.svelte';
  import RequestEditor from './RequestEditor.svelte';
  import ResponseViewer from './ResponseViewer.svelte';
  import { activeRequest, activeRequestId, requestEnvOverrides, commitRequest, currentRestResponse } from '$lib/modes/rest/stores';
  import { activeEnvId, getEffectiveEnvId } from '$lib/modes/rest/stores';
  import { executeRequest, quickExecute } from '$lib/modes/rest/commands';
  import { showToast } from '$lib/components/shared/toast';
  import { friendlyError } from '$lib/utils/errors';
  import { loadHistory } from '$lib/modes/rest/stores';
  import { mode } from '$lib/stores/app';
  import { tabs, activeTabId, getDraft, draftRequests } from '$lib/shared/stores/tabs';
  import type { HttpResponse } from '$lib/types';
  import { get } from 'svelte/store';

  // Per-tab response cache
  let responseMap = new Map<number, HttpResponse | null>();
  let response: HttpResponse | null = $state(null);
  let loading = $state(false);
  let currentMethod = $state('GET');

  // Sync response to shared store for AI panel
  $effect(() => {
    currentRestResponse.set(response);
  });

  // Clear/restore response when switching tabs
  let prevTabId = -1;
  $effect(() => {
    const tabId = $activeTabId;
    if (tabId !== prevTabId) {
      // Save current response for previous tab
      if (prevTabId >= 0) responseMap.set(prevTabId, response);
      // Restore response for new tab (or null if none)
      response = responseMap.get(tabId) ?? null;
      prevTabId = tabId;
    }
  });

  let rightPanePct = $state(55);
  let dragging = $state(false);
  let panesEl: HTMLDivElement;

  function onDividerDown(e: MouseEvent) {
    e.preventDefault();
    dragging = true;
    const onMove = (ev: MouseEvent) => {
      if (!panesEl) return;
      const rect = panesEl.getBoundingClientRect();
      const x = ev.clientX - rect.left;
      const pct = 100 - (x / rect.width) * 100;
      rightPanePct = Math.min(80, Math.max(20, pct));
    };
    const onUp = () => {
      dragging = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }

  function handleMethodChange(method: string) {
    currentMethod = method;
  }

  async function handleSend() {
    const reqId = get(activeRequestId);
    const req = get(activeRequest);

    loading = true;
    response = null;

    try {
      if (reqId && req) {
        // Saved request — commit any dirty changes before executing
        const tabId = get(activeTabId);
        const draft = getDraft(tabId);
        if (draft) {
          await commitRequest(reqId, draft);
        }
        const overrideKey = reqId || String(get(activeTabId));
        const overrides = get(requestEnvOverrides);
        const globalEnv = get(activeEnvId);
        const envId = getEffectiveEnvId(overrideKey, overrides, globalEnv) || '';
        response = await executeRequest(reqId, envId);
      } else {
        // Unsaved tab — read from draft
        const tabId = get(activeTabId);
        const draft = getDraft(tabId);
        const methodVal = draft?.method || currentMethod || 'GET';
        const urlVal = draft?.url?.trim();

        if (!urlVal) {
          showToast('Enter a URL first', 'error');
          loading = false;
          return;
        }

        // Build headers array for quickExecute
        const headerPairs: [string, string][] = (draft?.headers ?? [])
          .filter(h => h.enabled && h.key.trim())
          .map(h => [h.key, h.value] as [string, string]);

        // Append query params to URL
        let finalUrl = urlVal;
        const paramPairs = (draft?.params ?? []).filter(p => p.enabled && p.key.trim());
        if (paramPairs.length > 0) {
          const sep = finalUrl.includes('?') ? '&' : '?';
          const qs = paramPairs.map(p => `${encodeURIComponent(p.key)}=${encodeURIComponent(p.value)}`).join('&');
          finalUrl = finalUrl + sep + qs;
        }

        // Use global environment for unsaved requests
        const globalEnv = get(activeEnvId);
        const draftAuthType = draft?.authType || 'none';
        const draftAuthData = draft?.authData || '{}';
        const draftBodyType = draft?.bodyType || 'json';
        response = await quickExecute(methodVal, finalUrl, draft?.body || '', headerPairs, globalEnv || '', draftAuthType, draftAuthData, draftBodyType);
      }

      showToast(`${response.status} ${response.status_text}`, response.status < 400 ? 'success' : 'error');
      loadHistory();
    } catch (e: any) {
      showToast(friendlyError(e), 'error');
      response = {
        status: 0,
        status_text: 'Error',
        headers: [],
        body: friendlyError(e),
        duration_ms: 0,
        size_bytes: 0,
      };
    } finally {
      loading = false;
      // Cache response for this tab
      responseMap.set(get(activeTabId), response);
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      if (get(mode) === 'rest') {
        handleSend();
        e.preventDefault();
      }
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if $tabs.length === 0 || $activeTabId === -1}
  <div class="rest-empty">
    <div class="rest-empty-icon">
      <svg viewBox="0 0 24 24" width="40" height="40"><path d="M12 5v14M5 12h14" stroke="var(--t4)" fill="none" stroke-width="1.5" stroke-linecap="round"/></svg>
    </div>
    <p class="rest-empty-text">Create a new request or select one from collections</p>
    <p class="rest-empty-hint">Press <kbd>+</kbd> on a collection or use the <kbd>+</kbd> button in the tab bar</p>
    <p class="rest-empty-hint rest-empty-ai"><kbd>⌘L</kbd> AI Assistant</p>
  </div>
{:else}
  <div class="rest-panel">
    <div class="rest-bar-area">
      <RequestBar onsend={handleSend} onmethodchange={handleMethodChange} />
    </div>
    <div class="rest-panes" class:dragging bind:this={panesEl}>
      <div class="rest-pane-left" style="width:{100 - rightPanePct}%">
        <RequestEditor {currentMethod} />
      </div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="rest-divider" onmousedown={onDividerDown}></div>
      <div class="rest-pane-right" style="width:{rightPanePct}%">
        <ResponseViewer {response} {loading} />
      </div>
    </div>
  </div>
{/if}

<style>
  .rest-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--t3);
  }
  .rest-empty-icon {
    opacity: 0.4;
    margin-bottom: 4px;
  }
  .rest-empty-text {
    font-size: 13px;
    font-family: var(--ui);
    color: var(--t2);
    margin: 0;
  }
  .rest-empty-hint {
    font-size: 11px;
    font-family: var(--mono);
    color: var(--t3);
    margin: 0;
  }
  .rest-empty-hint kbd {
    background: var(--b1);
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 10px;
  }
  .rest-empty-ai {
    margin-top: 12px;
    padding: 5px 14px;
    border: 1px solid var(--b1);
    border-radius: 6px;
    font-size: 11px;
    color: var(--t4);
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .rest-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .rest-bar-area {
    flex-shrink: 0;
  }
  .rest-panes {
    flex: 1;
    display: flex;
    overflow: hidden;
  }
  .rest-pane-left {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 0;
  }
  .rest-divider {
    width: 5px;
    flex-shrink: 0;
    cursor: col-resize;
    position: relative;
    background: transparent;
  }
  .rest-divider::after {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    left: 2px;
    width: 1px;
    background: var(--b1);
    transition: background 0.15s;
  }
  .rest-divider:hover::after,
  .rest-panes.dragging .rest-divider::after {
    width: 3px;
    left: 1px;
    background: var(--acc);
    border-radius: 1px;
  }
  .rest-panes.dragging {
    cursor: col-resize;
    user-select: none;
  }
  .rest-pane-right {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 0;
  }
</style>

<script lang="ts">
  import { mod } from '$lib/utils/platform';
  const m = mod();
  import RequestBar from './RequestBar.svelte';
  import RequestEditor from './RequestEditor.svelte';
  import ResponseViewer from './ResponseViewer.svelte';
  import { activeRequest, activeRequestId, requestEnvOverrides, commitRequest, currentRestResponse } from '$lib/modes/rest/stores';
  import { activeEnvId, getEffectiveEnvId } from '$lib/modes/rest/stores';
  import { executeRequest, quickExecute } from '$lib/modes/rest/commands';
  import { showToast } from '$lib/shared/primitives/toast';
  import ConfirmDialog from '$lib/shared/primitives/ConfirmDialog.svelte';
  import { friendlyError } from '$lib/utils/errors';
  import { loadHistory } from '$lib/modes/rest/stores';
  import { mode } from '$lib/stores/app';
  import { tabs, activeTabId, getDraft, draftRequests } from '$lib/shared/stores/tabs';
  import { setSetting } from '$lib/stores/settings';
  import type { HttpResponse } from '$lib/types';
  import { get } from 'svelte/store';

  // SSL cert-error retry modal. When the Rust executor returns an
  // "ssl-error: <reason>" the catch block routes here instead of toasting
  // a raw error. Confirm flips Settings → REST → SSL Verification off and
  // re-fires the original send.
  let sslPromptShow = $state(false);
  let sslPromptReason = $state('');

  // Empty state guard: matches what other mode panels do
  // (SqlPanel.activeSqlTab, NoSqlPanel.activeNoSqlTab) — render the
  // editor only when the active tab is actually a REST tab.
  const activeRestTab = $derived($tabs.find(t => t.id === $activeTabId && t.mode === 'rest'));

  // Per-tab caches. Plain Maps — they don't need to be reactive themselves;
  // we project the active tab's slot into reactive `$state` below.
  const responseCache = new Map<number, HttpResponse | null>();
  const loadingCache = new Map<number, boolean>();

  // What's currently displayed (active tab's slot).
  let response: HttpResponse | null = $state(null);
  let loading = $state(false);
  let currentMethod = $state('GET');

  // On tab switch, swap visible state from the cache.
  let prevTabId = -1;
  $effect(() => {
    const tabId = $activeTabId;
    if (tabId !== prevTabId) {
      prevTabId = tabId;
      response = responseCache.get(tabId) ?? null;
      loading = loadingCache.get(tabId) ?? false;
    }
  });

  // Sync active-tab response to shared store for AI panel
  $effect(() => {
    currentRestResponse.set(response);
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
    // Capture the originating tab once. All async work below writes back to
    // THIS tab — even if the user switches tabs while the request is in flight.
    const tabId = get(activeTabId);
    if (tabId < 0) return;

    // Don't allow concurrent sends on the same tab.
    if (loadingCache.get(tabId)) return;

    const reqId = get(activeRequestId);
    const req = get(activeRequest);

    loadingCache.set(tabId, true);
    responseCache.set(tabId, null);
    if (get(activeTabId) === tabId) {
      loading = true;
      response = null;
    }

    let resp: HttpResponse | null = null;
    try {
      if (reqId && req) {
        // Saved request — commit any dirty changes before executing
        const draft = getDraft(tabId);
        if (draft) {
          await commitRequest(reqId, draft);
        }
        const overrideKey = reqId || String(tabId);
        const overrides = get(requestEnvOverrides);
        const globalEnv = get(activeEnvId);
        const envId = getEffectiveEnvId(overrideKey, overrides, globalEnv) || '';
        resp = await executeRequest(reqId, envId);
      } else {
        // Unsaved tab — read from draft
        const draft = getDraft(tabId);
        const methodVal = draft?.method || currentMethod || 'GET';
        const urlVal = draft?.url?.trim();

        if (!urlVal) {
          showToast('Enter a URL first', 'error');
          loadingCache.set(tabId, false);
          if (get(activeTabId) === tabId) loading = false;
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
        resp = await quickExecute(methodVal, finalUrl, draft?.body || '', headerPairs, globalEnv || '', draftAuthType, draftAuthData, draftBodyType);
      }

      // Only toast if user is still on the originating tab — otherwise the
      // notification appears on whatever tab they switched to, which is noise.
      if (get(activeTabId) === tabId) {
        showToast(`${resp.status} ${resp.status_text}`, resp.status < 400 ? 'success' : 'error');
      }
      loadHistory();
    } catch (e: any) {
      const errMsg = friendlyError(e);
      const sslMatch = /^ssl-error:\s*(.+)$/.exec(errMsg);
      if (sslMatch && get(activeTabId) === tabId) {
        // Surface a retry modal instead of a generic error. The retry path
        // flips the SSL verification setting off and re-fires the send.
        sslPromptReason = sslMatch[1];
        sslPromptShow = true;
      } else if (get(activeTabId) === tabId) {
        showToast(errMsg, 'error');
      }
      resp = {
        status: 0,
        status_text: 'Error',
        headers: [],
        body: errMsg,
        duration_ms: 0,
        size_bytes: 0,
      };
    } finally {
      responseCache.set(tabId, resp);
      loadingCache.set(tabId, false);
      // Only update the visible pane if the user is still on the originating tab.
      if (get(activeTabId) === tabId) {
        response = resp;
        loading = false;
      }
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

  async function handleSslRetryConfirm() {
    sslPromptShow = false;
    try {
      await setSetting('ssl_verification', 'false');
      showToast('SSL verification disabled. Retrying…', 'success');
      // Fire the original send again. Settings update is awaited above so
      // the next build_rest_http_client read sees verify=false.
      handleSend();
    } catch (err) {
      showToast(`Failed to update setting: ${friendlyError(err)}`, 'error');
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if !activeRestTab}
  <div class="rest-empty">
    <div class="rest-empty-icon">
      <svg viewBox="0 0 24 24" width="40" height="40"><path d="M12 5v14M5 12h14" stroke="var(--t4)" fill="none" stroke-width="1.5" stroke-linecap="round"/></svg>
    </div>
    <p class="rest-empty-text">Create a new request or select one from collections</p>
    <p class="rest-empty-hint">Press <kbd>+</kbd> on a collection or use the <kbd>+</kbd> button in the tab bar</p>
    <p class="rest-empty-hint rest-empty-ai"><kbd>{m}+L</kbd> AI Assistant</p>
  </div>
{:else}
  <div class="rest-panel">
    <div class="rest-bar-area">
      <RequestBar onsend={handleSend} onmethodchange={handleMethodChange} {loading} />
    </div>
    <div class="rest-panes" class:dragging bind:this={panesEl}>
      <!-- {#key $activeTabId}: forces RequestEditor (and its KVTable / BodyEditor /
           AuthEditor children) to remount when the active tab changes, so per-tab
           draft state can't bleed via stale component-local fields. -->
      <div class="rest-pane-left" style="width:{100 - rightPanePct}%">
        {#key $activeTabId}
          <RequestEditor {currentMethod} />
        {/key}
      </div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="rest-divider" onmousedown={onDividerDown}></div>
      <div class="rest-pane-right" style="width:{rightPanePct}%">
        <ResponseViewer {response} {loading} />
      </div>
    </div>
  </div>
{/if}

<!-- SSL cert-error retry modal. Triggered when the Rust executor returns
     "ssl-error: <reason>"; confirming flips Settings → REST → SSL Verification
     off and re-fires the original send. -->
<ConfirmDialog
  bind:show={sslPromptShow}
  title="Couldn't verify the server's certificate"
  message={`${sslPromptReason}.

This is usually safe for internal or self-hosted APIs but risky on the public internet — anyone on the network path could be intercepting traffic.

You can disable SSL verification globally and retry. Re-enable it from Settings → REST → SSL Verification when you're done.`}
  confirmText="Disable SSL verification & retry"
  confirmColor="var(--warn, #f5a623)"
  onconfirm={handleSslRetryConfirm}
/>

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

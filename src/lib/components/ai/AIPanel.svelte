<script lang="ts">
  import { aiPanelOpen, aiPanelOpenPerMode, mode, activeModal, getModeChatMessages, setModeChatMessages, clearModeChatMessages, type AppMode } from '$lib/stores/app';
  import { settings } from '$lib/stores/settings';
  import { loadCollections } from '$lib/stores/collections';
  import { activeTabId, draftRequests } from '$lib/stores/tabs';
  import { sendChatMessage, generateSessionId } from '$lib/services/ai-chat';
  import { REST_SYSTEM_PROMPT, REST_TOOLS } from '$lib/prompts/rest';
  import { SQL_SYSTEM_PROMPT, SQL_TOOLS } from '$lib/prompts/sql';
  import { NOSQL_SYSTEM_PROMPT, NOSQL_TOOLS } from '$lib/modes/nosql/ai/prompt';
  import { buildSshSystemPrompt, SSH_TOOLS } from '$lib/modes/ssh/ai/prompt';
  import { activeSshProfile } from '$lib/modes/ssh/stores';
  import { redactSecrets } from '$lib/modes/ssh/ai/safety';
  import { showToast } from '$lib/components/shared/toast';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import type { AIMessage, ChatMessage, ChatContext, AIActionBlock } from '$lib/types/ai';
  import { get } from 'svelte/store';
  import { onDestroy, onMount } from 'svelte';
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';
  import { highlightJSON } from '$lib/utils/json-highlight';
  import SshExecuteConfirmModal from '$lib/modes/ssh/components/SshExecuteConfirmModal.svelte';
  import { executeAndCaptureOnSsh } from '$lib/modes/ssh/ai/execute';
  import { getSshAutoRun, setSshAutoRun, getAiPanelWidth, setAiPanelWidth } from '$lib/shared/constants/storage';
  import { aiEvent } from '$lib/shared/constants/events';
  import { COPY_FEEDBACK_MS } from '$lib/shared/constants/timings';

  marked.setOptions({ breaks: true, gfm: true });

  function renderMarkdown(text: string): string {
    if (!text) return '';
    const html = marked.parse(text) as string;
    return DOMPurify.sanitize(html);
  }

  // Adds a small "Copy" button to the top-right of every <pre> block rendered
  // in the chat. Mode-agnostic — works for SSH commands, SQL queries, JSON, etc.
  // Re-runs on mutation so it covers blocks streamed in mid-message.
  function copyCodeBlockDecorator(node: HTMLElement) {
    const ICON_COPY = '<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>';
    const ICON_CHECK = '<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="5 13 10 18 19 7"></polyline></svg>';

    function decorate() {
      const blocks = node.querySelectorAll('pre');
      blocks.forEach((pre) => {
        if ((pre as HTMLElement).dataset.copyDecorated === '1') return;
        (pre as HTMLElement).dataset.copyDecorated = '1';
        const preEl = pre as HTMLElement;
        if (getComputedStyle(preEl).position === 'static') {
          preEl.style.position = 'relative';
        }

        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'ai-copy-btn';
        btn.innerHTML = ICON_COPY;
        btn.title = 'Copy';
        btn.setAttribute('aria-label', 'Copy code to clipboard');
        btn.addEventListener('click', async (e) => {
          e.stopPropagation();
          const code = pre.querySelector('code');
          const text = (code?.textContent ?? pre.textContent ?? '').replace(/\n$/, '');
          try {
            await navigator.clipboard.writeText(text);
            btn.innerHTML = ICON_CHECK;
            btn.classList.add('copied');
            setTimeout(() => {
              btn.innerHTML = ICON_COPY;
              btn.classList.remove('copied');
            }, COPY_FEEDBACK_MS);
          } catch { /* clipboard denied — silent */ }
        });
        preEl.appendChild(btn);
      });
    }

    decorate();
    const observer = new MutationObserver(() => decorate());
    observer.observe(node, { childList: true, subtree: true, characterData: true });
    return {
      destroy() { observer.disconnect(); },
    };
  }

  let messages = $state<AIMessage[]>([]);
  let inputText = $state('');
  let isStreaming = $state(false);
  let chatContainer: HTMLDivElement;
  let cleanup: (() => void) | null = null;
  let currentChatMode = $state('');

  // SSH confirmation modal state — driven by ai:tool_pending events from Rust.
  let sshModalShow = $state(false);
  let sshModalCommand = $state('');
  let sshModalReason = $state('');
  let sshModalTarget = $state('');
  let sshModalToolUseId = $state('');
  let sshModalDecision: ((approved: boolean) => void) | null = null;

  function openSshConfirmModal(command: string, reason: string, target: string, toolUseId: string): Promise<boolean> {
    sshModalCommand = command;
    sshModalReason = reason;
    sshModalTarget = target;
    sshModalToolUseId = toolUseId;
    sshModalShow = true;
    return new Promise<boolean>((resolve) => {
      sshModalDecision = resolve;
    });
  }

  function handleSshApprove() {
    sshModalShow = false;
    const r = sshModalDecision; sshModalDecision = null;
    r?.(true);
  }

  function handleSshCancel() {
    sshModalShow = false;
    const r = sshModalDecision; sshModalDecision = null;
    r?.(false);
  }

  // Per-chat-session tool_pending listener. Installed in sendMessage when SSH
  // mode is active; lives until that chat session's stream completes.
  let activeChatSessionId: string | null = null;

  // ── SSH auto-execute (power-user mode) ─────────────────────────────────────
  // Skips the confirmation modal — AI-invoked execute_shell calls run immediately.
  // Output still streams into the terminal so the user sees what happened.
  // Persisted in localStorage; first-time enable shows a one-time warning.
  let sshAutoRun = $state(false);
  let sshAutoRunWarnShow = $state(false);

  onMount(() => {
    sshAutoRun = getSshAutoRun();
  });

  function toggleSshAutoRun() {
    if (sshAutoRun) {
      // Turning off — silent
      sshAutoRun = false;
      setSshAutoRun(false);
    } else {
      // Turning on — confirm first so the user is unambiguous about what they're enabling
      sshAutoRunWarnShow = true;
    }
  }

  function confirmEnableAutoRun() {
    sshAutoRun = true;
    setSshAutoRun(true);
    sshAutoRunWarnShow = false;
  }

  async function handleSshToolPending(payload: { toolUseId: string; tool: string; command: string; reason: string }) {
    if (payload.tool !== 'execute_shell') return;
    const profile = get(activeSshProfile);
    if (!profile) {
      await invoke('ai_resolve_pending_tool', {
        toolUseId: payload.toolUseId,
        output: '[ERROR] No active SSH profile.',
      });
      return;
    }
    // Auto-run mode: skip the modal, execute immediately. User opted in via the
    // header toggle and saw the warning. Output still streams to the visible terminal.
    if (!sshAutoRun) {
      const target = `${profile.username}@${profile.host}`;
      const approved = await openSshConfirmModal(payload.command, payload.reason, target, payload.toolUseId);
      if (!approved) {
        try {
          await invoke('ai_resolve_pending_tool', {
            toolUseId: payload.toolUseId,
            output: '[USER CANCELLED] User declined to run this command.',
          });
        } catch (_) { /* swallow — chat may have ended */ }
        return;
      }
    }
    let captured = '';
    try {
      const raw = await executeAndCaptureOnSsh(profile.id, payload.command);
      captured = redactSecrets(raw);
    } catch (e) {
      captured = `[ERROR] ${e instanceof Error ? e.message : String(e)}`;
    }
    try {
      await invoke('ai_resolve_pending_tool', {
        toolUseId: payload.toolUseId,
        output: captured || '[INFO] Command produced no output.',
      });
    } catch (_) { /* swallow */ }
  }

  // Resizable panel width
  const MIN_WIDTH = 300;
  const MAX_WIDTH = 700;
  const DEFAULT_WIDTH = 380;
  let panelWidth = $state(getAiPanelWidth(DEFAULT_WIDTH));
  let isDragging = $state(false);

  function startDrag(e: MouseEvent) {
    e.preventDefault();
    isDragging = true;
    const startX = e.clientX;
    const startWidth = panelWidth;

    function onMove(e: MouseEvent) {
      // Panel is on the right, so dragging left increases width
      const delta = startX - e.clientX;
      panelWidth = Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, startWidth + delta));
    }

    function onUp() {
      isDragging = false;
      setAiPanelWidth(panelWidth);
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    }

    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }

  // Load persisted messages when mode changes
  $effect(() => {
    const m = $mode;
    if (m !== currentChatMode) {
      // Don't switch message arrays while streaming — would corrupt callback references
      if (isStreaming) return;
      // Save current mode's messages before switching
      if (currentChatMode && messages.length > 0) {
        setModeChatMessages(currentChatMode, messages.filter(m => !m.isStreaming));
      }
      currentChatMode = m;
      messages = getModeChatMessages(m);
      // Scroll to bottom after loading persisted messages
      requestAnimationFrame(() => scrollToBottom());
    }
  });

  // Scroll to bottom when panel opens
  $effect(() => {
    if ($aiPanelOpen && messages.length > 0) {
      requestAnimationFrame(() => scrollToBottom());
    }
  });

  // Persist messages after streaming completes
  $effect(() => {
    if (!isStreaming && currentChatMode && messages.length > 0) {
      setModeChatMessages(currentChatMode, messages);
    }
  });

  let hasApiKey = $derived(
    !!$settings[`ai_api_key_${$settings['ai_provider'] || 'claude'}`]?.trim()
  );

  const modeColors: Record<AppMode, string> = {
    rest: 'var(--acc)',
    sql: 'var(--acc)',
    nosql: 'var(--acc)',
    agent: 'var(--acc)',
    ssh: 'var(--ssh)',
    history: 'var(--t2)',
  };

  const modeLabels: Record<AppMode, string> = {
    rest: 'REST',
    sql: 'SQL',
    nosql: 'NoSQL',
    agent: 'Agent',
    ssh: 'SSH',
    history: 'History',
  };

  const modePlaceholders: Record<AppMode, string> = {
    rest: 'e.g. POST create user with email and role',
    sql: 'e.g. top 10 users by spend last 30 days',
    nosql: 'e.g. find pro users inactive 7 days',
    agent: 'Ask about your agent sessions',
    ssh: 'e.g. show disk usage on this server',
    history: 'Ask about your request history',
  };

  const modeGuideText: Record<AppMode, string> = {
    rest: 'Describe the API request you need — method, endpoint, headers, body — and I\'ll generate it for you.',
    sql: 'Describe the data you need and I\'ll write the SQL query. Works with your connected databases.',
    nosql: 'Describe what you\'re looking for and I\'ll generate the MongoDB query, filter, or aggregation pipeline.',
    agent: 'Agent mode has its own built-in AI assistance via Claude Code sessions.',
    ssh: 'Ask for shell commands to run on the connected server. Suggested commands appear as code blocks with an Insert button — destructive ones are blocked.',
    history: 'Ask about your request history and I\'ll help you find what you need.',
  };

  const toolLabels: Record<string, string> = {
    get_current_request: 'Reading current request...',
    get_current_response: 'Reading response...',
    list_collections: 'Browsing collections...',
    get_request_details: 'Reading request details...',
    list_environments: 'Checking environments...',
    get_history: 'Checking history...',
    apply_request: 'Preparing request...',
    execute_request: 'Executing request...',
    create_request: 'Creating request...',
    execute_current_request: 'Executing current request...',
    execute_collection: 'Running collection...',
    generate_curl: 'Generating cURL...',
    switch_environment: 'Switching environment...',
    rename_request: 'Renaming request...',
    delete_request: 'Deleting request...',
    duplicate_request: 'Duplicating request...',
    create_collection: 'Creating collection...',
    // SQL tools
    list_connections: 'Listing connections...',
    list_databases: 'Listing databases...',
    list_tables: 'Listing tables...',
    describe_table: 'Reading table schema...',
    execute_query: 'Executing query...',
    apply_query: 'Writing query to editor...',
    list_schemas: 'Listing schemas...',
    // NoSQL tools
    list_nosql_connections: 'Listing connections...',
    list_nosql_databases: 'Listing databases...',
    list_nosql_collections: 'Listing collections...',
    find_documents: 'Finding documents...',
    count_documents: 'Counting documents...',
    aggregate: 'Running aggregation...',
    apply_nosql_query: 'Writing query...',
    redis_list_keys: 'Listing Redis keys...',
    redis_execute: 'Executing Redis command...',
    sample_documents: 'Sampling documents...',
    insert_documents: 'Inserting documents...',
    get_collection_stats: 'Getting collection stats...',
    // REST extras
    set_env_variable: 'Setting environment variable...',
    search_history: 'Searching history...',
    // SQL extras
    get_schema: 'Loading database schema...',
    explain_query: 'Running EXPLAIN...',
    // (query_result handled as action card)
  };

  // Sync AI panel open state per mode
  $effect(() => {
    const currentMode = $mode;
    const perMode = $aiPanelOpenPerMode;
    aiPanelOpen.set(perMode[currentMode] ?? false);
  });

  onDestroy(() => {
    cleanup?.();
  });

  function close() {
    aiPanelOpen.set(false);
    aiPanelOpenPerMode.update(m => ({ ...m, [$mode]: false }));
  }

  function openAiSettings() {
    close();
    activeModal.set('settings:ai');
  }

  function cancelStreaming() {
    if (cleanup) {
      cleanup();
      cleanup = null;
    }
    // Mark the last message as complete
    const lastMsg = messages[messages.length - 1];
    if (lastMsg && lastMsg.isStreaming) {
      lastMsg.isStreaming = false;
      lastMsg.toolIndicator = undefined;
      if (!lastMsg.content) {
        lastMsg.content = '[Cancelled]';
      }
    }
    isStreaming = false;
  }

  function clearChat() {
    if (isStreaming) cancelStreaming();
    messages = [];
    clearModeChatMessages($mode);
  }

  function scrollToBottom() {
    if (chatContainer) {
      requestAnimationFrame(() => {
        chatContainer.scrollTop = chatContainer.scrollHeight;
      });
    }
  }

  async function gatherContext(): Promise<ChatContext> {
    const currentMode = get(mode);
    if (currentMode === 'sql') {
      const { gatherSqlContext } = await import('$lib/services/ai-context-sql');
      return gatherSqlContext();
    }
    if (currentMode === 'nosql') {
      const { gatherNosqlContext } = await import('$lib/modes/nosql/ai/context');
      return gatherNosqlContext();
    }
    const { gatherRestContext } = await import('$lib/services/ai-context-rest');
    return gatherRestContext();
  }

  async function sendMessage() {
    const text = inputText.trim();
    if (!text || isStreaming) return;

    const provider = $settings['ai_provider'] || 'claude';
    const apiKey = $settings[`ai_api_key_${provider}`] || '';

    // Add user message
    messages.push({
      role: 'user',
      content: text,
      timestamp: Date.now(),
    });

    // Add empty assistant message for streaming
    messages.push({
      role: 'assistant',
      content: '',
      isStreaming: true,
      timestamp: Date.now(),
    });

    inputText = '';
    isStreaming = true;
    scrollToBottom();

    // Gather context
    const context = await gatherContext();

    // Build chat history for API (only role + content, exclude current streaming msg)
    const chatHistory: ChatMessage[] = messages
      .filter(m => !m.isStreaming)
      .map(m => ({ role: m.role, content: m.content }));

    const sessionId = generateSessionId();
    activeChatSessionId = sessionId;
    const lastIdx = messages.length - 1;

    // Listen for SSH execute_shell tool_pending events for THIS chat session.
    // Cleaned up in onDestroy below + when a new chat starts (cleanup chain).
    let _toolPendingOff: UnlistenFn | null = null;
    if (currentChatMode === 'ssh' || get(mode) === 'ssh') {
      _toolPendingOff = await listen<{ toolUseId: string; tool: string; command: string; reason: string }>(
        aiEvent.toolPending(sessionId),
        (e) => { handleSshToolPending(e.payload); },
      );
    }

    const MODEL_MAP: Record<string, string> = {
      claude: 'claude-haiku-4-5-20251001',
      groq: 'meta-llama/llama-4-scout-17b-16e-instruct',
      mistral: 'mistral-large-latest',
      openai_gh: 'gpt-4.1-mini',
      nvidia: 'nvidia/nemotron-3-super-120b-a12b',
      openrouter: 'meta-llama/llama-3.3-70b-instruct:free',
      openai_direct: 'gpt-4.1-mini',
      gemini: 'gemini-2.5-flash',
    };
    const modelId = MODEL_MAP[provider] || 'claude-haiku-4-5-20251001';

    cleanup?.();
    const currentMode = get(mode);
    let systemPrompt: string;
    let tools: any[];
    if (currentMode === 'sql') {
      systemPrompt = SQL_SYSTEM_PROMPT;
      tools = SQL_TOOLS;
    } else if (currentMode === 'nosql') {
      systemPrompt = NOSQL_SYSTEM_PROMPT;
      tools = NOSQL_TOOLS;
    } else if (currentMode === 'ssh') {
      // SSH mode: prompt declares safety rules; execute_shell tool is bidirectional
      // (Rust blocks on user approval modal + captures terminal output before
      // returning the result). Output is redacted before reaching the model.
      const profile = get(activeSshProfile);
      systemPrompt = buildSshSystemPrompt(profile ? { username: profile.username, host: profile.host } : null);
      tools = SSH_TOOLS;
    } else {
      systemPrompt = REST_SYSTEM_PROMPT;
      tools = REST_TOOLS;
    }

    const streamCleanup = await sendChatMessage(
      apiKey,
      chatHistory,
      context,
      sessionId,
      systemPrompt,
      tools,
      {
        onText: (text) => {
          messages[lastIdx].content += text;
          scrollToBottom();
        },
        onToolStart: (toolName) => {
          messages[lastIdx].toolIndicator = toolLabels[toolName] || `Using ${toolName}...`;
          scrollToBottom();
        },
        onToolEnd: (_toolName) => {
          messages[lastIdx].toolIndicator = undefined;
        },
        onAction: (action, data) => {
          const existing = messages[lastIdx].actions ?? [];
          existing.push({ type: action as any, data });
          messages[lastIdx].actions = [...existing];
          // Refresh stores when data changes
          if (action === 'request_created' || action === 'request_modified' || action === 'collection_created') {
            loadCollections();
          }
          if (action === 'switch_environment') {
            import('$lib/stores/environments').then(({ setActiveEnv }) => {
              setActiveEnv(data.environmentId);
            });
          }
          // SQL: apply_query — write query to the active SQL editor via store
          if (action === 'apply_query' && data.query) {
            import('$lib/stores/sql').then(({ applyAiQuery }) => {
              if (typeof applyAiQuery === 'function') {
                applyAiQuery(data.query);
              }
            }).catch(() => { /* store may not export applyAiQuery yet */ });
          }
          // SQL: ai_execute_sql — switch to SQL mode, ensure tab, trigger execution
          if (action === 'ai_execute_sql' && data.query) {
            Promise.all([
              import('$lib/stores/sql'),
              import('$lib/stores/tabs'),
              import('$lib/stores/app'),
            ]).then(([{ triggerAiSqlExecution }, { addTab, tabs: tabStore, activeTabId: activeTabStore }, { mode: modeStore }]) => {
              // Switch to SQL mode
              modeStore.set('sql');
              // Ensure a SQL tab exists
              const currentTabs = get(tabStore);
              const sqlTab = currentTabs.find(t => t.mode === 'sql');
              if (!sqlTab) {
                addTab('AI Query', 'sql', null, 'var(--sql)');
              } else {
                activeTabStore.set(sqlTab.id);
              }
              // Trigger execution after a tick so SqlPanel mounts
              requestAnimationFrame(() => {
                triggerAiSqlExecution(data.query, data.connectionId, data.database);
              });
            });
          }
          // NoSQL: apply_nosql_query — write query to the active NoSQL editor via store
          if (action === 'apply_nosql_query' && data.query) {
            import('$lib/modes/nosql/stores').then(({ applyAiNoSqlQuery }) => {
              if (typeof applyAiNoSqlQuery === 'function') {
                applyAiNoSqlQuery(data.query);
              }
            }).catch(() => { /* store may not export applyAiNoSqlQuery yet */ });
          }
          // NoSQL: ai_execute_nosql — switch to NoSQL mode, ensure tab, trigger execution
          if (action === 'ai_execute_nosql' && data.filter) {
            Promise.all([
              import('$lib/modes/nosql/stores'),
              import('$lib/stores/tabs'),
              import('$lib/stores/app'),
            ]).then(([{ triggerAiNoSqlExecution }, { addTab, tabs: tabStore, activeTabId: activeTabStore }, { mode: modeStore }]) => {
              modeStore.set('nosql');
              const currentTabs = get(tabStore);
              const nosqlTab = currentTabs.find(t => t.mode === 'nosql');
              if (!nosqlTab) {
                addTab('AI Query', 'nosql', null, 'var(--nosql)');
              } else {
                activeTabStore.set(nosqlTab.id);
              }
              requestAnimationFrame(() => {
                triggerAiNoSqlExecution(data.filter, data.connectionId, data.database, data.collection);
              });
            });
          }
          scrollToBottom();
        },
        onDone: (_inputTokens, _outputTokens) => {
          messages[lastIdx].isStreaming = false;
          isStreaming = false;
          scrollToBottom();
        },
        onError: (error) => {
          const errLower = error.toLowerCase();
          if (errLower.includes('rate limit') || errLower.includes('429') || errLower.includes('too many')) {
            messages[lastIdx].error = { type: 'rate_limit', message: 'Rate limited. Wait a moment and try again.' };
          } else if (errLower.includes('invalid api key') || errLower.includes('401') || errLower.includes('unauthorized')) {
            messages[lastIdx].error = { type: 'auth', message: 'Invalid API key. Check your key in Settings.' };
          } else {
            messages[lastIdx].error = { type: 'generic', message: error };
          }
          messages[lastIdx].isStreaming = false;
          isStreaming = false;
          scrollToBottom();
        },
      },
      provider,
      modelId,
      currentMode,
    );

    // Combine the chat-stream cleanup with the per-session SSH listener cleanup.
    cleanup = () => {
      try { streamCleanup(); } catch (_) {}
      if (_toolPendingOff) { try { _toolPendingOff(); } catch (_) {} }
    };
  }

  function applyRequest(action: AIActionBlock) {
    if (action.applied) return;
    const data = action.data;
    const tabId = get(activeTabId);

    draftRequests.update(drafts => {
      const current = drafts.get(tabId) || {};
      const updated = { ...current };
      if (data.method) updated.method = data.method;
      if (data.url) updated.url = data.url;
      if (data.body) updated.body = data.body;
      if (data.body_type) updated.bodyType = data.body_type;
      if (data.headers) {
        updated.headers = data.headers.map((h: any) => ({
          key: h.key, value: h.value, enabled: 1,
        }));
      }
      if (data.params) {
        updated.params = data.params.map((p: any) => ({
          key: p.key, value: p.value, enabled: 1,
        }));
      }
      drafts.set(tabId, updated);
      return new Map(drafts);
    });

    action.applied = true;
    showToast('Request applied to active tab', 'success');
  }

  function retryLastMessage() {
    // Remove the failed assistant message
    const lastMsg = messages[messages.length - 1];
    if (lastMsg?.role === 'assistant') {
      messages.pop();
    }
    // Get the last user message content
    const lastUser = messages[messages.length - 1];
    if (lastUser?.role === 'user') {
      const text = lastUser.content;
      messages.pop();
      inputText = text;
      // Use tick to let state update, then send
      import('svelte').then(({ tick }) => tick()).then(() => sendMessage());
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      sendMessage();
    }
  }

  function handleInput(e: Event) {
    const target = e.target as HTMLTextAreaElement;
    target.style.height = 'auto';
    target.style.height = Math.min(target.scrollHeight, 80) + 'px';
  }
</script>

<aside
  class="ai-panel"
  class:open={$aiPanelOpen}
  class:dragging={isDragging}
  style:width={$aiPanelOpen ? panelWidth + 'px' : '0px'}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ai-drag-handle" onmousedown={startDrag}></div>
  <div class="ai-inner" style:width={panelWidth + 'px'}>
    <!-- Header -->
    <div class="ai-header">
      <div class="ai-header-left">
        <svg class="sparkle-icon" viewBox="0 0 24 24"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/></svg>
        <span class="ai-title">AI Assistant</span>
        <span class="mode-badge" style="background: {modeColors[$mode]}20; color: {modeColors[$mode]}; border-color: {modeColors[$mode]}40">
          {modeLabels[$mode]}
        </span>
      </div>
      <div class="ai-header-right">
        {#if $mode === 'ssh'}
          <button
            class="header-icon-btn ssh-auto-run-btn"
            class:active={sshAutoRun}
            title={sshAutoRun
              ? 'Auto-execute: ON — AI runs commands without asking. Click to disable.'
              : 'Auto-execute: OFF — AI commands require your approval. Click to enable.'}
            aria-label="Toggle AI auto-execute"
            onclick={toggleSshAutoRun}
          >
            <svg viewBox="0 0 24 24" width="13" height="13" fill={sshAutoRun ? 'currentColor' : 'none'} stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>
            </svg>
            {#if sshAutoRun}<span class="auto-run-dot" aria-hidden="true"></span>{/if}
          </button>
        {/if}
        {#if messages.length > 0}
          <button class="header-icon-btn" title="Clear chat" onclick={clearChat}>
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
          </button>
        {/if}
        <button class="close-btn" onclick={close}>&times;</button>
      </div>
    </div>

    {#if !hasApiKey}
      <div class="ai-chat">
        <div class="ai-welcome">
          <div class="welcome-icon">
            <svg viewBox="0 0 24 24"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/></svg>
          </div>
          <p class="welcome-text">Set up your API key to start using AI assistance</p>
          <button class="ai-setup-btn" onclick={openAiSettings}>Configure in Settings</button>
          <p class="welcome-hint">Toggle with <kbd>⌘L</kbd></p>
        </div>
      </div>
    {:else}
    <!-- Chat area -->
    <div class="ai-chat" bind:this={chatContainer}>
      {#if messages.length === 0}
        <div class="ai-welcome">
          <div class="welcome-icon">
            <svg viewBox="0 0 24 24"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/></svg>
          </div>
          <p class="welcome-text">{modeGuideText[$mode]}</p>
          <p class="welcome-hint">Toggle with <kbd>⌘L</kbd></p>
        </div>
      {/if}

      {#each messages as msg, i}
        {#if msg.role === 'user'}
          <div class="bubble user-bubble">{msg.content}</div>
        {:else}
          <div class="ai-response">
            {#if msg.content}
              <div class="ai-text" use:copyCodeBlockDecorator>{@html renderMarkdown(msg.content)}</div>
            {/if}

            {#if msg.toolIndicator}
              <div class="tool-indicator">
                <span class="tool-spinner"></span>
                <span class="tool-label">{msg.toolIndicator}</span>
              </div>
            {/if}

            {#if msg.actions}
              {#each msg.actions as action}
                {#if action.type === 'apply_request'}
                  <div class="action-card">
                    <div class="action-card-header">
                      <span class="method-tag" data-method={action.data.method}>{action.data.method}</span>
                      <span class="action-url">{action.data.url}</span>
                    </div>
                    {#if action.data.headers?.length}
                      <div class="action-detail-row">
                        <span class="action-detail-label">Headers</span>
                        <span class="action-detail-value">{action.data.headers.length} header{action.data.headers.length > 1 ? 's' : ''}</span>
                      </div>
                    {/if}
                    {#if action.data.body}
                      <div class="action-body-preview">
                        <pre>{action.data.body.length > 200 ? action.data.body.slice(0, 200) + '...' : action.data.body}</pre>
                      </div>
                    {/if}
                    <button
                      class="apply-btn"
                      class:applied={action.applied}
                      onclick={() => applyRequest(action)}
                      disabled={action.applied}
                    >
                      {#if action.applied}
                        <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
                        Applied
                      {:else}
                        <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 5v14M5 12h14"/></svg>
                        Apply to Tab
                      {/if}
                    </button>
                  </div>

                {:else if action.type === 'apply_query'}
                  <div class="action-card">
                    <div class="action-card-header">
                      <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="var(--acc)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
                      <span class="action-url">SQL Query</span>
                    </div>
                    {#if action.data.query}
                      <div class="action-body-preview">
                        <pre>{action.data.query.length > 300 ? action.data.query.slice(0, 300) + '...' : action.data.query}</pre>
                      </div>
                    {/if}
                  </div>

                {:else if action.type === 'apply_nosql_query'}
                  <div class="action-card">
                    <div class="action-card-header">
                      <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="var(--acc)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>
                      <span class="action-url">NoSQL Query</span>
                    </div>
                    {#if action.data.query}
                      <div class="action-body-preview">
                        <pre>{action.data.query.length > 300 ? action.data.query.slice(0, 300) + '...' : action.data.query}</pre>
                      </div>
                    {/if}
                  </div>

                {:else if action.type === 'execute_result'}
                  <div class="action-card exec-card">
                    <div class="action-card-header">
                      <span class="status-dot" class:ok={action.data.status < 400} class:err={action.data.status >= 400}></span>
                      <span class="exec-status-text">{action.data.status} {action.data.statusText}</span>
                      <span class="exec-meta">{action.data.durationMs}ms{action.data.sizeBytes ? ` · ${action.data.sizeBytes > 1024 ? (action.data.sizeBytes / 1024).toFixed(1) + ' KB' : action.data.sizeBytes + ' B'}` : ''}</span>
                      {#if action.data.body}
                        <button
                          class="copy-btn"
                          title="Copy response"
                          onclick={() => {
                            try { navigator.clipboard.writeText(JSON.stringify(JSON.parse(action.data.body), null, 2)); }
                            catch { navigator.clipboard.writeText(action.data.body); }
                            showToast('Copied to clipboard', 'success');
                          }}
                        >
                          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>
                        </button>
                      {/if}
                    </div>
                    {#if action.data.body}
                      <div class="exec-body">
                        {#if (() => { try { JSON.parse(action.data.body); return true; } catch { return false; } })()}
                          <pre><code>{@html highlightJSON(JSON.stringify(JSON.parse(action.data.body), null, 2))}</code></pre>
                        {:else}
                          <pre><code>{action.data.body}</code></pre>
                        {/if}
                      </div>
                    {/if}
                  </div>

                {:else if action.type === 'query_result' || action.type === 'ai_execute_sql'}
                  <div class="action-card exec-card">
                    <div class="action-card-header">
                      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="var(--acc)" stroke-width="1.8"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>
                      <span class="exec-status-text">{action.data.rowCount} row{action.data.rowCount !== 1 ? 's' : ''}</span>
                      <span class="exec-meta">{action.data.durationMs}ms</span>
                      {#if action.data.columns}
                        <span class="exec-meta exec-cols">{action.data.columns.join(', ')}</span>
                      {/if}
                      <span class="exec-route-label">results panel</span>
                    </div>
                  </div>

                {:else if action.type === 'ai_execute_nosql' || action.type === 'documents_result'}
                  <div class="action-card exec-card">
                    <div class="action-card-header">
                      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="var(--acc)" stroke-width="1.8"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
                      <span class="exec-status-text">{action.data.count} doc{action.data.count !== 1 ? 's' : ''}</span>
                      <span class="exec-meta">{action.data.collection}</span>
                      <span class="exec-route-label">document viewer</span>
                    </div>
                  </div>
                {:else if action.type === 'request_created'}
                  <div class="action-card created-card">
                    <div class="action-card-header">
                      <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="var(--ok)" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
                      <span class="method-tag" data-method={action.data.method}>{action.data.method}</span>
                      <span class="created-name">{action.data.name}</span>
                    </div>
                  </div>

                {:else if action.type === 'request_modified'}
                  <div class="action-card request-modified">
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="var(--acc)" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
                    <span class="action-label">{action.data.message || 'Request updated'}</span>
                  </div>
                {:else if action.type === 'collection_created'}
                  <div class="action-card collection-created">
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="var(--acc)" stroke-width="2"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>
                    <span class="action-label">{action.data.name || 'Collection created'}</span>
                  </div>
                {:else if action.type === 'collection_executed'}
                  <div class="action-card collection-report">
                    <div class="report-header">
                      <span class="report-title">Collection Report</span>
                      <span class="report-summary">
                        <span class="report-pass">{(action.data.results ?? []).filter((r: any) => r.status && r.status < 400).length} passed</span>
                        <span class="report-fail">{(action.data.results ?? []).length - (action.data.results ?? []).filter((r: any) => r.status && r.status < 400).length} failed</span>
                        <span class="report-total">{(action.data.results ?? []).length} total</span>
                      </span>
                    </div>
                    {#if action.data.results}
                      <div class="report-table">
                        {#each action.data.results as result}
                          <div class="report-row">
                            <span class="status-dot" class:ok={result.status && result.status < 400} class:err={result.error || (result.status && result.status >= 400)}></span>
                            <span class="method-tag sm" data-method={result.method}>{result.method}</span>
                            <span class="report-name">{result.name}</span>
                            {#if result.status}
                              <span class="report-status">{result.status}</span>
                              <span class="report-duration">{result.durationMs}ms</span>
                            {:else if result.error}
                              <span class="report-error">Error</span>
                            {/if}
                          </div>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/if}
              {/each}
            {/if}

            {#if msg.isStreaming && !msg.content && !msg.toolIndicator}
              <div class="typing-indicator">
                <span class="dot"></span>
                <span class="dot"></span>
                <span class="dot"></span>
              </div>
            {/if}

            {#if msg.error}
              <div class="ai-error-block" class:rate-limit={msg.error.type === 'rate_limit'} class:auth-error={msg.error.type === 'auth'}>
                <div class="ai-error-icon">
                  {#if msg.error.type === 'rate_limit'}
                    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
                  {:else if msg.error.type === 'auth'}
                    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0110 0v4"/></svg>
                  {:else}
                    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
                  {/if}
                </div>
                <div class="ai-error-body">
                  <span class="ai-error-msg">{msg.error.message}</span>
                  <div class="ai-error-actions">
                    {#if msg.error.type === 'rate_limit'}
                      <button class="ai-error-btn" onclick={retryLastMessage}>Retry</button>
                    {:else if msg.error.type === 'auth'}
                      <button class="ai-error-btn" onclick={openAiSettings}>Open Settings</button>
                    {:else}
                      <button class="ai-error-btn" onclick={retryLastMessage}>Retry</button>
                    {/if}
                  </div>
                </div>
              </div>
            {/if}
          </div>
        {/if}
      {/each}
    </div>

    <!-- Input area -->
    <div class="ai-input-area">
      <textarea
        class="ai-input"
        placeholder={modePlaceholders[$mode]}
        bind:value={inputText}
        onkeydown={handleKeydown}
        oninput={handleInput}
        rows="1"
      ></textarea>
      {#if isStreaming}
        <button
          class="send-btn stop-btn"
          onclick={cancelStreaming}
          title="Stop"
        >
          <svg viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="1" fill="currentColor" stroke="none"/></svg>
        </button>
      {:else}
        <button
          class="send-btn"
          style="background: {modeColors[$mode]}"
          onclick={sendMessage}
          disabled={!inputText.trim()}
          title="Send (Cmd+Enter)"
        >
          <svg viewBox="0 0 24 24"><path d="M22 2L11 13"/><path d="M22 2l-7 20-4-9-9-4z"/></svg>
        </button>
      {/if}
    </div>
    {/if}
  </div>
</aside>

<SshExecuteConfirmModal
  show={sshModalShow}
  command={sshModalCommand}
  reason={sshModalReason}
  target={sshModalTarget}
  onApprove={handleSshApprove}
  onCancel={handleSshCancel}
/>

{#if sshAutoRunWarnShow}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="auto-run-backdrop" onclick={() => (sshAutoRunWarnShow = false)}>
    <div class="auto-run-modal" role="dialog" aria-modal="true" aria-labelledby="auto-run-title" onclick={(e) => e.stopPropagation()}>
      <header class="auto-run-header">
        <span class="auto-run-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
            <line x1="12" y1="9" x2="12" y2="13"/>
            <line x1="12" y1="17" x2="12.01" y2="17"/>
          </svg>
        </span>
        <h3 id="auto-run-title">Enable auto-execute?</h3>
      </header>
      <div class="auto-run-body">
        <p>AI-suggested shell commands will run on the connected SSH server <strong>immediately, without asking you to approve each one</strong>.</p>
        <ul class="auto-run-list">
          <li>You'll still see every command in the terminal as it runs</li>
          <li>AI is told to refuse destructive ops, but mistakes can happen</li>
          <li>Use only on servers you trust to recover from a bad command</li>
          <li>You can turn this off any time from the same button</li>
        </ul>
      </div>
      <footer class="auto-run-footer">
        <button type="button" class="auto-run-btn auto-run-cancel" onclick={() => (sshAutoRunWarnShow = false)}>Cancel</button>
        <button type="button" class="auto-run-btn auto-run-enable" onclick={confirmEnableAutoRun}>Enable auto-execute</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  @import './AIPanel.svelte.css';
</style>

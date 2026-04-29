<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { settings, setSetting } from '$lib/stores/settings';
  import { activeModal } from '$lib/stores/app';
  import { get } from 'svelte/store';
  import { testAiKey } from '$lib/commands/ai';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  async function wcClose() { await getCurrentWindow().close(); }
  async function wcMinimize() { await getCurrentWindow().minimize(); }
  async function wcFullscreen() {
    const win = getCurrentWindow();
    await win.setFullscreen(!(await win.isFullscreen()));
  }

  let show = $state(false);
  let step = $state(0);
  let animating = $state(false);
  let mounted = $state(false);

  // AI setup
  let aiProvider = $state('claude');
  let aiApiKey = $state('');
  let showApiKey = $state(false);
  let aiTestStatus = $state<'idle' | 'testing' | 'success' | 'error'>('idle');
  let aiTestMessage = $state('');

  // GitHub OAuth
  let ghConnecting = $state(false);
  let ghConnected = $state(false);
  let ghUsername = $state('');
  let unlisten: (() => void) | null = null;

  const TOTAL_STEPS = 6;

  onMount(async () => {
    const s = get(settings);
    if (!s['onboarding_complete']) {
      show = true;
      setTimeout(() => { mounted = true; }, 50);
    }

    // Listen for OAuth deep link callback
    try {
      const { onOpenUrl } = await import('@tauri-apps/plugin-deep-link');
      unlisten = await onOpenUrl(async (urls) => {
        for (const url of urls) {
          if (url.includes('oauth-callback')) {
            const params = new URL(url).searchParams;
            const token = params.get('token');
            if (token) {
              ghConnecting = true;
              try {
                const { githubConnectWithToken, gistCheckExists } = await import('$lib/commands/github');
                const { setConnected, markSynced, showSyncRestorePrompt } = await import('$lib/stores/github');
                const { showToast: toast } = await import('$lib/components/shared/toast');
                const username = await githubConnectWithToken(token);
                setConnected(username);
                ghConnected = true;
                ghUsername = username;
                toast(`Connected as ${username}`, 'success');
                // Check if local is empty and cloud has data to restore
                const { collections } = await import('$lib/stores/collections');
                const { connections: sqlConns } = await import('$lib/stores/sql');
                const { nosqlConnections } = await import('$lib/modes/nosql/stores');
                const localEmpty = get(collections).length === 0
                  && get(sqlConns).length === 0
                  && get(nosqlConnections).length === 0;
                if (localEmpty) {
                  try {
                    const gistExists = await gistCheckExists();
                    if (gistExists) showSyncRestorePrompt.set(true);
                    else markSynced();
                  } catch { markSynced(); }
                } else {
                  markSynced();
                }
              } catch (e: any) {
                const { showToast } = await import('$lib/components/shared/toast');
                const { friendlyError } = await import('$lib/utils/errors');
                showToast(friendlyError(e), 'error');
              } finally {
                ghConnecting = false;
              }
            }
          }
        }
      });
    } catch {
      // Deep link not available in dev mode
    }
  });

  onDestroy(() => {
    unlisten?.();
  });

  $effect(() => {
    const s = $settings;
    if (s['onboarding_complete'] === 'true') {
      show = false;
    }
  });

  function next() {
    if (animating || step >= TOTAL_STEPS - 1) return;
    animating = true;
    step += 1;
    setTimeout(() => { animating = false; }, 320);
  }

  function goToStep(n: number) {
    if (animating || n === step) return;
    animating = true;
    step = n;
    setTimeout(() => { animating = false; }, 320);
  }

  async function saveAiKey() {
    if (aiApiKey.trim()) {
      await setSetting('ai_provider', aiProvider);
      await setSetting(`ai_api_key_${aiProvider}`, aiApiKey.trim());
      // Legacy compat for claude
      if (aiProvider === 'claude') {
        await setSetting('ai_api_key', aiApiKey.trim());
      }
    }
  }

  async function handleTestAiKeyOnboarding() {
    if (!aiApiKey.trim()) return;
    aiTestStatus = 'testing';
    aiTestMessage = '';
    try {
      const msg = await testAiKey(aiApiKey.trim(), aiProvider);
      aiTestStatus = 'success';
      aiTestMessage = msg;
    } catch (e: any) {
      aiTestStatus = 'error';
      aiTestMessage = typeof e === 'string' ? e : e.message || 'Test failed';
    }
  }

  async function finish() {
    await saveAiKey();
    await setSetting('onboarding_complete', 'true');
    show = false;
  }

  async function handleGitHubConnect() {
    ghConnecting = true;
    try {
      const { githubGetOauthUrl } = await import('$lib/commands/github');
      const url = await githubGetOauthUrl();
      try {
        const { openUrl } = await import('@tauri-apps/plugin-opener');
        await openUrl(url);
      } catch {
        window.open(url, '_blank');
      }
      // Wait for deep link callback — ghConnecting stays true
    } catch (e: any) {
      ghConnecting = false;
      const { showToast } = await import('$lib/components/shared/toast');
      const { friendlyError } = await import('$lib/utils/errors');
      showToast(friendlyError(e), 'error');
    }
  }
</script>

{#if show}
<div class="onboarding-overlay" class:visible={mounted}>
  <!-- Custom traffic-light controls (window has decorations:false) -->
  <div class="onb-wc-area" data-drag-region>
    <div class="onb-wc-dots">
      <button class="onb-wc-dot onb-wc-close" onclick={wcClose} aria-label="Close"></button>
      <button class="onb-wc-dot onb-wc-min" onclick={wcMinimize} aria-label="Minimize"></button>
      <button class="onb-wc-dot onb-wc-max" onclick={wcFullscreen} aria-label="Fullscreen"></button>
    </div>
  </div>
  <div class="onboarding-container">
    <div class="steps-viewport">
      <div class="steps-track" style="transform: translateX({-step * 100}%); transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);">

        <!-- Step 1: Welcome -->
        <div class="step">
          <div class="step-content welcome-step">
            <div class="bear-icon">
              <img src="/clauge-icon-animated.svg" alt="Clauge" width="140" height="140" />
            </div>
            <h1 class="ob-title">Welcome to Clauge</h1>
            <p class="ob-subtitle">Your developer super-app. Four powerful modes in one place — Agent, REST, SQL, and NoSQL — backed by AI to supercharge your workflow.</p>
            <button class="ob-btn primary" onclick={next}>Get Started</button>
          </div>
        </div>

        <!-- Step 2: Agent (headline feature) -->
        <div class="step">
          <div class="step-content">
            <div class="step-icon">
              <svg viewBox="0 0 24 24" width="44" height="44"><path d="M12 2a2 2 0 012 2c0 .74-.4 1.39-1 1.73V7h1a7 7 0 017 7v1h1a1 1 0 110 2h-1v1a7 7 0 01-7 7h-1v.27c.6.34 1 .99 1 1.73a2 2 0 11-4 0c0-.74.4-1.39 1-1.73V18h-1a7 7 0 01-7-7v-1H3a1 1 0 110-2h1v-1a7 7 0 017-7h1V5.73c-.6-.34-1-.99-1-1.73a2 2 0 012-2z" stroke="var(--acc)" fill="none" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/><circle cx="9" cy="13" r="1.5" fill="var(--acc)"/><circle cx="15" cy="13" r="1.5" fill="var(--acc)"/></svg>
            </div>
            <h2 class="ob-title sm">Agent Mode</h2>
            <p class="ob-subtitle">The star of Clauge. Spin up AI-powered sessions for coding, debugging, and automation.</p>
            <div class="agent-features">
              <div class="agent-feature-item" style="animation-delay: 0.05s;">
                <div class="agent-feature-bullet"></div>
                <span><strong>Sessions</strong> — Each task gets its own isolated workspace with full context</span>
              </div>
              <div class="agent-feature-item" style="animation-delay: 0.1s;">
                <div class="agent-feature-bullet"></div>
                <span><strong>Purposes</strong> — Code, Debug, Review, Refactor, and more built-in workflows</span>
              </div>
              <div class="agent-feature-item" style="animation-delay: 0.15s;">
                <div class="agent-feature-bullet"></div>
                <span><strong>Worktree Isolation</strong> — Each session runs in its own git worktree, so nothing conflicts</span>
              </div>
            </div>
            <p class="ob-hint">You can pick a project directory when you start your first session.</p>
            <button class="ob-btn primary" onclick={next}>Next</button>
          </div>
        </div>

        <!-- Step 3: API Tools (informational) -->
        <div class="step">
          <div class="step-content">
            <h2 class="ob-title sm">API & Database Tools</h2>
            <p class="ob-subtitle">Everything you need for backend development, all built in.</p>
            <div class="feature-cards">
              <div class="feature-card" style="animation-delay: 0.05s;">
                <div class="feature-icon" style="background: color-mix(in srgb, var(--rest) 12%, transparent);">
                  <svg viewBox="0 0 24 24" width="22" height="22"><path d="M10 13a5 5 0 007.54.54l3-3a5 5 0 00-7.07-7.07l-1.72 1.71" stroke="var(--rest)" fill="none" stroke-width="1.8" stroke-linecap="round"/><path d="M14 11a5 5 0 00-7.54-.54l-3 3a5 5 0 007.07 7.07l1.71-1.71" stroke="var(--rest)" fill="none" stroke-width="1.8" stroke-linecap="round"/></svg>
                </div>
                <h3 class="feature-title">REST Client</h3>
                <p class="feature-desc">Build, test, and organize APIs with collections, environments, and cURL import.</p>
              </div>
              <div class="feature-card" style="animation-delay: 0.1s;">
                <div class="feature-icon" style="background: color-mix(in srgb, var(--sql) 12%, transparent);">
                  <svg viewBox="0 0 24 24" width="22" height="22"><ellipse cx="12" cy="5" rx="9" ry="3" stroke="var(--sql)" fill="none" stroke-width="1.8"/><path d="M3 5v6c0 1.66 4.03 3 9 3s9-1.34 9-3V5" stroke="var(--sql)" fill="none" stroke-width="1.8"/><path d="M3 11v6c0 1.66 4.03 3 9 3s9-1.34 9-3v-6" stroke="var(--sql)" fill="none" stroke-width="1.8"/></svg>
                </div>
                <h3 class="feature-title">SQL Client</h3>
                <p class="feature-desc">PostgreSQL, MySQL, SQLite with query editor, autocomplete, and schema explorer.</p>
              </div>
              <div class="feature-card" style="animation-delay: 0.15s;">
                <div class="feature-icon" style="background: color-mix(in srgb, var(--nosql) 12%, transparent);">
                  <svg viewBox="0 0 24 24" width="22" height="22"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" stroke="var(--nosql)" fill="none" stroke-width="1.8" stroke-linecap="round"/><path d="M14 2v6h6M16 13H8M16 17H8M10 9H8" stroke="var(--nosql)" fill="none" stroke-width="1.8" stroke-linecap="round"/></svg>
                </div>
                <h3 class="feature-title">NoSQL Client</h3>
                <p class="feature-desc">MongoDB and Redis support. Browse, query, and edit documents inline.</p>
              </div>
            </div>
            <p class="ob-hint">No setup required — just start using them from the sidebar.</p>
            <button class="ob-btn primary" onclick={next}>Next</button>
          </div>
        </div>

        <!-- Step 4: AI Setup -->
        <div class="step">
          <div class="step-content">
            <h2 class="ob-title sm">AI Setup</h2>
            <p class="ob-subtitle">Connect an AI provider to power the Agent and inline AI features across all modes.</p>

            <div class="ai-setup-section">
              <span class="ai-setup-label">Connect your AI provider</span>
              <div class="ai-setup-row">
                <select class="ob-select ai-select" bind:value={aiProvider}>
                  <option value="claude">Claude</option>
                  <option value="groq">Groq</option>
                  <option value="mistral">Mistral AI</option>
                  <option value="openai_gh">OpenAI (GitHub)</option>
                  <option value="nvidia">NVIDIA NIM</option>
                  <option value="openrouter">OpenRouter</option>
                  <option value="openai_direct">OpenAI</option>
                  <option value="gemini">Google Gemini</option>
                </select>
                <div class="key-input-wrap ai-key-wrap">
                  <input
                    class="ob-input"
                    type={showApiKey ? 'text' : 'password'}
                    placeholder="Paste API key..."
                    bind:value={aiApiKey}
                  />
                  <button class="key-toggle" onclick={() => showApiKey = !showApiKey} type="button">
                    {#if showApiKey}
                      <svg viewBox="0 0 24 24" width="14" height="14"><path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24" stroke="currentColor" fill="none" stroke-width="1.8" stroke-linecap="round"/><line x1="1" y1="1" x2="23" y2="23" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>
                    {:else}
                      <svg viewBox="0 0 24 24" width="14" height="14"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" stroke="currentColor" fill="none" stroke-width="1.8"/><circle cx="12" cy="12" r="3" stroke="currentColor" fill="none" stroke-width="1.8"/></svg>
                    {/if}
                  </button>
                </div>
                <button
                  class="ob-btn ai-test-btn-ob"
                  onclick={handleTestAiKeyOnboarding}
                  disabled={!aiApiKey.trim() || aiTestStatus === 'testing'}
                  type="button"
                >
                  {#if aiTestStatus === 'testing'}
                    Testing...
                  {:else}
                    Test
                  {/if}
                </button>
              </div>
              {#if aiTestStatus === 'success'}
                <span class="ai-test-result-ob success">
                  <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
                  {aiTestMessage}
                </span>
              {:else if aiTestStatus === 'error'}
                <span class="ai-test-result-ob error">
                  <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>
                  {aiTestMessage}
                </span>
              {/if}
              <a class="ai-console-link-ob" href="https://console.anthropic.com" target="_blank" rel="noopener">
                Get your API key
                <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
              </a>
            </div>

            <div class="ob-actions">
              {#if aiApiKey.trim()}
                <button class="ob-btn primary" onclick={() => { saveAiKey(); next(); }}>Save & Continue</button>
              {:else}
                <button class="ob-btn secondary" onclick={next}>Skip for Now</button>
              {/if}
              <p class="ob-hint">You can always set this up later in Settings</p>
            </div>
          </div>
        </div>

        <!-- Step 5: GitHub Sync (OAuth) -->
        <div class="step">
          <div class="step-content">
            <div class="step-icon">
              <svg viewBox="0 0 24 24" width="40" height="40"><path d="M12 2C6.477 2 2 6.477 2 12c0 4.42 2.865 8.166 6.839 9.489.5.092.682-.217.682-.482 0-.237-.009-.866-.013-1.7-2.782.604-3.369-1.34-3.369-1.34-.454-1.156-1.11-1.464-1.11-1.464-.908-.62.069-.608.069-.608 1.003.07 1.531 1.03 1.531 1.03.892 1.529 2.341 1.087 2.91.831.092-.646.35-1.086.636-1.337-2.22-.253-4.555-1.11-4.555-4.943 0-1.091.39-1.984 1.029-2.683-.103-.253-.446-1.27.098-2.647 0 0 .84-.269 2.75 1.025A9.578 9.578 0 0112 6.836a9.59 9.59 0 012.504.337c1.909-1.294 2.747-1.025 2.747-1.025.546 1.377.203 2.394.1 2.647.64.699 1.028 1.592 1.028 2.683 0 3.842-2.339 4.687-4.566 4.935.359.309.678.919.678 1.852 0 1.336-.012 2.415-.012 2.743 0 .267.18.578.688.48C19.138 20.161 22 16.416 22 12c0-5.523-4.477-10-10-10z" fill="var(--t2)"/></svg>
            </div>
            <h2 class="ob-title sm">Sync across devices</h2>
            <p class="ob-subtitle">Connect your GitHub account to sync collections, environments, and connections via private Gists.</p>

            {#if ghConnected}
              <div class="gh-connected">
                <svg viewBox="0 0 24 24" width="18" height="18"><circle cx="12" cy="12" r="10" stroke="var(--ok)" fill="none" stroke-width="1.5"/><path d="M8 12l3 3 5-5" stroke="var(--ok)" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
                <span>Connected as <strong>{ghUsername}</strong></span>
              </div>
            {:else if ghConnecting}
              <div class="gh-waiting">
                <span class="gh-spinner"></span>
                <p class="ob-hint">Waiting for GitHub authorization... Complete it in your browser.</p>
                <button class="ob-btn secondary" style="padding: 0 16px; height: 32px; font-size: 12px;" onclick={() => ghConnecting = false}>Cancel</button>
              </div>
            {:else}
              <button class="ob-btn github" onclick={handleGitHubConnect}>
                <svg viewBox="0 0 24 24" width="16" height="16"><path d="M12 2C6.477 2 2 6.477 2 12c0 4.42 2.865 8.166 6.839 9.489.5.092.682-.217.682-.482 0-.237-.009-.866-.013-1.7-2.782.604-3.369-1.34-3.369-1.34-.454-1.156-1.11-1.464-1.11-1.464-.908-.62.069-.608.069-.608 1.003.07 1.531 1.03 1.531 1.03.892 1.529 2.341 1.087 2.91.831.092-.646.35-1.086.636-1.337-2.22-.253-4.555-1.11-4.555-4.943 0-1.091.39-1.984 1.029-2.683-.103-.253-.446-1.27.098-2.647 0 0 .84-.269 2.75 1.025A9.578 9.578 0 0112 6.836a9.59 9.59 0 012.504.337c1.909-1.294 2.747-1.025 2.747-1.025.546 1.377.203 2.394.1 2.647.64.699 1.028 1.592 1.028 2.683 0 3.842-2.339 4.687-4.566 4.935.359.309.678.919.678 1.852 0 1.336-.012 2.415-.012 2.743 0 .267.18.578.688.48C19.138 20.161 22 16.416 22 12c0-5.523-4.477-10-10-10z" fill="currentColor"/></svg>
                Connect with GitHub
              </button>
            {/if}

            <div class="ob-actions" style="margin-top: 4px;">
              <button class="ob-btn secondary" onclick={next}>
                {ghConnected ? 'Continue' : 'Skip for Now'}
              </button>
              <p class="ob-hint">You can connect GitHub anytime from the sidebar</p>
            </div>
          </div>
        </div>

        <!-- Step 6: Shortcuts -->
        <div class="step">
          <div class="step-content">
            <div class="step-icon check-icon">
              <svg viewBox="0 0 24 24" width="44" height="44"><circle cx="12" cy="12" r="10" stroke="var(--ok)" fill="none" stroke-width="1.5"/><path d="M8 12l3 3 5-5" stroke="var(--ok)" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </div>
            <h2 class="ob-title sm">You're all set!</h2>
            <div class="shortcuts-card">
              <div class="sc-row"><span class="sc-keys"><kbd>&#8984;</kbd><kbd>1</kbd></span><span class="sc-label">Agent</span></div>
              <div class="sc-row"><span class="sc-keys"><kbd>&#8984;</kbd><kbd>2</kbd></span><span class="sc-label">REST</span></div>
              <div class="sc-row"><span class="sc-keys"><kbd>&#8984;</kbd><kbd>3</kbd></span><span class="sc-label">SQL</span></div>
              <div class="sc-row"><span class="sc-keys"><kbd>&#8984;</kbd><kbd>4</kbd></span><span class="sc-label">NoSQL</span></div>
              <div class="sc-row"><span class="sc-keys"><kbd>&#8984;</kbd><kbd>L</kbd></span><span class="sc-label">AI Assistant</span></div>
              <div class="sc-row"><span class="sc-keys"><kbd>&#8984;</kbd><kbd>&#8679;</kbd><kbd>L</kbd></span><span class="sc-label">Shell</span></div>
              <div class="sc-row"><span class="sc-keys"><kbd>&#8984;</kbd><kbd>S</kbd></span><span class="sc-label">Save</span></div>
            </div>
            <button class="ob-btn primary" onclick={finish}>Start Building</button>
          </div>
        </div>

      </div>
    </div>

    <!-- Step indicators -->
    <div class="step-dots">
      {#each Array(TOTAL_STEPS) as _, i}
        <button
          class="dot"
          class:active={i === step}
          class:done={i < step}
          onclick={() => goToStep(i)}
          aria-label="Go to step {i + 1}"
        ></button>
      {/each}
    </div>
  </div>
</div>
{/if}

<style>
  .onboarding-overlay {
    position: fixed; inset: 0; z-index: 9999;
    background: rgba(4, 4, 12, 0.92);
    backdrop-filter: blur(24px); -webkit-backdrop-filter: blur(24px);
    display: flex; align-items: center; justify-content: center;
    opacity: 0; transition: opacity 0.4s ease;
  }
  .onboarding-overlay.visible { opacity: 1; }
  /* Custom traffic-light controls — top-left, mirrors sidebar's wc-area */
  .onb-wc-area {
    position: absolute; top: 0; left: 0;
    width: 72px; height: 46px;
    display: flex; align-items: center; justify-content: center;
    z-index: 1;
  }
  .onb-wc-dots { display: flex; align-items: center; gap: 8px; }
  .onb-wc-dot {
    width: 12px; height: 12px; border-radius: 50%;
    border: none; cursor: default; padding: 0;
    transition: filter 0.1s;
  }
  .onb-wc-dot:hover { filter: brightness(0.85); }
  .onb-wc-close { background: #ff5f57; }
  .onb-wc-min { background: #febc2e; }
  .onb-wc-max { background: #28c840; }
  :global(body.window-blurred) .onb-wc-dot { background: var(--t4) !important; }
  .onboarding-container {
    width: 100%; max-width: 640px;
    display: flex; flex-direction: column; align-items: center; gap: 32px;
  }
  .steps-viewport { width: 100%; overflow: hidden; border-radius: 16px; }
  .steps-track { display: flex; width: 100%; }
  .step {
    min-width: 100%; display: flex;
    align-items: center; justify-content: center; padding: 20px;
  }
  .step-content {
    display: flex; flex-direction: column; align-items: center;
    text-align: center; gap: 20px; max-width: 520px; width: 100%;
  }
  .welcome-step { gap: 24px; }
  .bear-icon { opacity: 0; animation: fadeInScale 0.5s ease 0.2s forwards; }
  @keyframes fadeInScale { from { opacity: 0; transform: scale(0.85); } to { opacity: 1; transform: scale(1); } }
  .step-icon { margin-bottom: 4px; opacity: 0.9; }
  .check-icon { animation: fadeInScale 0.4s ease forwards; }
  .ob-title { font-size: 28px; font-weight: 600; color: var(--t1); font-family: var(--ui); letter-spacing: -0.02em; }
  .ob-title.sm { font-size: 18px; }
  .ob-subtitle { font-size: 14px; color: var(--t2); line-height: 1.6; max-width: 400px; }

  .ob-btn {
    height: 40px; padding: 0 28px; border-radius: 8px; border: none;
    font-size: 12px; font-weight: 500; font-family: var(--ui);
    cursor: pointer; transition: background 0.15s, transform 0.1s;
    display: flex; align-items: center; gap: 8px;
  }
  .ob-btn:active { transform: scale(0.97); }
  .ob-btn.primary { background: var(--acc); color: #fff; }
  .ob-btn.primary:hover { background: color-mix(in srgb, var(--acc) 85%, black); }
  .ob-btn.github { background: #24292e; color: #fff; border: 1px solid #444; }
  .ob-btn.github:hover { background: #2f363d; }
  .ob-btn.secondary { background: var(--b1); color: var(--t1); border: 1px solid var(--b2); }
  .ob-btn.secondary:hover { background: var(--b2); }

  .ob-hint { font-size: 12px; color: var(--t3); font-family: var(--ui); line-height: 1.4; }
  .ob-hint kbd {
    font-family: var(--mono); background: var(--n2); padding: 1px 5px;
    border-radius: 3px; font-size: 11px; color: var(--t2);
  }
  .gh-connected {
    display: flex; align-items: center; gap: 8px;
    padding: 12px 20px; background: rgba(29,200,128,0.08);
    border: 1px solid rgba(29,200,128,0.2); border-radius: 10px;
    font-size: 13px; color: var(--t1);
  }
  .gh-connected strong { color: var(--ok); }
  .gh-waiting {
    display: flex; flex-direction: column; align-items: center; gap: 10px;
  }
  .gh-spinner {
    width: 24px; height: 24px; border: 3px solid var(--b1);
    border-top-color: var(--acc); border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .ob-actions { display: flex; flex-direction: column; align-items: center; gap: 12px; }

  /* Feature cards */
  .feature-cards { display: grid; grid-template-columns: repeat(3, 1fr); gap: 14px; width: 100%; }
  .feature-card {
    background: rgba(13,13,24,0.8); border: 1px solid var(--b1);
    border-radius: 12px; padding: 20px 16px;
    display: flex; flex-direction: column; align-items: center;
    text-align: center; gap: 10px; position: relative;
    opacity: 0; animation: cardIn 0.35s ease forwards;
  }
  @keyframes cardIn { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; } }
  .feature-icon {
    width: 44px; height: 44px; border-radius: 10px;
    display: flex; align-items: center; justify-content: center;
  }
  .feature-title { font-size: 13px; font-weight: 600; color: var(--t1); }
  .feature-desc { font-size: 11.5px; color: var(--t2); line-height: 1.5; }

  /* Agent features */
  .agent-features {
    display: flex; flex-direction: column; gap: 12px;
    width: 100%; max-width: 420px; text-align: left;
  }
  .agent-feature-item {
    display: flex; align-items: flex-start; gap: 10px;
    font-size: 13px; color: var(--t2); line-height: 1.5;
    opacity: 0; animation: cardIn 0.35s ease forwards;
  }
  .agent-feature-item strong { color: var(--t1); font-weight: 600; }
  .agent-feature-bullet {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--acc); flex-shrink: 0; margin-top: 7px;
  }

  /* AI setup */
  .ai-setup-section {
    width: 100%; max-width: 420px; text-align: left;
  }
  .ai-setup-label {
    display: block; font-size: 11px; font-weight: 500;
    color: var(--t3); margin-bottom: 8px; text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .ai-setup-row {
    display: flex; gap: 8px; align-items: center;
  }
  .ai-select {
    width: 120px !important; flex-shrink: 0;
  }
  .ai-key-wrap {
    flex: 1;
  }
  .ob-label { font-size: 11px; font-weight: 500; color: var(--t2); margin-top: 4px; }
  .ob-select, .ob-input {
    height: 36px; padding: 0 12px; border-radius: 8px;
    border: 1px solid var(--b1); background: var(--e);
    color: var(--t1); font-size: 12.5px; font-family: var(--ui);
    outline: none; transition: border-color 0.15s; width: 100%;
  }
  .ob-select:focus, .ob-input:focus { border-color: var(--acc); }
  .key-input-wrap { position: relative; }
  .key-input-wrap .ob-input { padding-right: 40px; }
  .key-toggle {
    position: absolute; right: 4px; top: 50%; transform: translateY(-50%);
    width: 30px; height: 30px; border: none; background: transparent;
    color: var(--t3); cursor: pointer; display: flex;
    align-items: center; justify-content: center; border-radius: 6px;
  }
  .key-toggle:hover { color: var(--t1); }

  /* Shortcuts */
  .shortcuts-card {
    background: rgba(13,13,24,0.8); border: 1px solid var(--b1);
    border-radius: 12px; padding: 16px 24px;
    display: flex; flex-direction: column; gap: 10px;
    width: 100%; max-width: 340px;
  }
  .sc-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .sc-keys { display: flex; gap: 4px; }
  .sc-keys kbd {
    display: inline-block; padding: 2px 7px; border-radius: 4px;
    border: 1px solid var(--b1); font-size: 11px; font-family: var(--mono);
    color: var(--t2); background: var(--n); min-width: 22px; text-align: center;
  }
  .sc-label { font-size: 12px; color: var(--t2); }

  /* Dots */
  .step-dots { display: flex; gap: 8px; }
  .dot {
    width: 8px; height: 8px; border-radius: 50%; border: none;
    background: var(--b1); cursor: pointer;
    transition: background 0.2s, transform 0.2s; padding: 0;
  }
  .dot.active { background: var(--acc); transform: scale(1.25); }
  .dot.done { background: var(--t3); }
  .dot:hover:not(.active) { background: var(--t3); }

  .ai-test-btn-ob {
    padding: 6px 14px !important;
    font-size: 12px !important;
    white-space: nowrap;
    min-width: auto !important;
    background: var(--b1, #1a1a2e) !important;
    color: var(--t1, #e0e0e0) !important;
    border: 1px solid var(--e, #333) !important;
  }
  .ai-test-btn-ob:hover:not(:disabled) {
    border-color: var(--acc) !important;
    color: var(--acc) !important;
  }
  .ai-test-btn-ob:disabled { opacity: 0.5; }
  .ai-test-result-ob {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    margin-top: 6px;
  }
  .ai-test-result-ob.success { color: var(--ok, #22c55e); }
  .ai-test-result-ob.error { color: var(--err, #ef4444); }
  .ai-console-link-ob {
    font-size: 12px;
    color: var(--acc);
    text-decoration: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 8px;
  }
  .ai-console-link-ob:hover { text-decoration: underline; }
</style>

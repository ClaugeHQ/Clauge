<script lang="ts">
  import Modal from '$lib/components/shared/Modal.svelte';
  import { activeModal } from '$lib/stores/app';
  import { settings, setSetting, appearance, saveAppearance } from '$lib/stores/settings';
  import { applyTheme, getThemes, getTheme } from '$lib/utils/theme';
  import { showToast } from '$lib/components/shared/toast';
  import type { AppearanceConfig } from '$lib/types';
  import { testAiKey, getAiUsageStats, getAiProviderStats, resetAiUsage } from '$lib/commands/ai';
  import type { AiUsageStat, AiProviderStat } from '$lib/types/ai';
  import {
    agentGetPlugins,
    agentTogglePlugin,
    agentUninstallPlugin,
    agentGetMarketplacePlugins,
    agentInstallPlugin,
    agentListContexts,
    agentSaveContext,
    agentDeleteContext,
    agentFetchUsageLimits,
    agentGetUsageAnalytics,
  } from '$lib/commands/agent';
  import type { ClaudePlugin, MarketplacePlugin, AgentContext, UsageAnalytics } from '$lib/types/agent';
  import { agentUsageLimits } from '$lib/stores/agent';

  type SettingsTab = 'general' | 'appearance' | 'ai' | 'agent' | 'proxy' | 'shortcuts' | 'about';

  let show = $state(false);
  let activeTab = $state<SettingsTab>('general');

  $effect(() => {
    const modal = $activeModal;
    if (modal === 'settings' || modal === 'settings:ai' || modal === 'settings:agent' || modal === 'settings:agent:usage') {
      show = true;
      if (modal === 'settings:ai') activeTab = 'ai';
      if (modal === 'settings:agent') activeTab = 'agent';
      if (modal === 'settings:agent:usage') { activeTab = 'agent'; agentSubTab = 'usage'; }
    } else {
      show = false;
    }
  });

  $effect(() => {
    if (!show && ($activeModal === 'settings' || $activeModal === 'settings:ai' || $activeModal === 'settings:agent' || $activeModal === 'settings:agent:usage')) {
      activeModal.set(null);
    }
  });

  // --- General ---
  let timeout = $derived(Number($settings['request_timeout'] ?? '30000'));
  let followRedirects = $derived(($settings['follow_redirects'] ?? 'true') === 'true');
  let sslVerification = $derived(($settings['ssl_verification'] ?? 'true') === 'true');
  let maxResponseSize = $derived(Number($settings['max_response_size'] ?? '10'));

  // --- Editor ---

  // --- Proxy ---
  let proxyUrl = $derived($settings['proxy_url'] ?? '');
  let proxyAuth = $derived(($settings['proxy_auth'] ?? 'false') === 'true');
  let proxyUsername = $derived($settings['proxy_username'] ?? '');
  let proxyPassword = $derived($settings['proxy_password'] ?? '');

  // AI Assistance state
  let aiSubTab = $state<'config' | 'usage'>('config');
  let aiProvider = $state<string>('claude');
  let aiApiKey = $state('');
  let showAiKey = $state(false);
  let aiTestStatus = $state<'idle' | 'testing' | 'success' | 'error'>('idle');
  let aiTestMessage = $state('');
  let aiUsageStats = $state<AiUsageStat[]>([]);
  let aiProviderStats = $state<AiProviderStat[]>([]);
  let showResetConfirm = $state(false);

  const AI_PROVIDERS: Record<string, { name: string; model: string; modelId: string; keyPrefix: string; keyPlaceholder: string; keyUrl: string }> = {
    claude: { name: 'Claude (Anthropic)', model: 'Haiku 4.5', modelId: 'claude-haiku-4-5-20251001', keyPrefix: 'sk-ant-', keyPlaceholder: 'sk-ant-api03-...', keyUrl: 'https://console.anthropic.com' },
    groq: { name: 'Groq', model: 'Llama 4 Scout 17B', modelId: 'meta-llama/llama-4-scout-17b-16e-instruct', keyPrefix: 'gsk_', keyPlaceholder: 'gsk_...', keyUrl: 'https://console.groq.com/keys' },
    mistral: { name: 'Mistral AI', model: 'Mistral Large 3', modelId: 'mistral-large-latest', keyPrefix: '', keyPlaceholder: 'API key...', keyUrl: 'https://console.mistral.ai/api-keys' },
    openai_gh: { name: 'OpenAI (GitHub)', model: 'GPT-4.1 Mini', modelId: 'gpt-4.1-mini', keyPrefix: '', keyPlaceholder: 'GitHub token...', keyUrl: 'https://github.com/marketplace/models' },
    nvidia: { name: 'NVIDIA NIM', model: 'Nemotron 3 Super 120B', modelId: 'nvidia/nemotron-3-super-120b-a12b', keyPrefix: '', keyPlaceholder: 'API key...', keyUrl: 'https://build.nvidia.com' },
    openrouter: { name: 'OpenRouter', model: 'Llama 3.3 70B', modelId: 'meta-llama/llama-3.3-70b-instruct:free', keyPrefix: '', keyPlaceholder: 'sk-or-...', keyUrl: 'https://openrouter.ai/keys' },
    openai_direct: { name: 'OpenAI', model: 'GPT-4.1 Mini', modelId: 'gpt-4.1-mini', keyPrefix: 'sk-', keyPlaceholder: 'sk-...', keyUrl: 'https://platform.openai.com/api-keys' },
    gemini: { name: 'Google Gemini', model: 'Gemini 2.5 Flash', modelId: 'gemini-2.5-flash', keyPrefix: '', keyPlaceholder: 'API key...', keyUrl: 'https://aistudio.google.com/apikey' },
  };

  let currentProviderConfig = $derived(AI_PROVIDERS[aiProvider]);
  let aiHasKey = $derived(!!$settings[`ai_api_key_${aiProvider}`]?.trim());

  // --- Appearance ---
  let currentTheme = $derived($appearance.theme || 'dark-glass');
  let accentColor = $derived($appearance.accentColor || '#7c5cf8');

  const ACCENT_COLORS = [
    { name: 'Purple', value: '#7c5cf8' },
    { name: 'Blue', value: '#4f94d4' },
    { name: 'Green', value: '#1dc880' },
    { name: 'Orange', value: '#f06830' },
    { name: 'Red', value: '#f04444' },
    { name: 'Pink', value: '#f472b6' },
    { name: 'Cyan', value: '#22d3ee' },
    { name: 'White', value: '#e0e0e0' },
  ];

  const THEME_DESCRIPTIONS: Record<string, string> = {
    'dark-glass': 'Transparent with macOS vibrancy',
    'dark-solid': 'Opaque dark with purple tints',
    'midnight': 'Pure black, zero distraction',
    'nord': 'Arctic blue-gray palette',
    'light': 'Warm off-white, easy on the eyes',
  };

  const THEME_PREVIEW_COLORS: Record<string, string[]> = {
    'dark-glass': ['rgba(7,7,15,0.55)', 'rgba(13,13,24,0.72)', 'rgba(19,19,32,0.82)'],
    'dark-solid': ['#0a0a14', '#0f0f1a', '#16162a'],
    'midnight': ['#000000', '#080808', '#0a0a0a'],
    'nord': ['#2e3440', '#3b4252', '#353d4b'],
    'light': ['#f0f0ec', '#f5f5f2', '#fafaf8'],
  };


  // Clear any previously saved zoom to prevent cursor issues
  $effect(() => {
    document.body.style.zoom = '';
  });

  const tabs: { key: SettingsTab; label: string; icon: string }[] = [
    { key: 'general', label: 'General', icon: 'M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z' },
    { key: 'appearance', label: 'Appearance', icon: 'M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z' },
    { key: 'ai', label: 'AI Assistance', icon: 'M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z' },
    { key: 'agent', label: 'Agent', icon: 'M12 2a4 4 0 0 0-4 4v2H6a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V10a2 2 0 0 0-2-2h-2V6a4 4 0 0 0-4-4zm0 2a2 2 0 0 1 2 2v2h-4V6a2 2 0 0 1 2-2zm-1 10v2h2v-2h2v-2h-2v-2h-2v2H9v2h2z' },
    { key: 'proxy', label: 'Proxy', icon: 'M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z' },
    { key: 'shortcuts', label: 'Shortcuts', icon: 'M18 3a3 3 0 00-3 3v12a3 3 0 003 3 3 3 0 003-3 3 3 0 00-3-3H6a3 3 0 00-3 3 3 3 0 003 3 3 3 0 003-3V6a3 3 0 00-3-3 3 3 0 00-3 3 3 3 0 003 3h12a3 3 0 003-3 3 3 0 00-3-3z' },
    { key: 'about', label: 'About', icon: 'M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10z' },
  ];

  const SHORTCUTS = [
    { desc: 'Send request', keys: ['Cmd', 'Enter'] },
    { desc: 'REST mode', keys: ['Cmd', '1'] },
    { desc: 'SQL mode', keys: ['Cmd', '2'] },
    { desc: 'NoSQL mode', keys: ['Cmd', '3'] },
    { desc: 'Toggle nav', keys: ['Cmd', 'B'] },
    { desc: 'Toggle AI', keys: ['Cmd', 'L'] },
    { desc: 'Show shortcuts', keys: ['Cmd', '/'] },
    { desc: 'Close modals', keys: ['Esc'] },
  ];

  async function handleSettingChange(key: string, value: string) {
    await setSetting(key, value);
    // If session key changed, update store and fetch limits immediately
    if (key === 'agent_session_key') {
      import('$lib/stores/agent').then(({ agentSessionKey, loadAgentUsageLimits }) => {
        agentSessionKey.set(value);
        if (value) loadAgentUsageLimits();
      });
    }
  }

  async function handleThemeChange(themeId: string) {
    applyTheme(themeId, accentColor);

    const config: AppearanceConfig = {
      theme: themeId,
      accentColor: accentColor,
    };
    appearance.set(config);
    await saveAppearance(config);
  }

  async function handleAccentChange(color: string) {
    document.documentElement.style.setProperty('--acc', color);
    const config: AppearanceConfig = {
      theme: currentTheme,
      accentColor: color,
    };
    appearance.set(config);
    await saveAppearance(config);
  }

  function handleClose() {
    activeTab = 'general';
  }

  async function loadAiSettings() {
    const s = $settings;
    aiProvider = s['ai_provider'] || 'claude';
    // Load key for current provider (fallback to legacy ai_api_key for claude)
    aiApiKey = s[`ai_api_key_${aiProvider}`] || '';
    try {
      aiUsageStats = await getAiUsageStats();
      aiProviderStats = await getAiProviderStats();
    } catch {
      aiUsageStats = [];
      aiProviderStats = [];
    }
  }

  async function handleSaveAiKey() {
    const key = aiApiKey.trim();
    if (!key) {
      showToast('Enter an API key first', 'error');
      return;
    }
    aiTestStatus = 'testing';
    aiTestMessage = '';
    try {
      const msg = await testAiKey(key, aiProvider);
      aiTestStatus = 'success';
      aiTestMessage = msg;
      await handleSettingChange(`ai_api_key_${aiProvider}`, key);
      await handleSettingChange('ai_provider', aiProvider);
      // Also save to legacy key for backward compat
      if (aiProvider === 'claude') {
        // Legacy key no longer used — per-provider keys only
      }
      showToast('API key verified and saved', 'success');
    } catch (e: any) {
      aiTestStatus = 'error';
      aiTestMessage = typeof e === 'string' ? e : e.message || 'Test failed';
      showToast('Invalid API key — not saved', 'error');
    }
  }

  async function handleProviderChange(provider: string) {
    aiProvider = provider;
    await handleSettingChange('ai_provider', provider);
    // Load the key for this provider
    const s = $settings;
    aiApiKey = s[`ai_api_key_${provider}`] || '';
    aiTestStatus = 'idle';
    aiTestMessage = '';
  }

  async function handleRemoveAiKey() {
    await handleSettingChange(`ai_api_key_${aiProvider}`, '');
    if (aiProvider === 'claude') {
      // Legacy key no longer used
    }
    aiApiKey = '';
    aiTestStatus = 'idle';
    aiTestMessage = '';
    showToast('API key removed', 'success');
  }

  async function handleResetUsage() {
    try {
      await resetAiUsage();
      aiUsageStats = [];
      showResetConfirm = false;
      showToast('Usage stats reset', 'success');
    } catch {
      showToast('Failed to reset stats', 'error');
    }
  }

  function formatTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'K';
    return n.toString();
  }

  function estimateCost(inputTokens: number, outputTokens: number): string {
    const cost = (inputTokens / 1_000_000) * 1.0 + (outputTokens / 1_000_000) * 5.0;
    if (cost < 0.01) return '<$0.01';
    return '$' + cost.toFixed(2);
  }

  function formatModelName(model: string): string {
    const names: Record<string, string> = {
      'claude-haiku-4-5-20251001': 'Haiku 4.5',
      'claude-sonnet-4-6-20250514': 'Sonnet 4.6',
      'claude-opus-4-7-20250514': 'Opus 4.7',
      'llama-3.3-70b-versatile': 'Llama 3.3 70B',
      'meta-llama/llama-4-scout-17b-16e-instruct': 'Llama 4 Scout 17B',
      'mistral-small-latest': 'Mistral Small',
      'mistral-large-latest': 'Mistral Large 3',
      'gpt-4.1-mini': 'GPT-4.1 Mini',
      'nvidia/nemotron-3-super-120b-a12b': 'Nemotron 3 Super 120B',
      'meta-llama/llama-3.3-70b-instruct:free': 'Llama 3.3 70B',
      'gemini-2.5-flash': 'Gemini 2.5 Flash',
      'qwen/qwen3-32b': 'Qwen3 32B',
    };
    return names[model] || model;
  }

  let aiSettingsLoaded = false;
  $effect(() => {
    if (activeTab === 'ai' && show && !aiSettingsLoaded) {
      aiSettingsLoaded = true;
      loadAiSettings();
    }
    if (!show) {
      aiSettingsLoaded = false;
    }
  });

  // --- Agent Tab ---
  type AgentSubTab = 'general' | 'plugins' | 'contexts' | 'usage';
  let agentSubTab = $state<AgentSubTab>('general');

  // Agent General
  let agentSessionKey = $derived($settings['agent_session_key'] ?? '');
  let agentSoundEnabled = $derived(($settings['agent_sound_enabled'] ?? 'true') === 'true');
  let agentDockBounceEnabled = $derived(($settings['agent_dock_bounce_enabled'] ?? 'true') === 'true');
  let agentRefreshMins = $derived(Number($settings['agent_refresh_mins'] ?? '5'));

  // Agent key management
  let agentKeyInput = $state('');
  let showAgentKey = $state(false);
  let agentKeyTestStatus = $state<'idle' | 'testing' | 'success' | 'error'>('idle');
  let agentKeyTestMessage = $state('');

  // Agent provider
  type AgentProvider = 'claude_code' | 'codex' | 'gemini_cli';
  let agentProvider = $state<AgentProvider>('claude_code');

  const AGENT_PROVIDERS: Record<string, { name: string; icon: string; description: string; available: boolean }> = {
    claude_code: { name: 'Claude Code', icon: 'M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z', description: 'Anthropic CLI agent', available: true },
    codex: { name: 'Codex', icon: 'M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5', description: 'OpenAI CLI agent', available: false },
    gemini_cli: { name: 'Gemini CLI', icon: 'M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z', description: 'Google CLI agent', available: false },
  };

  // Agent Usage Stats
  let agentUsageData = $state<UsageAnalytics | null>(null);
  let agentUsageLoading = $state(false);
  let agentUsageDays = $state(7);

  const REFRESH_OPTIONS = [
    { value: 5, label: '5 minutes' },
    { value: 15, label: '15 minutes' },
    { value: 30, label: '30 minutes' },
    { value: 60, label: '1 hour' },
  ];

  function agentLiveColor(pct: number): string {
    if (pct > 80) return '#f85149';
    if (pct > 50) return '#d29922';
    return 'var(--acc)';
  }

  function agentFormatCost(n: number) { return n < 0.01 && n > 0 ? '<$0.01' : '$' + n.toFixed(2); }
  function agentFormatTokens(n: number) { return n >= 1_000_000 ? (n / 1_000_000).toFixed(1) + 'M' : n >= 1_000 ? (n / 1_000).toFixed(1) + 'K' : String(n); }
  function agentDecodeName(s: string) { return s.replace(/-/g, '/'); }

  async function handleSaveAgentKey() {
    const key = agentKeyInput.trim();
    if (!key) {
      showToast('Enter a session key first', 'error');
      return;
    }
    agentKeyTestStatus = 'testing';
    agentKeyTestMessage = '';
    try {
      await agentFetchUsageLimits(key);
      agentKeyTestStatus = 'success';
      agentKeyTestMessage = 'Session key verified';
      await handleSettingChange('agent_session_key', key);
      showToast('Session key verified and saved', 'success');
    } catch (e: any) {
      agentKeyTestStatus = 'error';
      agentKeyTestMessage = typeof e === 'string' ? e : e.message || 'Invalid or expired session key';
      showToast('Invalid session key — not saved', 'error');
    }
  }

  async function handleRemoveAgentKey() {
    await handleSettingChange('agent_session_key', '');
    agentKeyInput = '';
    agentKeyTestStatus = 'idle';
    agentKeyTestMessage = '';
    showToast('Session key removed', 'success');
  }

  async function loadAgentUsage() {
    agentUsageLoading = true;
    try {
      agentUsageData = await agentGetUsageAnalytics(agentUsageDays);
    } catch { agentUsageData = null; }
    agentUsageLoading = false;
  }

  function selectAgentUsageDays(d: number) { agentUsageDays = d; loadAgentUsage(); }

  // Agent Plugins
  type PluginView = 'installed' | 'marketplace';
  let pluginView = $state<PluginView>('installed');
  let installedPlugins = $state<ClaudePlugin[]>([]);
  let marketplacePlugins = $state<MarketplacePlugin[]>([]);
  let pluginSearchQuery = $state('');
  let filteredMarketplacePlugins = $derived(
    pluginSearchQuery.trim()
      ? marketplacePlugins.filter(p =>
          p.name.toLowerCase().includes(pluginSearchQuery.toLowerCase()) ||
          p.description.toLowerCase().includes(pluginSearchQuery.toLowerCase())
        )
      : marketplacePlugins
  );

  // Agent Contexts
  let agentContexts = $state<AgentContext[]>([]);
  let editingContext = $state<AgentContext | null>(null);
  let editContextName = $state('');
  let editContextContent = $state('');
  let isNewContext = $state(false);
  let deleteConfirmId = $state<string | null>(null);

  async function loadAgentPlugins() {
    try {
      installedPlugins = await agentGetPlugins();
    } catch { installedPlugins = []; }
    try {
      marketplacePlugins = await agentGetMarketplacePlugins();
    } catch { marketplacePlugins = []; }
  }

  async function loadAgentContexts() {
    try {
      agentContexts = await agentListContexts();
    } catch { agentContexts = []; }
  }

  async function handleTogglePlugin(name: string, enabled: boolean) {
    try {
      await agentTogglePlugin(name, enabled);
      installedPlugins = installedPlugins.map(p => p.name === name ? { ...p, enabled } : p);
      showToast(`Plugin ${enabled ? 'enabled' : 'disabled'}`, 'success');
    } catch { showToast('Failed to toggle plugin', 'error'); }
  }

  async function handleUninstallPlugin(name: string, marketplace: string) {
    try {
      await agentUninstallPlugin(name, marketplace);
      installedPlugins = installedPlugins.filter(p => p.name !== name);
      marketplacePlugins = marketplacePlugins.map(p => p.name === name ? { ...p, installed: false } : p);
      showToast('Plugin uninstalled', 'success');
    } catch { showToast('Failed to uninstall plugin', 'error'); }
  }

  async function handleInstallPlugin(name: string, marketplace: string) {
    try {
      await agentInstallPlugin(name, marketplace);
      marketplacePlugins = marketplacePlugins.map(p => p.name === name ? { ...p, installed: true } : p);
      await loadAgentPlugins();
      showToast('Plugin installed', 'success');
    } catch { showToast('Failed to install plugin', 'error'); }
  }

  function startEditContext(ctx: AgentContext) {
    editingContext = ctx;
    editContextName = ctx.name;
    editContextContent = ctx.content;
    isNewContext = false;
  }

  function startNewContext() {
    editingContext = null;
    editContextName = '';
    editContextContent = '';
    isNewContext = true;
  }

  function cancelEditContext() {
    editingContext = null;
    isNewContext = false;
    editContextName = '';
    editContextContent = '';
  }

  async function handleSaveAgentContext() {
    const name = editContextName.trim();
    const content = editContextContent.trim();
    if (!name || !content) {
      showToast('Name and content are required', 'error');
      return;
    }
    try {
      await agentSaveContext({ id: editingContext?.id, name, content });
      await loadAgentContexts();
      cancelEditContext();
      showToast('Context saved', 'success');
    } catch { showToast('Failed to save context', 'error'); }
  }

  async function handleDeleteAgentContext(id: string) {
    try {
      await agentDeleteContext(id);
      agentContexts = agentContexts.filter(c => c.id !== id);
      deleteConfirmId = null;
      if (editingContext?.id === id) cancelEditContext();
      showToast('Context deleted', 'success');
    } catch { showToast('Failed to delete context', 'error'); }
  }

  let agentSettingsLoaded = false;
  $effect(() => {
    if (activeTab === 'agent' && show && !agentSettingsLoaded) {
      agentSettingsLoaded = true;
      loadAgentPlugins();
      loadAgentContexts();
      agentKeyInput = $settings['agent_session_key'] ?? '';
    }
    if (!show) {
      agentSettingsLoaded = false;
      agentSubTab = 'general';
      cancelEditContext();
      agentKeyTestStatus = 'idle';
      agentKeyTestMessage = '';
      showAgentKey = false;
    }
  });

  let agentUsageLoaded = false;
  $effect(() => {
    if (agentSubTab === 'usage' && show && !agentUsageLoaded) {
      agentUsageLoaded = true;
      loadAgentUsage();
    }
    if (agentSubTab !== 'usage') {
      agentUsageLoaded = false;
    }
  });
</script>

<Modal bind:show title="Settings" width={activeTab === 'agent' && agentSubTab === 'usage' ? '960px' : '780px'} onclose={handleClose}>
  <div class="stg-layout">
    <!-- Tab sidebar -->
    <div class="stg-tabs">
      {#each tabs as tab}
        <button
          class="stg-tab"
          class:active={activeTab === tab.key}
          onclick={() => activeTab = tab.key}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d={tab.icon} />
          </svg>
          {tab.label}
        </button>
      {/each}
    </div>

    <!-- Content pane -->
    <div class="stg-content">
      {#if activeTab === 'general'}
        <div class="stg-section">
          <span class="stg-section-label">Request Settings</span>

          <div class="stg-field">
            <label class="stg-label">Request Timeout (ms)</label>
            <input
              class="stg-input"
              type="number"
              value={timeout}
              min="1000"
              step="1000"
              onchange={(e) => handleSettingChange('request_timeout', e.currentTarget.value)}
            />
          </div>

          <div class="stg-field">
            <label class="stg-label">Follow Redirects</label>
            <label class="stg-toggle">
              <input type="checkbox" checked={followRedirects}
                onchange={(e) => handleSettingChange('follow_redirects', String(e.currentTarget.checked))} />
              <span class="stg-toggle-slider"></span>
            </label>
          </div>

          <div class="stg-field">
            <label class="stg-label">SSL Verification</label>
            <label class="stg-toggle">
              <input type="checkbox" checked={sslVerification}
                onchange={(e) => handleSettingChange('ssl_verification', String(e.currentTarget.checked))} />
              <span class="stg-toggle-slider"></span>
            </label>
          </div>

          <div class="stg-field">
            <label class="stg-label">Max Response Size (MB)</label>
            <input
              class="stg-input"
              type="number"
              value={maxResponseSize}
              min="1"
              max="100"
              onchange={(e) => handleSettingChange('max_response_size', e.currentTarget.value)}
            />
          </div>
        </div>

      {:else if activeTab === 'appearance'}
        <div class="stg-section">
          <span class="stg-section-label">Theme</span>
          <div class="theme-grid">
            {#each getThemes() as theme}
              <button
                class="theme-card"
                class:active={currentTheme === theme.id}
                onclick={() => handleThemeChange(theme.id)}
              >
                <div class="theme-preview">
                  {#each THEME_PREVIEW_COLORS[theme.id] || [] as color, i}
                    <div class="theme-preview-bar" style="background:{color}; opacity:{0.6 + i * 0.2}"></div>
                  {/each}
                </div>
                <div class="theme-info">
                  <span class="theme-name">{theme.name}</span>
                  <span class="theme-desc">{THEME_DESCRIPTIONS[theme.id] || ''}</span>
                </div>
              </button>
            {/each}
          </div>
        </div>

        <div class="stg-section">
          <span class="stg-section-label">Accent Color</span>
          <div class="stg-swatches">
            {#each ACCENT_COLORS as color}
              <button
                class="stg-swatch"
                class:active={accentColor === color.value}
                style="background: {color.value}"
                title={color.name}
                onclick={() => handleAccentChange(color.value)}
              ></button>
            {/each}
          </div>
        </div>

      {:else if activeTab === 'ai'}
        <!-- AI sub-tabs -->
        <div class="ai-subtabs">
          <button class="ai-subtab" class:active={aiSubTab === 'config'} onclick={() => aiSubTab = 'config'}>
            Configuration
          </button>
          <button
            class="ai-subtab"
            class:active={aiSubTab === 'usage'}
            class:disabled={!aiHasKey}
            onclick={() => { if (aiHasKey) aiSubTab = 'usage'; }}
          >
            Usage Stats
            {#if !aiHasKey}
              <svg class="ai-subtab-lock" viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0110 0v4"/></svg>
            {/if}
          </button>
        </div>

        {#if aiSubTab === 'config'}
          <!-- Config card -->
          <div class="ai-cfg">
            <!-- Provider & Model header -->
            <div class="ai-cfg-row">
              <div class="ai-cfg-field">
                <label class="ai-cfg-label">Provider</label>
                <select class="ai-cfg-select" value={aiProvider} onchange={(e) => handleProviderChange(e.currentTarget.value)}>
                  <option value="claude">Claude (Anthropic)</option>
                  <option value="groq">Groq</option>
                  <option value="mistral">Mistral AI</option>
                  <option value="openai_gh">OpenAI (GitHub)</option>
                  <option value="nvidia">NVIDIA NIM</option>
                  <option value="openrouter">OpenRouter</option>
                  <option value="openai_direct">OpenAI</option>
                  <option value="gemini">Google Gemini</option>
                </select>
              </div>
              <div class="ai-cfg-field">
                <label class="ai-cfg-label">Model</label>
                <span class="ai-model-tag">{currentProviderConfig.model}</span>
              </div>
            </div>

            <!-- Divider -->
            <div class="ai-cfg-divider"></div>

            <!-- API Key -->
            <div class="ai-cfg-section">
              <label class="ai-cfg-label">API Key</label>
              <div class="ai-key-input-wrap">
                <input
                  class="ai-cfg-input"
                  type={showAiKey ? 'text' : 'password'}
                  placeholder={currentProviderConfig.keyPlaceholder}
                  bind:value={aiApiKey}
                />
                <button class="ai-key-toggle" onclick={() => showAiKey = !showAiKey} type="button">
                  {#if showAiKey}
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
                  {:else}
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
                  {/if}
                </button>
              </div>

              {#if aiTestStatus === 'success'}
                <span class="ai-test-result success">
                  <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
                  {aiTestMessage}
                </span>
              {:else if aiTestStatus === 'error'}
                <span class="ai-test-result error">
                  <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>
                  {aiTestMessage}
                </span>
              {/if}

              <div class="ai-key-actions">
                <button
                  class="ai-action-btn primary"
                  onclick={() => handleSaveAiKey()}
                  disabled={!aiApiKey.trim() || aiTestStatus === 'testing'}
                >
                  {#if aiTestStatus === 'testing'}
                    Verifying...
                  {:else}
                    Save & Verify
                  {/if}
                </button>
                {#if aiHasKey}
                  <button class="ai-action-btn danger" onclick={handleRemoveAiKey}>Remove Key</button>
                {/if}
              </div>
            </div>

            <!-- Divider -->
            <div class="ai-cfg-divider"></div>

            <!-- Footer: links + status -->
            <div class="ai-cfg-footer">
              <div class="ai-cfg-links">
                <a class="ai-link" href={currentProviderConfig.keyUrl} target="_blank" rel="noopener">
                  <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M15 3h6v6"/><path d="M10 14L21 3"/><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/></svg>
                  Get API Key
                </a>
                <span class="ai-link-sep">&middot;</span>
                <a class="ai-link" href="https://github.com/mnfst/awesome-free-llm-apis" target="_blank" rel="noopener">
                  <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M15 3h6v6"/><path d="M10 14L21 3"/><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/></svg>
                  Free LLM APIs
                </a>
              </div>
              {#if aiHasKey}
                <span class="ai-status-badge">
                  <span class="ai-status-dot"></span>
                  Connected
                </span>
              {/if}
            </div>
          </div>

        {:else if aiSubTab === 'usage'}
          {#if aiUsageStats.length === 0}
            <div class="ai-usage-empty">
              <svg viewBox="0 0 24 24" width="36" height="36" fill="none" stroke="var(--t4)" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 20V10M12 20V4M6 20v-6"/></svg>
              <p>No API calls recorded yet</p>
              <span>Start chatting with AI to see usage data here</span>
            </div>
          {:else}
            <div class="stg-section">
              <div class="ai-summary-header">
                <span class="stg-section-label">Overview</span>
                {#if showResetConfirm}
                  <div class="ai-reset-confirm">
                    <span>Reset all data?</span>
                    <button class="ai-action-btn danger sm" onclick={handleResetUsage}>Reset</button>
                    <button class="ai-action-btn sm" onclick={() => showResetConfirm = false}>Cancel</button>
                  </div>
                {:else}
                  <button class="ai-action-btn sm" onclick={() => showResetConfirm = true}>
                    <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.8"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
                    Reset
                  </button>
                {/if}
              </div>
              <div class="ai-summary-grid">
                <div class="ai-summary-card">
                  <span class="ai-summary-val">{aiUsageStats.reduce((s, v) => s + v.totalCalls, 0)}</span>
                  <span class="ai-summary-lbl">Total Calls</span>
                </div>
                <div class="ai-summary-card">
                  <span class="ai-summary-val">{formatTokens(aiUsageStats.reduce((s, v) => s + v.inputTokens, 0))}</span>
                  <span class="ai-summary-lbl">Input Tokens</span>
                </div>
                <div class="ai-summary-card">
                  <span class="ai-summary-val">{formatTokens(aiUsageStats.reduce((s, v) => s + v.outputTokens, 0))}</span>
                  <span class="ai-summary-lbl">Output Tokens</span>
                </div>
                <div class="ai-summary-card accent">
                  <span class="ai-summary-val">{estimateCost(aiUsageStats.reduce((s, v) => s + v.inputTokens, 0), aiUsageStats.reduce((s, v) => s + v.outputTokens, 0))}</span>
                  <span class="ai-summary-lbl">Est. Cost</span>
                </div>
              </div>
            </div>

            <div class="stg-section">
              <span class="stg-section-label">By Mode</span>
              <div class="ai-table">
                <div class="ai-table-head">
                  <span>Mode</span>
                  <span>Calls</span>
                  <span>Input</span>
                  <span>Output</span>
                  <span>Cost</span>
                </div>
                {#each aiUsageStats as stat}
                  <div class="ai-table-row">
                    <span class="ai-table-label">
                      <span class="ai-dot" data-mode={stat.mode}></span>
                      {stat.mode.toUpperCase()}
                    </span>
                    <span class="ai-table-val">{stat.totalCalls}</span>
                    <span class="ai-table-val">{formatTokens(stat.inputTokens)}</span>
                    <span class="ai-table-val">{formatTokens(stat.outputTokens)}</span>
                    <span class="ai-table-val accent">{estimateCost(stat.inputTokens, stat.outputTokens)}</span>
                  </div>
                {/each}
              </div>
            </div>

            {#if aiProviderStats.length > 0}
              <div class="stg-section">
                <span class="stg-section-label">By Provider</span>
                <div class="ai-table">
                  <div class="ai-table-head">
                    <span>Model</span>
                    <span>Calls</span>
                    <span>Input</span>
                    <span>Output</span>
                    <span>Cost</span>
                  </div>
                  {#each aiProviderStats as pstat}
                    <div class="ai-table-row">
                      <span class="ai-table-label">
                        <span class="ai-dot" style="background: var(--acc)"></span>
                        {formatModelName(pstat.model)}
                      </span>
                      <span class="ai-table-val">{pstat.totalCalls}</span>
                      <span class="ai-table-val">{formatTokens(pstat.inputTokens)}</span>
                      <span class="ai-table-val">{formatTokens(pstat.outputTokens)}</span>
                      <span class="ai-table-val accent">{estimateCost(pstat.inputTokens, pstat.outputTokens)}</span>
                    </div>
                  {/each}
                </div>
                <p class="ai-pricing-note">Haiku 4.5: $1.00 / MTok in &middot; $5.00 / MTok out</p>
              </div>
            {/if}
          {/if}
        {/if}

      {:else if activeTab === 'agent'}
        <!-- Agent sub-tabs -->
        <div class="ai-subtabs">
          <button class="ai-subtab" class:active={agentSubTab === 'general'} onclick={() => agentSubTab = 'general'}>
            General
          </button>
          <button class="ai-subtab" class:active={agentSubTab === 'plugins'} onclick={() => agentSubTab = 'plugins'}>
            Plugins
          </button>
          <button class="ai-subtab" class:active={agentSubTab === 'contexts'} onclick={() => agentSubTab = 'contexts'}>
            Contexts
          </button>
          <button
            class="ai-subtab"
            class:active={agentSubTab === 'usage'}
            class:disabled={!agentSessionKey}
            onclick={() => { if (agentSessionKey) agentSubTab = 'usage'; }}
          >
            Usage Stats
            {#if !agentSessionKey}
              <svg class="ai-subtab-lock" viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0110 0v4"/></svg>
            {/if}
          </button>
        </div>

        {#if agentSubTab === 'general'}
          <!-- Agent Provider -->
          <div class="stg-section">
            <span class="stg-section-label">Agent Provider</span>
            <div class="agent-provider-grid">
              {#each Object.entries(AGENT_PROVIDERS) as [key, provider]}
                <button
                  class="agent-provider-card"
                  class:active={agentProvider === key}
                  class:disabled={!provider.available}
                  onclick={() => { if (provider.available) agentProvider = key as AgentProvider; }}
                >
                  <div class="agent-provider-icon">
                    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d={provider.icon} /></svg>
                  </div>
                  <div class="agent-provider-info">
                    <span class="agent-provider-name">{provider.name}</span>
                    <span class="agent-provider-desc">{provider.description}</span>
                  </div>
                  {#if provider.available}
                    {#if agentProvider === key}
                      <span class="agent-provider-badge active">
                        <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg>
                        Active
                      </span>
                    {/if}
                  {:else}
                    <span class="agent-provider-badge soon">Soon</span>
                  {/if}
                </button>
              {/each}
            </div>
          </div>

          <!-- Session Key -->
          <div class="stg-section">
            <span class="stg-section-label">Session Key</span>
            <div class="ai-cfg">
              <div class="ai-cfg-section">
                <label class="ai-cfg-label">Claude AI Session Key</label>
                <div class="ai-key-input-wrap">
                  <input
                    class="ai-cfg-input"
                    type={showAgentKey ? 'text' : 'password'}
                    placeholder="sk-ant-sid01-..."
                    bind:value={agentKeyInput}
                  />
                  <button class="ai-key-toggle" onclick={() => showAgentKey = !showAgentKey} type="button">
                    {#if showAgentKey}
                      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
                    {:else}
                      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
                    {/if}
                  </button>
                </div>

                {#if agentKeyTestStatus === 'success'}
                  <span class="ai-test-result success">
                    <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
                    {agentKeyTestMessage}
                  </span>
                {:else if agentKeyTestStatus === 'error'}
                  <span class="ai-test-result error">
                    <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>
                    {agentKeyTestMessage}
                  </span>
                {/if}

                <div class="ai-key-actions">
                  <button
                    class="ai-action-btn primary"
                    onclick={() => handleSaveAgentKey()}
                    disabled={!agentKeyInput.trim() || agentKeyTestStatus === 'testing'}
                  >
                    {#if agentKeyTestStatus === 'testing'}
                      Verifying...
                    {:else}
                      Save & Verify
                    {/if}
                  </button>
                  {#if agentSessionKey}
                    <button class="ai-action-btn danger" onclick={handleRemoveAgentKey}>Remove Key</button>
                  {/if}
                </div>
              </div>

              <div class="ai-cfg-divider"></div>

              <div class="ai-cfg-footer">
                <div class="ai-cfg-links">
                  <span class="agent-key-hint">claude.ai &rarr; DevTools &rarr; Application &rarr; Cookies &rarr; sessionKey</span>
                </div>
                {#if agentSessionKey}
                  <span class="ai-status-badge">
                    <span class="ai-status-dot"></span>
                    Connected
                  </span>
                {/if}
              </div>
            </div>
          </div>

          <!-- Polling -->
          <div class="stg-section">
            <span class="stg-section-label">Polling</span>
            <div class="stg-field">
              <label class="stg-label">Usage refresh interval</label>
              <select
                class="stg-select"
                value={agentRefreshMins}
                onchange={(e) => handleSettingChange('agent_refresh_mins', e.currentTarget.value)}
              >
                {#each REFRESH_OPTIONS as opt}
                  <option value={opt.value}>{opt.label}</option>
                {/each}
              </select>
            </div>
          </div>

          <!-- Notifications -->
          <div class="stg-section">
            <span class="stg-section-label">Notifications</span>
            <div class="stg-field">
              <label class="stg-label">Enable sound alerts</label>
              <label class="stg-toggle">
                <input type="checkbox" checked={agentSoundEnabled}
                  onchange={(e) => handleSettingChange('agent_sound_enabled', String(e.currentTarget.checked))} />
                <span class="stg-toggle-slider"></span>
              </label>
            </div>
            <div class="stg-field">
              <label class="stg-label">Enable dock bounce</label>
              <label class="stg-toggle">
                <input type="checkbox" checked={agentDockBounceEnabled}
                  onchange={(e) => handleSettingChange('agent_dock_bounce_enabled', String(e.currentTarget.checked))} />
                <span class="stg-toggle-slider"></span>
              </label>
            </div>
          </div>

        {:else if agentSubTab === 'plugins'}
          <!-- Installed / Marketplace toggle -->
          <div class="agent-plugin-views">
            <button class="ai-action-btn" class:primary={pluginView === 'installed'} onclick={() => pluginView = 'installed'}>Installed</button>
            <button class="ai-action-btn" class:primary={pluginView === 'marketplace'} onclick={() => pluginView = 'marketplace'}>Marketplace</button>
          </div>

          {#if pluginView === 'installed'}
            {#if installedPlugins.length === 0}
              <div class="ai-usage-empty">
                <svg viewBox="0 0 24 24" width="36" height="36" fill="none" stroke="var(--t4)" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M20.5 7.27783L12 12.0001M12 12.0001L3.49997 7.27783M12 12.0001L12 21.5001M14 20.6701L12.7 21.4001C12.2 21.6001 11.8 21.6001 11.3 21.4001L4.8 17.7001C4.3 17.4001 4 16.9001 4 16.3001V7.70011C4 7.10011 4.3 6.60011 4.8 6.30011L11.3 2.60011C11.8 2.40011 12.2 2.40011 12.7 2.60011L19.2 6.30011C19.7 6.60011 20 7.10011 20 7.70011V16.3001"/></svg>
                <p>No plugins installed</p>
                <span>Browse the marketplace to install plugins</span>
              </div>
            {:else}
              <div class="agent-plugin-list">
                {#each installedPlugins as plugin}
                  <div class="agent-plugin-card">
                    <div class="agent-plugin-info">
                      <span class="agent-plugin-name">{plugin.name}</span>
                      <span class="agent-plugin-meta">
                        {plugin.marketplace}
                        {#if plugin.version}
                          <span class="ai-link-sep">&middot;</span> v{plugin.version}
                        {/if}
                      </span>
                    </div>
                    <div class="agent-plugin-actions">
                      <label class="stg-toggle">
                        <input type="checkbox" checked={plugin.enabled}
                          onchange={() => handleTogglePlugin(plugin.name, !plugin.enabled)} />
                        <span class="stg-toggle-slider"></span>
                      </label>
                      <button class="ai-action-btn danger sm" onclick={() => handleUninstallPlugin(plugin.name, plugin.marketplace)}>
                        Uninstall
                      </button>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}

          {:else}
            <div class="agent-marketplace-search">
              <input
                class="stg-input"
                type="text"
                style="width: 100%;"
                placeholder="Search plugins..."
                bind:value={pluginSearchQuery}
              />
            </div>
            {#if filteredMarketplacePlugins.length === 0}
              <div class="ai-usage-empty">
                <p>No plugins found</p>
              </div>
            {:else}
              <div class="agent-plugin-list">
                {#each filteredMarketplacePlugins as plugin}
                  <div class="agent-plugin-card">
                    <div class="agent-plugin-info">
                      <span class="agent-plugin-name">{plugin.name}</span>
                      <span class="agent-plugin-desc">{plugin.description}</span>
                      {#if plugin.installs != null}
                        <span class="agent-plugin-meta">{plugin.installs.toLocaleString()} installs</span>
                      {/if}
                    </div>
                    <div class="agent-plugin-actions">
                      {#if plugin.installed}
                        <span class="ai-status-badge">
                          <span class="ai-status-dot"></span>
                          Installed
                        </span>
                      {:else}
                        <button class="ai-action-btn primary sm" onclick={() => handleInstallPlugin(plugin.name, plugin.marketplace)}>
                          Install
                        </button>
                      {/if}
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          {/if}

        {:else if agentSubTab === 'contexts'}
          {#if isNewContext || editingContext}
            <!-- Context editor -->
            <div class="agent-ctx-editor">
              <div class="stg-field" style="flex-direction: column; align-items: stretch;">
                <label class="stg-label">Name</label>
                <input
                  class="stg-input"
                  type="text"
                  style="width: 100%;"
                  placeholder="Context name..."
                  bind:value={editContextName}
                />
              </div>
              <div class="stg-field" style="flex-direction: column; align-items: stretch;">
                <label class="stg-label">Content</label>
                <textarea
                  class="agent-ctx-textarea"
                  placeholder="Context content..."
                  bind:value={editContextContent}
                ></textarea>
              </div>
              <div class="ai-key-actions">
                <button class="ai-action-btn primary" onclick={handleSaveAgentContext}>Save</button>
                <button class="ai-action-btn" onclick={cancelEditContext}>Cancel</button>
              </div>
            </div>
          {:else}
            <div class="agent-ctx-header">
              <span class="stg-section-label" style="margin-bottom: 0;">Contexts</span>
              <button class="ai-action-btn primary sm" onclick={startNewContext}>New Context</button>
            </div>

            {#if agentContexts.length === 0}
              <div class="ai-usage-empty">
                <svg viewBox="0 0 24 24" width="36" height="36" fill="none" stroke="var(--t4)" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/></svg>
                <p>No contexts yet</p>
                <span>Create a context to attach to agent sessions</span>
              </div>
            {:else}
              <div class="agent-plugin-list">
                {#each agentContexts as ctx}
                  <div class="agent-plugin-card agent-ctx-card" onclick={() => startEditContext(ctx)} role="button" tabindex="0" onkeydown={(e) => { if (e.key === 'Enter') startEditContext(ctx); }}>
                    <div class="agent-plugin-info">
                      <span class="agent-plugin-name">{ctx.name}</span>
                      <span class="agent-plugin-desc">{ctx.content.split('\n')[0].slice(0, 80)}{ctx.content.length > 80 ? '...' : ''}</span>
                    </div>
                    <div class="agent-plugin-actions" onclick={(e) => { e.stopPropagation(); }}>
                      {#if deleteConfirmId === ctx.id}
                        <span class="ai-reset-confirm">
                          <span>Delete?</span>
                          <button class="ai-action-btn danger sm" onclick={(e) => { e.stopPropagation(); handleDeleteAgentContext(ctx.id); }}>Yes</button>
                          <button class="ai-action-btn sm" onclick={(e) => { e.stopPropagation(); deleteConfirmId = null; }}>No</button>
                        </span>
                      {:else}
                        <button class="ai-action-btn danger sm" onclick={(e) => { e.stopPropagation(); deleteConfirmId = ctx.id; }}>Delete</button>
                      {/if}
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          {/if}

        {:else if agentSubTab === 'usage'}
          <!-- Day range selector -->
          <div class="ud-days">
            {#each [7, 14, 30, 90] as d}
              <button class="ud-day-btn" class:active={agentUsageDays === d} onclick={() => selectAgentUsageDays(d)}>{d}d</button>
            {/each}
            <button class="ai-action-btn sm" style="margin-left: auto;" onclick={loadAgentUsage}>
              <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
              Refresh
            </button>
          </div>

          {#if agentUsageLoading}
            <div class="ud-loading"><div class="ud-spinner"></div>Loading analytics...</div>
          {:else if agentUsageData}
            <!-- Summary cards -->
            <div class="ud-cards">
              <div class="ud-card"><span class="ud-val">{agentFormatCost(agentUsageData.totalCost)}</span><span class="ud-lbl">Total Cost</span></div>
              <div class="ud-card"><span class="ud-val">{agentUsageData.totalApiCalls.toLocaleString()}</span><span class="ud-lbl">API Calls</span></div>
              <div class="ud-card"><span class="ud-val">{agentUsageData.totalSessions}</span><span class="ud-lbl">Sessions</span></div>
              <div class="ud-card"><span class="ud-val">{agentUsageData.cacheHitPercent.toFixed(1)}%</span><span class="ud-lbl">Cache Hit</span></div>
            </div>

            <!-- Token breakdown -->
            <div class="ud-token-row">
              <span><strong>In:</strong> {agentFormatTokens(agentUsageData.totalInputTokens)}</span>
              <span><strong>Out:</strong> {agentFormatTokens(agentUsageData.totalOutputTokens)}</span>
              <span><strong>Cache R:</strong> {agentFormatTokens(agentUsageData.totalCacheReadTokens)}</span>
              <span><strong>Cache W:</strong> {agentFormatTokens(agentUsageData.totalCacheWriteTokens)}</span>
            </div>

            <!-- Daily chart -->
            {#if agentUsageData.daily.length > 0}
              <div class="ud-section-inline">
                <div class="ud-section-title">Daily Activity</div>
                <div class="ud-chart">
                  {#each agentUsageData.daily.slice(-21) as day}
                    {@const maxCost = Math.max(...agentUsageData.daily.slice(-21).map(d => d.cost), 0.01)}
                    <div class="ud-bar-wrap" title="{day.date}: {agentFormatCost(day.cost)} / {day.calls} calls">
                      <div class="ud-bar" style="height:{Math.max(3, (day.cost / maxCost) * 100)}%"></div>
                      <span class="ud-bar-lbl">{day.date.slice(8)}</span>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}
            <!-- Live Usage + Models -->
            <div class="ud-grid">
              <div class="ud-section">
                <div class="ud-section-title">Live Usage</div>
                {#if $agentUsageLimits}
                  {@const sessionPct = $agentUsageLimits.five_hour?.utilization ?? $agentUsageLimits.standard?.percentUsed ?? null}
                  {@const weeklyPct = $agentUsageLimits.seven_day?.utilization ?? $agentUsageLimits.extended?.percentUsed ?? null}
                  {@const sonnetPct = $agentUsageLimits.seven_day_sonnet?.utilization ?? null}
                  <div class="ud-live-rows">
                    {#if sessionPct != null}
                      <div class="ud-live-row">
                        <span class="ud-live-lbl">Session</span>
                        <div class="ud-live-bar"><div class="ud-live-fill" style="width:{sessionPct}%;background:{agentLiveColor(sessionPct)}"></div></div>
                        <span class="ud-live-pct" style="color:{agentLiveColor(sessionPct)}">{sessionPct.toFixed(1)}%</span>
                      </div>
                    {/if}
                    {#if weeklyPct != null}
                      <div class="ud-live-row">
                        <span class="ud-live-lbl">Weekly</span>
                        <div class="ud-live-bar"><div class="ud-live-fill" style="width:{weeklyPct}%;background:{agentLiveColor(weeklyPct)}"></div></div>
                        <span class="ud-live-pct" style="color:{agentLiveColor(weeklyPct)}">{weeklyPct.toFixed(1)}%</span>
                      </div>
                    {/if}
                    {#if sonnetPct != null}
                      <div class="ud-live-row">
                        <span class="ud-live-lbl">Sonnet</span>
                        <div class="ud-live-bar"><div class="ud-live-fill" style="width:{sonnetPct}%;background:{agentLiveColor(sonnetPct)}"></div></div>
                        <span class="ud-live-pct" style="color:{agentLiveColor(sonnetPct)}">{sonnetPct.toFixed(1)}%</span>
                      </div>
                    {/if}
                  </div>
                {:else}
                  <div style="padding:8px 0;font-size:11px;color:var(--t3);">Fetching live data...</div>
                {/if}
              </div>
              <div class="ud-section">
                <div class="ud-section-title">Models</div>
                <div class="ud-scroll">
                  {#each agentUsageData.byModel as m}
                    <div class="ud-row">
                      <div class="ud-row-info">
                        <span class="ud-row-name">{m.model}</span>
                        <span class="ud-row-meta">{m.calls} calls &middot; {m.cacheHitPercent.toFixed(0)}% cache</span>
                      </div>
                      <span class="ud-row-cost">{agentFormatCost(m.cost)}</span>
                    </div>
                  {/each}
                </div>
              </div>
            </div>

            <!-- Projects + Top Sessions (2-col) -->
            <div class="ud-grid">
              <div class="ud-section">
                <div class="ud-section-title">Projects ({agentUsageData.byProject.length})</div>
                <div class="ud-scroll">
                  {#each agentUsageData.byProject as p}
                    <div class="ud-row">
                      <div class="ud-row-info">
                        <span class="ud-row-name" title={p.project}>{agentDecodeName(p.project)}</span>
                        <span class="ud-row-meta">{p.sessions} sess &middot; {p.calls} calls</span>
                      </div>
                      <span class="ud-row-cost">{agentFormatCost(p.cost)}</span>
                    </div>
                  {/each}
                </div>
              </div>
              <div class="ud-section">
                <div class="ud-section-title">Top Sessions</div>
                <div class="ud-scroll">
                  {#each agentUsageData.topSessions.slice(0, 6) as s}
                    <div class="ud-row">
                      <div class="ud-row-info">
                        <span class="ud-row-name" title={s.project}>{agentDecodeName(s.project)}</span>
                        <span class="ud-row-meta">{s.model} &middot; {s.sessionId.slice(0, 8)}</span>
                      </div>
                      <span class="ud-row-cost">{agentFormatCost(s.cost)}</span>
                    </div>
                  {/each}
                </div>
              </div>
            </div>

            <!-- Tools + Shell (2-col) -->
            <div class="ud-grid">
              <div class="ud-section">
                <div class="ud-section-title">Tools</div>
                <div class="ud-scroll">
                  {#each agentUsageData.tools.slice(0, 6) as t}
                    <div class="ud-tool-row">
                      <span class="ud-tool-name">{t.name}</span>
                      <div class="ud-tool-bar"><div class="ud-tool-fill" style="width:{Math.max(3, (t.count / (agentUsageData.tools[0]?.count || 1)) * 100)}%"></div></div>
                      <span class="ud-tool-ct">{t.count.toLocaleString()}</span>
                    </div>
                  {/each}
                </div>
              </div>
              <div class="ud-section">
                <div class="ud-section-title">Shell</div>
                <div class="ud-scroll">
                  {#each agentUsageData.shellCommands.slice(0, 6) as cmd}
                    <div class="ud-tool-row">
                      <span class="ud-tool-name" style="font-family:var(--mono)">{cmd.name}</span>
                      <div class="ud-tool-bar"><div class="ud-tool-fill" style="width:{Math.max(3, (cmd.count / (agentUsageData.shellCommands[0]?.count || 1)) * 100)}%"></div></div>
                      <span class="ud-tool-ct">{cmd.count.toLocaleString()}</span>
                    </div>
                  {/each}
                </div>
              </div>
            </div>
          {:else}
            <div class="ud-loading">
              <svg viewBox="0 0 24 24" width="36" height="36" fill="none" stroke="var(--t4)" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 20V10M12 20V4M6 20v-6"/></svg>
              <p style="margin: 0; font-size: 13px; color: var(--t2); font-weight: 500;">No usage data found</p>
              <span style="font-size: 11px; color: var(--t3);">Start using Claude Code sessions to see analytics here</span>
            </div>
          {/if}
        {/if}

      {:else if activeTab === 'proxy'}
        <div class="stg-section">
          <span class="stg-section-label">Proxy Configuration</span>

          <div class="stg-field">
            <label class="stg-label">Proxy URL</label>
            <input
              class="stg-input"
              type="text"
              placeholder="http://proxy:8080"
              value={proxyUrl}
              onchange={(e) => handleSettingChange('proxy_url', e.currentTarget.value)}
            />
          </div>

          <div class="stg-field">
            <label class="stg-label">Proxy Authentication</label>
            <label class="stg-toggle">
              <input type="checkbox" checked={proxyAuth}
                onchange={(e) => handleSettingChange('proxy_auth', String(e.currentTarget.checked))} />
              <span class="stg-toggle-slider"></span>
            </label>
          </div>

          {#if proxyAuth}
            <div class="stg-field">
              <label class="stg-label">Username</label>
              <input
                class="stg-input"
                type="text"
                value={proxyUsername}
                placeholder="username"
                onchange={(e) => handleSettingChange('proxy_username', e.currentTarget.value)}
              />
            </div>

            <div class="stg-field">
              <label class="stg-label">Password</label>
              <input
                class="stg-input"
                type="password"
                value={proxyPassword}
                placeholder="password"
                onchange={(e) => handleSettingChange('proxy_password', e.currentTarget.value)}
              />
            </div>
          {/if}
        </div>

      {:else if activeTab === 'shortcuts'}
        <div class="stg-section">
          <span class="stg-section-label">Keyboard Shortcuts</span>
          <div class="stg-shortcuts">
            {#each SHORTCUTS as shortcut}
              <div class="stg-shortcut-row">
                <span class="stg-shortcut-desc">{shortcut.desc}</span>
                <span class="stg-shortcut-keys">
                  {#each shortcut.keys as key, i}
                    <kbd class="kbd">{key}</kbd>
                    {#if i < shortcut.keys.length - 1}
                      <span class="stg-shortcut-plus">+</span>
                    {/if}
                  {/each}
                </span>
              </div>
            {/each}
          </div>
        </div>

      {:else if activeTab === 'about'}
        <div class="stg-about">
          <!-- App identity -->
          <div class="about-header">
            <span class="about-app-name">Clauge</span>
            <span class="about-version">v0.9.0</span>
          </div>
          <p class="about-desc">The world's first AI-powered developer toolkit with built-in REST client, SQL & NoSQL database management, environment variables, GitHub sync, and more — all in one beautiful desktop app.</p>

          <!-- Tech Stack -->
          <div class="about-section-label">TECH STACK</div>
          <div class="about-tech-grid">
            <span class="about-tech-pill">
              <svg viewBox="0 0 106 106" class="tech-icon"><path d="M103.3 53.1c0-3.8-2-7.2-5.2-10.2-2.1-2-4.7-3.7-7.6-5.2.4-3.5.3-6.7-.4-9.5-1-4-3.2-7-6.5-8.9-3.1-1.8-6.8-2.2-10.6-1.4-2.6.5-5.3 1.6-8 3.1-2.4-2.6-5-4.8-7.8-6.4-4-2.3-8-3.3-11.8-2.8-3.9.5-7.2 2.4-9.6 5.6-1.7 2.3-2.9 5.1-3.7 8.2-3.4-.6-6.6-.6-9.4 0-4 .9-7.2 3-9.3 6.2-2 3-2.5 6.7-1.8 10.5.5 2.6 1.5 5.4 3 8.2-2.7 2.3-4.9 4.9-6.5 7.6-2.3 3.9-3.3 7.9-2.8 11.7.5 3.9 2.4 7.2 5.7 9.6 2.3 1.7 5.2 2.9 8.3 3.6-.6 3.4-.6 6.6 0 9.4.9 3.9 3 7.1 6.2 9.2 3 2 6.7 2.6 10.5 1.9 2.6-.5 5.4-1.6 8.2-3.1 2.4 2.7 5 4.9 7.7 6.5 3.9 2.3 7.9 3.3 11.7 2.8 3.9-.5 7.2-2.4 9.6-5.7 1.7-2.3 2.9-5.2 3.7-8.3 3.4.6 6.6.6 9.4 0 4-.9 7.2-3 9.3-6.2 2-3 2.5-6.7 1.8-10.5-.5-2.6-1.5-5.4-3-8.2 2.7-2.4 4.9-5 6.5-7.7 2.3-3.9 3.3-7.9 2.8-11.7z" fill="none" stroke="currentColor" stroke-width="6"/></svg>
              Rust
            </span>
            <span class="about-tech-pill">
              <svg viewBox="0 0 128 128" class="tech-icon"><path d="M64 0C28.6 0 0 28.6 0 64s28.6 64 64 64 64-28.6 64-64S99.4 0 64 0zm0 110c-25.4 0-46-20.6-46-46S38.6 18 64 18s46 20.6 46 46-20.6 46-46 46z" fill="currentColor"/><circle cx="64" cy="64" r="24" fill="currentColor"/></svg>
              Tauri v2
            </span>
            <span class="about-tech-pill">
              <svg viewBox="0 0 24 24" class="tech-icon"><path d="M12.1 2L1 21h22L12.1 2z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/></svg>
              SvelteKit
            </span>
            <span class="about-tech-pill">
              <svg viewBox="0 0 24 24" class="tech-icon"><rect x="2" y="3" width="20" height="18" rx="2" fill="none" stroke="currentColor" stroke-width="1.5"/><text x="12" y="16" text-anchor="middle" font-size="10" font-weight="700" font-family="sans-serif" fill="currentColor">TS</text></svg>
              TypeScript
            </span>
            <span class="about-tech-pill">
              <svg viewBox="0 0 24 24" class="tech-icon"><ellipse cx="12" cy="6" rx="8" ry="3" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M4 6v12c0 1.66 3.58 3 8 3s8-1.34 8-3V6" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>
              SQLite
            </span>
            <span class="about-tech-pill">
              <svg viewBox="0 0 24 24" class="tech-icon"><polyline points="16 18 22 12 16 6" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/><polyline points="8 6 2 12 8 18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
              CodeMirror
            </span>
          </div>

          <!-- Links -->
          <div class="about-section-label">LINKS</div>
          <div class="about-links">
            <a class="about-link-btn" href="https://github.com/ansxuman/Clauge" target="_blank" rel="noopener" title="GitHub Repository">
              <svg viewBox="0 0 24 24"><path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 00-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0020 4.77 5.07 5.07 0 0019.91 1S18.73.65 16 2.48a13.38 13.38 0 00-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 005 4.77a5.44 5.44 0 00-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 009 18.13V22"/></svg>
              <span>Project</span>
            </a>
            <a class="about-link-btn" href="https://github.com/ansxuman/Clauge/issues/new" target="_blank" rel="noopener" title="Report an Issue">
              <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M12 8v4M12 16h.01"/></svg>
              <span>Report Issue</span>
            </a>
            <a class="about-link-btn" href="https://github.com/ansxuman" target="_blank" rel="noopener" title="Developer">
              <svg viewBox="0 0 24 24"><path d="M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>
              <span>Developer</span>
            </a>
            <a class="about-link-btn" href="https://clauge.dev" target="_blank" rel="noopener" title="Website">
              <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z"/></svg>
              <span>Website</span>
            </a>
          </div>

          <!-- Support -->
          <div class="about-section-label">SUPPORT</div>
          <a class="about-coffee" href="https://buymeacoffee.com/ansxuman" target="_blank" rel="noopener">
            <svg viewBox="0 0 24 24"><path d="M17 8h1a4 4 0 110 8h-1"/><path d="M3 8h14v9a4 4 0 01-4 4H7a4 4 0 01-4-4V8z"/><line x1="6" y1="2" x2="6" y2="4"/><line x1="10" y1="2" x2="10" y2="4"/><line x1="14" y1="2" x2="14" y2="4"/></svg>
            Buy me a coffee
          </a>

        </div>
      {/if}
    </div>
  </div>
</Modal>

<style>
  .stg-layout {
    display: flex;
    margin: -18px;
    min-height: 440px;
  }

  /* -- Tab sidebar -- */
  .stg-tabs {
    width: 160px;
    background: var(--e);
    border-right: 1px solid var(--b1);
    padding: 6px 0;
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
  }

  .stg-tab {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border: none;
    background: transparent;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 12px;
    cursor: default;
    transition: color 0.12s, background 0.12s;
    border-left: 2px solid transparent;
    text-align: left;
  }

  .stg-tab:hover {
    color: var(--t1);
    background: var(--n2);
  }

  .stg-tab.active {
    color: var(--t1);
    background: var(--n2);
    border-left-color: var(--acc);
  }

  .stg-tab svg {
    width: 15px;
    height: 15px;
    flex-shrink: 0;
  }

  /* -- Content pane -- */
  .stg-content {
    flex: 1;
    padding: 20px 24px;
    overflow-y: auto;
    min-width: 0;
  }

  /* -- Section -- */
  .stg-section {
    margin-bottom: 24px;
  }

  .stg-section-label {
    display: block;
    font-size: 14px;
    text-transform: uppercase;
    color: var(--t3);
    letter-spacing: 0.1em;
    margin-bottom: 14px;
    font-family: var(--ui);
    font-weight: 600;
  }

  /* -- Field row -- */
  .stg-field {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
    gap: 12px;
  }

  .stg-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--t2);
    font-family: var(--ui);
    white-space: nowrap;
  }

  .stg-field-col {
    flex-direction: column;
    align-items: stretch;
  }
  .stg-slider-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .stg-slider-value {
    font-size: 13px;
    font-weight: 600;
    color: var(--acc);
    font-family: var(--mono);
  }
  .stg-slider {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 4px;
    background: var(--b1);
    border-radius: 2px;
    outline: none;
    cursor: default;
  }
  .stg-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--acc);
    border: 2px solid var(--t1);
    cursor: default;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
    transition: transform 0.1s;
  }
  .stg-slider::-webkit-slider-thumb:hover {
    transform: scale(1.15);
  }
  .stg-zoom-marks {
    display: flex;
    justify-content: space-between;
    font-size: 10px;
    color: var(--t4);
    font-family: var(--ui);
    margin-bottom: 4px;
  }

  /* -- Inputs -- */
  .stg-input {
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md);
    padding: 6px 10px;
    color: var(--t1);
    font-family: var(--mono);
    font-size: 12.5px;
    outline: none;
    width: 140px;
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .stg-input:focus {
    border-color: var(--acc);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 12%, transparent);
  }

  .stg-select {
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md);
    padding: 6px 10px;
    color: var(--t1);
    font-family: var(--mono);
    font-size: 12.5px;
    outline: none;
    cursor: default;
    min-width: 120px;
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .stg-select:focus {
    border-color: var(--acc);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 12%, transparent);
  }

  .stg-select option {
    background: var(--n);
    color: var(--t1);
  }

  /* -- Toggle switch -- */
  .stg-toggle {
    position: relative;
    display: inline-block;
    width: 36px;
    height: 20px;
    flex-shrink: 0;
    cursor: default;
  }

  .stg-toggle input {
    opacity: 0;
    width: 0;
    height: 0;
    position: absolute;
  }

  .stg-toggle-slider {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: var(--b1);
    border-radius: 10px;
    transition: background 0.2s;
  }

  .stg-toggle-slider::after {
    content: '';
    position: absolute;
    width: 16px;
    height: 16px;
    left: 2px;
    top: 2px;
    background: #fff;
    border-radius: 50%;
    transition: transform 0.2s, background 0.2s;
  }

  .stg-toggle input:checked + .stg-toggle-slider {
    background: var(--acc);
  }

  .stg-toggle input:checked + .stg-toggle-slider::after {
    left: 18px;
    background: #fff;
  }

  /* -- Theme grid -- */
  .theme-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }

  .theme-card {
    background: rgba(255,255,255,0.03);
    border: 1px solid var(--b1);
    border-radius: var(--radius-lg);
    padding: 10px;
    cursor: default;
    transition: border-color 0.15s, background 0.15s;
    text-align: left;
  }

  .theme-card:hover {
    border-color: var(--b2);
    background: rgba(255,255,255,0.05);
  }

  .theme-card.active {
    border-color: transparent;
    outline: 2px solid var(--acc);
    outline-offset: 2px;
  }

  .theme-preview {
    height: 36px;
    border-radius: var(--radius-sm);
    overflow: hidden;
    display: flex;
    margin-bottom: 8px;
  }

  .theme-preview-bar {
    flex: 1;
  }

  .theme-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .theme-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }

  .theme-desc {
    font-size: 10px;
    color: var(--t3);
    font-family: var(--ui);
  }

  /* -- Color swatches -- */
  .stg-swatches {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }

  .stg-swatch {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: default;
    transition: transform 0.1s, border-color 0.15s;
    outline: none;
  }

  .stg-swatch:hover {
    transform: scale(1.15);
  }

  .stg-swatch.active {
    border-color: var(--t1);
    box-shadow: 0 0 0 2px var(--n);
  }

  /* -- Shortcuts -- */
  .stg-shortcuts {
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .stg-shortcut-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 0;
    border-bottom: 1px solid var(--b1);
  }

  .stg-shortcut-row:last-child {
    border-bottom: none;
  }

  .stg-shortcut-desc {
    font-size: 13px;
    color: var(--t2);
    font-family: var(--ui);
  }

  .stg-shortcut-keys {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .stg-shortcut-plus {
    font-size: 10px;
    color: var(--t3);
  }

  .kbd {
    display: inline-block;
    padding: 2px 7px;
    font-size: 11px;
    font-family: var(--ui);
    color: var(--t1);
    background: var(--n2);
    border: 1px solid var(--b1);
    border-radius: 4px;
    box-shadow: 0 1px 0 var(--b1);
    min-width: 20px;
    text-align: center;
    font-weight: 500;
  }

  /* -- About -- */
  .stg-about {
    display: flex;
    flex-direction: column;
    padding: 8px 0;
    gap: 20px;
  }
  .about-header {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .about-app-name {
    font-size: 26px;
    font-weight: 700;
    color: var(--t1);
    font-family: var(--ui);
    letter-spacing: -0.5px;
  }
  .about-version {
    font-size: 12px;
    color: var(--acc);
    font-family: var(--mono);
    font-weight: 600;
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    padding: 2px 8px;
    border-radius: 4px;
  }
  .about-desc {
    font-size: 12.5px;
    color: var(--t3);
    font-family: var(--ui);
    line-height: 1.5;
    margin: 0;
  }
  .about-section-label {
    font-size: 10px;
    font-weight: 600;
    color: var(--t4);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .about-tech-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .about-tech-pill {
    font-size: 11.5px;
    font-family: var(--mono);
    color: var(--t2);
    background: var(--n2);
    border: 1px solid var(--b1);
    padding: 5px 14px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .tech-icon {
    width: 14px;
    height: 14px;
    color: var(--t3);
    flex-shrink: 0;
  }
  .about-links {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .about-link-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 14px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 11.5px;
    font-family: var(--ui);
    text-decoration: none;
    transition: border-color 0.12s, color 0.12s, background 0.12s;
    cursor: default;
  }
  .about-link-btn:hover {
    border-color: var(--b2);
    color: var(--t1);
    background: rgba(255,255,255,0.03);
  }
  .about-link-btn svg {
    width: 14px;
    height: 14px;
    stroke: currentColor;
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .about-coffee {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 18px;
    border-radius: 8px;
    border: 1px solid rgba(245,166,35,0.3);
    background: rgba(245,166,35,0.06);
    color: #f5a623;
    font-size: 13px;
    font-weight: 600;
    font-family: var(--ui);
    text-decoration: none;
    transition: background 0.12s, border-color 0.12s;
    cursor: default;
  }
  .about-coffee:hover {
    background: rgba(245,166,35,0.12);
    border-color: rgba(245,166,35,0.5);
  }
  .about-coffee svg {
    width: 18px;
    height: 18px;
    stroke: #f5a623;
    fill: none;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* -- AI Sub-tabs -- */
  .ai-subtabs {
    display: flex;
    gap: 2px;
    margin-bottom: 20px;
    background: var(--n2);
    border-radius: 8px;
    padding: 3px;
    border: 1px solid var(--b1);
  }
  .ai-subtab {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 7px 14px;
    border-radius: 6px;
    border: none;
    background: transparent;
    color: var(--t3);
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 500;
    cursor: default;
    transition: color 0.12s, background 0.12s;
  }
  .ai-subtab:hover:not(.disabled) { color: var(--t1); }
  .ai-subtab.active {
    background: var(--acc);
    color: #fff;
  }
  .ai-subtab.disabled { opacity: 0.35; cursor: not-allowed; }
  .ai-subtab-lock { flex-shrink: 0; margin-left: 2px; }

  /* -- AI Config Card -- */
  .ai-cfg {
    background: var(--n2);
    border: 1px solid var(--b1);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }
  .ai-cfg-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 16px;
    padding: 18px 20px;
    align-items: end;
  }
  .ai-cfg-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ai-cfg-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--t3);
    font-family: var(--ui);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }
  .ai-cfg-select {
    -webkit-appearance: menulist;
    appearance: menulist;
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md);
    padding: 8px 12px;
    color: var(--t1);
    font-family: var(--ui);
    font-size: 12.5px;
    outline: none;
    width: 100%;
    cursor: default;
    transition: border-color 0.15s, box-shadow 0.15s;
    height: 34px;
    box-sizing: border-box;
  }
  .ai-cfg-select:focus {
    border-color: var(--acc);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 12%, transparent);
  }
  .ai-cfg-select option {
    background: var(--n);
    color: var(--t1);
  }
  .ai-model-tag {
    font-size: 11px;
    color: var(--acc);
    background: color-mix(in srgb, var(--acc) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--acc) 20%, transparent);
    padding: 8px 16px;
    border-radius: var(--radius-md);
    font-weight: 600;
    font-family: var(--mono);
    letter-spacing: 0.3px;
    white-space: nowrap;
    text-align: center;
    line-height: 1.25;
    box-sizing: border-box;
    height: 34px;
    display: inline-flex;
    align-items: center;
  }
  .ai-cfg-divider {
    height: 1px;
    background: var(--b1);
  }
  .ai-cfg-section {
    padding: 18px 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .ai-cfg-input {
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md);
    padding: 8px 12px;
    padding-right: 36px;
    color: var(--t1);
    font-family: var(--mono);
    font-size: 12px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .ai-cfg-input:focus {
    border-color: var(--acc);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 12%, transparent);
  }
  .ai-key-input-wrap {
    position: relative;
  }
  .ai-key-toggle {
    position: absolute;
    right: 10px;
    top: 50%;
    transform: translateY(-50%);
    background: none;
    border: none;
    color: var(--t3);
    cursor: default;
    padding: 2px;
    display: flex;
  }
  .ai-key-toggle:hover { color: var(--t1); }
  .ai-test-result {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    font-family: var(--ui);
  }
  .ai-test-result.success { color: var(--ok); }
  .ai-test-result.error { color: var(--err); }
  .ai-key-actions {
    display: flex;
    gap: 8px;
    padding-top: 2px;
  }
  .ai-cfg-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 20px;
    background: rgba(255,255,255,0.02);
  }
  .ai-cfg-links {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ai-status-badge {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    font-weight: 500;
    color: var(--ok);
    font-family: var(--ui);
  }
  .ai-status-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
    animation: ai-pulse 2s ease-in-out infinite;
  }
  @keyframes ai-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }

  /* -- AI Action buttons (shared) -- */
  .ai-action-btn {
    padding: 7px 16px;
    border-radius: var(--radius-md);
    border: 1px solid var(--b1);
    background: rgba(255,255,255,0.04);
    color: var(--t2);
    font-family: var(--ui);
    font-size: 12px;
    font-weight: 500;
    cursor: default;
    transition: border-color 0.12s, color 0.12s, background 0.12s, opacity 0.12s;
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .ai-action-btn:hover:not(:disabled) { border-color: var(--b2); color: var(--t1); }
  .ai-action-btn:disabled { opacity: 0.4; cursor: default; }
  .ai-action-btn.primary { background: var(--acc); color: #fff; border-color: var(--acc); }
  .ai-action-btn.primary:hover:not(:disabled) { opacity: 0.85; }
  .ai-action-btn.danger { color: var(--t3); }
  .ai-action-btn.danger:hover { color: var(--err); border-color: var(--err); }
  .ai-action-btn.sm { padding: 4px 10px; font-size: 11px; }

  /* -- AI Links -- */
  .ai-link {
    font-size: 11.5px;
    color: var(--t3);
    text-decoration: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-family: var(--ui);
    transition: color 0.12s;
  }
  .ai-link:hover { color: var(--acc); }
  .ai-link-sep { color: var(--t4); font-size: 10px; }

  /* -- AI Reset confirm -- */
  .ai-reset-confirm {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: var(--t2);
    font-family: var(--ui);
  }

  /* -- AI Usage Empty -- */
  .ai-usage-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 48px 20px;
    gap: 8px;
    text-align: center;
  }
  .ai-usage-empty p {
    margin: 0;
    font-size: 13px;
    color: var(--t2);
    font-weight: 500;
    font-family: var(--ui);
  }
  .ai-usage-empty span {
    font-size: 11px;
    color: var(--t3);
    font-family: var(--ui);
  }

  /* -- AI Summary -- */
  .ai-summary-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 14px;
  }
  .ai-summary-header .stg-section-label { margin-bottom: 0; }
  .ai-summary-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
  }
  .ai-summary-card {
    background: var(--n2);
    border: 1px solid var(--b1);
    border-radius: var(--radius-lg);
    padding: 14px 12px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }
  .ai-summary-val {
    font-size: 20px;
    font-weight: 700;
    color: var(--t1);
    font-family: var(--mono);
  }
  .ai-summary-card.accent .ai-summary-val { color: var(--acc); }
  .ai-summary-lbl {
    font-size: 10px;
    color: var(--t3);
    text-transform: uppercase;
    letter-spacing: 0.4px;
    font-weight: 500;
    font-family: var(--ui);
  }

  /* -- AI Table -- */
  .ai-table {
    background: var(--n2);
    border: 1px solid var(--b1);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }
  .ai-table-head {
    display: grid;
    grid-template-columns: 1.8fr 1fr 1fr 1fr 1fr;
    padding: 9px 14px;
    font-size: 10px;
    color: var(--t3);
    text-transform: uppercase;
    letter-spacing: 0.3px;
    font-weight: 600;
    font-family: var(--ui);
    border-bottom: 1px solid var(--b1);
  }
  .ai-table-row {
    display: grid;
    grid-template-columns: 1.8fr 1fr 1fr 1fr 1fr;
    padding: 9px 14px;
    font-size: 12px;
    border-bottom: 1px solid var(--b1);
    align-items: center;
  }
  .ai-table-row:last-child { border-bottom: none; }
  .ai-table-label {
    display: flex;
    align-items: center;
    gap: 7px;
    font-weight: 600;
    font-size: 11.5px;
    color: var(--t1);
    font-family: var(--ui);
    letter-spacing: 0.2px;
  }
  .ai-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--t3);
    flex-shrink: 0;
  }
  .ai-dot[data-mode="rest"] { background: var(--rest); }
  .ai-dot[data-mode="sql"] { background: var(--sql); }
  .ai-dot[data-mode="nosql"] { background: var(--nosql); }
  .ai-table-val {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--t2);
  }
  .ai-table-val.accent { color: var(--acc); font-weight: 500; }
  .ai-pricing-note {
    font-size: 10px;
    color: var(--t4);
    font-family: var(--mono);
    margin: 8px 0 0;
    text-align: right;
  }

  /* -- Agent Tab -- */
  .agent-field-desc {
    font-size: 10.5px;
    color: var(--t4);
    font-family: var(--ui);
    margin-bottom: 6px;
  }
  .agent-plugin-views {
    display: flex;
    gap: 8px;
    margin-bottom: 16px;
  }
  .agent-plugin-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .agent-plugin-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    background: var(--n2);
    border: 1px solid var(--b1);
    border-radius: var(--radius-lg);
    gap: 12px;
  }
  .agent-ctx-card {
    cursor: default;
    transition: border-color 0.12s;
  }
  .agent-ctx-card:hover {
    border-color: var(--b2);
  }
  .agent-plugin-info {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    flex: 1;
  }
  .agent-plugin-name {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }
  .agent-plugin-desc {
    font-size: 11px;
    color: var(--t3);
    font-family: var(--ui);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .agent-plugin-meta {
    font-size: 10.5px;
    color: var(--t4);
    font-family: var(--mono);
  }
  .agent-plugin-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
  }
  .agent-marketplace-search {
    margin-bottom: 14px;
  }
  .agent-ctx-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 14px;
  }
  .agent-ctx-editor {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .agent-ctx-textarea {
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: var(--radius-md);
    padding: 8px 12px;
    color: var(--t1);
    font-family: var(--mono);
    font-size: 12px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
    min-height: 160px;
    resize: vertical;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .agent-ctx-textarea:focus {
    border-color: var(--acc);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--acc) 12%, transparent);
  }

  /* -- Agent Provider Grid -- */
  .agent-provider-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .agent-provider-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 14px 10px 10px;
    background: var(--n2);
    border: 1px solid var(--b1);
    border-radius: var(--radius-lg);
    cursor: default;
    transition: border-color 0.15s, background 0.15s;
    position: relative;
    text-align: center;
  }
  .agent-provider-card:hover:not(.disabled) {
    border-color: var(--b2);
    background: rgba(255,255,255,0.05);
  }
  .agent-provider-card.active {
    border-color: transparent;
    outline: 2px solid var(--acc);
    outline-offset: 1px;
  }
  .agent-provider-card.disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .agent-provider-icon {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: rgba(255,255,255,0.06);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--t2);
  }
  .agent-provider-card.active .agent-provider-icon {
    background: color-mix(in srgb, var(--acc) 15%, transparent);
    color: var(--acc);
  }
  .agent-provider-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .agent-provider-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }
  .agent-provider-desc {
    font-size: 10px;
    color: var(--t3);
    font-family: var(--ui);
  }
  .agent-provider-badge {
    font-size: 9px;
    font-weight: 600;
    font-family: var(--ui);
    padding: 2px 8px;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }
  .agent-provider-badge.active {
    color: var(--ok);
    background: color-mix(in srgb, var(--ok) 12%, transparent);
  }
  .agent-provider-badge.soon {
    color: var(--t4);
    background: rgba(255,255,255,0.04);
  }
  .agent-key-hint {
    font-size: 10px;
    color: var(--t4);
    font-family: var(--mono);
  }

  /* -- Agent Usage Stats (inline) -- */
  .ud-days { display: flex; gap: 6px; align-items: center; margin-bottom: 16px; }
  .ud-day-btn { padding: 4px 12px; border-radius: 6px; border: 1px solid var(--b1); background: transparent; color: var(--t3); font-size: 11px; font-family: var(--ui); cursor: default; transition: all 0.12s; }
  .ud-day-btn.active { background: var(--acc); color: #fff; border-color: var(--acc); }
  .ud-day-btn:hover:not(.active) { background: var(--c); color: var(--t1); }

  .ud-loading { padding: 48px; text-align: center; font-size: 13px; color: var(--t3); display: flex; flex-direction: column; align-items: center; gap: 12px; }
  .ud-spinner { width: 22px; height: 22px; border: 2px solid var(--b1); border-top-color: var(--acc); border-radius: 50%; animation: ud-spin 0.6s linear infinite; }
  @keyframes ud-spin { to { transform: rotate(360deg); } }

  .ud-cards { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; margin-bottom: 16px; }
  .ud-card { display: flex; flex-direction: column; align-items: center; padding: 12px 8px; border-radius: 8px; background: var(--n2); border: 1px solid var(--b1); }
  .ud-val { font-size: 18px; font-weight: 700; color: var(--t1); font-family: var(--mono); font-variant-numeric: tabular-nums; }
  .ud-lbl { font-size: 9px; color: var(--t3); text-transform: uppercase; letter-spacing: 0.3px; margin-top: 4px; font-family: var(--ui); }

  .ud-token-row { display: flex; gap: 16px; justify-content: center; padding: 8px 12px; border-radius: 6px; background: rgba(255,255,255,0.02); font-size: 10px; color: var(--t3); margin-bottom: 16px; font-family: var(--mono); }
  .ud-token-row strong { color: var(--t1); font-weight: 500; }

  .ud-section-inline { margin-bottom: 16px; }
  .ud-section { flex: 1; min-width: 0; }
  .ud-section-title { font-size: 10px; font-weight: 600; color: var(--t3); text-transform: uppercase; letter-spacing: 0.1em; margin-bottom: 8px; font-family: var(--ui); }

  .ud-chart { display: flex; align-items: flex-end; gap: 3px; height: 80px; padding: 4px 0; }
  .ud-bar-wrap { flex: 1; display: flex; flex-direction: column; align-items: center; height: 100%; justify-content: flex-end; cursor: default; }
  .ud-bar { width: 100%; border-radius: 2px 2px 0 0; min-height: 2px; background: var(--acc); opacity: 0.8; transition: height 0.3s ease; }
  .ud-bar-wrap:hover .ud-bar { opacity: 1; }
  .ud-bar-lbl { font-size: 8px; color: var(--t4); margin-top: 3px; }

  .ud-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-bottom: 16px; }
  .ud-scroll { max-height: 160px; overflow-y: auto; }

  .ud-row { display: flex; align-items: center; gap: 10px; padding: 5px 8px; border-radius: 5px; }
  .ud-row:hover { background: rgba(255,255,255,0.03); }
  .ud-row-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .ud-row-name { font-size: 12px; font-weight: 500; color: var(--t1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--ui); }
  .ud-row-meta { font-size: 10px; color: var(--t3); font-family: var(--mono); }
  .ud-row-cost { font-size: 12px; font-weight: 600; color: var(--acc); font-family: var(--mono); font-variant-numeric: tabular-nums; flex-shrink: 0; }

  .ud-tool-row { display: flex; align-items: center; gap: 8px; padding: 3px 8px; font-size: 11px; }
  .ud-tool-name { width: 80px; flex-shrink: 0; color: var(--t1); font-weight: 500; font-family: var(--mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ud-tool-bar { flex: 1; height: 4px; background: rgba(255,255,255,0.06); border-radius: 2px; overflow: hidden; }
  .ud-tool-fill { height: 100%; background: var(--acc); border-radius: 2px; opacity: 0.7; }
  .ud-tool-ct { width: 44px; text-align: right; color: var(--t3); font-family: var(--mono); font-variant-numeric: tabular-nums; flex-shrink: 0; }
  /* -- Live Usage Bars -- */
  .ud-live-rows { display: flex; flex-direction: column; gap: 8px; }
  .ud-live-row { display: flex; align-items: center; gap: 8px; }
  .ud-live-lbl { font-size: 11px; color: var(--t3); width: 52px; flex-shrink: 0; font-family: var(--ui); font-weight: 500; }
  .ud-live-bar { flex: 1; height: 6px; background: rgba(255,255,255,0.06); border-radius: 3px; overflow: hidden; }
  .ud-live-fill { height: 100%; border-radius: 2px; transition: width 0.3s ease; }
  .ud-live-pct { font-size: 12px; font-weight: 600; width: 44px; text-align: right; flex-shrink: 0; font-variant-numeric: tabular-nums; font-family: var(--mono); }
</style>

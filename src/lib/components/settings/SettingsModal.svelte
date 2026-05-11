<script lang="ts">
    import Modal from "$lib/shared/primitives/Modal.svelte";
    import ConfirmDialog from "$lib/shared/primitives/ConfirmDialog.svelte";
    import AccountTabContent from "$lib/components/settings/AccountTabContent.svelte";
    import { getVersion } from "@tauri-apps/api/app";
    import {
        activeModal,
        clearAllChatMessages,
        countAllChatMessages,
        chatHistorySizeBytes,
        purgeOldChatMessages,
    } from "$lib/stores/app";
    import {
        clearHistory as clearRestHistory,
        retentionSeconds,
        DEFAULT_CHAT_RETENTION,
    } from "$lib/modes/rest/stores";
    import { countHistory, purgeHistory } from "$lib/modes/rest/commands";
    import {
        workspaceMcpStatus,
        workspaceMcpStart,
        workspaceMcpStop,
        workspaceMcpRegister,
        workspaceMcpUnregister,
        workspaceMcpNewToken,
        type McpStatus,
    } from "$lib/modes/workspace/commands";
    import { mcpStatus as mcpStatusStore } from "$lib/modes/workspace/stores";
    import {
        getUpdateChannel,
        setUpdateChannel,
        type UpdateChannel,
    } from "$lib/utils/updater";
    import {
        settings,
        setSetting,
        appearance,
        saveAppearance,
    } from "$lib/stores/settings";
    import { applyTheme, getThemes, getTheme } from "$lib/utils/theme";
    import { showToast } from "$lib/shared/primitives/toast";
    import type { AppearanceConfig } from "$lib/types";
    import {
        testAiKey,
        getAiUsageStats,
        getAiProviderStats,
        resetAiUsage,
    } from "$lib/commands/ai";
    import type { AiUsageStat, AiProviderStat } from "$lib/types/ai";
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
        agentFetchCodexUsageLimits,
        agentGetUsageAnalytics,
    } from "$lib/modes/agent/commands";
    import type {
        ClaudePlugin,
        MarketplacePlugin,
        AgentContext,
        UsageAnalytics,
    } from "$lib/modes/agent/types";
    import {
        agentUsageLimits,
        agentUsageAuthStatus,
        agentSessionKey as agentSessionKeyStore,
        agentCodexToken as agentCodexTokenStore,
        agentFooterProvider as agentFooterProviderStore,
        loadAgentUsageLimits,
    } from "$lib/modes/agent/stores";
    import {
        ACCENT_PALETTE,
        THEME_PREVIEW_COLORS,
        FALLBACK_ACCENT_COLOR,
        USAGE_DANGER,
        USAGE_WARN,
    } from "$lib/shared/constants/colors";
    import {
        PROVIDERS as AI_PROVIDER_REGISTRY,
        getDefaultModelFor,
        type ProviderId,
    } from "$lib/shared/ai/providers";
    import { isMac, mod } from "$lib/utils/platform";

    // 'general' is app-wide (currently: Proxy — applies to REST + AI + GitHub
    // + Updater + ClickHouse). 'rest' holds REST-only knobs (timeout, redirects,
    // SSL verify, max body).
    type SettingsTab =
        | "account"
        | "general"
        | "appearance"
        | "shortcuts"
        | "ai"
        | "rest"
        | "agent"
        | "workspace"
        | "about";

    let show = $state(false);
    let activeTab = $state<SettingsTab>("general");
    let appVersion = $state("");
    let updateChannel = $state<UpdateChannel>(getUpdateChannel());

    function onPreReleaseToggle(e: Event) {
        const checked = (e.currentTarget as HTMLInputElement).checked;
        updateChannel = checked ? "pre" : "stable";
        setUpdateChannel(updateChannel);
    }

    getVersion()
        .then((v) => {
            appVersion = v;
        })
        .catch(() => {
            appVersion = "";
        });

    $effect(() => {
        const modal = $activeModal;
        if (
            modal === "settings" ||
            modal === "settings:account" ||
            modal === "settings:ai" ||
            modal === "settings:agent" ||
            modal === "settings:agent:usage" ||
            modal === "settings:agent:contexts" ||
            modal === "settings:agent:plugins" ||
            modal === "settings:workspace"
        ) {
            show = true;
            if (modal === "settings:account") activeTab = "account";
            if (modal === "settings:ai") activeTab = "ai";
            if (modal === "settings:agent") activeTab = "agent";
            if (modal === "settings:agent:usage") {
                activeTab = "agent";
                agentSubTab = "usage";
            }
            if (modal === "settings:agent:contexts") {
                activeTab = "agent";
                agentSubTab = "contexts";
            }
            if (modal === "settings:agent:plugins") {
                activeTab = "agent";
                agentSubTab = "plugins";
            }
            if (modal === "settings:workspace") activeTab = "workspace";
        } else {
            show = false;
        }
    });

    $effect(() => {
        if (
            !show &&
            ($activeModal === "settings" ||
                $activeModal === "settings:ai" ||
                $activeModal === "settings:agent" ||
                $activeModal === "settings:agent:usage" ||
                $activeModal === "settings:agent:contexts" ||
                $activeModal === "settings:agent:plugins" ||
                $activeModal === "settings:workspace")
        ) {
            activeModal.set(null);
        }
    });

    // --- General ---
    let timeout = $derived(Number($settings["request_timeout"] ?? "30000"));
    let followRedirects = $derived(
        ($settings["follow_redirects"] ?? "true") === "true",
    );
    let sslVerification = $derived(
        ($settings["ssl_verification"] ?? "true") === "true",
    );
    let maxResponseSize = $derived(
        Number($settings["max_response_size"] ?? "10"),
    );

    // --- Editor ---

    // --- Proxy ---
    let proxyUrl = $derived($settings["proxy_url"] ?? "");
    let proxyAuth = $derived(($settings["proxy_auth"] ?? "false") === "true");
    let proxyUsername = $derived($settings["proxy_username"] ?? "");
    let proxyPassword = $derived($settings["proxy_password"] ?? "");

    // --- Logs ---
    let logDir = $state("");

    $effect(() => {
        if (activeTab === "general" && !logDir) {
            import("$lib/commands/logs").then(({ getLogDir }) =>
                getLogDir()
                    .then((p) => {
                        logDir = p;
                    })
                    .catch(() => {}),
            );
        }
    });

    async function handleOpenLogFolder() {
        try {
            const { openLogFolder } = await import("$lib/commands/logs");
            await openLogFolder();
        } catch {
            showToast("Failed to open log folder", "error");
        }
    }

    // --- Chat History (General) ---
    // Combined retention + size + clear for REST request history (DB) AND
    // AI Assistance chat history (per-mode, localStorage).
    let chatRetention = $derived(
        ($settings["chat_history_retention"] ??
            DEFAULT_CHAT_RETENTION) as string,
    );
    let restHistoryCount = $state<number>(0);
    let aiChatCount = $state<number>(0);
    let aiChatBytes = $state<number>(0);
    let showClearChatHistoryConfirm = $state(false);

    async function refreshHistorySizes() {
        try {
            restHistoryCount = await countHistory();
        } catch {
            restHistoryCount = 0;
        }
        aiChatCount = countAllChatMessages();
        aiChatBytes = chatHistorySizeBytes();
    }

    // Refresh sizes whenever the General tab becomes visible (cheap; one COUNT(*)).
    $effect(() => {
        if (activeTab === "general" && show) {
            refreshHistorySizes();
        }
    });

    function formatBytes(bytes: number): string {
        if (!bytes) return "0 B";
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
        return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
    }

    async function handleChatRetentionChange(value: string) {
        await setSetting("chat_history_retention", value);
        // Apply immediately: purge anything beyond the new window so the size
        // reflects the user's choice without waiting for the next app load.
        const seconds = retentionSeconds(value);
        if (seconds !== null) {
            try {
                await purgeHistory(seconds);
            } catch {
                /* non-fatal */
            }
            purgeOldChatMessages(seconds * 1000);
        }
        await refreshHistorySizes();
    }

    async function handleClearChatHistory() {
        try {
            await clearRestHistory();
            clearAllChatMessages();
            await refreshHistorySizes();
            showToast("History cleared", "success");
        } catch {
            showToast("Failed to clear history", "error");
        }
    }

    // ── Workspace MCP server ───────────────────────────────────────────
    // Settings keys: workspace_mcp_port (default 7421), workspace_mcp_token
    // (auto-generated). On enable: start server + write to ~/.claude
    // /settings.json. On disable: stop + remove our entry. The token
    // is regenerable from the UI when the server is stopped.
    let mcpStatus = $state<McpStatus>({ running: false, port: null });
    let mcpPort = $derived(
        Number($settings["workspace_mcp_port"] ?? "7421") || 7421,
    );
    let mcpToken = $derived(($settings["workspace_mcp_token"] ?? "") as string);
    let showMcpToken = $state(false);

    async function refreshMcpStatus() {
        try {
            mcpStatus = await workspaceMcpStatus();
            mcpStatusStore.set(mcpStatus);
        } catch { /* ignore */ }
    }

    /** Lazy-init token + load status when the Workspace tab opens.
     *  The Rust setup() task auto-starts the server on app boot, so
     *  by the time this fires we usually just refresh status. The
     *  token-init branch only matters for legacy installs that
     *  somehow wound up without one. */
    $effect(() => {
        if (activeTab === "workspace" && show) {
            refreshMcpStatus();
            if (!mcpToken) {
                workspaceMcpNewToken(mcpPort).catch(() => {});
            }
        }
    });

    async function handleMcpToggle(next: boolean) {
        try {
            if (next) {
                let token = mcpToken;
                if (!token) {
                    // workspaceMcpNewToken persists the value internally.
                    token = await workspaceMcpNewToken(mcpPort);
                }
                await setSetting("workspace_mcp_port", String(mcpPort));
                mcpStatus = await workspaceMcpStart(mcpPort, token);
                mcpStatusStore.set(mcpStatus);
                await workspaceMcpRegister(mcpStatus.port ?? mcpPort, token);
                // Persist the desire — drives auto-start on next app boot.
                await setSetting("workspace_mcp_enabled", "true");
                showToast(`MCP server running on :${mcpStatus.port}`, "success");
            } else {
                mcpStatus = await workspaceMcpStop();
                mcpStatusStore.set(mcpStatus);
                await workspaceMcpUnregister();
                await setSetting("workspace_mcp_enabled", "false");
                showToast("MCP server stopped", "success");
            }
        } catch (e) {
            showToast(`MCP toggle failed: ${e}`, "error");
            await refreshMcpStatus();
        }
    }

    async function handleRotateMcpToken() {
        try {
            // workspaceMcpNewToken atomically: generates a new token,
            // persists it, and rewrites ~/.claude.json if an entry
            // exists. Server restart isn't required — the running
            // instance still uses the old token until next start, but
            // ~/.claude.json is back in sync with whatever's persisted
            // for the next start cycle.
            await workspaceMcpNewToken(mcpStatus.port ?? mcpPort);
            if (mcpStatus.running) {
                showToast(
                    "Token rotated. Restart the server (toggle off + on) for the new token to take effect.",
                    "success",
                );
            } else {
                showToast("Token rotated", "success");
            }
        } catch (e) {
            showToast(`Rotate failed: ${e}`, "error");
        }
    }

    async function handleCopyMcpToken() {
        try {
            await navigator.clipboard.writeText(mcpToken);
            showToast("Token copied", "success");
        } catch { /* ignore */ }
    }

    // AI Assistance state
    let aiSubTab = $state<"config" | "usage">("config");
    let aiProvider = $state<string>("claude");
    let aiApiKey = $state("");
    let showAiKey = $state(false);
    let aiTestStatus = $state<"idle" | "testing" | "success" | "error">("idle");
    let aiTestMessage = $state("");
    let aiUsageStats = $state<AiUsageStat[]>([]);
    let aiProviderStats = $state<AiProviderStat[]>([]);
    let showResetConfirm = $state(false);

    // Provider/model metadata is sourced from the shared registry mirror
    // (`$lib/shared/ai/providers`) so this component and the Rust backend
    // stay in lockstep. Adding a provider/model is one entry there, zero
    // edits here.
    const FALLBACK_PROVIDER_CONFIG = AI_PROVIDER_REGISTRY[0];
    let currentProviderConfig = $derived(
        getDefaultModelFor(aiProvider as ProviderId) ??
            FALLBACK_PROVIDER_CONFIG,
    );
    let aiHasKey = $derived(!!$settings[`ai_api_key_${aiProvider}`]?.trim());

    // --- Appearance ---
    let currentTheme = $derived($appearance.theme || "dark-glass");
    let accentColor = $derived(
        $appearance.accentColor || FALLBACK_ACCENT_COLOR,
    );

    // Re-export for template usage (Svelte each blocks resolve from script scope).
    const ACCENT_COLORS = ACCENT_PALETTE;

    const THEME_DESCRIPTIONS: Record<string, string> = {
        "dark-glass": "Transparent with macOS vibrancy",
        "dark-solid": "Opaque dark with purple tints",
        midnight: "Pure black, zero distraction",
        nord: "Arctic blue-gray palette",
        light: "Warm off-white, easy on the eyes",
    };

    // Clear any previously saved zoom to prevent cursor issues
    $effect(() => {
        document.body.style.zoom = "";
    });

    // Sidebar items. `kind: 'header'` rows render as a small uppercase
    // section label; `kind: 'tab'` rows are the clickable tabs.
    //
    // `icon` is the inner SVG markup (paths/circles/lines) — rendered with
    // `{@html}` inside a fixed-attr <svg>. The mode icons (Agent / REST /
    // etc.) are kept identical to the main Sidebar so users see the same
    // glyph in both places.
    type TabsItem =
        | { kind: "tab"; key: SettingsTab; label: string; icon: string }
        | { kind: "header"; label: string };

    const tabs: TabsItem[] = [
        {
            kind: "tab",
            key: "account",
            label: "Account",
            icon: '<path d="M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2"/><circle cx="12" cy="7" r="4"/>',
        },
        {
            kind: "tab",
            key: "general",
            label: "General",
            icon: '<path d="M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z"/>',
        },
        {
            kind: "tab",
            key: "appearance",
            label: "Appearance",
            icon: '<path d="M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z"/>',
        },
        {
            kind: "tab",
            key: "shortcuts",
            label: "Shortcuts",
            icon: '<path d="M18 3a3 3 0 00-3 3v12a3 3 0 003 3 3 3 0 003-3 3 3 0 00-3-3H6a3 3 0 00-3 3 3 3 0 003 3 3 3 0 003-3V6a3 3 0 00-3-3 3 3 0 00-3 3 3 3 0 003 3h12a3 3 0 003-3 3 3 0 00-3-3z"/>',
        },
        {
            kind: "tab",
            key: "ai",
            label: "AI Assistance",
            icon: '<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/>',
        },
        { kind: "header", label: "Modes" },
        // Order + glyphs MUST match the main Sidebar (Agent, REST, ...).
        {
            kind: "tab",
            key: "agent",
            label: "Agent",
            icon: '<path d="M12 3l1.6 4.8L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.2L12 3z"/><path d="M18.5 14l.9 2.6 2.6.9-2.6.9-.9 2.6-.9-2.6-2.6-.9 2.6-.9.9-2.6z"/>',
        },
        {
            kind: "tab",
            key: "rest",
            label: "REST",
            icon: '<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z"/>',
        },
        {
            kind: "tab",
            key: "workspace",
            label: "Workspace",
            // 2×2 grid — same identity glyph used in the main Sidebar.
            icon: '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
        },
        {
            kind: "tab",
            key: "about",
            label: "About",
            icon: '<path d="M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10z"/>',
        },
    ];

    // Platform-aware shortcut list. `m` = "Cmd" on macOS, "Ctrl" elsewhere.
    // Fullscreen + Minimize differ by OS so they're branched.
    const m = mod();
    const SHORTCUTS: { desc: string; keys: string[] }[] = [
        { desc: "Switch to Agent mode", keys: [m, "1"] },
        { desc: "Switch to REST mode", keys: [m, "2"] },
        { desc: "Switch to SQL mode", keys: [m, "3"] },
        { desc: "Switch to NoSQL mode", keys: [m, "4"] },
        { desc: "Send request / Execute query", keys: [m, "Enter"] },
        { desc: "Save", keys: [m, "S"] },
        { desc: "Close active tab", keys: [m, "W"] },
        { desc: "Toggle nav sidebar", keys: [m, "B"] },
        { desc: "Toggle AI assistant", keys: [m, "L"] },
        { desc: "Show shortcuts overlay", keys: [m, "/"] },
        {
            desc: "Toggle fullscreen",
            keys: isMac() ? ["Cmd", "Ctrl", "F"] : ["F11"],
        },
        ...(isMac() ? [{ desc: "Minimize window", keys: ["Cmd", "M"] }] : []),
        { desc: "Close modal / overlay", keys: ["Esc"] },
    ];

    async function handleSettingChange(key: string, value: string) {
        await setSetting(key, value);
        // Sync the matching agent-mode store and refresh the footer
        // chip whenever a usage-tracking setting changes.
        if (key === "agent_session_key") {
            agentSessionKeyStore.set(value);
            if (value) {
                loadAgentUsageLimits();
            } else {
                agentUsageLimits.set(null);
                agentUsageAuthStatus.set({ state: "unconfigured", message: "" });
            }
        } else if (key === "agent_codex_access_token") {
            agentCodexTokenStore.set(value);
            if (value) {
                loadAgentUsageLimits();
            } else {
                agentUsageLimits.set(null);
                agentUsageAuthStatus.set({ state: "unconfigured", message: "" });
            }
        } else if (key === "agent_footer_usage_provider") {
            if (value === "claude" || value === "codex") {
                agentFooterProviderStore.set(value);
            }
            // Provider switch — clear stale chip state and re-fetch using
            // the new provider. The dispatcher in loadAgentUsageLimits
            // handles the unconfigured case.
            agentUsageLimits.set(null);
            agentUsageAuthStatus.set({ state: "unconfigured", message: "" });
            loadAgentUsageLimits();
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
        document.documentElement.style.setProperty("--acc", color);
        const config: AppearanceConfig = {
            theme: currentTheme,
            accentColor: color,
        };
        appearance.set(config);
        await saveAppearance(config);
    }

    function handleClose() {
        activeTab = "general";
    }

    async function loadAiSettings() {
        const s = $settings;
        aiProvider = s["ai_provider"] || "claude";
        // Load key for current provider (fallback to legacy ai_api_key for claude)
        aiApiKey = s[`ai_api_key_${aiProvider}`] || "";
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
            showToast("Enter an API key first", "error");
            return;
        }
        aiTestStatus = "testing";
        aiTestMessage = "";
        try {
            const msg = await testAiKey(key, aiProvider);
            aiTestStatus = "success";
            aiTestMessage = msg;
            await handleSettingChange(`ai_api_key_${aiProvider}`, key);
            await handleSettingChange("ai_provider", aiProvider);
            // Also save to legacy key for backward compat
            if (aiProvider === "claude") {
                // Legacy key no longer used — per-provider keys only
            }
            showToast("API key verified and saved", "success");
        } catch (e: any) {
            aiTestStatus = "error";
            aiTestMessage =
                typeof e === "string" ? e : e.message || "Test failed";
            showToast("Invalid API key — not saved", "error");
        }
    }

    async function handleProviderChange(provider: string) {
        aiProvider = provider;
        await handleSettingChange("ai_provider", provider);
        // Load the key for this provider
        const s = $settings;
        aiApiKey = s[`ai_api_key_${provider}`] || "";
        aiTestStatus = "idle";
        aiTestMessage = "";
    }

    async function handleRemoveAiKey() {
        await handleSettingChange(`ai_api_key_${aiProvider}`, "");
        if (aiProvider === "claude") {
            // Legacy key no longer used
        }
        aiApiKey = "";
        aiTestStatus = "idle";
        aiTestMessage = "";
        showToast("API key removed", "success");
    }

    async function handleResetUsage() {
        try {
            await resetAiUsage();
            aiUsageStats = [];
            showResetConfirm = false;
            showToast("Usage stats reset", "success");
        } catch {
            showToast("Failed to reset stats", "error");
        }
    }

    function formatTokens(n: number): string {
        if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
        if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
        return n.toString();
    }

    function estimateCost(inputTokens: number, outputTokens: number): string {
        const cost =
            (inputTokens / 1_000_000) * 1.0 + (outputTokens / 1_000_000) * 5.0;
        if (cost < 0.01) return "<$0.01";
        return "$" + cost.toFixed(2);
    }

    function formatModelName(model: string): string {
        const names: Record<string, string> = {
            "claude-haiku-4-5-20251001": "Haiku 4.5",
            "claude-sonnet-4-6-20250514": "Sonnet 4.6",
            "claude-opus-4-7-20250514": "Opus 4.7",
            "llama-3.3-70b-versatile": "Llama 3.3 70B",
            "meta-llama/llama-4-scout-17b-16e-instruct": "Llama 4 Scout 17B",
            "mistral-small-latest": "Mistral Small",
            "mistral-large-latest": "Mistral Large 3",
            "gpt-4.1-mini": "GPT-4.1 Mini",
            "nvidia/nemotron-3-super-120b-a12b": "Nemotron 3 Super 120B",
            "meta-llama/llama-3.3-70b-instruct:free": "Llama 3.3 70B",
            "gemini-2.5-flash": "Gemini 2.5 Flash",
            "qwen/qwen3-32b": "Qwen3 32B",
        };
        return names[model] || model;
    }

    let aiSettingsLoaded = false;
    $effect(() => {
        if (activeTab === "ai" && show && !aiSettingsLoaded) {
            aiSettingsLoaded = true;
            loadAiSettings();
        }
        if (!show) {
            aiSettingsLoaded = false;
        }
    });

    // --- Agent Tab ---
    type AgentSubTab = "general" | "plugins" | "contexts" | "usage";
    let agentSubTab = $state<AgentSubTab>("general");

    // Agent General
    let agentSessionKey = $derived($settings["agent_session_key"] ?? "");
    let agentCodexToken = $derived($settings["agent_codex_access_token"] ?? "");
    let agentFooterProvider = $derived(
        $settings["agent_footer_usage_provider"] ?? "claude",
    );
    let agentSoundEnabled = $derived(
        ($settings["agent_sound_enabled"] ?? "true") === "true",
    );
    let agentDockBounceEnabled = $derived(
        ($settings["agent_dock_bounce_enabled"] ?? "true") === "true",
    );
    let agentRefreshMins = $derived(
        Number($settings["agent_refresh_mins"] ?? "5"),
    );

    // Agent key management
    let agentKeyInput = $state("");
    let showAgentKey = $state(false);
    let agentKeyTestStatus = $state<"idle" | "testing" | "success" | "error">(
        "idle",
    );
    let agentKeyTestMessage = $state("");
    let codexTokenInput = $state("");
    let showCodexToken = $state(false);
    let codexTokenTestStatus = $state<"idle" | "testing" | "success" | "error">(
        "idle",
    );
    let codexTokenTestMessage = $state("");

    // Agent Usage Stats
    let agentUsageData = $state<UsageAnalytics | null>(null);
    let agentUsageLoading = $state(false);
    let agentUsageDays = $state(7);

    const REFRESH_OPTIONS = [
        { value: 5, label: "5 minutes" },
        { value: 15, label: "15 minutes" },
        { value: 30, label: "30 minutes" },
        { value: 60, label: "1 hour" },
    ];

    function agentLiveColor(pct: number): string {
        if (pct > 80) return USAGE_DANGER;
        if (pct > 50) return USAGE_WARN;
        return "var(--acc)";
    }

    function agentFormatCost(n: number) {
        return n < 0.01 && n > 0 ? "<$0.01" : "$" + n.toFixed(2);
    }
    function agentFormatTokens(n: number) {
        return n >= 1_000_000
            ? (n / 1_000_000).toFixed(1) + "M"
            : n >= 1_000
              ? (n / 1_000).toFixed(1) + "K"
              : String(n);
    }
    function agentDecodeName(s: string) {
        return s.replace(/-/g, "/");
    }

    async function handleSaveAgentKey() {
        const key = agentKeyInput.trim();
        if (!key) {
            showToast("Enter a session key first", "error");
            return;
        }
        agentKeyTestStatus = "testing";
        agentKeyTestMessage = "";
        try {
            await agentFetchUsageLimits(key);
            agentKeyTestStatus = "success";
            agentKeyTestMessage = "Session key verified";
            await handleSettingChange("agent_session_key", key);
            showToast("Session key verified and saved", "success");
        } catch (e: any) {
            agentKeyTestStatus = "error";
            agentKeyTestMessage =
                typeof e === "string"
                    ? e
                    : e.message || "Invalid or expired session key";
            showToast("Invalid session key — not saved", "error");
        }
    }

    async function handleRemoveAgentKey() {
        await handleSettingChange("agent_session_key", "");
        agentKeyInput = "";
        agentKeyTestStatus = "idle";
        agentKeyTestMessage = "";
        showToast("Session key removed", "success");
    }

    async function handleSaveCodexToken() {
        const token = codexTokenInput.trim();
        if (!token) return;
        codexTokenTestStatus = "testing";
        codexTokenTestMessage = "";
        try {
            await agentFetchCodexUsageLimits(token);
            codexTokenTestStatus = "success";
            codexTokenTestMessage = "Codex token verified";
            await handleSettingChange("agent_codex_access_token", token);
            showToast("Codex token verified and saved", "success");
        } catch (e: any) {
            codexTokenTestStatus = "error";
            codexTokenTestMessage =
                typeof e === "string"
                    ? e
                    : e.message || "Invalid or expired Codex token";
            showToast("Invalid Codex token — not saved", "error");
        }
    }

    async function handleRemoveCodexToken() {
        await handleSettingChange("agent_codex_access_token", "");
        codexTokenInput = "";
        codexTokenTestStatus = "idle";
        codexTokenTestMessage = "";
        showToast("Codex access token removed", "success");
    }

    /** Auto-commit on blur / Enter for the Claude session-key field —
     *  fires only when the input has a non-empty value that differs from
     *  the currently-saved key. Keeps the user out of "click Save after
     *  every paste" purgatory. Verification + save happens inside
     *  handleSaveAgentKey, which already updates the inline status. */
    function commitAgentKeyIfChanged() {
        const v = agentKeyInput.trim();
        if (!v || v === agentSessionKey) return;
        handleSaveAgentKey();
    }

    /** Same auto-commit pattern for the Codex access-token field. */
    function commitCodexTokenIfChanged() {
        const v = codexTokenInput.trim();
        if (!v || v === agentCodexToken) return;
        handleSaveCodexToken();
    }

    async function loadAgentUsage() {
        agentUsageLoading = true;
        try {
            agentUsageData = await agentGetUsageAnalytics(agentUsageDays);
        } catch {
            agentUsageData = null;
        }
        agentUsageLoading = false;
    }

    function selectAgentUsageDays(d: number) {
        agentUsageDays = d;
        loadAgentUsage();
    }

    // Agent Plugins
    type PluginView = "installed" | "marketplace";
    let pluginView = $state<PluginView>("installed");
    let installedPlugins = $state<ClaudePlugin[]>([]);
    let marketplacePlugins = $state<MarketplacePlugin[]>([]);
    let pluginSearchQuery = $state("");
    let filteredMarketplacePlugins = $derived(
        pluginSearchQuery.trim()
            ? marketplacePlugins.filter(
                  (p) =>
                      p.name
                          .toLowerCase()
                          .includes(pluginSearchQuery.toLowerCase()) ||
                      p.description
                          .toLowerCase()
                          .includes(pluginSearchQuery.toLowerCase()),
              )
            : marketplacePlugins,
    );

    // Agent Contexts
    let agentContexts = $state<AgentContext[]>([]);
    let editingContext = $state<AgentContext | null>(null);
    let editContextName = $state("");
    let editContextContent = $state("");
    let isNewContext = $state(false);
    let deleteConfirmId = $state<string | null>(null);

    async function loadAgentPlugins() {
        try {
            installedPlugins = await agentGetPlugins();
        } catch {
            installedPlugins = [];
        }
        try {
            marketplacePlugins = await agentGetMarketplacePlugins();
        } catch {
            marketplacePlugins = [];
        }
    }

    async function loadAgentContexts() {
        try {
            agentContexts = await agentListContexts();
        } catch {
            agentContexts = [];
        }
    }

    async function handleTogglePlugin(name: string, enabled: boolean) {
        try {
            await agentTogglePlugin(name, enabled);
            installedPlugins = installedPlugins.map((p) =>
                p.name === name ? { ...p, enabled } : p,
            );
            showToast(`Plugin ${enabled ? "enabled" : "disabled"}`, "success");
        } catch {
            showToast("Failed to toggle plugin", "error");
        }
    }

    async function handleUninstallPlugin(name: string, marketplace: string) {
        try {
            await agentUninstallPlugin(name, marketplace);
            installedPlugins = installedPlugins.filter((p) => p.name !== name);
            marketplacePlugins = marketplacePlugins.map((p) =>
                p.name === name ? { ...p, installed: false } : p,
            );
            showToast("Plugin uninstalled", "success");
        } catch {
            showToast("Failed to uninstall plugin", "error");
        }
    }

    async function handleInstallPlugin(name: string, marketplace: string) {
        try {
            await agentInstallPlugin(name, marketplace);
            marketplacePlugins = marketplacePlugins.map((p) =>
                p.name === name ? { ...p, installed: true } : p,
            );
            await loadAgentPlugins();
            showToast("Plugin installed", "success");
        } catch {
            showToast("Failed to install plugin", "error");
        }
    }

    function startEditContext(ctx: AgentContext) {
        editingContext = ctx;
        editContextName = ctx.name;
        editContextContent = ctx.content;
        isNewContext = false;
    }

    function startNewContext() {
        editingContext = null;
        editContextName = "";
        editContextContent = "";
        isNewContext = true;
    }

    function cancelEditContext() {
        editingContext = null;
        isNewContext = false;
        editContextName = "";
        editContextContent = "";
    }

    async function handleSaveAgentContext() {
        const name = editContextName.trim();
        const content = editContextContent.trim();
        if (!name || !content) {
            showToast("Name and content are required", "error");
            return;
        }
        try {
            await agentSaveContext({ id: editingContext?.id, name, content });
            await loadAgentContexts();
            cancelEditContext();
            showToast("Context saved", "success");
        } catch {
            showToast("Failed to save context", "error");
        }
    }

    async function handleDeleteAgentContext(id: string) {
        try {
            await agentDeleteContext(id);
            agentContexts = agentContexts.filter((c) => c.id !== id);
            deleteConfirmId = null;
            if (editingContext?.id === id) cancelEditContext();
            showToast("Context deleted", "success");
        } catch {
            showToast("Failed to delete context", "error");
        }
    }

    let agentSettingsLoaded = false;
    $effect(() => {
        if (activeTab === "agent" && show && !agentSettingsLoaded) {
            agentSettingsLoaded = true;
            loadAgentPlugins();
            loadAgentContexts();
            agentKeyInput = $settings["agent_session_key"] ?? "";
            codexTokenInput = $settings["agent_codex_access_token"] ?? "";
        }
        if (!show) {
            agentSettingsLoaded = false;
            agentSubTab = "general";
            cancelEditContext();
            agentKeyTestStatus = "idle";
            agentKeyTestMessage = "";
            showAgentKey = false;
            showCodexToken = false;
        }
    });

    let agentUsageLoaded = false;
    $effect(() => {
        if (agentSubTab === "usage" && show && !agentUsageLoaded) {
            agentUsageLoaded = true;
            loadAgentUsage();
        }
        if (agentSubTab !== "usage") {
            agentUsageLoaded = false;
        }
    });
</script>

<Modal
    bind:show
    title="Settings"
    width={activeTab === "agent" && agentSubTab === "usage"
        ? "min(1180px, 92vw)"
        : "min(1040px, 92vw)"}
    onclose={handleClose}
>
    <div class="stg-layout">
        <!-- Tab sidebar -->
        <div class="stg-tabs">
            {#each tabs as item}
                {#if item.kind === "header"}
                    <span class="stg-tab-section">{item.label}</span>
                {:else}
                    <button
                        class="stg-tab"
                        class:active={activeTab === item.key}
                        onclick={() => (activeTab = item.key)}
                    >
                        <svg
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="1.5"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                        >
                            {@html item.icon}
                        </svg>
                        {item.label}
                    </button>
                {/if}
            {/each}
        </div>

        <!-- Content pane -->
        <div class="stg-content">
            {#if activeTab === "account"}
                <AccountTabContent />
            {:else if activeTab === "general"}
                <div class="stg-card-stack">
                    <!-- Proxy card -->
                    <section class="stg-card">
                        <header class="stg-card-hd">
                            <span class="stg-card-icon" aria-hidden="true">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="14"
                                    height="14"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                >
                                    <circle cx="12" cy="12" r="10" /><path
                                        d="M2 12h20"
                                    /><path
                                        d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"
                                    />
                                </svg>
                            </span>
                            <div class="stg-card-titles">
                                <h3 class="stg-card-title">Proxy</h3>
                                <p class="stg-card-sub">
                                    Applies system-wide. Leave blank to connect
                                    directly.
                                </p>
                            </div>
                        </header>
                        <div class="stg-card-body">
                            <div class="stg-card-row">
                                <label class="stg-card-row-label"
                                    >Proxy URL</label
                                >
                                <input
                                    class="stg-input stg-card-input-lg"
                                    type="text"
                                    placeholder="http://proxy:8080"
                                    value={proxyUrl}
                                    onchange={(e) =>
                                        handleSettingChange(
                                            "proxy_url",
                                            e.currentTarget.value,
                                        )}
                                />
                            </div>
                            <div class="stg-card-row">
                                <label class="stg-card-row-label"
                                    >Proxy Authentication</label
                                >
                                <label class="stg-toggle">
                                    <input
                                        type="checkbox"
                                        checked={proxyAuth}
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "proxy_auth",
                                                String(e.currentTarget.checked),
                                            )}
                                    />
                                    <span class="stg-toggle-slider"></span>
                                </label>
                            </div>
                            {#if proxyAuth}
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Username</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="text"
                                        value={proxyUsername}
                                        placeholder="username"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "proxy_username",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Password</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="password"
                                        value={proxyPassword}
                                        placeholder="password"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "proxy_password",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                            {/if}
                        </div>
                    </section>

                    <!-- Logs card -->
                    <section class="stg-card">
                        <header class="stg-card-hd">
                            <span class="stg-card-icon" aria-hidden="true">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="14"
                                    height="14"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                >
                                    <path
                                        d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"
                                    /><polyline points="14 2 14 8 20 8" /><line
                                        x1="9"
                                        y1="13"
                                        x2="15"
                                        y2="13"
                                    /><line x1="9" y1="17" x2="15" y2="17" />
                                </svg>
                            </span>
                            <div class="stg-card-titles">
                                <h3 class="stg-card-title">Logs</h3>
                                <p class="stg-card-sub">
                                    Per-hour log files organized by day. Logs
                                    older than 30 days are removed
                                    automatically.
                                </p>
                            </div>
                        </header>
                        <div class="stg-card-body">
                            <div class="stg-card-row">
                                <label class="stg-card-row-label"
                                    >Log Folder</label
                                >
                                <div class="log-path-row">
                                    <span class="log-path">{logDir}</span>
                                    <button
                                        class="log-open-btn"
                                        onclick={handleOpenLogFolder}
                                        title="Open in file manager"
                                        aria-label="Open log folder"
                                    >
                                        <svg
                                            width="14"
                                            height="14"
                                            viewBox="0 0 24 24"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="2"
                                            stroke-linecap="round"
                                            stroke-linejoin="round"
                                            aria-hidden="true"
                                        >
                                            <path
                                                d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"
                                            />
                                        </svg>
                                    </button>
                                </div>
                            </div>
                        </div>
                    </section>

                    <!-- Chat History card -->
                    <section class="stg-card">
                        <header class="stg-card-hd">
                            <span class="stg-card-icon" aria-hidden="true">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="14"
                                    height="14"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                >
                                    <path
                                        d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"
                                    />
                                </svg>
                            </span>
                            <div class="stg-card-titles">
                                <h3 class="stg-card-title">Chat History</h3>
                                <p class="stg-card-sub">
                                    Applies to the History section
                                    (request/response log) and the AI Assistance
                                    chat across all modes.
                                </p>
                            </div>
                        </header>

                        <!-- Stat strip: prominent counts, scannable. -->
                        <div class="stg-card-stats">
                            <div class="stg-card-stat">
                                <span class="stg-card-stat-num"
                                    >{restHistoryCount.toLocaleString()}</span
                                >
                                <span class="stg-card-stat-lbl"
                                    >Request{restHistoryCount === 1
                                        ? ""
                                        : "s"}</span
                                >
                            </div>
                            <div
                                class="stg-card-stat-divider"
                                aria-hidden="true"
                            ></div>
                            <div class="stg-card-stat">
                                <span class="stg-card-stat-num"
                                    >{aiChatCount.toLocaleString()}</span
                                >
                                <span class="stg-card-stat-lbl"
                                    >AI message{aiChatCount === 1
                                        ? ""
                                        : "s"}</span
                                >
                            </div>
                            <div
                                class="stg-card-stat-divider"
                                aria-hidden="true"
                            ></div>
                            <div class="stg-card-stat">
                                <span class="stg-card-stat-num"
                                    >{formatBytes(aiChatBytes)}</span
                                >
                                <span class="stg-card-stat-lbl">Storage</span>
                            </div>
                        </div>

                        <div class="stg-card-body">
                            <div class="stg-card-row">
                                <label class="stg-card-row-label"
                                    >Auto-clear after</label
                                >
                                <select
                                    class="stg-input stg-card-input-lg"
                                    value={chatRetention}
                                    onchange={(e) =>
                                        handleChatRetentionChange(
                                            e.currentTarget.value,
                                        )}
                                >
                                    <option value="24h">24 Hours</option>
                                    <option value="7d">7 Days</option>
                                    <option value="30d"
                                        >30 Days (default)</option
                                    >
                                    <option value="1y">1 Year</option>
                                    <option value="never">Never</option>
                                </select>
                            </div>
                            <div class="stg-card-row stg-card-row-action">
                                <div class="stg-card-row-action-text">
                                    <span class="stg-card-row-label"
                                        >Clear all chat history</span
                                    >
                                    <span class="stg-card-row-help"
                                        >Removes every request log and AI
                                        conversation. This cannot be undone.</span
                                    >
                                </div>
                                <button
                                    class="stg-card-danger-btn"
                                    onclick={() =>
                                        (showClearChatHistoryConfirm = true)}
                                >
                                    <svg
                                        viewBox="0 0 24 24"
                                        width="12"
                                        height="12"
                                        fill="none"
                                        stroke="currentColor"
                                        stroke-width="2"
                                        stroke-linecap="round"
                                        stroke-linejoin="round"
                                        aria-hidden="true"
                                    >
                                        <polyline points="3 6 5 6 21 6" /><path
                                            d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"
                                        /><path d="M10 11v6" /><path
                                            d="M14 11v6"
                                        />
                                    </svg>
                                    <span>Clear History</span>
                                </button>
                            </div>
                        </div>
                    </section>

                </div>
            {:else if activeTab === "workspace"}
                <div class="stg-card-stack">
                    <section class="stg-card">
                        <header class="stg-card-hd">
                            <span class="stg-card-icon" aria-hidden="true">
                                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
                            </span>
                            <div class="stg-card-titles">
                                <h3 class="stg-card-title">MCP Server</h3>
                                <p class="stg-card-sub">
                                    Local MCP server that lets agents (Claude Code, etc.) read and edit your
                                    Notes and Boards via tool calls. Auto-registers in <span class="stg-card-mono">~/.claude/settings.json</span>
                                    when enabled.
                                </p>
                            </div>
                            <span class="stg-card-pill" class:on={mcpStatus.running}>
                                <span class="stg-card-pill-dot"></span>
                                {mcpStatus.running ? `Running · :${mcpStatus.port}` : 'Stopped'}
                            </span>
                        </header>

                        <div class="stg-card-body">
                            <div class="stg-card-row">
                                <label class="stg-card-row-label">Enable MCP server</label>
                                <label class="stg-toggle">
                                    <input type="checkbox" checked={mcpStatus.running} onchange={(e) => handleMcpToggle(e.currentTarget.checked)} />
                                    <span class="stg-toggle-slider"></span>
                                </label>
                            </div>

                            <div class="stg-card-row">
                                <label class="stg-card-row-label">Port</label>
                                <input
                                    class="stg-input"
                                    type="number"
                                    min="1024"
                                    max="65535"
                                    value={mcpPort}
                                    disabled={mcpStatus.running}
                                    onchange={(e) => mcpPort = Number(e.currentTarget.value) || 7421}
                                />
                            </div>

                            <div class="stg-card-row">
                                <label class="stg-card-row-label">Auth token</label>
                                <div class="stg-card-token-row">
                                    <input class="stg-input stg-card-input-lg" type={showMcpToken ? 'text' : 'password'} value={mcpToken} readonly />
                                    <button class="stg-card-mini-btn" onclick={() => showMcpToken = !showMcpToken} title={showMcpToken ? 'Hide' : 'Show'}>
                                        {showMcpToken ? 'Hide' : 'Show'}
                                    </button>
                                    <button class="stg-card-mini-btn" onclick={handleCopyMcpToken} title="Copy">Copy</button>
                                    <button class="stg-card-mini-btn" onclick={handleRotateMcpToken} disabled={mcpStatus.running} title="Generate new token">Rotate</button>
                                </div>
                            </div>
                        </div>
                    </section>
                </div>
            {:else if activeTab === "rest"}
                <div class="stg-card-stack">
                    <section class="stg-card">
                        <header class="stg-card-hd">
                            <span class="stg-card-icon" aria-hidden="true">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="14"
                                    height="14"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                >
                                    <path
                                        d="M4 12a8 8 0 1 0 16 0 8 8 0 0 0-16 0z"
                                    /><path d="M12 8v4l3 2" />
                                </svg>
                            </span>
                            <div class="stg-card-titles">
                                <h3 class="stg-card-title">Request Settings</h3>
                                <p class="stg-card-sub">
                                    Defaults applied to every REST request you
                                    send.
                                </p>
                            </div>
                        </header>
                        <div class="stg-card-body">
                            <div class="stg-card-row">
                                <label class="stg-card-row-label"
                                    >Request Timeout (ms)</label
                                >
                                <input
                                    class="stg-input"
                                    type="number"
                                    value={timeout}
                                    min="1000"
                                    step="1000"
                                    onchange={(e) =>
                                        handleSettingChange(
                                            "request_timeout",
                                            e.currentTarget.value,
                                        )}
                                />
                            </div>
                            <div class="stg-card-row">
                                <label class="stg-card-row-label"
                                    >Follow Redirects</label
                                >
                                <label class="stg-toggle">
                                    <input
                                        type="checkbox"
                                        checked={followRedirects}
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "follow_redirects",
                                                String(e.currentTarget.checked),
                                            )}
                                    />
                                    <span class="stg-toggle-slider"></span>
                                </label>
                            </div>
                            <div class="stg-card-row">
                                <label class="stg-card-row-label"
                                    >SSL Verification</label
                                >
                                <label class="stg-toggle">
                                    <input
                                        type="checkbox"
                                        checked={sslVerification}
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "ssl_verification",
                                                String(e.currentTarget.checked),
                                            )}
                                    />
                                    <span class="stg-toggle-slider"></span>
                                </label>
                            </div>
                            <div class="stg-card-row">
                                <label class="stg-card-row-label"
                                    >Max Response Size (MB)</label
                                >
                                <input
                                    class="stg-input"
                                    type="number"
                                    value={maxResponseSize}
                                    min="1"
                                    max="100"
                                    onchange={(e) =>
                                        handleSettingChange(
                                            "max_response_size",
                                            e.currentTarget.value,
                                        )}
                                />
                            </div>
                        </div>
                    </section>
                </div>
            {:else if activeTab === "appearance"}
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
                                        <div
                                            class="theme-preview-bar"
                                            style="background:{color}; opacity:{0.6 +
                                                i * 0.2}"
                                        ></div>
                                    {/each}
                                </div>
                                <div class="theme-info">
                                    <span class="theme-name">{theme.name}</span>
                                    <span class="theme-desc"
                                        >{THEME_DESCRIPTIONS[theme.id] ||
                                            ""}</span
                                    >
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
            {:else if activeTab === "ai"}
                <!-- AI sub-tabs -->
                <div class="ai-subtabs">
                    <button
                        class="ai-subtab"
                        class:active={aiSubTab === "config"}
                        onclick={() => (aiSubTab = "config")}
                    >
                        Configuration
                    </button>
                    <button
                        class="ai-subtab"
                        class:active={aiSubTab === "usage"}
                        class:disabled={!aiHasKey}
                        onclick={() => {
                            if (aiHasKey) aiSubTab = "usage";
                        }}
                    >
                        Usage Stats
                        {#if !aiHasKey}
                            <svg
                                class="ai-subtab-lock"
                                viewBox="0 0 24 24"
                                width="11"
                                height="11"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2"
                                ><rect
                                    x="3"
                                    y="11"
                                    width="18"
                                    height="11"
                                    rx="2"
                                    ry="2"
                                /><path d="M7 11V7a5 5 0 0110 0v4" /></svg
                            >
                        {/if}
                    </button>
                </div>

                {#if aiSubTab === "config"}
                    <div class="stg-card-stack">
                        <section class="stg-card">
                            <header class="stg-card-hd">
                                <span class="stg-card-icon" aria-hidden="true">
                                    <svg
                                        viewBox="0 0 24 24"
                                        width="14"
                                        height="14"
                                        fill="none"
                                        stroke="currentColor"
                                        stroke-width="2"
                                        stroke-linecap="round"
                                        stroke-linejoin="round"
                                    >
                                        <path
                                            d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"
                                        />
                                    </svg>
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">
                                        Configuration
                                    </h3>
                                    <p class="stg-card-sub">
                                        Provider, model, and API key for the
                                        in-app AI assistant.
                                    </p>
                                </div>
                            </header>
                            <div class="ai-cfg">
                                <!-- Provider & Model header -->
                                <div class="ai-cfg-row">
                                    <div class="ai-cfg-field">
                                        <label class="ai-cfg-label"
                                            >Provider</label
                                        >
                                        <select
                                            class="ai-cfg-select"
                                            value={aiProvider}
                                            onchange={(e) =>
                                                handleProviderChange(
                                                    e.currentTarget.value,
                                                )}
                                        >
                                            {#each AI_PROVIDER_REGISTRY as p (p.providerId)}
                                                <option value={p.providerId}
                                                    >{p.providerLabel}</option
                                                >
                                            {/each}
                                        </select>
                                    </div>
                                    <div class="ai-cfg-field">
                                        <label class="ai-cfg-label">Model</label
                                        >
                                        <span class="ai-model-tag"
                                            >{currentProviderConfig.modelLabel}</span
                                        >
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
                                            type={showAiKey
                                                ? "text"
                                                : "password"}
                                            placeholder={currentProviderConfig.keyPlaceholder}
                                            bind:value={aiApiKey}
                                        />
                                        <button
                                            class="ai-key-toggle"
                                            onclick={() =>
                                                (showAiKey = !showAiKey)}
                                            type="button"
                                        >
                                            {#if showAiKey}
                                                <svg
                                                    viewBox="0 0 24 24"
                                                    width="14"
                                                    height="14"
                                                    fill="none"
                                                    stroke="currentColor"
                                                    stroke-width="1.8"
                                                    ><path
                                                        d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"
                                                    /><line
                                                        x1="1"
                                                        y1="1"
                                                        x2="23"
                                                        y2="23"
                                                    /></svg
                                                >
                                            {:else}
                                                <svg
                                                    viewBox="0 0 24 24"
                                                    width="14"
                                                    height="14"
                                                    fill="none"
                                                    stroke="currentColor"
                                                    stroke-width="1.8"
                                                    ><path
                                                        d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"
                                                    /><circle
                                                        cx="12"
                                                        cy="12"
                                                        r="3"
                                                    /></svg
                                                >
                                            {/if}
                                        </button>
                                    </div>

                                    {#if aiTestStatus === "success"}
                                        <span class="ai-test-result success">
                                            <svg
                                                viewBox="0 0 24 24"
                                                width="12"
                                                height="12"
                                                fill="none"
                                                stroke="currentColor"
                                                stroke-width="2"
                                                ><polyline
                                                    points="20 6 9 17 4 12"
                                                /></svg
                                            >
                                            {aiTestMessage}
                                        </span>
                                    {:else if aiTestStatus === "error"}
                                        <span class="ai-test-result error">
                                            <svg
                                                viewBox="0 0 24 24"
                                                width="12"
                                                height="12"
                                                fill="none"
                                                stroke="currentColor"
                                                stroke-width="2"
                                                ><circle
                                                    cx="12"
                                                    cy="12"
                                                    r="10"
                                                /><line
                                                    x1="15"
                                                    y1="9"
                                                    x2="9"
                                                    y2="15"
                                                /><line
                                                    x1="9"
                                                    y1="9"
                                                    x2="15"
                                                    y2="15"
                                                /></svg
                                            >
                                            {aiTestMessage}
                                        </span>
                                    {/if}

                                    <div class="ai-key-actions">
                                        <button
                                            class="ai-action-btn primary"
                                            onclick={() => handleSaveAiKey()}
                                            disabled={!aiApiKey.trim() ||
                                                aiTestStatus === "testing"}
                                        >
                                            {#if aiTestStatus === "testing"}
                                                Verifying...
                                            {:else}
                                                Save & Verify
                                            {/if}
                                        </button>
                                        {#if aiHasKey}
                                            <button
                                                class="ai-action-btn danger"
                                                onclick={handleRemoveAiKey}
                                                >Remove Key</button
                                            >
                                        {/if}
                                    </div>
                                </div>

                                <!-- Divider -->
                                <div class="ai-cfg-divider"></div>

                                <!-- Footer: links + status -->
                                <div class="ai-cfg-footer">
                                    <div class="ai-cfg-links">
                                        <a
                                            class="ai-link"
                                            href={currentProviderConfig.keyUrl}
                                            target="_blank"
                                            rel="noopener"
                                        >
                                            <svg
                                                viewBox="0 0 24 24"
                                                width="12"
                                                height="12"
                                                fill="none"
                                                stroke="currentColor"
                                                stroke-width="2"
                                                ><path d="M15 3h6v6" /><path
                                                    d="M10 14L21 3"
                                                /><path
                                                    d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"
                                                /></svg
                                            >
                                            Get API Key
                                        </a>
                                        <span class="ai-link-sep">&middot;</span
                                        >
                                        <a
                                            class="ai-link"
                                            href="https://github.com/mnfst/awesome-free-llm-apis"
                                            target="_blank"
                                            rel="noopener"
                                        >
                                            <svg
                                                viewBox="0 0 24 24"
                                                width="12"
                                                height="12"
                                                fill="none"
                                                stroke="currentColor"
                                                stroke-width="2"
                                                ><path d="M15 3h6v6" /><path
                                                    d="M10 14L21 3"
                                                /><path
                                                    d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"
                                                /></svg
                                            >
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
                        </section>
                    </div>
                {:else if aiSubTab === "usage"}
                    {#if aiUsageStats.length === 0}
                        <div class="ai-usage-empty">
                            <svg
                                viewBox="0 0 24 24"
                                width="36"
                                height="36"
                                fill="none"
                                stroke="var(--t4)"
                                stroke-width="1.2"
                                stroke-linecap="round"
                                stroke-linejoin="round"
                                ><path d="M18 20V10M12 20V4M6 20v-6" /></svg
                            >
                            <p>No API calls recorded yet</p>
                            <span
                                >Start chatting with AI to see usage data here</span
                            >
                        </div>
                    {:else}
                        <div class="stg-card-stack">
                            <section class="stg-card">
                                <header class="stg-card-hd">
                                    <span
                                        class="stg-card-icon"
                                        aria-hidden="true"
                                    >
                                        <svg
                                            viewBox="0 0 24 24"
                                            width="14"
                                            height="14"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="2"
                                            stroke-linecap="round"
                                            stroke-linejoin="round"
                                            ><line
                                                x1="18"
                                                y1="20"
                                                x2="18"
                                                y2="10"
                                            /><line
                                                x1="12"
                                                y1="20"
                                                x2="12"
                                                y2="4"
                                            /><line
                                                x1="6"
                                                y1="20"
                                                x2="6"
                                                y2="14"
                                            /></svg
                                        >
                                    </span>
                                    <div class="stg-card-titles">
                                        <h3 class="stg-card-title">Overview</h3>
                                        <p class="stg-card-sub">
                                            Aggregated calls, tokens, and
                                            estimated cost across all AI
                                            activity.
                                        </p>
                                    </div>
                                    {#if showResetConfirm}
                                        <div class="ai-reset-confirm">
                                            <span>Reset all data?</span>
                                            <button
                                                class="ai-action-btn danger sm"
                                                onclick={handleResetUsage}
                                                >Reset</button
                                            >
                                            <button
                                                class="ai-action-btn sm"
                                                onclick={() =>
                                                    (showResetConfirm = false)}
                                                >Cancel</button
                                            >
                                        </div>
                                    {:else}
                                        <button
                                            class="ai-action-btn sm"
                                            onclick={() =>
                                                (showResetConfirm = true)}
                                        >
                                            <svg
                                                viewBox="0 0 24 24"
                                                width="11"
                                                height="11"
                                                fill="none"
                                                stroke="currentColor"
                                                stroke-width="1.8"
                                                ><polyline
                                                    points="3 6 5 6 21 6"
                                                /><path
                                                    d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"
                                                /></svg
                                            >
                                            Reset
                                        </button>
                                    {/if}
                                </header>
                                <div class="stg-card-body">
                                    <div class="ai-summary-grid">
                                        <div class="ai-summary-card">
                                            <span class="ai-summary-val"
                                                >{aiUsageStats.reduce(
                                                    (s, v) => s + v.totalCalls,
                                                    0,
                                                )}</span
                                            >
                                            <span class="ai-summary-lbl"
                                                >Total Calls</span
                                            >
                                        </div>
                                        <div class="ai-summary-card">
                                            <span class="ai-summary-val"
                                                >{formatTokens(
                                                    aiUsageStats.reduce(
                                                        (s, v) =>
                                                            s + v.inputTokens,
                                                        0,
                                                    ),
                                                )}</span
                                            >
                                            <span class="ai-summary-lbl"
                                                >Input Tokens</span
                                            >
                                        </div>
                                        <div class="ai-summary-card">
                                            <span class="ai-summary-val"
                                                >{formatTokens(
                                                    aiUsageStats.reduce(
                                                        (s, v) =>
                                                            s + v.outputTokens,
                                                        0,
                                                    ),
                                                )}</span
                                            >
                                            <span class="ai-summary-lbl"
                                                >Output Tokens</span
                                            >
                                        </div>
                                        <div class="ai-summary-card accent">
                                            <span class="ai-summary-val"
                                                >{estimateCost(
                                                    aiUsageStats.reduce(
                                                        (s, v) =>
                                                            s + v.inputTokens,
                                                        0,
                                                    ),
                                                    aiUsageStats.reduce(
                                                        (s, v) =>
                                                            s + v.outputTokens,
                                                        0,
                                                    ),
                                                )}</span
                                            >
                                            <span class="ai-summary-lbl"
                                                >Est. Cost</span
                                            >
                                        </div>
                                    </div>
                                </div>
                            </section>

                            <section class="stg-card">
                                <header class="stg-card-hd">
                                    <span
                                        class="stg-card-icon"
                                        aria-hidden="true"
                                    >
                                        <svg
                                            viewBox="0 0 24 24"
                                            width="14"
                                            height="14"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="2"
                                            stroke-linecap="round"
                                            stroke-linejoin="round"
                                            ><polygon
                                                points="12 2 2 7 12 12 22 7 12 2"
                                            /><polyline
                                                points="2 17 12 22 22 17"
                                            /><polyline
                                                points="2 12 12 17 22 12"
                                            /></svg
                                        >
                                    </span>
                                    <div class="stg-card-titles">
                                        <h3 class="stg-card-title">By Mode</h3>
                                        <p class="stg-card-sub">
                                            Per-mode breakdown of AI calls and
                                            token spend.
                                        </p>
                                    </div>
                                </header>
                                <div class="stg-card-body">
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
                                                    <span
                                                        class="ai-dot"
                                                        data-mode={stat.mode}
                                                    ></span>
                                                    {stat.mode.toUpperCase()}
                                                </span>
                                                <span class="ai-table-val"
                                                    >{stat.totalCalls}</span
                                                >
                                                <span class="ai-table-val"
                                                    >{formatTokens(
                                                        stat.inputTokens,
                                                    )}</span
                                                >
                                                <span class="ai-table-val"
                                                    >{formatTokens(
                                                        stat.outputTokens,
                                                    )}</span
                                                >
                                                <span
                                                    class="ai-table-val accent"
                                                    >{estimateCost(
                                                        stat.inputTokens,
                                                        stat.outputTokens,
                                                    )}</span
                                                >
                                            </div>
                                        {/each}
                                    </div>
                                </div>
                            </section>

                            {#if aiProviderStats.length > 0}
                                <section class="stg-card">
                                    <header class="stg-card-hd">
                                        <span
                                            class="stg-card-icon"
                                            aria-hidden="true"
                                        >
                                            <svg
                                                viewBox="0 0 24 24"
                                                width="14"
                                                height="14"
                                                fill="none"
                                                stroke="currentColor"
                                                stroke-width="2"
                                                stroke-linecap="round"
                                                stroke-linejoin="round"
                                                ><rect
                                                    x="4"
                                                    y="4"
                                                    width="16"
                                                    height="16"
                                                    rx="2"
                                                /><rect
                                                    x="9"
                                                    y="9"
                                                    width="6"
                                                    height="6"
                                                /><line
                                                    x1="9"
                                                    y1="2"
                                                    x2="9"
                                                    y2="4"
                                                /><line
                                                    x1="15"
                                                    y1="2"
                                                    x2="15"
                                                    y2="4"
                                                /><line
                                                    x1="9"
                                                    y1="20"
                                                    x2="9"
                                                    y2="22"
                                                /><line
                                                    x1="15"
                                                    y1="20"
                                                    x2="15"
                                                    y2="22"
                                                /><line
                                                    x1="20"
                                                    y1="9"
                                                    x2="22"
                                                    y2="9"
                                                /><line
                                                    x1="20"
                                                    y1="15"
                                                    x2="22"
                                                    y2="15"
                                                /><line
                                                    x1="2"
                                                    y1="9"
                                                    x2="4"
                                                    y2="9"
                                                /><line
                                                    x1="2"
                                                    y1="15"
                                                    x2="4"
                                                    y2="15"
                                                /></svg
                                            >
                                        </span>
                                        <div class="stg-card-titles">
                                            <h3 class="stg-card-title">
                                                By Provider
                                            </h3>
                                            <p class="stg-card-sub">
                                                Spend grouped by model — useful
                                                when switching providers.
                                            </p>
                                        </div>
                                    </header>
                                    <div class="stg-card-body">
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
                                                    <span
                                                        class="ai-table-label"
                                                    >
                                                        <span
                                                            class="ai-dot"
                                                            style="background: var(--acc)"
                                                        ></span>
                                                        {formatModelName(
                                                            pstat.model,
                                                        )}
                                                    </span>
                                                    <span class="ai-table-val"
                                                        >{pstat.totalCalls}</span
                                                    >
                                                    <span class="ai-table-val"
                                                        >{formatTokens(
                                                            pstat.inputTokens,
                                                        )}</span
                                                    >
                                                    <span class="ai-table-val"
                                                        >{formatTokens(
                                                            pstat.outputTokens,
                                                        )}</span
                                                    >
                                                    <span
                                                        class="ai-table-val accent"
                                                        >{estimateCost(
                                                            pstat.inputTokens,
                                                            pstat.outputTokens,
                                                        )}</span
                                                    >
                                                </div>
                                            {/each}
                                        </div>
                                        <p class="ai-pricing-note">
                                            Haiku 4.5: $1.00 / MTok in &middot;
                                            $5.00 / MTok out
                                        </p>
                                    </div>
                                </section>
                            {/if}
                        </div>
                    {/if}
                {/if}
            {:else if activeTab === "agent"}
                <div class="agent-settings-pane">
                    <!-- Agent sub-tabs -->
                    <div class="ai-subtabs">
                        <button
                            class="ai-subtab"
                            class:active={agentSubTab === "general"}
                            onclick={() => (agentSubTab = "general")}
                        >
                            General
                        </button>
                        <button
                            class="ai-subtab"
                            class:active={agentSubTab === "plugins"}
                            onclick={() => (agentSubTab = "plugins")}
                        >
                            Plugins
                        </button>
                        <button
                            class="ai-subtab"
                            class:active={agentSubTab === "contexts"}
                            onclick={() => (agentSubTab = "contexts")}
                        >
                            Contexts
                        </button>
                        <button
                            class="ai-subtab"
                            class:active={agentSubTab === "usage"}
                            class:disabled={!agentSessionKey}
                            onclick={() => {
                                if (agentSessionKey) agentSubTab = "usage";
                            }}
                        >
                            Usage Stats
                            {#if !agentSessionKey}
                                <svg
                                    class="ai-subtab-lock"
                                    viewBox="0 0 24 24"
                                    width="11"
                                    height="11"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    ><rect
                                        x="3"
                                        y="11"
                                        width="18"
                                        height="11"
                                        rx="2"
                                        ry="2"
                                    /><path d="M7 11V7a5 5 0 0110 0v4" /></svg
                                >
                            {/if}
                        </button>
                    </div>

                    <div class="agent-settings-scroll">
                {#if agentSubTab === "general"}
                    <div class="stg-card-stack">
                        <section class="stg-card agent-usage-card">
                            <header class="stg-card-hd">
                                <span class="stg-card-icon" aria-hidden="true">
                                    <svg
                                        viewBox="0 0 24 24"
                                        width="14"
                                        height="14"
                                        fill="none"
                                        stroke="currentColor"
                                        stroke-width="2"
                                        stroke-linecap="round"
                                        stroke-linejoin="round"
                                        ><path d="M18 20V10M12 20V4M6 20v-6" /></svg
                                    >
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">
                                        Live usage tracking
                                    </h3>
                                    <p class="stg-card-sub">
                                        Configure the provider limits shown in
                                        the Agent footer.
                                    </p>
                                </div>
                                {#if agentFooterProvider === "claude" && agentSessionKey && $agentUsageAuthStatus.state === "valid"}
                                    <span class="ai-status-badge">
                                        <span class="ai-status-dot"></span>
                                        Claude connected
                                    </span>
                                {:else if agentFooterProvider === "claude" && agentSessionKey && $agentUsageAuthStatus.state === "checking"}
                                    <span class="ai-status-badge checking">
                                        <span class="ai-status-dot"></span>
                                        Checking Claude
                                    </span>
                                {:else if agentFooterProvider === "claude" && agentSessionKey && $agentUsageAuthStatus.state === "invalid"}
                                    <span class="ai-status-badge error" title={$agentUsageAuthStatus.message}>
                                        <span class="ai-status-dot"></span>
                                        Claude needs reconfigure
                                    </span>
                                {:else if agentFooterProvider === "codex" && agentCodexToken}
                                    <span class="ai-status-badge">
                                        <span class="ai-status-dot"></span>
                                        Codex configured
                                    </span>
                                {:else}
                                    <span class="ai-status-badge muted">
                                        Footer not configured
                                    </span>
                                {/if}
                            </header>
                            <div class="stg-card-body">
                                <div class="stg-card-row">
                                    <div class="stg-card-row-action-text">
                                        <span class="stg-card-row-label"
                                            >Show in Agent footer</span
                                        >
                                        <span class="stg-card-row-help">
                                            Choose the provider whose live limit
                                            appears next to the Agent terminal.
                                        </span>
                                    </div>
                                    <select
                                        class="stg-select"
                                        value={agentFooterProvider}
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "agent_footer_usage_provider",
                                                e.currentTarget.value,
                                            )}
                                    >
                                        <option
                                            value="claude"
                                            disabled={!agentSessionKey}
                                            >Claude Code{agentSessionKey
                                                ? ""
                                                : " (not configured)"}</option
                                        >
                                        <option
                                            value="codex"
                                            disabled={!agentCodexToken}
                                            >Codex{agentCodexToken
                                                ? ""
                                                : " (not configured)"}</option
                                        >
                                    </select>
                                </div>

                                <div class="stg-card-row stg-card-row-action">
                                    <div class="stg-card-row-action-text">
                                        <span class="stg-card-row-label"
                                            >Claude session key</span
                                        >
                                        <span class="stg-card-row-help">
                                            Used only to read Claude usage
                                            limits. Find it in claude.ai
                                            DevTools > Application > Cookies >
                                            sessionKey.
                                        </span>
                                        {#if agentKeyTestStatus === "success"}
                                            <span class="ai-test-result success">
                                                <svg
                                                    viewBox="0 0 24 24"
                                                    width="12"
                                                    height="12"
                                                    fill="none"
                                                    stroke="currentColor"
                                                    stroke-width="2"
                                                    ><polyline
                                                        points="20 6 9 17 4 12"
                                                    /></svg
                                                >
                                                {agentKeyTestMessage}
                                            </span>
                                        {:else if agentKeyTestStatus === "error"}
                                            <span class="ai-test-result error">
                                                <svg
                                                    viewBox="0 0 24 24"
                                                    width="12"
                                                    height="12"
                                                    fill="none"
                                                    stroke="currentColor"
                                                    stroke-width="2"
                                                    ><circle
                                                        cx="12"
                                                        cy="12"
                                                        r="10"
                                                    /><line
                                                        x1="15"
                                                        y1="9"
                                                        x2="9"
                                                        y2="15"
                                                    /><line
                                                        x1="9"
                                                        y1="9"
                                                        x2="15"
                                                        y2="15"
                                                    /></svg
                                                >
                                                {agentKeyTestMessage}
                                            </span>
                                        {:else if agentSessionKey && $agentUsageAuthStatus.state === "invalid"}
                                            <span class="ai-test-result error">
                                                {$agentUsageAuthStatus.message}
                                            </span>
                                        {/if}
                                    </div>
                                    <div class="agent-session-key-control">
                                        <div class="ai-key-input-wrap">
                                            <input
                                                class="ai-cfg-input"
                                                type={showAgentKey
                                                    ? "text"
                                                    : "password"}
                                                placeholder="sk-ant-sid01-..."
                                                bind:value={agentKeyInput}
                                                onblur={commitAgentKeyIfChanged}
                                                onkeydown={(e) => {
                                                    if (e.key === "Enter") {
                                                        e.preventDefault();
                                                        commitAgentKeyIfChanged();
                                                    }
                                                }}
                                            />
                                            <button
                                                class="ai-key-toggle"
                                                onclick={() =>
                                                    (showAgentKey =
                                                        !showAgentKey)}
                                                type="button"
                                                title={showAgentKey
                                                    ? "Hide key"
                                                    : "Show key"}
                                            >
                                                {#if showAgentKey}
                                                    <svg
                                                        viewBox="0 0 24 24"
                                                        width="14"
                                                        height="14"
                                                        fill="none"
                                                        stroke="currentColor"
                                                        stroke-width="1.8"
                                                        ><path
                                                            d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"
                                                        /><line
                                                            x1="1"
                                                            y1="1"
                                                            x2="23"
                                                            y2="23"
                                                        /></svg
                                                    >
                                                {:else}
                                                    <svg
                                                        viewBox="0 0 24 24"
                                                        width="14"
                                                        height="14"
                                                        fill="none"
                                                        stroke="currentColor"
                                                        stroke-width="1.8"
                                                        ><path
                                                            d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"
                                                        /><circle
                                                            cx="12"
                                                            cy="12"
                                                            r="3"
                                                        /></svg
                                                    >
                                                {/if}
                                            </button>
                                        </div>
                                        <div class="agent-session-key-actions">
                                            {#if agentKeyTestStatus === "testing"}
                                                <span class="ai-test-result">Verifying…</span>
                                            {/if}
                                            {#if agentSessionKey}
                                                <button
                                                    class="ai-action-btn danger"
                                                    onclick={handleRemoveAgentKey}
                                                    >Remove</button
                                                >
                                            {/if}
                                        </div>
                                    </div>
                                </div>

                                <div class="stg-card-row stg-card-row-action">
                                    <div class="stg-card-row-action-text">
                                        <span class="stg-card-row-label"
                                            >Codex access token</span
                                        >
                                        <span class="stg-card-row-help">
                                            Grab from <strong>chatgpt.com/codex/cloud/settings/analytics</strong>
                                            → DevTools → Network → <code>wham/usage</code>
                                            → <code>authorization</code> header (after <code>Bearer </code>).
                                        </span>
                                        {#if codexTokenTestStatus === "success"}
                                            <span class="ai-test-result success">
                                                <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="20 6 9 17 4 12"/></svg>
                                                {codexTokenTestMessage}
                                            </span>
                                        {:else if codexTokenTestStatus === "error"}
                                            <span class="ai-test-result error">
                                                <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>
                                                {codexTokenTestMessage}
                                            </span>
                                        {/if}
                                    </div>
                                    <div class="agent-session-key-control">
                                        <div class="ai-key-input-wrap">
                                            <input
                                                class="ai-cfg-input"
                                                type={showCodexToken
                                                    ? "text"
                                                    : "password"}
                                                placeholder="Bearer token..."
                                                bind:value={codexTokenInput}
                                                onblur={commitCodexTokenIfChanged}
                                                onkeydown={(e) => {
                                                    if (e.key === "Enter") {
                                                        e.preventDefault();
                                                        commitCodexTokenIfChanged();
                                                    }
                                                }}
                                            />
                                            <button
                                                class="ai-key-toggle"
                                                onclick={() =>
                                                    (showCodexToken =
                                                        !showCodexToken)}
                                                type="button"
                                                title={showCodexToken
                                                    ? "Hide token"
                                                    : "Show token"}
                                            >
                                                {#if showCodexToken}
                                                    <svg
                                                        viewBox="0 0 24 24"
                                                        width="14"
                                                        height="14"
                                                        fill="none"
                                                        stroke="currentColor"
                                                        stroke-width="1.8"
                                                        ><path
                                                            d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"
                                                        /><line
                                                            x1="1"
                                                            y1="1"
                                                            x2="23"
                                                            y2="23"
                                                        /></svg
                                                    >
                                                {:else}
                                                    <svg
                                                        viewBox="0 0 24 24"
                                                        width="14"
                                                        height="14"
                                                        fill="none"
                                                        stroke="currentColor"
                                                        stroke-width="1.8"
                                                        ><path
                                                            d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"
                                                        /><circle
                                                            cx="12"
                                                            cy="12"
                                                            r="3"
                                                        /></svg
                                                    >
                                                {/if}
                                            </button>
                                        </div>
                                        <div class="agent-session-key-actions">
                                            {#if codexTokenTestStatus === "testing"}
                                                <span class="ai-test-result">Verifying…</span>
                                            {/if}
                                            {#if agentCodexToken}
                                                <button
                                                    class="ai-action-btn danger"
                                                    onclick={handleRemoveCodexToken}
                                                    >Remove</button
                                                >
                                            {/if}
                                        </div>
                                    </div>
                                </div>

                                <div class="stg-card-row">
                                    <div class="stg-card-row-action-text">
                                        <span class="stg-card-row-label"
                                            >Refresh interval</span
                                        >
                                        <span class="stg-card-row-help">
                                            How often Clauge refreshes live
                                            limits while Agent mode is open.
                                        </span>
                                    </div>
                                    <select
                                        class="stg-select"
                                        value={agentRefreshMins}
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "agent_refresh_mins",
                                                e.currentTarget.value,
                                            )}
                                    >
                                        {#each REFRESH_OPTIONS as opt}
                                            <option value={opt.value}
                                                >{opt.label}</option
                                            >
                                        {/each}
                                    </select>
                                </div>
                            </div>
                        </section>

                        <section class="stg-card">
                            <header class="stg-card-hd">
                                <span class="stg-card-icon" aria-hidden="true">
                                    <svg
                                        viewBox="0 0 24 24"
                                        width="14"
                                        height="14"
                                        fill="none"
                                        stroke="currentColor"
                                        stroke-width="2"
                                        stroke-linecap="round"
                                        stroke-linejoin="round"
                                        ><path
                                            d="M18 8a6 6 0 0 0-12 0c0 7-3 9-3 9h18s-3-2-3-9"
                                        /><path
                                            d="M13.73 21a2 2 0 0 1-3.46 0"
                                        /></svg
                                    >
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">
                                        Notifications
                                    </h3>
                                    <p class="stg-card-sub">
                                        Sound and dock bounce when an agent
                                        session finishes its work.
                                    </p>
                                </div>
                            </header>
                            <div class="stg-card-body">
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Enable sound alerts</label
                                    >
                                    <label class="stg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={agentSoundEnabled}
                                            onchange={(e) =>
                                                handleSettingChange(
                                                    "agent_sound_enabled",
                                                    String(
                                                        e.currentTarget.checked,
                                                    ),
                                                )}
                                        />
                                        <span class="stg-toggle-slider"></span>
                                    </label>
                                </div>
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Enable dock bounce</label
                                    >
                                    <label class="stg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={agentDockBounceEnabled}
                                            onchange={(e) =>
                                                handleSettingChange(
                                                    "agent_dock_bounce_enabled",
                                                    String(
                                                        e.currentTarget.checked,
                                                    ),
                                                )}
                                        />
                                        <span class="stg-toggle-slider"></span>
                                    </label>
                                </div>
                            </div>
                        </section>
                    </div>
                {:else if agentSubTab === "plugins"}
                    <!-- Installed / Marketplace toggle -->
                    <div class="agent-plugin-views">
                        <button
                            class="ai-action-btn"
                            class:primary={pluginView === "installed"}
                            onclick={() => (pluginView = "installed")}
                            >Installed</button
                        >
                        <button
                            class="ai-action-btn"
                            class:primary={pluginView === "marketplace"}
                            onclick={() => (pluginView = "marketplace")}
                            >Marketplace</button
                        >
                    </div>

                    {#if pluginView === "installed"}
                        {#if installedPlugins.length === 0}
                            <div class="ai-usage-empty">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="36"
                                    height="36"
                                    fill="none"
                                    stroke="var(--t4)"
                                    stroke-width="1.2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    ><path
                                        d="M20.5 7.27783L12 12.0001M12 12.0001L3.49997 7.27783M12 12.0001L12 21.5001M14 20.6701L12.7 21.4001C12.2 21.6001 11.8 21.6001 11.3 21.4001L4.8 17.7001C4.3 17.4001 4 16.9001 4 16.3001V7.70011C4 7.10011 4.3 6.60011 4.8 6.30011L11.3 2.60011C11.8 2.40011 12.2 2.40011 12.7 2.60011L19.2 6.30011C19.7 6.60011 20 7.10011 20 7.70011V16.3001"
                                    /></svg
                                >
                                <p>No plugins installed</p>
                                <span
                                    >Browse the marketplace to install plugins</span
                                >
                            </div>
                        {:else}
                            <div class="agent-plugin-list">
                                {#each installedPlugins as plugin}
                                    <div class="agent-plugin-card">
                                        <div class="agent-plugin-info">
                                            <span class="agent-plugin-name"
                                                >{plugin.name}</span
                                            >
                                            <span class="agent-plugin-meta">
                                                {plugin.marketplace}
                                                {#if plugin.version}
                                                    <span class="ai-link-sep"
                                                        >&middot;</span
                                                    >
                                                    v{plugin.version}
                                                {/if}
                                            </span>
                                        </div>
                                        <div class="agent-plugin-actions">
                                            <label class="stg-toggle">
                                                <input
                                                    type="checkbox"
                                                    checked={plugin.enabled}
                                                    onchange={() =>
                                                        handleTogglePlugin(
                                                            plugin.name,
                                                            !plugin.enabled,
                                                        )}
                                                />
                                                <span class="stg-toggle-slider"
                                                ></span>
                                            </label>
                                            <button
                                                class="ai-action-btn danger sm"
                                                onclick={() =>
                                                    handleUninstallPlugin(
                                                        plugin.name,
                                                        plugin.marketplace,
                                                    )}
                                            >
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
                                            <span class="agent-plugin-name"
                                                >{plugin.name}</span
                                            >
                                            <span class="agent-plugin-desc"
                                                >{plugin.description}</span
                                            >
                                            {#if plugin.installs != null}
                                                <span class="agent-plugin-meta"
                                                    >{plugin.installs.toLocaleString()}
                                                    installs</span
                                                >
                                            {/if}
                                        </div>
                                        <div class="agent-plugin-actions">
                                            {#if plugin.installed}
                                                <span class="ai-status-badge">
                                                    <span class="ai-status-dot"
                                                    ></span>
                                                    Installed
                                                </span>
                                            {:else}
                                                <button
                                                    class="ai-action-btn primary sm"
                                                    onclick={() =>
                                                        handleInstallPlugin(
                                                            plugin.name,
                                                            plugin.marketplace,
                                                        )}
                                                >
                                                    Install
                                                </button>
                                            {/if}
                                        </div>
                                    </div>
                                {/each}
                            </div>
                        {/if}
                    {/if}
                {:else if agentSubTab === "contexts"}
                    {#if isNewContext || editingContext}
                        <!-- Context editor -->
                        <div class="agent-ctx-editor">
                            <div
                                class="stg-field"
                                style="flex-direction: column; align-items: stretch;"
                            >
                                <label class="stg-label">Name</label>
                                <input
                                    class="stg-input"
                                    type="text"
                                    style="width: 100%;"
                                    placeholder="Context name..."
                                    bind:value={editContextName}
                                />
                            </div>
                            <div
                                class="stg-field"
                                style="flex-direction: column; align-items: stretch;"
                            >
                                <label class="stg-label">Content</label>
                                <textarea
                                    class="agent-ctx-textarea"
                                    placeholder="Context content..."
                                    bind:value={editContextContent}
                                ></textarea>
                            </div>
                            <div class="ai-key-actions">
                                <button
                                    class="ai-action-btn primary"
                                    onclick={handleSaveAgentContext}
                                    >Save</button
                                >
                                <button
                                    class="ai-action-btn"
                                    onclick={cancelEditContext}>Cancel</button
                                >
                            </div>
                        </div>
                    {:else}
                        <div class="agent-ctx-header">
                            <span
                                class="stg-section-label"
                                style="margin-bottom: 0;">Contexts</span
                            >
                            <button
                                class="ai-action-btn primary sm"
                                onclick={startNewContext}>New Context</button
                            >
                        </div>

                        {#if agentContexts.length === 0}
                            <div class="ai-usage-empty">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="36"
                                    height="36"
                                    fill="none"
                                    stroke="var(--t4)"
                                    stroke-width="1.2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    ><path
                                        d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"
                                    /><polyline points="14 2 14 8 20 8" /><line
                                        x1="16"
                                        y1="13"
                                        x2="8"
                                        y2="13"
                                    /><line
                                        x1="16"
                                        y1="17"
                                        x2="8"
                                        y2="17"
                                    /><polyline points="10 9 9 9 8 9" /></svg
                                >
                                <p>No contexts yet</p>
                                <span
                                    >Create a context to attach to agent
                                    sessions</span
                                >
                            </div>
                        {:else}
                            <div class="agent-plugin-list">
                                {#each agentContexts as ctx}
                                    <div
                                        class="agent-plugin-card agent-ctx-card"
                                        onclick={() => startEditContext(ctx)}
                                        role="button"
                                        tabindex="0"
                                        onkeydown={(e) => {
                                            if (e.key === "Enter")
                                                startEditContext(ctx);
                                        }}
                                    >
                                        <div class="agent-plugin-info">
                                            <span class="agent-plugin-name"
                                                >{ctx.name}</span
                                            >
                                            <span class="agent-plugin-desc"
                                                >{ctx.content
                                                    .split("\n")[0]
                                                    .slice(0, 80)}{ctx.content
                                                    .length > 80
                                                    ? "..."
                                                    : ""}</span
                                            >
                                        </div>
                                        <div
                                            class="agent-plugin-actions"
                                            onclick={(e) => {
                                                e.stopPropagation();
                                            }}
                                        >
                                            {#if deleteConfirmId === ctx.id}
                                                <span class="ai-reset-confirm">
                                                    <span>Delete?</span>
                                                    <button
                                                        class="ai-action-btn danger sm"
                                                        onclick={(e) => {
                                                            e.stopPropagation();
                                                            handleDeleteAgentContext(
                                                                ctx.id,
                                                            );
                                                        }}>Yes</button
                                                    >
                                                    <button
                                                        class="ai-action-btn sm"
                                                        onclick={(e) => {
                                                            e.stopPropagation();
                                                            deleteConfirmId =
                                                                null;
                                                        }}>No</button
                                                    >
                                                </span>
                                            {:else}
                                                <button
                                                    class="ai-action-btn danger sm"
                                                    onclick={(e) => {
                                                        e.stopPropagation();
                                                        deleteConfirmId =
                                                            ctx.id;
                                                    }}>Delete</button
                                                >
                                            {/if}
                                        </div>
                                    </div>
                                {/each}
                            </div>
                        {/if}
                    {/if}
                {:else if agentSubTab === "usage"}
                    <!-- Day range selector -->
                    <div class="ud-days">
                        {#each [7, 14, 30, 90] as d}
                            <button
                                class="ud-day-btn"
                                class:active={agentUsageDays === d}
                                onclick={() => selectAgentUsageDays(d)}
                                >{d}d</button
                            >
                        {/each}
                        <button
                            class="ai-action-btn sm"
                            style="margin-left: auto;"
                            onclick={loadAgentUsage}
                        >
                            <svg
                                viewBox="0 0 24 24"
                                width="11"
                                height="11"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2"
                                ><polyline points="23 4 23 10 17 10" /><path
                                    d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"
                                /></svg
                            >
                            Refresh
                        </button>
                    </div>

                    {#if agentUsageLoading}
                        <div class="ud-loading">
                            <div class="ud-spinner"></div>
                            Loading analytics...
                        </div>
                    {:else if agentUsageData}
                        <!-- Summary cards -->
                        <div class="ud-cards">
                            <div class="ud-card">
                                <span class="ud-val"
                                    >{agentFormatCost(
                                        agentUsageData.totalCost,
                                    )}</span
                                ><span class="ud-lbl">Total Cost</span>
                            </div>
                            <div class="ud-card">
                                <span class="ud-val"
                                    >{agentUsageData.totalApiCalls.toLocaleString()}</span
                                ><span class="ud-lbl">API Calls</span>
                            </div>
                            <div class="ud-card">
                                <span class="ud-val"
                                    >{agentUsageData.totalSessions}</span
                                ><span class="ud-lbl">Sessions</span>
                            </div>
                            <div class="ud-card">
                                <span class="ud-val"
                                    >{agentUsageData.cacheHitPercent.toFixed(
                                        1,
                                    )}%</span
                                ><span class="ud-lbl">Cache Hit</span>
                            </div>
                        </div>

                        <!-- Token breakdown -->
                        <div class="ud-token-row">
                            <span
                                ><strong>In:</strong>
                                {agentFormatTokens(
                                    agentUsageData.totalInputTokens,
                                )}</span
                            >
                            <span
                                ><strong>Out:</strong>
                                {agentFormatTokens(
                                    agentUsageData.totalOutputTokens,
                                )}</span
                            >
                            <span
                                ><strong>Cache R:</strong>
                                {agentFormatTokens(
                                    agentUsageData.totalCacheReadTokens,
                                )}</span
                            >
                            <span
                                ><strong>Cache W:</strong>
                                {agentFormatTokens(
                                    agentUsageData.totalCacheWriteTokens,
                                )}</span
                            >
                        </div>

                        <!-- Daily chart -->
                        {#if agentUsageData.daily.length > 0}
                            <div class="ud-section-inline">
                                <div class="ud-section-title">
                                    Daily Activity
                                </div>
                                <div class="ud-chart">
                                    {#each agentUsageData.daily.slice(-21) as day}
                                        {@const maxCost = Math.max(
                                            ...agentUsageData.daily
                                                .slice(-21)
                                                .map((d) => d.cost),
                                            0.01,
                                        )}
                                        <div
                                            class="ud-bar-wrap"
                                            title="{day.date}: {agentFormatCost(
                                                day.cost,
                                            )} / {day.calls} calls"
                                        >
                                            <div
                                                class="ud-bar"
                                                style="height:{Math.max(
                                                    3,
                                                    (day.cost / maxCost) * 100,
                                                )}%"
                                            ></div>
                                            <span class="ud-bar-lbl"
                                                >{day.date.slice(8)}</span
                                            >
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
                                    {@const sessionPct =
                                        $agentUsageLimits.five_hour
                                            ?.utilization ??
                                        $agentUsageLimits.standard
                                            ?.percentUsed ??
                                        null}
                                    {@const weeklyPct =
                                        $agentUsageLimits.seven_day
                                            ?.utilization ??
                                        $agentUsageLimits.extended
                                            ?.percentUsed ??
                                        null}
                                    {@const sonnetPct =
                                        $agentUsageLimits.seven_day_sonnet
                                            ?.utilization ?? null}
                                    <div class="ud-live-rows">
                                        {#if sessionPct != null}
                                            <div class="ud-live-row">
                                                <span class="ud-live-lbl"
                                                    >Session</span
                                                >
                                                <div class="ud-live-bar">
                                                    <div
                                                        class="ud-live-fill"
                                                        style="width:{sessionPct}%;background:{agentLiveColor(
                                                            sessionPct,
                                                        )}"
                                                    ></div>
                                                </div>
                                                <span
                                                    class="ud-live-pct"
                                                    style="color:{agentLiveColor(
                                                        sessionPct,
                                                    )}"
                                                    >{sessionPct.toFixed(
                                                        1,
                                                    )}%</span
                                                >
                                            </div>
                                        {/if}
                                        {#if weeklyPct != null}
                                            <div class="ud-live-row">
                                                <span class="ud-live-lbl"
                                                    >Weekly</span
                                                >
                                                <div class="ud-live-bar">
                                                    <div
                                                        class="ud-live-fill"
                                                        style="width:{weeklyPct}%;background:{agentLiveColor(
                                                            weeklyPct,
                                                        )}"
                                                    ></div>
                                                </div>
                                                <span
                                                    class="ud-live-pct"
                                                    style="color:{agentLiveColor(
                                                        weeklyPct,
                                                    )}"
                                                    >{weeklyPct.toFixed(
                                                        1,
                                                    )}%</span
                                                >
                                            </div>
                                        {/if}
                                        {#if sonnetPct != null}
                                            <div class="ud-live-row">
                                                <span class="ud-live-lbl"
                                                    >Sonnet</span
                                                >
                                                <div class="ud-live-bar">
                                                    <div
                                                        class="ud-live-fill"
                                                        style="width:{sonnetPct}%;background:{agentLiveColor(
                                                            sonnetPct,
                                                        )}"
                                                    ></div>
                                                </div>
                                                <span
                                                    class="ud-live-pct"
                                                    style="color:{agentLiveColor(
                                                        sonnetPct,
                                                    )}"
                                                    >{sonnetPct.toFixed(
                                                        1,
                                                    )}%</span
                                                >
                                            </div>
                                        {/if}
                                    </div>
                                {:else}
                                    <div
                                        style="padding:8px 0;font-size:11px;color:var(--t3);"
                                    >
                                        Fetching live data...
                                    </div>
                                {/if}
                            </div>
                            <div class="ud-section">
                                <div class="ud-section-title">Models</div>
                                <div class="ud-scroll">
                                    {#each agentUsageData.byModel as m}
                                        <div class="ud-row">
                                            <div class="ud-row-info">
                                                <span class="ud-row-name"
                                                    >{m.model}</span
                                                >
                                                <span class="ud-row-meta"
                                                    >{m.calls} calls &middot; {m.cacheHitPercent.toFixed(
                                                        0,
                                                    )}% cache</span
                                                >
                                            </div>
                                            <span class="ud-row-cost"
                                                >{agentFormatCost(m.cost)}</span
                                            >
                                        </div>
                                    {/each}
                                </div>
                            </div>
                        </div>

                        <!-- Projects + Top Sessions (2-col) -->
                        <div class="ud-grid">
                            <div class="ud-section">
                                <div class="ud-section-title">
                                    Projects ({agentUsageData.byProject.length})
                                </div>
                                <div class="ud-scroll">
                                    {#each agentUsageData.byProject as p}
                                        <div class="ud-row">
                                            <div class="ud-row-info">
                                                <span
                                                    class="ud-row-name"
                                                    title={p.project}
                                                    >{agentDecodeName(
                                                        p.project,
                                                    )}</span
                                                >
                                                <span class="ud-row-meta"
                                                    >{p.sessions} sess &middot; {p.calls}
                                                    calls</span
                                                >
                                            </div>
                                            <span class="ud-row-cost"
                                                >{agentFormatCost(p.cost)}</span
                                            >
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
                                                <span
                                                    class="ud-row-name"
                                                    title={s.project}
                                                    >{agentDecodeName(
                                                        s.project,
                                                    )}</span
                                                >
                                                <span class="ud-row-meta"
                                                    >{s.model} &middot; {s.sessionId.slice(
                                                        0,
                                                        8,
                                                    )}</span
                                                >
                                            </div>
                                            <span class="ud-row-cost"
                                                >{agentFormatCost(s.cost)}</span
                                            >
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
                                            <span class="ud-tool-name"
                                                >{t.name}</span
                                            >
                                            <div class="ud-tool-bar">
                                                <div
                                                    class="ud-tool-fill"
                                                    style="width:{Math.max(
                                                        3,
                                                        (t.count /
                                                            (agentUsageData
                                                                .tools[0]
                                                                ?.count || 1)) *
                                                            100,
                                                    )}%"
                                                ></div>
                                            </div>
                                            <span class="ud-tool-ct"
                                                >{t.count.toLocaleString()}</span
                                            >
                                        </div>
                                    {/each}
                                </div>
                            </div>
                            <div class="ud-section">
                                <div class="ud-section-title">Shell</div>
                                <div class="ud-scroll">
                                    {#each agentUsageData.shellCommands.slice(0, 6) as cmd}
                                        <div class="ud-tool-row">
                                            <span
                                                class="ud-tool-name"
                                                style="font-family:var(--mono)"
                                                >{cmd.name}</span
                                            >
                                            <div class="ud-tool-bar">
                                                <div
                                                    class="ud-tool-fill"
                                                    style="width:{Math.max(
                                                        3,
                                                        (cmd.count /
                                                            (agentUsageData
                                                                .shellCommands[0]
                                                                ?.count || 1)) *
                                                            100,
                                                    )}%"
                                                ></div>
                                            </div>
                                            <span class="ud-tool-ct"
                                                >{cmd.count.toLocaleString()}</span
                                            >
                                        </div>
                                    {/each}
                                </div>
                            </div>
                        </div>
                    {:else}
                        <div class="ud-loading">
                            <svg
                                viewBox="0 0 24 24"
                                width="36"
                                height="36"
                                fill="none"
                                stroke="var(--t4)"
                                stroke-width="1.2"
                                stroke-linecap="round"
                                stroke-linejoin="round"
                                ><path d="M18 20V10M12 20V4M6 20v-6" /></svg
                            >
                            <p
                                style="margin: 0; font-size: 13px; color: var(--t2); font-weight: 500;"
                            >
                                No usage data found
                            </p>
                            <span style="font-size: 11px; color: var(--t3);"
                                >Start using Claude Code sessions to see
                                analytics here</span
                            >
                        </div>
                    {/if}
                    {/if}
                    </div>
                </div>
            {:else if activeTab === "shortcuts"}
                <div class="stg-section">
                    <span class="stg-section-label">Keyboard Shortcuts</span>
                    <div class="stg-shortcuts">
                        {#each SHORTCUTS as shortcut}
                            <div class="stg-shortcut-row">
                                <span class="stg-shortcut-desc"
                                    >{shortcut.desc}</span
                                >
                                <span class="stg-shortcut-keys">
                                    {#each shortcut.keys as key, i}
                                        <kbd class="kbd">{key}</kbd>
                                        {#if i < shortcut.keys.length - 1}
                                            <span class="stg-shortcut-plus"
                                                >+</span
                                            >
                                        {/if}
                                    {/each}
                                </span>
                            </div>
                        {/each}
                    </div>
                </div>
            {:else if activeTab === "about"}
                <div class="stg-card-stack stg-about">
                    <section class="stg-card stg-card-bare about-identity">
                        <div class="about-header">
                            <span class="about-app-name">Clauge</span>
                            <span class="about-version"
                                >v{appVersion || "—"}</span
                            >
                        </div>
                        <p class="about-desc">
                            An AI-powered cross-platform desktop super-app for
                            developers. One shell, many tools.
                        </p>
                    </section>

                    <section class="stg-card">
                        <header class="stg-card-hd">
                            <span class="stg-card-icon" aria-hidden="true">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="14"
                                    height="14"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    ><path
                                        d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"
                                    /><polyline
                                        points="7 10 12 15 17 10"
                                    /><line
                                        x1="12"
                                        y1="15"
                                        x2="12"
                                        y2="3"
                                    /></svg
                                >
                            </span>
                            <div class="stg-card-titles">
                                <h3 class="stg-card-title">Updates</h3>
                                <p class="stg-card-sub">
                                    Choose the release channel Clauge checks for
                                    new versions.
                                </p>
                            </div>
                        </header>
                        <div class="stg-card-body">
                            <label class="about-channel-row">
                                <input
                                    type="checkbox"
                                    checked={updateChannel === "pre"}
                                    onchange={onPreReleaseToggle}
                                />
                                <span class="about-channel-text">
                                    <span class="about-channel-title"
                                        >Receive pre-release updates</span
                                    >
                                    <span class="about-channel-desc"
                                        >Get alpha and beta builds before
                                        they're stable. May contain bugs.</span
                                    >
                                </span>
                            </label>
                        </div>
                    </section>

                    <section class="stg-card">
                        <header class="stg-card-hd">
                            <span class="stg-card-icon" aria-hidden="true">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="14"
                                    height="14"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    ><polyline
                                        points="12 2 2 7 12 12 22 7 12 2"
                                    /><polyline
                                        points="2 17 12 22 22 17"
                                    /><polyline
                                        points="2 12 12 17 22 12"
                                    /></svg
                                >
                            </span>
                            <div class="stg-card-titles">
                                <h3 class="stg-card-title">Tech Stack</h3>
                                <p class="stg-card-sub">
                                    What Clauge is built on.
                                </p>
                            </div>
                        </header>
                        <div class="stg-card-body">
                            <div class="about-tech-grid">
                                <span class="about-tech-pill">
                                    <svg viewBox="0 0 106 106" class="tech-icon"
                                        ><path
                                            d="M103.3 53.1c0-3.8-2-7.2-5.2-10.2-2.1-2-4.7-3.7-7.6-5.2.4-3.5.3-6.7-.4-9.5-1-4-3.2-7-6.5-8.9-3.1-1.8-6.8-2.2-10.6-1.4-2.6.5-5.3 1.6-8 3.1-2.4-2.6-5-4.8-7.8-6.4-4-2.3-8-3.3-11.8-2.8-3.9.5-7.2 2.4-9.6 5.6-1.7 2.3-2.9 5.1-3.7 8.2-3.4-.6-6.6-.6-9.4 0-4 .9-7.2 3-9.3 6.2-2 3-2.5 6.7-1.8 10.5.5 2.6 1.5 5.4 3 8.2-2.7 2.3-4.9 4.9-6.5 7.6-2.3 3.9-3.3 7.9-2.8 11.7.5 3.9 2.4 7.2 5.7 9.6 2.3 1.7 5.2 2.9 8.3 3.6-.6 3.4-.6 6.6 0 9.4.9 3.9 3 7.1 6.2 9.2 3 2 6.7 2.6 10.5 1.9 2.6-.5 5.4-1.6 8.2-3.1 2.4 2.7 5 4.9 7.7 6.5 3.9 2.3 7.9 3.3 11.7 2.8 3.9-.5 7.2-2.4 9.6-5.7 1.7-2.3 2.9-5.2 3.7-8.3 3.4.6 6.6.6 9.4 0 4-.9 7.2-3 9.3-6.2 2-3 2.5-6.7 1.8-10.5-.5-2.6-1.5-5.4-3-8.2 2.7-2.4 4.9-5 6.5-7.7 2.3-3.9 3.3-7.9 2.8-11.7z"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="6"
                                        /></svg
                                    >
                                    Rust
                                </span>
                                <span class="about-tech-pill">
                                    <svg viewBox="0 0 128 128" class="tech-icon"
                                        ><path
                                            d="M64 0C28.6 0 0 28.6 0 64s28.6 64 64 64 64-28.6 64-64S99.4 0 64 0zm0 110c-25.4 0-46-20.6-46-46S38.6 18 64 18s46 20.6 46 46-20.6 46-46 46z"
                                            fill="currentColor"
                                        /><circle
                                            cx="64"
                                            cy="64"
                                            r="24"
                                            fill="currentColor"
                                        /></svg
                                    >
                                    Tauri v2
                                </span>
                                <span class="about-tech-pill">
                                    <svg viewBox="0 0 24 24" class="tech-icon"
                                        ><path
                                            d="M12.1 2L1 21h22L12.1 2z"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="1.5"
                                            stroke-linejoin="round"
                                        /></svg
                                    >
                                    SvelteKit
                                </span>
                                <span class="about-tech-pill">
                                    <svg viewBox="0 0 24 24" class="tech-icon"
                                        ><rect
                                            x="2"
                                            y="3"
                                            width="20"
                                            height="18"
                                            rx="2"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="1.5"
                                        /><text
                                            x="12"
                                            y="16"
                                            text-anchor="middle"
                                            font-size="10"
                                            font-weight="700"
                                            font-family="sans-serif"
                                            fill="currentColor">TS</text
                                        ></svg
                                    >
                                    TypeScript
                                </span>
                                <span class="about-tech-pill">
                                    <svg viewBox="0 0 24 24" class="tech-icon"
                                        ><ellipse
                                            cx="12"
                                            cy="6"
                                            rx="8"
                                            ry="3"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="1.5"
                                        /><path
                                            d="M4 6v12c0 1.66 3.58 3 8 3s8-1.34 8-3V6"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="1.5"
                                        /><path
                                            d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="1.5"
                                        /></svg
                                    >
                                    SQLite
                                </span>
                                <span class="about-tech-pill">
                                    <svg viewBox="0 0 24 24" class="tech-icon"
                                        ><polyline
                                            points="16 18 22 12 16 6"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="1.5"
                                            stroke-linecap="round"
                                            stroke-linejoin="round"
                                        /><polyline
                                            points="8 6 2 12 8 18"
                                            fill="none"
                                            stroke="currentColor"
                                            stroke-width="1.5"
                                            stroke-linecap="round"
                                            stroke-linejoin="round"
                                        /></svg
                                    >
                                    CodeMirror
                                </span>
                            </div>
                        </div>
                    </section>

                    <section class="stg-card">
                        <header class="stg-card-hd">
                            <span class="stg-card-icon" aria-hidden="true">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="14"
                                    height="14"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    ><path
                                        d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"
                                    /><path
                                        d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"
                                    /></svg
                                >
                            </span>
                            <div class="stg-card-titles">
                                <h3 class="stg-card-title">Links</h3>
                                <p class="stg-card-sub">
                                    Project, issues, developer, website.
                                </p>
                            </div>
                        </header>
                        <div class="stg-card-body">
                            <div class="about-links">
                                <a
                                    class="about-link-btn"
                                    href="https://github.com/ansxuman/Clauge"
                                    target="_blank"
                                    rel="noopener"
                                    title="GitHub Repository"
                                >
                                    <svg viewBox="0 0 24 24"
                                        ><path
                                            d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 00-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0020 4.77 5.07 5.07 0 0019.91 1S18.73.65 16 2.48a13.38 13.38 0 00-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 005 4.77a5.44 5.44 0 00-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 009 18.13V22"
                                        /></svg
                                    >
                                    <span>Project</span>
                                </a>
                                <a
                                    class="about-link-btn"
                                    href="https://github.com/ansxuman/Clauge/issues/new"
                                    target="_blank"
                                    rel="noopener"
                                    title="Report an Issue"
                                >
                                    <svg viewBox="0 0 24 24"
                                        ><circle cx="12" cy="12" r="10" /><path
                                            d="M12 8v4M12 16h.01"
                                        /></svg
                                    >
                                    <span>Report Issue</span>
                                </a>
                                <a
                                    class="about-link-btn"
                                    href="https://github.com/ansxuman"
                                    target="_blank"
                                    rel="noopener"
                                    title="Developer"
                                >
                                    <svg viewBox="0 0 24 24"
                                        ><path
                                            d="M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2"
                                        /><circle cx="12" cy="7" r="4" /></svg
                                    >
                                    <span>Developer</span>
                                </a>
                                <a
                                    class="about-link-btn"
                                    href="https://clauge.in/"
                                    target="_blank"
                                    rel="noopener"
                                    title="Website"
                                >
                                    <svg viewBox="0 0 24 24"
                                        ><circle cx="12" cy="12" r="10" /><line
                                            x1="2"
                                            y1="12"
                                            x2="22"
                                            y2="12"
                                        /><path
                                            d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z"
                                        /></svg
                                    >
                                    <span>Website</span>
                                </a>
                            </div>
                        </div>
                    </section>

                    <section class="stg-card">
                        <header class="stg-card-hd">
                            <span class="stg-card-icon" aria-hidden="true">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="14"
                                    height="14"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    ><path
                                        d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"
                                    /></svg
                                >
                            </span>
                            <div class="stg-card-titles">
                                <h3 class="stg-card-title">Support</h3>
                                <p class="stg-card-sub">
                                    If Clauge helped you in development,
                                    consider buying the developer a coffee.
                                </p>
                            </div>
                        </header>
                        <div class="stg-card-body">
                            <a
                                class="about-coffee"
                                href="https://buymeacoffee.com/ansxuman"
                                target="_blank"
                                rel="noopener"
                            >
                                <svg viewBox="0 0 24 24"
                                    ><path d="M17 8h1a4 4 0 110 8h-1" /><path
                                        d="M3 8h14v9a4 4 0 01-4 4H7a4 4 0 01-4-4V8z"
                                    /><line x1="6" y1="2" x2="6" y2="4" /><line
                                        x1="10"
                                        y1="2"
                                        x2="10"
                                        y2="4"
                                    /><line
                                        x1="14"
                                        y1="2"
                                        x2="14"
                                        y2="4"
                                    /></svg
                                >
                                Buy me a coffee
                            </a>
                        </div>
                    </section>
                </div>
            {/if}
        </div>
    </div>
</Modal>

<ConfirmDialog
    bind:show={showClearChatHistoryConfirm}
    title="Clear History"
    message={`This will clear ${restHistoryCount.toLocaleString()} request${restHistoryCount === 1 ? "" : "s"} from the History section and ${aiChatCount.toLocaleString()} AI Assistance chat message${aiChatCount === 1 ? "" : "s"} across all modes.\n\nThis cannot be undone.`}
    confirmText="Clear History"
    onconfirm={handleClearChatHistory}
/>

<style>
    @import "./SettingsModal.svelte.css";

    /* ------- Settings cards ------- */
    /* Reusable across all settings tabs (General / REST / AI / Agent /
     About). Each .stg-card has a header (icon + title + subtitle) and a
     body of .stg-card-row rows. */

    .stg-card-stack {
        display: flex;
        flex-direction: column;
        gap: 14px;
    }

    .stg-card {
        border: 1px solid var(--b1);
        border-radius: 10px;
        background: linear-gradient(
            180deg,
            rgba(255, 255, 255, 0.025) 0%,
            rgba(255, 255, 255, 0.005) 100%
        );
        overflow: hidden;
    }
    /* Frameless variant — no border, no background. Used by the About
     identity block so the app name/version doesn't read as "boxed in". */
    .stg-card.stg-card-bare {
        border: none;
        background: none;
        overflow: visible;
    }
    .about-identity {
        padding: 8px 4px 16px;
    }
    .about-identity .about-header {
        margin-bottom: 6px;
    }
    .about-identity .about-desc {
        margin: 0;
    }

    .stg-card-hd {
        display: flex;
        align-items: flex-start;
        gap: 12px;
        padding: 14px 16px 12px;
    }
    /* Pin the last child of the header to the right (used for trailing
     actions like the AI Overview "Reset" button). The .stg-card-titles
     keeps its natural width so the action floats. */
    .stg-card-hd > :last-child:not(.stg-card-titles):not(.stg-card-icon) {
        margin-left: auto;
        flex-shrink: 0;
    }

    .stg-card-icon {
        flex-shrink: 0;
        width: 28px;
        height: 28px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border-radius: 8px;
        border: 1px solid var(--b1);
        background: rgba(255, 255, 255, 0.03);
        color: var(--t2);
    }

    .stg-card-titles {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
    }

    .stg-card-title {
        margin: 0;
        font-size: 13px;
        font-weight: 600;
        color: var(--t1);
        font-family: var(--ui);
        letter-spacing: 0;
        text-transform: none;
    }

    .stg-card-sub {
        margin: 0;
        font-size: 11.5px;
        line-height: 1.5;
        color: var(--t3);
        font-family: var(--ui);
    }

    .stg-card-body {
        padding: 4px 16px 14px;
        display: flex;
        flex-direction: column;
    }

    .stg-card-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        padding: 10px 0;
        border-top: 1px solid var(--b-subtle, rgba(255, 255, 255, 0.05));
    }
    .stg-card-row:first-child {
        border-top: none;
    }

    .stg-card-row-label {
        font-size: 11.5px;
        font-weight: 500;
        color: var(--t2);
        font-family: var(--ui);
        white-space: nowrap;
    }

    .stg-card-row-help {
        display: block;
        margin-top: 2px;
        font-size: 11px;
        color: var(--t4);
        font-family: var(--ui);
        line-height: 1.4;
        white-space: normal;
    }

    .stg-card-row-action {
        align-items: flex-start;
    }

    .stg-card-row-action-text {
        display: flex;
        flex-direction: column;
        min-width: 0;
        flex: 1;
    }

    .stg-card-input-lg {
        flex: 1;
        min-width: 0;
        max-width: 240px;
    }

    /* Stat strip — sits between the card header and body. Big numbers,
     small uppercase labels. Mono font keeps everything aligned. */
    .stg-card-stats {
        display: flex;
        align-items: stretch;
        gap: 0;
        margin: 2px 16px 6px;
        padding: 10px 4px;
        border-top: 1px solid var(--b-subtle, rgba(255, 255, 255, 0.05));
        border-bottom: 1px solid var(--b-subtle, rgba(255, 255, 255, 0.05));
    }

    .stg-card-stat {
        flex: 1;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 2px;
        padding: 0 8px;
        text-align: center;
        min-width: 0;
    }

    .stg-card-stat-num {
        font-family: var(--mono);
        font-size: 16px;
        font-weight: 600;
        color: var(--t1);
        letter-spacing: -0.01em;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        max-width: 100%;
    }

    .stg-card-stat-lbl {
        font-size: 9.5px;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        color: var(--t4);
        font-family: var(--ui);
        font-weight: 500;
    }

    .stg-card-stat-divider {
        width: 1px;
        background: var(--b1);
        margin: 4px 0;
    }

    .stg-card-danger-btn {
        flex-shrink: 0;
        height: 30px;
        padding: 0 14px;
        display: inline-flex;
        align-items: center;
        gap: 6px;
        border-radius: 6px;
        border: 1px solid var(--b1);
        background: transparent;
        color: var(--err);
        font-size: 11.5px;
        font-family: var(--ui);
        font-weight: 500;
        cursor: default;
        transition:
            border-color 0.12s,
            background 0.12s,
            color 0.12s;
    }
    .stg-card-danger-btn:hover {
        border-color: var(--err);
        background: rgba(240, 68, 68, 0.08);
    }
    .stg-card-danger-btn:active {
        transform: translateY(1px);
    }

    /* ── MCP card extras ───────────────────────────────────────── */
    .stg-card-mono {
        font-family: var(--mono);
        font-size: 11px;
        background: rgba(255, 255, 255, 0.05);
        padding: 1px 5px;
        border-radius: 3px;
        color: var(--t2);
    }
    .stg-card-pill {
        margin-left: auto;
        flex-shrink: 0;
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 4px 10px;
        border-radius: 12px;
        border: 1px solid var(--b1);
        background: rgba(255, 255, 255, 0.03);
        font-family: var(--mono);
        font-size: 10.5px;
        color: var(--t3);
    }
    .stg-card-pill.on {
        border-color: color-mix(in srgb, var(--ok, #1dc880) 40%, var(--b1));
        background: color-mix(in srgb, var(--ok, #1dc880) 10%, transparent);
        color: var(--ok, #1dc880);
    }
    .stg-card-pill-dot {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--t4);
    }
    .stg-card-pill.on .stg-card-pill-dot {
        background: var(--ok, #1dc880);
        box-shadow: 0 0 6px var(--ok, #1dc880);
    }

    .stg-card-token-row {
        display: flex;
        align-items: center;
        gap: 6px;
        flex: 1;
        min-width: 0;
    }
    .stg-card-mini-btn {
        flex-shrink: 0;
        height: 26px;
        padding: 0 10px;
        border-radius: 5px;
        border: 1px solid var(--b1);
        background: transparent;
        color: var(--t2);
        font-family: var(--ui);
        font-size: 11px;
        cursor: default;
        transition: border-color 0.12s, color 0.12s;
    }
    .stg-card-mini-btn:hover:not(:disabled) {
        border-color: var(--b2);
        color: var(--t1);
    }
    .stg-card-mini-btn:disabled {
        opacity: 0.4;
        cursor: not-allowed;
    }
</style>

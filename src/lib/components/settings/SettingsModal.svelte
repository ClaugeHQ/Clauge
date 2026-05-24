<script lang="ts">
    import ConfirmDialog from "$lib/shared/primitives/ConfirmDialog.svelte";
    import ClaugeAIBalance from "$lib/components/settings/ClaugeAIBalance.svelte";
    import { invoke } from "@tauri-apps/api/core";
    import { cloudPlan, cloudSub, upgradeModalOpen } from "$lib/stores/cloud";
    import AccountTabContent from "$lib/components/settings/AccountTabContent.svelte";
    import { getVersion } from "@tauri-apps/api/app";
    import {
        tabs as sharedTabs,
        activeTabId,
        updateTab,
    } from "$lib/shared/stores/tabs";
    import {
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
    import {
        countHistory,
        purgeHistory,
        restHistorySizeBytes,
    } from "$lib/modes/rest/commands";
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
  import { errorToast, friendlyError } from '$lib/utils/errors';
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
        | "sql"
        | "nosql"
        | "explorer"
        | "about";

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

    // Settings is now a topbar tab (mode: 'settings'). Visibility +
    // initial sub-tab are derived from the active tab; callers open it
    // via `openSettingsTab(subKey)` which sets tab.key.
    const settingsTab = $derived(
        $sharedTabs.find((t) => t.id === $activeTabId && t.mode === "settings"),
    );
    let show = $derived(!!settingsTab);

    $effect(() => {
        const key = settingsTab?.key ?? null;
        if (!key) return;
        // Map sub-key → activeTab + (optional) agentSubTab. Mirrors the
        // legacy `activeModal === 'settings:*'` mapping but flattened
        // (the 'settings:' prefix is implicit now).
        if (key === "account") activeTab = "account";
        else if (key === "general") activeTab = "general";
        else if (key === "appearance") activeTab = "appearance";
        else if (key === "shortcuts") activeTab = "shortcuts";
        else if (key === "ai") activeTab = "ai";
        else if (key === "rest") activeTab = "rest";
        else if (key === "agent") activeTab = "agent";
        else if (key === "agent:usage") {
            activeTab = "agent";
            agentSubTab = "usage";
        } else if (key === "agent:contexts") {
            activeTab = "agent";
            agentSubTab = "contexts";
        } else if (key === "agent:plugins") {
            activeTab = "agent";
            agentSubTab = "plugins";
        } else if (key === "workspace") activeTab = "workspace";
        else if (key === "sql") activeTab = "sql";
        else if (key === "nosql") activeTab = "nosql";
        else if (key === "explorer") activeTab = "explorer";
        else if (key === "about") activeTab = "about";
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

    // --- SQL ---
    // Defaults match the previously-hardcoded values in
    // `src-tauri/src/modes/sql/{client,d1_client,clickhouse_client}.rs`
    // and `modes/sql/ai_tools.rs`. Changes apply to NEW connections /
    // queries — already-open pools keep their old timeouts until
    // reconnect.
    let sqlAcquireTimeoutMs = $derived(
        Number($settings["sql_acquire_timeout_ms"] ?? "10000"),
    );
    let sqlIdleTimeoutMin = $derived(
        Number($settings["sql_idle_timeout_min"] ?? "30"),
    );
    let sqlHttpQueryTimeoutSec = $derived(
        Number($settings["sql_http_query_timeout_sec"] ?? "60"),
    );
    let sqlTableListLimit = $derived(
        Number($settings["sql_table_list_limit"] ?? "200"),
    );

    // --- NoSQL ---
    // Defaults match Mongo client config in
    // `src-tauri/src/modes/nosql/client.rs` + the AI-tool find caps in
    // `modes/nosql/ai_tools.rs`.
    let nosqlServerSelectionTimeoutMs = $derived(
        Number($settings["nosql_server_selection_timeout_ms"] ?? "10000"),
    );
    let nosqlConnectTimeoutMs = $derived(
        Number($settings["nosql_connect_timeout_ms"] ?? "10000"),
    );
    let nosqlDefaultFindLimit = $derived(
        Number($settings["nosql_default_find_limit"] ?? "50"),
    );
    let nosqlMaxFindLimit = $derived(
        Number($settings["nosql_max_find_limit"] ?? "100"),
    );

    // --- Explorer ---
    // S3/Azure folder listings can recursively walk every subfolder to
    // compute aggregate folder sizes. On large buckets this slows
    // navigation from sub-second to multi-second. Default OFF; the user
    // opts in when accurate sizes matter more than latency. Backend
    // reads on session open, so changes apply on next reconnect.
    let explorerShowFolderSizes = $derived(
        ($settings["explorer_show_folder_sizes"] ?? "false") === "true",
    );

    // --- Editor ---

    // --- Proxy ---
    let proxyUrl = $derived($settings["proxy_url"] ?? "");
    let proxyAuth = $derived(($settings["proxy_auth"] ?? "false") === "true");
    let proxyUsername = $derived($settings["proxy_username"] ?? "");
    let proxyPassword = $derived($settings["proxy_password"] ?? "");

    // --- Privacy / telemetry ---
    // Telemetry is opt-IN by default (false === sending). The toggle
    // below flips `telemetry_optout`; UI label is positive ("Share
    // anonymous usage data") so the checkbox feels intuitive.
    let telemetryOptedOut = $derived(
        ($settings["telemetry_optout"] ?? "false") === "true",
    );

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
    let restHistoryBytes = $state<number>(0);
    let showClearChatHistoryConfirm = $state(false);
    /** Combined storage = AI chat localStorage + REST history DB bytes.
     *  Before this was just AI chat, which made the "Storage" stat
     *  understate the real footprint by ignoring the (now-metadata-only)
     *  REST history table. */
    let totalStorageBytes = $derived(aiChatBytes + restHistoryBytes);

    async function refreshHistorySizes() {
        try {
            restHistoryCount = await countHistory();
        } catch {
            restHistoryCount = 0;
        }
        try {
            restHistoryBytes = await restHistorySizeBytes();
        } catch {
            restHistoryBytes = 0;
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

    // Auto-move-on-PR-merge toggle. Default ON — the feature is
    // self-disabling for boards whose final column doesn't match the
    // regex, so leaving it on is safe; this toggle is for users who
    // want manual control even on boards that DO match.
    let autoMoveMergedPrs = $derived(
        ($settings["workspace_automove_merged_prs"] ?? "true") === "true",
    );

    async function refreshMcpStatus() {
        try {
            mcpStatus = await workspaceMcpStatus();
            mcpStatusStore.set(mcpStatus);
        } catch {
            /* ignore */
        }
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
                showToast(
                    `MCP server running on :${mcpStatus.port}`,
                    "success",
                );
            } else {
                mcpStatus = await workspaceMcpStop();
                mcpStatusStore.set(mcpStatus);
                await workspaceMcpUnregister();
                await setSetting("workspace_mcp_enabled", "false");
                showToast("MCP server stopped", "success");
            }
        } catch (e) {
            errorToast('MCP toggle failed', e);
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
            errorToast('Rotate failed', e);
        }
    }

    async function handleCopyMcpToken() {
        try {
            await navigator.clipboard.writeText(mcpToken);
            showToast("Token copied", "success");
        } catch {
            /* ignore */
        }
    }

    // AI Assistance state
    let aiTopTab = $state<"clauge" | "byok">("clauge");
    let aiSubTab = $state<"config" | "usage">("config");

    // Clauge AI usage history — fetched from /api/ai/usage via cloud_ai_usage.
    // Worker returns the most recent entries first; we render the last ~30.
    type CloudAiUsageEntry = {
        occurred_at: string;
        operation: string;
        clauge_credits: number;
        cost_usd_micros: number;
        request_id: string;
        mode: string | null;
    };
    let cloudAiUsage = $state<CloudAiUsageEntry[]>([]);
    let cloudAiUsageLoading = $state(false);
    // Cursor for the next page (oldest visible row's occurred_at). When
    // null we've hit the end of history.
    let cloudAiUsageNextBefore = $state<string | null>(null);
    let cloudAiUsageLoadingMore = $state(false);

    const CLOUD_AI_USAGE_PAGE = 30;

    async function loadCloudAiUsage() {
        cloudAiUsageLoading = true;
        try {
            const r = await invoke<{
                entries: CloudAiUsageEntry[];
                next_before: string | null;
            }>("cloud_ai_usage", { limit: CLOUD_AI_USAGE_PAGE });
            cloudAiUsage = r.entries ?? [];
            cloudAiUsageNextBefore = r.next_before;
        } catch {
            cloudAiUsage = [];
            cloudAiUsageNextBefore = null;
        } finally {
            cloudAiUsageLoading = false;
        }
    }

    // Append the next page when the user scrolls near the bottom of the
    // activity table. Cursor is the oldest row's occurred_at returned by
    // the previous response.
    async function loadMoreCloudAiUsage() {
        if (cloudAiUsageLoadingMore) return;
        if (!cloudAiUsageNextBefore) return;
        cloudAiUsageLoadingMore = true;
        try {
            const r = await invoke<{
                entries: CloudAiUsageEntry[];
                next_before: string | null;
            }>("cloud_ai_usage", {
                limit: CLOUD_AI_USAGE_PAGE,
                before: cloudAiUsageNextBefore,
            });
            if (r.entries?.length) {
                cloudAiUsage = [...cloudAiUsage, ...r.entries];
            }
            cloudAiUsageNextBefore = r.next_before;
        } catch {
            // Silent — keep cursor, user can scroll again to retry.
        } finally {
            cloudAiUsageLoadingMore = false;
        }
    }

    // Fires from the activity table's scroll listener. Trigger ~120px
    // before the bottom so the next page is already loading by the time
    // the user reaches it (no perceptible pause).
    function handleActivityScroll(e: Event) {
        const el = e.currentTarget as HTMLElement;
        if (
            el.scrollHeight - el.scrollTop - el.clientHeight < 120 &&
            cloudAiUsageNextBefore
        ) {
            loadMoreCloudAiUsage();
        }
    }

    function fmtUsageTime(iso: string): string {
        try {
            // The worker writes `occurred_at` via SQLite's CURRENT_TIMESTAMP,
            // which returns "YYYY-MM-DD HH:MM:SS" in UTC but WITHOUT any
            // timezone marker. `new Date(naiveString)` treats it as LOCAL,
            // so non-UTC users saw times shifted by their offset. Normalise
            // the SQLite shape to a proper UTC ISO before parsing; leave
            // already-tagged strings (T+Z, T+offset) alone.
            const isNaiveSqlite = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(
                iso,
            );
            const utcIso = isNaiveSqlite ? iso.replace(" ", "T") + "Z" : iso;
            const d = new Date(utcIso);
            return d.toLocaleString(undefined, {
                month: "short",
                day: "numeric",
                hour: "numeric",
                minute: "2-digit",
            });
        } catch {
            return iso;
        }
    }

    // Distinct color per mode for the per-mode breakdown bars and the
    // MODE column in the activity table. Only the modes that actually
    // host the AI panel (rest / sql / nosql / ssh / explorer) get colors;
    // anything else falls back to muted grey.
    function modeColor(mode: string | null | undefined): string {
        switch (mode) {
            case "sql":
                return "#8b7bf5";
            case "rest":
                return "#f06b9e";
            case "nosql":
                return "#4ec9ff";
            case "ssh":
                return "#2dd4bf";
            case "explorer":
                return "#f59e0b";
            default:
                return "var(--t3, #888)";
        }
    }
    function capitalizeMode(s: string): string {
        if (!s) return "—";
        if (s === "sql" || s === "rest" || s === "ssh") return s.toUpperCase();
        if (s === "nosql") return "NoSQL";
        return s[0].toUpperCase() + s.slice(1);
    }
    // Per-mode credit totals derived from the loaded usage entries. Sorted
    // descending so the heaviest mode comes first in the bar chart.
    type ModeBreakdown = { mode: string; credits: number; percent: number };
    let cloudAiByMode = $derived.by<ModeBreakdown[]>(() => {
        if (cloudAiUsage.length === 0) return [];
        const totals = new Map<string, number>();
        let grand = 0;
        for (const e of cloudAiUsage) {
            const m = e.mode ?? "other";
            totals.set(m, (totals.get(m) ?? 0) + e.clauge_credits);
            grand += e.clauge_credits;
        }
        if (grand === 0) return [];
        return Array.from(totals.entries())
            .map(([mode, credits]) => ({
                mode,
                credits,
                percent: Math.round((credits / grand) * 100),
            }))
            .sort((a, b) => b.credits - a.credits);
    });
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

    // Every provider that has a saved API key — feeds the "Configured
    // providers" list under the Configuration sub-tab so the user always
    // sees what they've set up.
    let configuredProviders = $derived(
        AI_PROVIDER_REGISTRY.filter(
            (p) => !!$settings[p.keySettingName]?.trim(),
        ),
    );

    // --- Clauge AI balance (worker-tracked managed-AI credits) ---
    let cloudCreditsLocal = $state<{
        remaining: number;
        allowance: number;
        resets_at: string | null;
    } | null>(null);
    let cloudSubLocal = $state<{
        status: string;
        cancel_at_period_end: boolean;
    } | null>(null);

    async function loadCloudBalance() {
        try {
            const bal = await invoke<{
                remaining: number;
                allowance: number;
                resetsAt: string | null;
            }>("cloud_ai_balance");
            cloudCreditsLocal = {
                remaining: bal.remaining,
                allowance: bal.allowance,
                resets_at: bal.resetsAt,
            };
        } catch {
            cloudCreditsLocal = null;
        }
    }

    function handleUpgradeClick() {
        upgradeModalOpen.set(true);
    }

    // --- Appearance ---
    let currentTheme = $derived($appearance.theme || "dark-glass");
    let accentColor = $derived(
        $appearance.accentColor || FALLBACK_ACCENT_COLOR,
    );
    // Pro themes can lock the accent — the picker is disabled and a small
    // note tells the user why. `getTheme()` looks up the theme registry.
    let activeThemeDef = $derived(getTheme(currentTheme));
    let accentLocked = $derived(activeThemeDef?.lockAccent === true);

    // Re-export for template usage (Svelte each blocks resolve from script scope).
    const ACCENT_COLORS = ACCENT_PALETTE;

    const THEME_DESCRIPTIONS: Record<string, string> = {
        "dark-glass": "Translucent with native blur",
        "dark-solid": "Opaque dark with purple tints",
        midnight: "Pure black, zero distraction",
        "rose-pine-dawn": "Warm cream light — sister to Moon",
        atelier:
            "Blush + botanical wallpaper — pixel critters wander the footer",
        hearth: "Warm charcoal with rising embers — animated firelight",
        petal: "Plum twilight with falling cherry petals",
        celeste: "Cosmic violet-black with twinkling stars",
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
            key: "workspace",
            label: "Workspace",
            // 2×2 grid — same identity glyph used in the main Sidebar.
            icon: '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
        },
        {
            kind: "tab",
            key: "rest",
            label: "REST",
            icon: '<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z"/>',
        },
        {
            kind: "tab",
            key: "sql",
            label: "SQL",
            // DB cylinder — matches the SQL tab icon in Topbar.
            icon: '<ellipse cx="12" cy="5" rx="8" ry="2.5"/><path d="M4 5v14c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5V5"/><path d="M4 12c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5"/>',
        },
        {
            kind: "tab",
            key: "nosql",
            label: "NoSQL",
            // Curly braces — matches NoSQL identity in Topbar.
            icon: '<path d="M8 3a2 2 0 00-2 2v4a2 2 0 01-2 2H3a1 1 0 000 2h1a2 2 0 012 2v4a2 2 0 002 2"/><path d="M16 3a2 2 0 012 2v4a2 2 0 002 2h1a1 1 0 010 2h-1a2 2 0 00-2 2v4a2 2 0 01-2 2"/>',
        },
        {
            kind: "tab",
            key: "explorer",
            label: "Explorer",
            // Folder — matches Explorer identity in Topbar.
            icon: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z"/>',
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
        { desc: "Switch to Nth tab", keys: [m, "1…9"] },
        {
            desc: "Switch mode (Agent · Workspace · REST · SQL · NoSQL · SSH · Explorer · History)",
            keys: [m, "Shift", "1…8"],
        },
        { desc: "New tab (current mode)", keys: [m, "T"] },
        { desc: "Close active tab", keys: [m, "W"] },
        { desc: "Send request / Run query", keys: [m, "Enter"] },
        { desc: "Save current request / query", keys: [m, "S"] },
        { desc: "Toggle nav sidebar", keys: [m, "B"] },
        { desc: "Toggle AI panel · in Agent: shell panel", keys: [m, "L"] },
        { desc: "Show shortcuts overlay", keys: [m, "/"] },
        { desc: "Show shortcuts overlay (not in input)", keys: ["?"] },
        { desc: "Close modal / overlay", keys: ["Esc"] },
        {
            desc: "Toggle fullscreen",
            keys: isMac() ? ["Cmd", "Ctrl", "F"] : ["F11"],
        },
        ...(isMac() ? [{ desc: "Minimize window", keys: ["Cmd", "M"] }] : []),
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
                agentUsageAuthStatus.set({
                    state: "unconfigured",
                    message: "",
                });
            }
        } else if (key === "agent_codex_access_token") {
            agentCodexTokenStore.set(value);
            if (value) {
                loadAgentUsageLimits();
            } else {
                agentUsageLimits.set(null);
                agentUsageAuthStatus.set({
                    state: "unconfigured",
                    message: "",
                });
            }
        } else if (key === "agent_footer_usage_provider") {
            if (
                value === "claude" ||
                value === "codex" ||
                value === "gemini" ||
                value === "opencode"
            ) {
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

    // Theme preview state for free users. When non-null, a pro theme is
    // being previewed DOM-only (not persisted). previewOriginalTheme is
    // the saved theme we revert to on cancel / close / tab-leave.
    let previewThemeId = $state<string | null>(null);
    let previewOriginalTheme = $state<string | null>(null);

    // What the picker should highlight as active — the previewed theme
    // if one is being tried, else the actually-saved theme.
    let displayTheme = $derived(previewThemeId ?? currentTheme);

    async function handleThemeChange(themeId: string) {
        const themeDef = getTheme(themeId);
        const isPremium = !!themeDef?.premium;
        const isFreeUser = $cloudPlan !== "pro";

        if (isPremium && isFreeUser) {
            // Enter / swap preview mode. Apply DOM-only — do NOT persist.
            if (previewOriginalTheme === null) {
                previewOriginalTheme = currentTheme;
            }
            previewThemeId = themeId;
            applyTheme(themeId, accentColor);
            return;
        }

        // Non-premium pick OR pro user: drop any active preview and persist.
        if (previewThemeId !== null) {
            previewThemeId = null;
            previewOriginalTheme = null;
        }
        applyTheme(themeId, accentColor);
        const config: AppearanceConfig = {
            theme: themeId,
            accentColor: accentColor,
        };
        appearance.set(config);
        await saveAppearance(config);
    }

    // User clicked Cancel in the preview banner — revert to their saved
    // theme and exit preview.
    function cancelThemePreview() {
        if (previewOriginalTheme === null) return;
        applyTheme(previewOriginalTheme, accentColor);
        previewThemeId = null;
        previewOriginalTheme = null;
    }

    // User clicked Upgrade in the preview banner — open the upgrade
    // modal. The plan-flip $effect below handles auto-finalize if the
    // upgrade succeeds; if they dismiss the modal we keep the preview
    // active so they can see it and choose Cancel.
    function upgradeFromThemePreview() {
        upgradeModalOpen.set(true);
    }

    // Auto-revert preview when the user navigates away from Appearance or
    // closes the Settings modal entirely — otherwise they'd return to a
    // pro theme they never actually own.
    $effect(() => {
        if (previewThemeId === null) return;
        if (!show || activeTab !== "appearance") {
            cancelThemePreview();
        }
    });

    // If the user successfully upgrades while a preview is showing, save
    // the previewed theme as their real choice and exit preview mode.
    $effect(() => {
        if (previewThemeId === null) return;
        if ($cloudPlan === "pro") {
            const themeId = previewThemeId;
            previewThemeId = null;
            previewOriginalTheme = null;
            const config: AppearanceConfig = {
                theme: themeId,
                accentColor: accentColor,
            };
            appearance.set(config);
            saveAppearance(config).catch(() => {});
        }
    });

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
            "gemini-3.1-flash-lite-preview": "Gemini 3.1 Flash-Lite",
            "qwen/qwen3-32b": "Qwen3 32B",
        };
        return names[model] || model;
    }

    let aiSettingsLoaded = false;
    $effect(() => {
        if (activeTab === "ai" && show && !aiSettingsLoaded) {
            aiSettingsLoaded = true;
            loadAiSettings();
            loadCloudBalance();
            loadCloudAiUsage();
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
            // Provider tracks the footer toggle so Settings reflects
            // the same scope as the floating Usage Dashboard.
            agentUsageData = await agentGetUsageAnalytics(
                agentUsageDays,
                agentFooterProvider,
            );
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

    // Provider tabs. Claude uses marketplace-directory + CLI subcommand
    // model; Codex's plugin lifecycle lives in its TUI (`/plugins`
    // slash command) so we list/toggle by parsing ~/.codex/config.toml
    // and show an "install inside Codex" hint. OpenCode is npm-based
    // (no marketplace) — excluded.
    const PLUGIN_PROVIDER_TABS: { id: string; label: string }[] = [
        { id: "claude", label: "Claude" },
        { id: "codex", label: "Codex" },
    ];
    let pluginProvider = $state<string>("claude");

    async function loadAgentPlugins() {
        try {
            installedPlugins = await agentGetPlugins(pluginProvider);
        } catch {
            installedPlugins = [];
        }
        try {
            marketplacePlugins =
                await agentGetMarketplacePlugins(pluginProvider);
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
            await agentTogglePlugin(name, enabled, pluginProvider);
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
            await agentUninstallPlugin(name, marketplace, pluginProvider);
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
            await agentInstallPlugin(name, marketplace, pluginProvider);
            marketplacePlugins = marketplacePlugins.map((p) =>
                p.name === name ? { ...p, installed: true } : p,
            );
            await loadAgentPlugins();
            showToast("Plugin installed", "success");
        } catch {
            showToast("Failed to install plugin", "error");
        }
    }

    // Switching tabs reloads the list against the new provider's tooling.
    // Codex has no marketplace browser in Clauge, so we lock its view
    // to 'installed' — flipping to 'marketplace' there would show
    // perpetual-empty state and confuse users.
    function selectPluginProvider(id: string) {
        if (pluginProvider === id) return;
        pluginProvider = id;
        if (id === "codex") pluginView = "installed";
        installedPlugins = [];
        marketplacePlugins = [];
        loadAgentPlugins();
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

    // Reload Usage Stats whenever the panel is open AND the footer
    // provider toggle changes — picking OpenCode in the dropdown
    // should immediately retarget the analytics query, same as the
    // floating Usage Dashboard. The dependency on agentFooterProvider
    // makes the effect re-run.
    $effect(() => {
        const _ = agentFooterProvider; // dependency
        if (agentSubTab === "usage" && show) {
            loadAgentUsage();
        }
    });
</script>

{#if show}
    <div class="stg-pane glass-surface" role="region" aria-label="Settings">
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
                            onclick={() => {
                                if (settingsTab)
                                    updateTab(settingsTab.id, {
                                        key: item.key,
                                    });
                                else activeTab = item.key;
                            }}
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
                                        Applies system-wide. Leave blank to
                                        connect directly.
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
                                                    String(
                                                        e.currentTarget.checked,
                                                    ),
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
                                        /><polyline
                                            points="14 2 14 8 20 8"
                                        /><line
                                            x1="9"
                                            y1="13"
                                            x2="15"
                                            y2="13"
                                        /><line
                                            x1="9"
                                            y1="17"
                                            x2="15"
                                            y2="17"
                                        />
                                    </svg>
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">Logs</h3>
                                    <p class="stg-card-sub">
                                        Per-hour log files organized by day.
                                        Logs older than 30 days are removed
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
                                        (request/response log) and the AI
                                        Assistance chat across all modes.
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
                                        >{formatBytes(totalStorageBytes)}</span
                                    >
                                    <span class="stg-card-stat-lbl"
                                        >Storage</span
                                    >
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
                                            <polyline
                                                points="3 6 5 6 21 6"
                                            /><path
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

                        <!-- Privacy / telemetry card -->
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
                                            d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"
                                        />
                                    </svg>
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">Privacy</h3>
                                    <p class="stg-card-sub">
                                        Help improve Clauge with anonymous usage
                                        data.
                                    </p>
                                </div>
                            </header>
                            <div class="stg-card-body">
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Share anonymous usage data</label
                                    >
                                    <label class="stg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={!telemetryOptedOut}
                                            onchange={(e) =>
                                                handleSettingChange(
                                                    "telemetry_optout",
                                                    String(
                                                        !e.currentTarget
                                                            .checked,
                                                    ),
                                                )}
                                        />
                                        <span class="stg-toggle-slider"></span>
                                    </label>
                                </div>
                            </div>
                        </section>
                    </div>
                {:else if activeTab === "workspace"}
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
                                        ><polyline
                                            points="16 18 22 12 16 6"
                                        /><polyline
                                            points="8 6 2 12 8 18"
                                        /></svg
                                    >
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">MCP Server</h3>
                                    <p class="stg-card-sub">
                                        Local MCP server that lets MCP-aware
                                        coding agents read and edit your Notes
                                        and Boards via tool calls. Each
                                        installed agent's config is registered
                                        automatically when enabled.
                                    </p>
                                </div>
                                <span
                                    class="stg-card-pill"
                                    class:on={mcpStatus.running}
                                >
                                    <span class="stg-card-pill-dot"></span>
                                    {mcpStatus.running
                                        ? `Running · :${mcpStatus.port}`
                                        : "Stopped"}
                                </span>
                            </header>

                            <div class="stg-card-body">
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Enable MCP server</label
                                    >
                                    <label class="stg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={mcpStatus.running}
                                            onchange={(e) =>
                                                handleMcpToggle(
                                                    e.currentTarget.checked,
                                                )}
                                        />
                                        <span class="stg-toggle-slider"></span>
                                    </label>
                                </div>

                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Port</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="number"
                                        min="1024"
                                        max="65535"
                                        value={mcpPort}
                                        disabled={mcpStatus.running}
                                        onchange={(e) =>
                                            (mcpPort =
                                                Number(e.currentTarget.value) ||
                                                7421)}
                                    />
                                </div>

                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Auth token</label
                                    >
                                    <div class="stg-card-token-row">
                                        <input
                                            class="stg-input stg-card-input-lg"
                                            type={showMcpToken
                                                ? "text"
                                                : "password"}
                                            value={mcpToken}
                                            readonly
                                        />
                                        <button
                                            class="stg-card-mini-btn"
                                            onclick={() =>
                                                (showMcpToken = !showMcpToken)}
                                            title={showMcpToken
                                                ? "Hide"
                                                : "Show"}
                                        >
                                            {showMcpToken ? "Hide" : "Show"}
                                        </button>
                                        <button
                                            class="stg-card-mini-btn"
                                            onclick={handleCopyMcpToken}
                                            title="Copy">Copy</button
                                        >
                                        <button
                                            class="stg-card-mini-btn"
                                            onclick={handleRotateMcpToken}
                                            disabled={mcpStatus.running}
                                            title="Generate new token"
                                            >Rotate</button
                                        >
                                    </div>
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
                                            d="M3 7h18M3 12h12M3 17h18"
                                        /></svg
                                    >
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">
                                        Board automation
                                    </h3>
                                    <p class="stg-card-sub">
                                        When a card's PR merges on GitHub or
                                        GitLab, automatically move it to your
                                        board's final column. Checked on app
                                        focus, debounced to 5 minutes. The card
                                        thread gets a system comment so the move
                                        is auditable.
                                    </p>
                                </div>
                            </header>
                            <div class="stg-card-body">
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Auto-move cards on PR merge</label
                                    >
                                    <label class="stg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={autoMoveMergedPrs}
                                            onchange={(e) =>
                                                handleSettingChange(
                                                    "workspace_automove_merged_prs",
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
                                    <h3 class="stg-card-title">
                                        Request Settings
                                    </h3>
                                    <p class="stg-card-sub">
                                        Defaults applied to every REST request
                                        you send.
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
                                        >SSL Verification</label
                                    >
                                    <label class="stg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={sslVerification}
                                            onchange={(e) =>
                                                handleSettingChange(
                                                    "ssl_verification",
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
                                    class:active={displayTheme === theme.id}
                                    class:locked={theme.premium &&
                                        $cloudPlan !== "pro"}
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
                                        <span class="theme-name"
                                            >{theme.name}</span
                                        >
                                        <span class="theme-desc"
                                            >{THEME_DESCRIPTIONS[theme.id] ||
                                                theme.description}</span
                                        >
                                    </div>
                                    {#if theme.premium}
                                        <span class="pro-badge">PRO</span>
                                    {/if}
                                </button>
                            {/each}
                        </div>

                        {#if previewThemeId}
                            {@const previewName =
                                getTheme(previewThemeId)?.name ?? "Theme"}
                            <div class="theme-preview-banner" role="alert">
                                <div class="theme-preview-info">
                                    <span class="theme-preview-eyebrow"
                                        >PREVIEWING</span
                                    >
                                    <strong class="theme-preview-name"
                                        >{previewName}</strong
                                    >
                                    <p class="theme-preview-sub">
                                        This is a premium theme. Upgrade to keep
                                        it, or cancel to go back to your current
                                        theme.
                                    </p>
                                </div>
                                <div class="theme-preview-actions">
                                    <button
                                        class="theme-preview-btn"
                                        onclick={cancelThemePreview}
                                    >
                                        Cancel
                                    </button>
                                    <button
                                        class="theme-preview-btn theme-preview-btn-primary"
                                        onclick={upgradeFromThemePreview}
                                    >
                                        Upgrade to Pro
                                    </button>
                                </div>
                            </div>
                        {/if}
                    </div>

                    <div class="stg-section">
                        <span class="stg-section-label">Accent Color</span>
                        {#if accentLocked}
                            <div class="stg-accent-locked-note">
                                <svg
                                    viewBox="0 0 24 24"
                                    width="14"
                                    height="14"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="1.8"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    aria-hidden="true"
                                    ><rect
                                        x="3"
                                        y="11"
                                        width="18"
                                        height="11"
                                        rx="2"
                                    /><path d="M7 11V7a5 5 0 0110 0v4" /></svg
                                >
                                <span
                                    ><strong>{activeThemeDef?.name}</strong> provides
                                    its own accent. Pick a different theme to customize.</span
                                >
                            </div>
                        {/if}
                        <div
                            class="stg-swatches"
                            class:stg-swatches-disabled={accentLocked}
                        >
                            {#each ACCENT_COLORS as color}
                                <button
                                    class="stg-swatch"
                                    class:active={accentColor === color.value}
                                    style="background: {color.value}"
                                    title={accentLocked
                                        ? `${activeThemeDef?.name} controls the accent`
                                        : color.name}
                                    disabled={accentLocked}
                                    onclick={() =>
                                        handleAccentChange(color.value)}
                                ></button>
                            {/each}
                        </div>
                    </div>
                {:else if activeTab === "ai"}
                    <!-- Top-level split: Clauge AI (managed credits + history)
                         vs BYOK (your own keys + per-mode/per-model stats). -->
                    <div class="ai-toptabs">
                        <button
                            class="ai-toptab"
                            class:active={aiTopTab === "clauge"}
                            onclick={() => (aiTopTab = "clauge")}
                        >
                            Clauge AI
                        </button>
                        <button
                            class="ai-toptab"
                            class:active={aiTopTab === "byok"}
                            onclick={() => (aiTopTab = "byok")}
                        >
                            BYOK
                        </button>
                    </div>

                    {#if aiTopTab === "clauge"}
                        {#if $cloudPlan !== "pro"}
                            <!-- Free user: upsell card lives here, full data
                                 panels only appear after upgrade. -->
                            <ClaugeAIBalance
                                plan={$cloudPlan ?? "free"}
                                credits={cloudCreditsLocal}
                                subscription={cloudSubLocal}
                                onUpgradeClick={handleUpgradeClick}
                            />
                        {:else}
                            {@const remaining =
                                cloudCreditsLocal?.remaining ?? 0}
                            {@const allowance =
                                cloudCreditsLocal?.allowance ?? 0}
                            {@const used = Math.max(0, allowance - remaining)}
                            {@const pctRemaining =
                                allowance > 0
                                    ? Math.min(
                                          100,
                                          Math.round(
                                              (remaining / allowance) * 100,
                                          ),
                                      )
                                    : 0}
                            {@const intervalLabel =
                                $cloudSub?.interval === "yearly"
                                    ? "Yearly"
                                    : "Monthly"}
                            {@const periodLabel =
                                $cloudSub?.interval === "yearly"
                                    ? "this year"
                                    : "this month"}
                            <div class="cai2-pane">
                                <!-- Card 1: Credits hero -->
                                <section class="cai2-card">
                                    <header class="cai2-head">
                                        <span
                                            class="cai2-icon"
                                            aria-hidden="true"
                                        >
                                            <svg
                                                viewBox="0 0 24 24"
                                                width="14"
                                                height="14"
                                                fill="currentColor"
                                            >
                                                <path
                                                    d="M13 2L3 14h7l-1 8 10-12h-7l1-8z"
                                                />
                                            </svg>
                                        </span>
                                        <div class="cai2-titles">
                                            <h3 class="cai2-title">Credits</h3>
                                            <p class="cai2-sub">
                                                {intervalLabel} allocation{#if cloudCreditsLocal?.resets_at}
                                                    <span class="cai2-dot"
                                                        >·</span
                                                    >
                                                    resets {new Date(
                                                        cloudCreditsLocal.resets_at,
                                                    ).toLocaleDateString(
                                                        undefined,
                                                        {
                                                            month: "short",
                                                            day: "numeric",
                                                            year: "numeric",
                                                        },
                                                    )}
                                                {/if}
                                            </p>
                                        </div>
                                        {#if cloudSubLocal?.status}
                                            <span class="cai2-status">
                                                {cloudSubLocal.cancel_at_period_end
                                                    ? "Cancelling"
                                                    : "Active"}
                                            </span>
                                        {/if}
                                    </header>

                                    <div class="cai2-hero">
                                        <span class="cai2-hero-val"
                                            >{remaining.toLocaleString()}</span
                                        >
                                        <span class="cai2-hero-tot"
                                            >/ {allowance.toLocaleString()} remaining</span
                                        >
                                    </div>
                                    <div class="cai2-bar-wrap">
                                        <div
                                            class="cai2-bar"
                                            style="width: {pctRemaining}%"
                                        ></div>
                                    </div>
                                    <p class="cai2-meta">
                                        {used.toLocaleString()} credits used {periodLabel}
                                    </p>
                                </section>

                                <!-- Card 2: Credits by mode -->
                                {#if cloudAiByMode.length > 0}
                                    <section class="cai2-card">
                                        <header class="cai2-head">
                                            <span
                                                class="cai2-icon"
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
                                                >
                                                    <line
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
                                                    />
                                                </svg>
                                            </span>
                                            <div class="cai2-titles">
                                                <h3 class="cai2-title">
                                                    Credits by mode
                                                </h3>
                                                <p class="cai2-sub">
                                                    Breakdown of usage across
                                                    modes {periodLabel}.
                                                </p>
                                            </div>
                                        </header>
                                        <ul class="cai2-mode-list">
                                            {#each cloudAiByMode as m (m.mode)}
                                                <li class="cai2-mode-row">
                                                    <span
                                                        class="cai2-mode-name"
                                                    >
                                                        <span
                                                            class="cai2-mode-dot"
                                                            style="background: {modeColor(
                                                                m.mode,
                                                            )}"
                                                        ></span>
                                                        {capitalizeMode(m.mode)}
                                                    </span>
                                                    <div
                                                        class="cai2-mode-bar-wrap"
                                                    >
                                                        <div
                                                            class="cai2-mode-bar"
                                                            style="width: {m.percent}%; background: {modeColor(
                                                                m.mode,
                                                            )}"
                                                        ></div>
                                                    </div>
                                                    <span class="cai2-mode-val"
                                                        >{m.credits.toLocaleString()}
                                                        cr</span
                                                    >
                                                    <span class="cai2-mode-pct"
                                                        >{m.percent}%</span
                                                    >
                                                </li>
                                            {/each}
                                        </ul>
                                    </section>
                                {/if}

                                <!-- Card 3: Recent activity table -->
                                <section class="cai2-card cai2-card-activity">
                                    <header class="cai2-head">
                                        <span
                                            class="cai2-icon"
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
                                            >
                                                <circle
                                                    cx="12"
                                                    cy="12"
                                                    r="10"
                                                /><polyline
                                                    points="12 6 12 12 16 14"
                                                />
                                            </svg>
                                        </span>
                                        <div class="cai2-titles">
                                            <h3 class="cai2-title">
                                                Recent activity
                                            </h3>
                                        </div>
                                        <span class="cai2-sub-right">
                                            {cloudAiUsage.length}
                                            {cloudAiUsage.length === 1
                                                ? "request"
                                                : "requests"}
                                            {#if cloudAiUsageNextBefore}loaded ·
                                                scroll for more{/if}
                                        </span>
                                    </header>
                                    {#if cloudAiUsageLoading && cloudAiUsage.length === 0}
                                        <p class="cai2-empty">Loading…</p>
                                    {:else if cloudAiUsage.length === 0}
                                        <p class="cai2-empty">
                                            No Clauge AI requests yet. Start
                                            chatting from any mode with Clauge
                                            AI selected in the panel.
                                        </p>
                                    {:else}
                                        <div
                                            class="cai2-table"
                                            onscroll={handleActivityScroll}
                                        >
                                            <div class="cai2-tr cai2-th">
                                                <span>TIME</span>
                                                <span>TYPE</span>
                                                <span>MODE</span>
                                                <span class="cai2-th-right"
                                                    >CREDITS</span
                                                >
                                            </div>
                                            {#each cloudAiUsage as e (e.request_id)}
                                                <div class="cai2-tr">
                                                    <span class="cai2-td-time"
                                                        >{fmtUsageTime(
                                                            e.occurred_at,
                                                        )}</span
                                                    >
                                                    <span class="cai2-td-op"
                                                        >{e.operation}</span
                                                    >
                                                    <span
                                                        class="cai2-td-mode"
                                                        style="color: {modeColor(
                                                            e.mode,
                                                        )}"
                                                    >
                                                        {e.mode
                                                            ? capitalizeMode(
                                                                  e.mode,
                                                              )
                                                            : "—"}
                                                    </span>
                                                    <span class="cai2-td-cost"
                                                        >−{e.clauge_credits.toLocaleString()}
                                                        cr</span
                                                    >
                                                </div>
                                            {/each}
                                            {#if cloudAiUsageLoadingMore}
                                                <div class="cai2-loading-more">
                                                    Loading more…
                                                </div>
                                            {:else if !cloudAiUsageNextBefore && cloudAiUsage.length >= CLOUD_AI_USAGE_PAGE}
                                                <div
                                                    class="cai2-loading-more cai2-end"
                                                >
                                                    End of history
                                                </div>
                                            {/if}
                                        </div>
                                    {/if}
                                </section>
                            </div>
                        {/if}
                    {:else if aiTopTab === "byok"}
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
                                        /><path
                                            d="M7 11V7a5 5 0 0110 0v4"
                                        /></svg
                                    >
                                {/if}
                            </button>
                        </div>

                        {#if aiSubTab === "config"}
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
                                                Provider, model, and API key for
                                                the in-app AI assistant.
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
                                                            e.currentTarget
                                                                .value,
                                                        )}
                                                >
                                                    {#each AI_PROVIDER_REGISTRY as p (p.providerId)}
                                                        <option
                                                            value={p.providerId}
                                                            >{p.providerLabel}</option
                                                        >
                                                    {/each}
                                                </select>
                                            </div>
                                            <div class="ai-cfg-field">
                                                <label class="ai-cfg-label"
                                                    >Model</label
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
                                            <label class="ai-cfg-label"
                                                >API Key</label
                                            >
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
                                                        (showAiKey =
                                                            !showAiKey)}
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
                                                <span
                                                    class="ai-test-result success"
                                                >
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
                                                <span
                                                    class="ai-test-result error"
                                                >
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
                                                    onclick={() =>
                                                        handleSaveAiKey()}
                                                    disabled={!aiApiKey.trim() ||
                                                        aiTestStatus ===
                                                            "testing"}
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
                                                        ><path
                                                            d="M15 3h6v6"
                                                        /><path
                                                            d="M10 14L21 3"
                                                        /><path
                                                            d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"
                                                        /></svg
                                                    >
                                                    Get API Key
                                                </a>
                                                <span class="ai-link-sep"
                                                    >&middot;</span
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
                                                        ><path
                                                            d="M15 3h6v6"
                                                        /><path
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
                                                    <span class="ai-status-dot"
                                                    ></span>
                                                    Connected
                                                </span>
                                            {/if}
                                        </div>
                                    </div>
                                </section>

                                <!-- Configured providers list — every provider
                                 with a saved key, count + delete + click-to-edit.
                                 Sits directly under the Configuration card so
                                 the user can see what they have without opening
                                 the provider dropdown. -->
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
                                            >
                                                <path d="M9 11l3 3L22 4" /><path
                                                    d="M21 12v7a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2h11"
                                                />
                                            </svg>
                                        </span>
                                        <div class="stg-card-titles">
                                            <h3 class="stg-card-title">
                                                Configured providers
                                                <span class="cfg-prov-count"
                                                    >{configuredProviders.length}</span
                                                >
                                            </h3>
                                            <p class="stg-card-sub">
                                                Keys you've saved. Click a row
                                                to edit, or remove with delete.
                                            </p>
                                        </div>
                                    </header>

                                    {#if configuredProviders.length === 0}
                                        <p class="cfg-prov-empty">
                                            No providers configured yet. Save a
                                            key above to add one.
                                        </p>
                                    {:else}
                                        <ul class="cfg-prov-list">
                                            {#each configuredProviders as p (p.providerId)}
                                                {@const key =
                                                    $settings[
                                                        p.keySettingName
                                                    ] ?? ""}
                                                {@const tail =
                                                    key.length > 4
                                                        ? key.slice(-4)
                                                        : key}
                                                <!-- svelte-ignore a11y_click_events_have_key_events -->
                                                <!-- svelte-ignore a11y_no_static_element_interactions -->
                                                <li
                                                    class="cfg-prov-row"
                                                    class:is-active={p.providerId ===
                                                        aiProvider}
                                                    onclick={() =>
                                                        handleProviderChange(
                                                            p.providerId,
                                                        )}
                                                >
                                                    <div class="cfg-prov-info">
                                                        <span
                                                            class="cfg-prov-name"
                                                        >
                                                            {p.providerLabel}
                                                            {#if p.providerId === aiProvider}
                                                                <span
                                                                    class="cfg-prov-active-tag"
                                                                    >Active</span
                                                                >
                                                            {/if}
                                                        </span>
                                                        <span
                                                            class="cfg-prov-key"
                                                            title="Key ends in {tail}"
                                                        >
                                                            ••••••••{tail}
                                                        </span>
                                                    </div>
                                                    <button
                                                        class="cfg-prov-del"
                                                        title="Remove saved key for {p.providerLabel}"
                                                        onclick={(e) => {
                                                            e.stopPropagation();
                                                            handleSettingChange(
                                                                p.keySettingName,
                                                                "",
                                                            );
                                                            if (
                                                                p.providerId ===
                                                                aiProvider
                                                            ) {
                                                                aiApiKey = "";
                                                                aiTestStatus =
                                                                    "idle";
                                                            }
                                                        }}
                                                    >
                                                        <svg
                                                            viewBox="0 0 24 24"
                                                            width="13"
                                                            height="13"
                                                            fill="none"
                                                            stroke="currentColor"
                                                            stroke-width="2"
                                                            stroke-linecap="round"
                                                            stroke-linejoin="round"
                                                            ><polyline
                                                                points="3 6 5 6 21 6"
                                                            /><path
                                                                d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"
                                                            /></svg
                                                        >
                                                    </button>
                                                </li>
                                            {/each}
                                        </ul>
                                    {/if}
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
                                        ><path
                                            d="M18 20V10M12 20V4M6 20v-6"
                                        /></svg
                                    >
                                    <p>No API calls recorded yet</p>
                                    <span
                                        >Start chatting with AI to see usage
                                        data here</span
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
                                                <h3 class="stg-card-title">
                                                    Overview
                                                </h3>
                                                <p class="stg-card-sub">
                                                    Aggregated calls, tokens,
                                                    and estimated cost across
                                                    all AI activity.
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
                                                            (s, v) =>
                                                                s +
                                                                v.totalCalls,
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
                                                                    s +
                                                                    v.inputTokens,
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
                                                                    s +
                                                                    v.outputTokens,
                                                                0,
                                                            ),
                                                        )}</span
                                                    >
                                                    <span class="ai-summary-lbl"
                                                        >Output Tokens</span
                                                    >
                                                </div>
                                                <div
                                                    class="ai-summary-card accent"
                                                >
                                                    <span class="ai-summary-val"
                                                        >{estimateCost(
                                                            aiUsageStats.reduce(
                                                                (s, v) =>
                                                                    s +
                                                                    v.inputTokens,
                                                                0,
                                                            ),
                                                            aiUsageStats.reduce(
                                                                (s, v) =>
                                                                    s +
                                                                    v.outputTokens,
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
                                                <h3 class="stg-card-title">
                                                    By Mode
                                                </h3>
                                                <p class="stg-card-sub">
                                                    Per-mode breakdown of AI
                                                    calls and token spend.
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
                                                        <span
                                                            class="ai-table-label"
                                                        >
                                                            <span
                                                                class="ai-dot"
                                                                data-mode={stat.mode}
                                                            ></span>
                                                            {stat.mode.toUpperCase()}
                                                        </span>
                                                        <span
                                                            class="ai-table-val"
                                                            >{stat.totalCalls}</span
                                                        >
                                                        <span
                                                            class="ai-table-val"
                                                            >{formatTokens(
                                                                stat.inputTokens,
                                                            )}</span
                                                        >
                                                        <span
                                                            class="ai-table-val"
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
                                                        Spend grouped by model —
                                                        useful when switching
                                                        providers.
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
                                                        <div
                                                            class="ai-table-row"
                                                        >
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
                                                            <span
                                                                class="ai-table-val"
                                                                >{pstat.totalCalls}</span
                                                            >
                                                            <span
                                                                class="ai-table-val"
                                                                >{formatTokens(
                                                                    pstat.inputTokens,
                                                                )}</span
                                                            >
                                                            <span
                                                                class="ai-table-val"
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
                                                    Haiku 4.5: $1.00 / MTok in
                                                    &middot; $5.00 / MTok out
                                                </p>
                                            </div>
                                        </section>
                                    {/if}
                                </div>
                            {/if}
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
                                onclick={() => (agentSubTab = "usage")}
                            >
                                Usage Stats
                            </button>
                        </div>

                        <div class="agent-settings-scroll">
                            {#if agentSubTab === "general"}
                                <div class="stg-card-stack">
                                    <section class="stg-card agent-usage-card">
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
                                                    ><path
                                                        d="M18 20V10M12 20V4M6 20v-6"
                                                    /></svg
                                                >
                                            </span>
                                            <div class="stg-card-titles">
                                                <h3 class="stg-card-title">
                                                    Live usage tracking
                                                </h3>
                                                <p class="stg-card-sub">
                                                    Configure the provider
                                                    limits shown in the Agent
                                                    footer.
                                                </p>
                                            </div>
                                            {#if agentFooterProvider === "claude" && agentSessionKey && $agentUsageAuthStatus.state === "valid"}
                                                <span class="ai-status-badge">
                                                    <span class="ai-status-dot"
                                                    ></span>
                                                    Claude connected
                                                </span>
                                            {:else if agentFooterProvider === "claude" && agentSessionKey && $agentUsageAuthStatus.state === "checking"}
                                                <span
                                                    class="ai-status-badge checking"
                                                >
                                                    <span class="ai-status-dot"
                                                    ></span>
                                                    Checking Claude
                                                </span>
                                            {:else if agentFooterProvider === "claude" && agentSessionKey && $agentUsageAuthStatus.state === "invalid"}
                                                <span
                                                    class="ai-status-badge error"
                                                    title={$agentUsageAuthStatus.message}
                                                >
                                                    <span class="ai-status-dot"
                                                    ></span>
                                                    Claude needs reconfigure
                                                </span>
                                            {:else if agentFooterProvider === "codex" && agentCodexToken}
                                                <span class="ai-status-badge">
                                                    <span class="ai-status-dot"
                                                    ></span>
                                                    Codex configured
                                                </span>
                                            {:else}
                                                <span
                                                    class="ai-status-badge muted"
                                                >
                                                    Footer not configured
                                                </span>
                                            {/if}
                                        </header>
                                        <div class="stg-card-body">
                                            <div class="stg-card-row">
                                                <div
                                                    class="stg-card-row-action-text"
                                                >
                                                    <span
                                                        class="stg-card-row-label"
                                                        >Show in Agent footer</span
                                                    >
                                                    <span
                                                        class="stg-card-row-help"
                                                    >
                                                        Choose the provider
                                                        whose live limit appears
                                                        next to the Agent
                                                        terminal.
                                                    </span>
                                                </div>
                                                <select
                                                    class="stg-select"
                                                    value={agentFooterProvider}
                                                    onchange={(e) =>
                                                        handleSettingChange(
                                                            "agent_footer_usage_provider",
                                                            e.currentTarget
                                                                .value,
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
                                                    <option value="gemini"
                                                        >Gemini (local data
                                                        only)</option
                                                    >
                                                    <option value="opencode"
                                                        >OpenCode (local data
                                                        only)</option
                                                    >
                                                </select>
                                            </div>

                                            <div
                                                class="stg-card-row stg-card-row-action"
                                            >
                                                <div
                                                    class="stg-card-row-action-text"
                                                >
                                                    <span
                                                        class="stg-card-row-label"
                                                        >Claude session key</span
                                                    >
                                                    <span
                                                        class="stg-card-row-help"
                                                    >
                                                        Used only to read Claude
                                                        usage limits. Find it in
                                                        claude.ai DevTools >
                                                        Application > Cookies >
                                                        sessionKey.
                                                    </span>
                                                    {#if agentKeyTestStatus === "success"}
                                                        <span
                                                            class="ai-test-result success"
                                                        >
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
                                                        <span
                                                            class="ai-test-result error"
                                                        >
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
                                                        <span
                                                            class="ai-test-result error"
                                                        >
                                                            {$agentUsageAuthStatus.message}
                                                        </span>
                                                    {/if}
                                                </div>
                                                <div
                                                    class="agent-session-key-control"
                                                >
                                                    <div
                                                        class="ai-key-input-wrap"
                                                    >
                                                        <input
                                                            class="ai-cfg-input"
                                                            type={showAgentKey
                                                                ? "text"
                                                                : "password"}
                                                            placeholder="sk-ant-sid01-..."
                                                            bind:value={
                                                                agentKeyInput
                                                            }
                                                            onblur={commitAgentKeyIfChanged}
                                                            onkeydown={(e) => {
                                                                if (
                                                                    e.key ===
                                                                    "Enter"
                                                                ) {
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
                                                    <div
                                                        class="agent-session-key-actions"
                                                    >
                                                        {#if agentKeyTestStatus === "testing"}
                                                            <span
                                                                class="ai-test-result"
                                                                >Verifying…</span
                                                            >
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

                                            <div
                                                class="stg-card-row stg-card-row-action"
                                            >
                                                <div
                                                    class="stg-card-row-action-text"
                                                >
                                                    <span
                                                        class="stg-card-row-label"
                                                        >Codex access token</span
                                                    >
                                                    <span
                                                        class="stg-card-row-help"
                                                    >
                                                        Grab from <strong
                                                            >chatgpt.com/codex/cloud/settings/analytics</strong
                                                        >
                                                        → DevTools → Network →
                                                        <code>wham/usage</code>
                                                        →
                                                        <code
                                                            >authorization</code
                                                        >
                                                        header (after
                                                        <code>Bearer </code>).
                                                    </span>
                                                    {#if codexTokenTestStatus === "success"}
                                                        <span
                                                            class="ai-test-result success"
                                                        >
                                                            <svg
                                                                viewBox="0 0 24 24"
                                                                width="11"
                                                                height="11"
                                                                fill="none"
                                                                stroke="currentColor"
                                                                stroke-width="2.5"
                                                                stroke-linecap="round"
                                                                ><polyline
                                                                    points="20 6 9 17 4 12"
                                                                /></svg
                                                            >
                                                            {codexTokenTestMessage}
                                                        </span>
                                                    {:else if codexTokenTestStatus === "error"}
                                                        <span
                                                            class="ai-test-result error"
                                                        >
                                                            <svg
                                                                viewBox="0 0 24 24"
                                                                width="11"
                                                                height="11"
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
                                                            {codexTokenTestMessage}
                                                        </span>
                                                    {/if}
                                                </div>
                                                <div
                                                    class="agent-session-key-control"
                                                >
                                                    <div
                                                        class="ai-key-input-wrap"
                                                    >
                                                        <input
                                                            class="ai-cfg-input"
                                                            type={showCodexToken
                                                                ? "text"
                                                                : "password"}
                                                            placeholder="Bearer token..."
                                                            bind:value={
                                                                codexTokenInput
                                                            }
                                                            onblur={commitCodexTokenIfChanged}
                                                            onkeydown={(e) => {
                                                                if (
                                                                    e.key ===
                                                                    "Enter"
                                                                ) {
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
                                                    <div
                                                        class="agent-session-key-actions"
                                                    >
                                                        {#if codexTokenTestStatus === "testing"}
                                                            <span
                                                                class="ai-test-result"
                                                                >Verifying…</span
                                                            >
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
                                                <div
                                                    class="stg-card-row-action-text"
                                                >
                                                    <span
                                                        class="stg-card-row-label"
                                                        >Refresh interval</span
                                                    >
                                                    <span
                                                        class="stg-card-row-help"
                                                    >
                                                        How often Clauge
                                                        refreshes live limits
                                                        while Agent mode is
                                                        open.
                                                    </span>
                                                </div>
                                                <select
                                                    class="stg-select"
                                                    value={agentRefreshMins}
                                                    onchange={(e) =>
                                                        handleSettingChange(
                                                            "agent_refresh_mins",
                                                            e.currentTarget
                                                                .value,
                                                        )}
                                                >
                                                    {#each REFRESH_OPTIONS as opt}
                                                        <option
                                                            value={opt.value}
                                                            >{opt.label}</option
                                                        >
                                                    {/each}
                                                </select>
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
                                                    Sound and dock bounce when
                                                    an agent session finishes
                                                    its work.
                                                </p>
                                            </div>
                                        </header>
                                        <div class="stg-card-body">
                                            <div class="stg-card-row">
                                                <label
                                                    class="stg-card-row-label"
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
                                                                    e
                                                                        .currentTarget
                                                                        .checked,
                                                                ),
                                                            )}
                                                    />
                                                    <span
                                                        class="stg-toggle-slider"
                                                    ></span>
                                                </label>
                                            </div>
                                            <div class="stg-card-row">
                                                <label
                                                    class="stg-card-row-label"
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
                                                                    e
                                                                        .currentTarget
                                                                        .checked,
                                                                ),
                                                            )}
                                                    />
                                                    <span
                                                        class="stg-toggle-slider"
                                                    ></span>
                                                </label>
                                            </div>
                                        </div>
                                    </section>
                                </div>
                            {:else if agentSubTab === "plugins"}
                                <!-- Plugins pane lays itself out as a flex column that
                         fills the right pane. The provider tab strip and
                         the Installed/Marketplace toggle stay pinned at
                         the top; only the list itself scrolls. Without
                         this wrapper the whole settings pane scrolls when
                         the list grows, which hides the toggle as you
                         scan plugins. -->
                                <div class="agent-plugins-pane">
                                    <!-- CLI provider tab strip — each provider has its own
                         marketplace + installed list. OpenCode isn't here
                         because its plugins are npm packages with no
                         marketplace concept (handled at the runner level). -->
                                    <div class="plugin-provider-tabs">
                                        {#each PLUGIN_PROVIDER_TABS as t}
                                            <button
                                                class="plugin-provider-tab"
                                                class:active={pluginProvider ===
                                                    t.id}
                                                onclick={() =>
                                                    selectPluginProvider(t.id)}
                                                >{t.label}</button
                                            >
                                        {/each}
                                    </div>

                                    <!-- Installed / Marketplace toggle. Codex doesn't
                         have a Clauge-side marketplace browser (installs
                         happen inside the codex TUI), so we hide that
                         half of the toggle and show only Installed. -->
                                    {#if pluginProvider !== "codex"}
                                        <div class="agent-plugin-views">
                                            <button
                                                class="ai-action-btn"
                                                class:primary={pluginView ===
                                                    "installed"}
                                                onclick={() =>
                                                    (pluginView = "installed")}
                                                >Installed</button
                                            >
                                            <button
                                                class="ai-action-btn"
                                                class:primary={pluginView ===
                                                    "marketplace"}
                                                onclick={() =>
                                                    (pluginView =
                                                        "marketplace")}
                                                >Marketplace</button
                                            >
                                        </div>
                                    {/if}

                                    {#if pluginProvider === "codex"}
                                        <div
                                            class="plugin-codex-hint"
                                            role="status"
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
                                            >
                                                <circle
                                                    cx="12"
                                                    cy="12"
                                                    r="10"
                                                /><line
                                                    x1="12"
                                                    y1="16"
                                                    x2="12"
                                                    y2="12"
                                                /><line
                                                    x1="12"
                                                    y1="8"
                                                    x2="12.01"
                                                    y2="8"
                                                />
                                            </svg>
                                            <span>
                                                Codex installs plugins from
                                                inside its own UI — run <code
                                                    >codex</code
                                                >
                                                in a terminal and use the
                                                <code>/plugins</code> slash command.
                                                Installed plugins show up here and
                                                can be enabled or disabled.
                                            </span>
                                        </div>
                                    {/if}

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
                                                    >Browse the marketplace to
                                                    install plugins</span
                                                >
                                            </div>
                                        {:else}
                                            <div class="agent-plugin-list">
                                                {#each installedPlugins as plugin}
                                                    <div
                                                        class="agent-plugin-card"
                                                    >
                                                        <div
                                                            class="agent-plugin-info"
                                                        >
                                                            <span
                                                                class="agent-plugin-name"
                                                                >{plugin.name}</span
                                                            >
                                                            <span
                                                                class="agent-plugin-meta"
                                                            >
                                                                {plugin.marketplace}
                                                                {#if plugin.version}
                                                                    <span
                                                                        class="ai-link-sep"
                                                                        >&middot;</span
                                                                    >
                                                                    v{plugin.version}
                                                                {/if}
                                                            </span>
                                                        </div>
                                                        <div
                                                            class="agent-plugin-actions"
                                                        >
                                                            <label
                                                                class="stg-toggle"
                                                            >
                                                                <input
                                                                    type="checkbox"
                                                                    checked={plugin.enabled}
                                                                    onchange={() =>
                                                                        handleTogglePlugin(
                                                                            plugin.name,
                                                                            !plugin.enabled,
                                                                        )}
                                                                />
                                                                <span
                                                                    class="stg-toggle-slider"
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
                                                    <div
                                                        class="agent-plugin-card"
                                                    >
                                                        <div
                                                            class="agent-plugin-info"
                                                        >
                                                            <span
                                                                class="agent-plugin-name"
                                                                >{plugin.name}</span
                                                            >
                                                            <span
                                                                class="agent-plugin-desc"
                                                                >{plugin.description}</span
                                                            >
                                                            {#if plugin.installs != null}
                                                                <span
                                                                    class="agent-plugin-meta"
                                                                    >{plugin.installs.toLocaleString()}
                                                                    installs</span
                                                                >
                                                            {/if}
                                                        </div>
                                                        <div
                                                            class="agent-plugin-actions"
                                                        >
                                                            {#if plugin.installed}
                                                                <span
                                                                    class="ai-status-badge"
                                                                >
                                                                    <span
                                                                        class="ai-status-dot"
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
                                </div>
                                <!-- /.agent-plugins-pane -->
                            {:else if agentSubTab === "contexts"}
                                {#if isNewContext || editingContext}
                                    <!-- Context editor -->
                                    <div class="agent-ctx-editor">
                                        <div
                                            class="stg-field"
                                            style="flex-direction: column; align-items: stretch;"
                                        >
                                            <label class="stg-label">Name</label
                                            >
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
                                            <label class="stg-label"
                                                >Content</label
                                            >
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
                                                onclick={cancelEditContext}
                                                >Cancel</button
                                            >
                                        </div>
                                    </div>
                                {:else}
                                    <div class="agent-ctx-header">
                                        <span
                                            class="stg-section-label"
                                            style="margin-bottom: 0;"
                                            >Contexts</span
                                        >
                                        <button
                                            class="ai-action-btn primary sm"
                                            onclick={startNewContext}
                                            >New Context</button
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
                                                /><polyline
                                                    points="14 2 14 8 20 8"
                                                /><line
                                                    x1="16"
                                                    y1="13"
                                                    x2="8"
                                                    y2="13"
                                                /><line
                                                    x1="16"
                                                    y1="17"
                                                    x2="8"
                                                    y2="17"
                                                /><polyline
                                                    points="10 9 9 9 8 9"
                                                /></svg
                                            >
                                            <p>No contexts yet</p>
                                            <span
                                                >Create a context to attach to
                                                agent sessions</span
                                            >
                                        </div>
                                    {:else}
                                        <div class="agent-plugin-list">
                                            {#each agentContexts as ctx}
                                                <div
                                                    class="agent-plugin-card agent-ctx-card"
                                                    onclick={() =>
                                                        startEditContext(ctx)}
                                                    role="button"
                                                    tabindex="0"
                                                    onkeydown={(e) => {
                                                        if (e.key === "Enter")
                                                            startEditContext(
                                                                ctx,
                                                            );
                                                    }}
                                                >
                                                    <div
                                                        class="agent-plugin-info"
                                                    >
                                                        <span
                                                            class="agent-plugin-name"
                                                            >{ctx.name}</span
                                                        >
                                                        <span
                                                            class="agent-plugin-desc"
                                                            >{ctx.content
                                                                .split("\n")[0]
                                                                .slice(
                                                                    0,
                                                                    80,
                                                                )}{ctx.content
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
                                                            <span
                                                                class="ai-reset-confirm"
                                                            >
                                                                <span
                                                                    >Delete?</span
                                                                >
                                                                <button
                                                                    class="ai-action-btn danger sm"
                                                                    onclick={(
                                                                        e,
                                                                    ) => {
                                                                        e.stopPropagation();
                                                                        handleDeleteAgentContext(
                                                                            ctx.id,
                                                                        );
                                                                    }}
                                                                    >Yes</button
                                                                >
                                                                <button
                                                                    class="ai-action-btn sm"
                                                                    onclick={(
                                                                        e,
                                                                    ) => {
                                                                        e.stopPropagation();
                                                                        deleteConfirmId =
                                                                            null;
                                                                    }}
                                                                    >No</button
                                                                >
                                                            </span>
                                                        {:else}
                                                            <button
                                                                class="ai-action-btn danger sm"
                                                                onclick={(
                                                                    e,
                                                                ) => {
                                                                    e.stopPropagation();
                                                                    deleteConfirmId =
                                                                        ctx.id;
                                                                }}
                                                                >Delete</button
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
                                            onclick={() =>
                                                selectAgentUsageDays(d)}
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
                                            ><polyline
                                                points="23 4 23 10 17 10"
                                            /><path
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
                                            ><span class="ud-lbl"
                                                >Total Cost</span
                                            >
                                        </div>
                                        <div class="ud-card">
                                            <span class="ud-val"
                                                >{agentUsageData.totalApiCalls.toLocaleString()}</span
                                            ><span class="ud-lbl"
                                                >API Calls</span
                                            >
                                        </div>
                                        <div class="ud-card">
                                            <span class="ud-val"
                                                >{agentUsageData.totalSessions}</span
                                            ><span class="ud-lbl">Sessions</span
                                            >
                                        </div>
                                        <div class="ud-card">
                                            <span class="ud-val"
                                                >{agentUsageData.cacheHitPercent.toFixed(
                                                    1,
                                                )}%</span
                                            ><span class="ud-lbl"
                                                >Cache Hit</span
                                            >
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
                                                                (day.cost /
                                                                    maxCost) *
                                                                    100,
                                                            )}%"
                                                        ></div>
                                                        <span class="ud-bar-lbl"
                                                            >{day.date.slice(
                                                                8,
                                                            )}</span
                                                        >
                                                    </div>
                                                {/each}
                                            </div>
                                        </div>
                                    {/if}
                                    <!-- Live Usage + Models -->
                                    <div class="ud-grid">
                                        <div class="ud-section">
                                            <div class="ud-section-title">
                                                Live Usage
                                            </div>
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
                                                    $agentUsageLimits
                                                        .seven_day_sonnet
                                                        ?.utilization ?? null}
                                                <div class="ud-live-rows">
                                                    {#if sessionPct != null}
                                                        <div
                                                            class="ud-live-row"
                                                        >
                                                            <span
                                                                class="ud-live-lbl"
                                                                >Session</span
                                                            >
                                                            <div
                                                                class="ud-live-bar"
                                                            >
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
                                                        <div
                                                            class="ud-live-row"
                                                        >
                                                            <span
                                                                class="ud-live-lbl"
                                                                >Weekly</span
                                                            >
                                                            <div
                                                                class="ud-live-bar"
                                                            >
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
                                                        <div
                                                            class="ud-live-row"
                                                        >
                                                            <span
                                                                class="ud-live-lbl"
                                                                >Sonnet</span
                                                            >
                                                            <div
                                                                class="ud-live-bar"
                                                            >
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
                                            <div class="ud-section-title">
                                                Models
                                            </div>
                                            <div class="ud-scroll">
                                                {#each agentUsageData.byModel as m}
                                                    <div class="ud-row">
                                                        <div
                                                            class="ud-row-info"
                                                        >
                                                            <span
                                                                class="ud-row-name"
                                                                >{m.model}</span
                                                            >
                                                            <span
                                                                class="ud-row-meta"
                                                                >{m.calls} calls &middot;
                                                                {m.cacheHitPercent.toFixed(
                                                                    0,
                                                                )}% cache</span
                                                            >
                                                        </div>
                                                        <span
                                                            class="ud-row-cost"
                                                            >{agentFormatCost(
                                                                m.cost,
                                                            )}</span
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
                                                Projects ({agentUsageData
                                                    .byProject.length})
                                            </div>
                                            <div class="ud-scroll">
                                                {#each agentUsageData.byProject as p}
                                                    <div class="ud-row">
                                                        <div
                                                            class="ud-row-info"
                                                        >
                                                            <span
                                                                class="ud-row-name"
                                                                title={p.project}
                                                                >{agentDecodeName(
                                                                    p.project,
                                                                )}</span
                                                            >
                                                            <span
                                                                class="ud-row-meta"
                                                                >{p.sessions} sess
                                                                &middot; {p.calls}
                                                                calls</span
                                                            >
                                                        </div>
                                                        <span
                                                            class="ud-row-cost"
                                                            >{agentFormatCost(
                                                                p.cost,
                                                            )}</span
                                                        >
                                                    </div>
                                                {/each}
                                            </div>
                                        </div>
                                        <div class="ud-section">
                                            <div class="ud-section-title">
                                                Top Sessions
                                            </div>
                                            <div class="ud-scroll">
                                                {#each agentUsageData.topSessions.slice(0, 6) as s}
                                                    <div class="ud-row">
                                                        <div
                                                            class="ud-row-info"
                                                        >
                                                            <span
                                                                class="ud-row-name"
                                                                title={s.project}
                                                                >{agentDecodeName(
                                                                    s.project,
                                                                )}</span
                                                            >
                                                            <span
                                                                class="ud-row-meta"
                                                                >{s.model} &middot;
                                                                {s.sessionId.slice(
                                                                    0,
                                                                    8,
                                                                )}</span
                                                            >
                                                        </div>
                                                        <span
                                                            class="ud-row-cost"
                                                            >{agentFormatCost(
                                                                s.cost,
                                                            )}</span
                                                        >
                                                    </div>
                                                {/each}
                                            </div>
                                        </div>
                                    </div>

                                    <!-- Tools + Shell (2-col) -->
                                    <div class="ud-grid">
                                        <div class="ud-section">
                                            <div class="ud-section-title">
                                                Tools
                                            </div>
                                            <div class="ud-scroll">
                                                {#each agentUsageData.tools.slice(0, 6) as t}
                                                    <div class="ud-tool-row">
                                                        <span
                                                            class="ud-tool-name"
                                                            >{t.name}</span
                                                        >
                                                        <div
                                                            class="ud-tool-bar"
                                                        >
                                                            <div
                                                                class="ud-tool-fill"
                                                                style="width:{Math.max(
                                                                    3,
                                                                    (t.count /
                                                                        (agentUsageData
                                                                            .tools[0]
                                                                            ?.count ||
                                                                            1)) *
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
                                            <div class="ud-section-title">
                                                Shell
                                            </div>
                                            <div class="ud-scroll">
                                                {#each agentUsageData.shellCommands.slice(0, 6) as cmd}
                                                    <div class="ud-tool-row">
                                                        <span
                                                            class="ud-tool-name"
                                                            style="font-family:var(--mono)"
                                                            >{cmd.name}</span
                                                        >
                                                        <div
                                                            class="ud-tool-bar"
                                                        >
                                                            <div
                                                                class="ud-tool-fill"
                                                                style="width:{Math.max(
                                                                    3,
                                                                    (cmd.count /
                                                                        (agentUsageData
                                                                            .shellCommands[0]
                                                                            ?.count ||
                                                                            1)) *
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
                                            ><path
                                                d="M18 20V10M12 20V4M6 20v-6"
                                            /></svg
                                        >
                                        <p
                                            style="margin: 0; font-size: 13px; color: var(--t2); font-weight: 500;"
                                        >
                                            No usage data found
                                        </p>
                                        <span
                                            style="font-size: 11px; color: var(--t3);"
                                            >Start using Claude Code sessions to
                                            see analytics here</span
                                        >
                                    </div>
                                {/if}
                            {/if}
                        </div>
                    </div>
                {:else if activeTab === "shortcuts"}
                    <div class="stg-section">
                        <span class="stg-section-label">Keyboard Shortcuts</span
                        >
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
                {:else if activeTab === "sql"}
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
                                        ><ellipse
                                            cx="12"
                                            cy="5"
                                            rx="8"
                                            ry="2.5"
                                        /><path
                                            d="M4 5v14c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5V5"
                                        /><path
                                            d="M4 12c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5"
                                        /></svg
                                    >
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">
                                        Connection & Query
                                    </h3>
                                    <p class="stg-card-sub">
                                        Defaults for SQL connections. Timeout
                                        changes apply to new connections —
                                        already-open pools keep their existing
                                        values until reconnect.
                                    </p>
                                </div>
                            </header>
                            <div class="stg-card-body">
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Pool Acquire Timeout (ms)</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="number"
                                        value={sqlAcquireTimeoutMs}
                                        min="1000"
                                        step="1000"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "sql_acquire_timeout_ms",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Pool Idle Timeout (minutes)</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="number"
                                        value={sqlIdleTimeoutMin}
                                        min="1"
                                        step="1"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "sql_idle_timeout_min",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >HTTP Query Timeout (s)</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="number"
                                        value={sqlHttpQueryTimeoutSec}
                                        min="5"
                                        step="5"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "sql_http_query_timeout_sec",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Schema Browser Limit</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="number"
                                        value={sqlTableListLimit}
                                        min="10"
                                        max="10000"
                                        step="10"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "sql_table_list_limit",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                            </div>
                        </section>
                    </div>
                {:else if activeTab === "nosql"}
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
                                        ><path
                                            d="M8 3a2 2 0 00-2 2v4a2 2 0 01-2 2H3a1 1 0 000 2h1a2 2 0 012 2v4a2 2 0 002 2"
                                        /><path
                                            d="M16 3a2 2 0 012 2v4a2 2 0 002 2h1a1 1 0 010 2h-1a2 2 0 00-2 2v4a2 2 0 01-2 2"
                                        /></svg
                                    >
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">
                                        Connection & Query
                                    </h3>
                                    <p class="stg-card-sub">
                                        Defaults for NoSQL connections. Timeouts
                                        apply to new connections; the find-limit
                                        caps gate both the document viewer and
                                        the AI-tool query path.
                                    </p>
                                </div>
                            </header>
                            <div class="stg-card-body">
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Server Selection Timeout (ms)</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="number"
                                        value={nosqlServerSelectionTimeoutMs}
                                        min="1000"
                                        step="1000"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "nosql_server_selection_timeout_ms",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Connect Timeout (ms)</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="number"
                                        value={nosqlConnectTimeoutMs}
                                        min="1000"
                                        step="1000"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "nosql_connect_timeout_ms",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Default Find Limit</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="number"
                                        value={nosqlDefaultFindLimit}
                                        min="1"
                                        step="10"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "nosql_default_find_limit",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Max Find Limit</label
                                    >
                                    <input
                                        class="stg-input"
                                        type="number"
                                        value={nosqlMaxFindLimit}
                                        min="1"
                                        step="10"
                                        onchange={(e) =>
                                            handleSettingChange(
                                                "nosql_max_find_limit",
                                                e.currentTarget.value,
                                            )}
                                    />
                                </div>
                            </div>
                        </section>
                    </div>
                {:else if activeTab === "explorer"}
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
                                        ><path
                                            d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z"
                                        /></svg
                                    >
                                </span>
                                <div class="stg-card-titles">
                                    <h3 class="stg-card-title">Listing</h3>
                                    <p class="stg-card-sub">
                                        Defaults for the Explorer file browser.
                                        Folder-size aggregation walks every
                                        object under each subfolder — accurate,
                                        but adds N×pages extra round-trips per
                                        listing on object stores (S3, Azure).
                                        Off keeps folder navigation snappy on
                                        large buckets. Change takes effect on
                                        the next connection.
                                    </p>
                                </div>
                            </header>
                            <div class="stg-card-body">
                                <div class="stg-card-row">
                                    <label class="stg-card-row-label"
                                        >Show folder sizes (S3, Azure)</label
                                    >
                                    <label class="stg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={explorerShowFolderSizes}
                                            onchange={(e) =>
                                                handleSettingChange(
                                                    "explorer_show_folder_sizes",
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
                                An AI-powered cross-platform desktop super-app
                                for developers. One shell, many tools.
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
                                        Choose the release channel Clauge checks
                                        for new versions.
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
                                        <svg
                                            viewBox="0 0 106 106"
                                            class="tech-icon"
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
                                        <svg
                                            viewBox="0 0 128 128"
                                            class="tech-icon"
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
                                        <svg
                                            viewBox="0 0 24 24"
                                            class="tech-icon"
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
                                        <svg
                                            viewBox="0 0 24 24"
                                            class="tech-icon"
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
                                        <svg
                                            viewBox="0 0 24 24"
                                            class="tech-icon"
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
                                        <svg
                                            viewBox="0 0 24 24"
                                            class="tech-icon"
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
                                            ><circle
                                                cx="12"
                                                cy="12"
                                                r="10"
                                            /><path
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
                                            /><circle
                                                cx="12"
                                                cy="7"
                                                r="4"
                                            /></svg
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
                                            ><circle
                                                cx="12"
                                                cy="12"
                                                r="10"
                                            /><line
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
                    </div>
                {/if}
            </div>
        </div>
    </div>
{/if}

<ConfirmDialog
    bind:show={showClearChatHistoryConfirm}
    title="Clear History"
    message={`This will clear ${restHistoryCount.toLocaleString()} request${restHistoryCount === 1 ? "" : "s"} from the History section and ${aiChatCount.toLocaleString()} AI Assistance chat message${aiChatCount === 1 ? "" : "s"} across all modes.\n\nThis cannot be undone.`}
    confirmText="Clear History"
    onconfirm={handleClearChatHistory}
/>

<style>
    @import "./SettingsModal.svelte.css";

    /* Settings pane — rendered inside the 'settings' panel slot in
       +page.svelte. Fills the panel (which is the workspace area). */
    .stg-pane {
        flex: 1;
        min-width: 0;
        min-height: 0;
        display: flex;
        background: var(--c);
    }
    /* In glass mode, bump Settings' background opacity so its dense
       text + form rows stay legible (var(--c) is rgba 0.40 — fine
       for chrome surfaces, too translucent for a full content panel).
       glass-surface adds the backdrop-filter blur via app.css. */
    :global(body.glass-mode) .stg-pane {
        background: var(--modal-bg);
    }

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
        background: var(--surface-hover);
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
        background: var(--surface-hover);
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
        background: var(--surface-hover);
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
        transition:
            border-color 0.12s,
            color 0.12s;
    }
    .stg-card-mini-btn:hover:not(:disabled) {
        border-color: var(--b2);
        color: var(--t1);
    }
    .stg-card-mini-btn:disabled {
        opacity: 0.4;
        cursor: not-allowed;
    }

    /* CLI provider tab strip above the plugins panel. Hairline underline
     * on the active tab so it reads as "scope filter" rather than a
     * primary CTA. */
    .plugin-provider-tabs {
        display: flex;
        gap: 4px;
        border-bottom: 1px solid var(--b1);
        margin-bottom: 12px;
    }
    .plugin-provider-tab {
        padding: 8px 14px;
        background: transparent;
        border: none;
        border-bottom: 2px solid transparent;
        color: var(--t3);
        font-family: var(--ui);
        font-size: 12.5px;
        font-weight: 500;
        cursor: default;
        transition:
            color 0.12s,
            border-color 0.12s;
    }
    .plugin-provider-tab:hover {
        color: var(--t1);
    }
    .plugin-provider-tab.active {
        color: var(--t1);
        border-bottom-color: var(--acc);
    }

    .plugin-codex-hint {
        display: flex;
        align-items: flex-start;
        gap: 8px;
        padding: 10px 12px;
        margin: 8px 0 12px;
        background: color-mix(in srgb, var(--acc) 8%, transparent);
        border: 1px solid color-mix(in srgb, var(--acc) 25%, var(--b1));
        border-radius: 8px;
        font-size: 12px;
        line-height: 1.55;
        color: var(--t2);
        font-family: var(--ui);
    }
    .plugin-codex-hint svg {
        flex-shrink: 0;
        margin-top: 2px;
        color: var(--acc);
    }
    .plugin-codex-hint code {
        font-family: var(--mono, monospace);
        font-size: 11px;
        background: var(--c);
        border: 1px solid var(--b1);
        border-radius: 4px;
        padding: 0 5px;
        color: var(--t1);
    }

    /* BYOK add button */
    .byok-add-btn {
        padding: 5px 12px;
        border-radius: var(--radius-md, 6px);
        border: 1px solid var(--b2, var(--b1));
        background: transparent;
        color: var(--t2);
        font-family: var(--ui);
        font-size: 12px;
        font-weight: 500;
        cursor: pointer;
        transition:
            background 0.12s,
            border-color 0.12s,
            color 0.12s;
    }
    .byok-add-btn:hover {
        background: var(--surface-hover);
        border-color: var(--acc);
        color: var(--t1);
    }

    /* BYOK multi-config section */
    .byok-section {
        margin-top: 1.5rem;
    }
    .byok-section-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-bottom: 0.75rem;
    }
    .byok-section-title {
        margin: 0;
        font-size: 1rem;
        font-weight: 600;
        font-family: var(--ui);
        color: var(--t1);
    }
    .byok-empty-state {
        color: var(--t3);
        font-size: 0.875rem;
        font-family: var(--ui);
        text-align: center;
        padding: 1.5rem;
    }
    .byok-config-list {
        list-style: none;
        padding: 0;
        margin: 0;
    }
    .byok-config-row {
        display: grid;
        grid-template-columns: 1fr auto;
        grid-template-areas:
            "info actions"
            "stats stats";
        column-gap: 0.5rem;
        row-gap: 0.45rem;
        align-items: center;
        padding: 0.75rem 1rem;
        border: 1px solid var(--b1);
        border-radius: var(--radius-md, 6px);
        margin-bottom: 0.5rem;
        background: var(--surface-hover, transparent);
    }
    .byok-config-info {
        grid-area: info;
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-width: 0;
    }
    .byok-config-actions {
        grid-area: actions;
    }
    .byok-config-stats {
        grid-area: stats;
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 11.5px;
        color: var(--t3);
        font-family: var(--ui);
        padding-top: 0.4rem;
        border-top: 1px dashed color-mix(in srgb, var(--b1) 80%, transparent);
    }
    .byok-stat {
        white-space: nowrap;
        font-variant-numeric: tabular-nums;
    }
    .byok-stat strong {
        color: var(--t2);
        font-weight: 600;
    }
    .byok-stat.muted {
        font-style: italic;
        opacity: 0.7;
    }
    .byok-stat-sep {
        opacity: 0.5;
    }
    .byok-config-label {
        font-weight: 500;
        font-family: var(--ui);
        font-size: 13px;
        color: var(--t1);
    }
    .byok-default-badge {
        padding: 0.125rem 0.5rem;
        font-size: 0.7rem;
        font-family: var(--ui);
        background: var(--acc);
        color: #fff;
        border-radius: 3px;
        flex-shrink: 0;
    }
    .byok-config-provider {
        color: var(--t3);
        font-size: 0.85rem;
        font-family: var(--ui);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .byok-config-actions-inner {
        display: flex;
        gap: 0.5rem;
        flex-shrink: 0;
    }
    .byok-btn-link {
        background: transparent;
        border: 0;
        color: var(--acc);
        cursor: pointer;
        font-size: 0.85rem;
        font-family: var(--ui);
        padding: 0;
    }
    .byok-btn-link.danger {
        color: var(--err, #ff6b6b);
    }
    .byok-btn-link:hover {
        text-decoration: underline;
    }
</style>

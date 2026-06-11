<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { get } from "svelte/store";
    import {
        cloudConnected,
        cloudUser,
        cloudProviders,
        cloudPlan,
        cloudCredits,
        cloudSub,
        upgradeModalOpen,
        syncing,
        setSyncing,
        setConnected,
        setDisconnected,
        lastSyncedByKind,
        setLastSyncedForKinds,
        cloudConflicts,
        type Provider,
    } from "$lib/stores/cloud";
    import { invoke } from "@tauri-apps/api/core";
    import ConflictResolverModal from "$lib/components/cloud/ConflictResolverModal.svelte";

    let conflictResolverOpen = $state(false);
    import {
        cloudGetStatus,
        cloudGithubLoginUrl,
        cloudGoogleLoginUrl,
        cloudExchangeCode,
        cloudLinkProvider,
        cloudUnlinkProvider,
        cloudLogout,
        cloudWipeRemote,
        cloudDeleteAccount,
        cloudSyncPushNow,
        cloudSyncRestore,
        cloudUpdateProfile,
        cloudListSnapshots,
        cloudRestoreSnapshot,
        cloudRemoteState,
        cloudHistoryList,
        cloudHistoryRestore,
        type SnapshotInfo,
        type SyncStateRow,
        type SyncHistoryEntry,
    } from "$lib/commands/cloud";
    import { reloadSyncedStores } from "$lib/commands/syncReload";
    import { decideFirstSync } from "$lib/services/firstSync";
    import { showToast } from "$lib/shared/primitives/toast";
    import { friendlyError } from "$lib/utils/errors";
    import Dropdown from "$lib/shared/primitives/Dropdown.svelte";
    import ConfirmDialog from "$lib/shared/primitives/ConfirmDialog.svelte";
    import { APP_EVENT } from "$lib/shared/constants/events";
    import { settings, setSetting, loadSettings } from "$lib/stores/settings";
    import { kindLabel } from "$lib/shared/utils/kind-label";

    let displayNameInput = $state("");
    let firstNameInput = $state("");
    let lastNameInput = $state("");
    let savingProfile = $state(false);
    let refreshing = $state(false);
    let linking = $state<Provider | null>(null);
    let signingIn = $state<Provider | null>(null);

    let confirmingWipe = $state(false);
    let confirmingDelete = $state(false);
    let deleteSlugInput = $state("");
    // In-flight flags so the danger buttons disable + show "…ing" copy
    // and a fat-finger double-click can't fire the action twice.
    let wiping = $state(false);
    let deleting = $state(false);
    // Live slug-match indicator — drives the validation hint and the
    // submit-button disabled state. `null` (cloudUser missing) is treated
    // as "no match" so the button stays disabled in the edge case.
    let slugExpected = $derived($cloudUser?.slug ?? "");
    let slugMatches = $derived(
        deleteSlugInput.trim().length > 0 &&
            deleteSlugInput.trim() === slugExpected,
    );
    // Any Pro user (recurring OR lifetime) needs to know their Pro plan +
    // remaining credits are part of what gets deleted — used to gate the
    // "Your Pro plan and N credits" bullet in the confirm panel.
    let isPro = $derived($cloudPlan === "pro");
    // Pro recurring users get a callout specifically about subscription
    // cancellation (next renewal date, no further charges). Free + lifetime
    // users don't see this one — different message paths for each.
    let isProRecurring = $derived(
        isPro &&
            !!$cloudSub &&
            !$cloudSub.isLifetime &&
            ($cloudSub.interval === "monthly" || $cloudSub.interval === "yearly"),
    );
    // Lifetime Pro users get their own callout — different copy because
    // there's no recurring charge to cancel, but the one-time purchase
    // amount IS non-refundable, which the user needs to acknowledge.
    //
    // Accept EITHER `isLifetime === true` OR `interval === "lifetime"` as
    // the lifetime signal. In practice both are always set together by
    // the worker's `buildMeBody`, but `is_lifetime` is `#[serde(default)]`
    // on the Rust side — a partial/legacy response that ships `interval`
    // but drops `is_lifetime` would otherwise drop a lifetime user into
    // the free-user UI on delete confirm (no warning, wrong toast). Mirror
    // of the both-signals approach `isProRecurring` already uses.
    let isProLifetime = $derived(
        isPro &&
            !!$cloudSub &&
            ($cloudSub.isLifetime === true || $cloudSub.interval === "lifetime"),
    );

    let menuOpen = $state(false);
    let menuAnchor: HTMLElement | null = $state(null);
    let confirmPull = $state(false);

    let snapshots = $state<SnapshotInfo[]>([]);
    let restoringSnapshot = $state<string | null>(null);
    let confirmRestoreSnapshot = $state(false);
    let snapshotToRestore = $state<SnapshotInfo | null>(null);

    // Cloud version history — lazy-loaded per kind on selection.
    let historyKind = $state<string | null>(null);
    let historyEntries = $state<SyncHistoryEntry[]>([]);
    // Monotonic request token — switching kind pills quickly must not let a
    // slow stale response overwrite the newer kind's list.
    let historyReq = 0;
    let loadingHistory = $state(false);
    let restoringHistory = $state<string | null>(null);
    let confirmRestoreHistory = $state(false);
    let historyToRestore = $state<SyncHistoryEntry | null>(null);

    let now = $state(Date.now());
    let tickerId: ReturnType<typeof setInterval> | null = null;

    $effect(() => {
        const u = $cloudUser;
        if (u) {
            displayNameInput = u.displayName ?? "";
            firstNameInput = u.firstName ?? "";
            lastNameInput = u.lastName ?? "";
        }
    });

    async function handleOAuthCallback(e: Event) {
        if (!get(settings)["onboarding_complete"]) return;
        const detail = (e as CustomEvent<{ provider: Provider; code: string }>)
            .detail;
        if (!detail?.code || !detail?.provider) return;

        if (get(cloudConnected) && linking === detail.provider) {
            try {
                const s = await cloudLinkProvider(detail.provider, detail.code);
                if (s.user)
                    setConnected(s.user, s.providers, s.activeProvider, s.plan);
                showToast(`Linked ${detail.provider}`, "success");
            } catch (err) {
                showToast(friendlyError(err), "error");
            } finally {
                linking = null;
            }
            return;
        }

        if (signingIn !== detail.provider) return;
        try {
            const s = await cloudExchangeCode(detail.provider, detail.code);
            if (s.user) {
                setConnected(s.user, s.providers, s.activeProvider, s.plan);
                setLastSyncedForKinds(s.lastSynced);
                showToast(
                    `Connected as ${s.user.displayName || s.user.slug}`,
                    "success",
                );
            }
            // Shared 4-case first-sync decision (restore prompt / push /
            // device setup) — same path the layout boot block runs.
            await decideFirstSync();
            // The lazily-loaded per-kind device/time diagnostics were
            // fetched (or skipped) while signed out — refresh them now.
            loadRemoteState();
        } catch (err) {
            showToast(friendlyError(err), "error");
        } finally {
            signingIn = null;
        }
    }

    onMount(() => {
        if (get(cloudConnected)) {
            refreshStatus().catch(() => {});
            loadRemoteState();
        }
        initDeviceName();
        loadSnapshots();
        tickerId = setInterval(() => {
            now = Date.now();
        }, 30_000);
        window.addEventListener(APP_EVENT.OAUTH_CALLBACK, handleOAuthCallback);
    });
    onDestroy(() => {
        if (tickerId) clearInterval(tickerId);
        window.removeEventListener(
            APP_EVENT.OAUTH_CALLBACK,
            handleOAuthCallback,
        );
    });

    async function refreshStatus() {
        refreshing = true;
        try {
            const s = await cloudGetStatus();
            if (s.user) {
                // Identity-side stores only — entitlements (plan, credits,
                // subscription) are owned by Rust's ProStateManager. The
                // cloud_get_status command applies them server-side and emits
                // cloud:pro-state, which the layout's subscription pipes
                // into proState → derived cloudPlan / cloudCredits / cloudSub.
                setConnected(s.user, s.providers, s.activeProvider);
                setLastSyncedForKinds(s.lastSynced);
            }
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            refreshing = false;
        }
    }

    function profileChanged(): boolean {
        const u = $cloudUser;
        if (!u) return false;
        return (
            displayNameInput.trim() !== (u.displayName ?? "") ||
            firstNameInput.trim() !== (u.firstName ?? "") ||
            lastNameInput.trim() !== (u.lastName ?? "")
        );
    }

    async function saveProfile() {
        if (savingProfile) return;
        savingProfile = true;
        try {
            const s = await cloudUpdateProfile({
                displayName: displayNameInput.trim(),
                firstName: firstNameInput.trim(),
                lastName: lastNameInput.trim(),
            });
            if (s.user)
                setConnected(s.user, s.providers, s.activeProvider, s.plan);
            showToast("Profile updated", "success");
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            savingProfile = false;
        }
    }

    async function openOAuth(provider: Provider) {
        try {
            const url =
                provider === "github"
                    ? await cloudGithubLoginUrl()
                    : await cloudGoogleLoginUrl();
            const { openUrl } = await import("@tauri-apps/plugin-opener");
            await openUrl(url);
        } catch (e) {
            showToast(friendlyError(e), "error");
            signingIn = null;
            linking = null;
        }
    }

    async function signIn(provider: Provider) {
        signingIn = provider;
        await openOAuth(provider);
    }

    async function linkAdditional(provider: Provider) {
        linking = provider;
        await openOAuth(provider);
    }

    async function unlink(provider: Provider) {
        try {
            const s = await cloudUnlinkProvider(provider);
            if (s.user)
                setConnected(s.user, s.providers, s.activeProvider, s.plan);
            showToast(`Unlinked ${provider}`, "info");
        } catch (e) {
            showToast(friendlyError(e), "error");
        }
    }

    async function signOut() {
        try {
            await cloudLogout();
            setDisconnected();
            showToast("Signed out", "info");
        } catch (e) {
            showToast(friendlyError(e), "error");
        }
    }

    async function syncNow() {
        if ($syncing) return;
        setSyncing(true);
        const startedAt = Date.now();
        try {
            const pushed = await cloudSyncPushNow();
            try {
                const s = await cloudGetStatus();
                if (s.user) {
                    setConnected(s.user, s.providers, s.activeProvider, s.plan);
                    setLastSyncedForKinds(s.lastSynced);
                }
            } catch {
                /* non-fatal */
            }
            loadRemoteState();
            const elapsed = Date.now() - startedAt;
            if (elapsed < 350)
                await new Promise((r) => setTimeout(r, 350 - elapsed));
            showToast(
                pushed.length ? "Synced" : "Already up to date",
                "success",
            );
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            setSyncing(false);
        }
    }

    async function pullFromCloud() {
        if ($syncing) return;
        setSyncing(true);
        try {
            await cloudSyncRestore();
            await reloadSyncedStores();
            const { announceRestoreCompletion } = await import(
                "$lib/stores/missingCredentials"
            );
            const shown = await announceRestoreCompletion();
            if (!shown) showToast("Restored from cloud", "success");
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            setSyncing(false);
        }
    }

    async function loadSnapshots() {
        try {
            snapshots = await cloudListSnapshots();
        } catch {
            snapshots = [];
        }
    }

    // createdAt is a compact stamp like "20260610T142233.123Z-ab12cd" —
    // parse the YYYYMMDDTHHMMSS prefix (UTC) into epoch ms, or 0 if odd.
    function parseSnapshotStamp(stamp: string): number {
        const m = stamp.match(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})/);
        if (!m) return 0;
        return Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]);
    }

    function fmtSnapshotTime(stamp: string): string {
        void now;
        const t = parseSnapshotStamp(stamp);
        if (!t) return stamp;
        const diff = Math.max(0, Date.now() - t);
        if (diff < 60_000) return "just now";
        if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
        if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
        return new Date(t).toLocaleDateString();
    }

    function fmtSnapshotSize(bytes: number): string {
        return `${(bytes / 1024).toFixed(1)} KB`;
    }

    async function restoreSnapshot() {
        const s = snapshotToRestore;
        snapshotToRestore = null;
        if (!s || restoringSnapshot) return;
        restoringSnapshot = s.fileName;
        try {
            await cloudRestoreSnapshot(s.fileName);
            await reloadSyncedStores();
            showToast("Snapshot restored", "success");
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            restoringSnapshot = null;
            loadSnapshots();
        }
    }

    // replacedAt is D1 CURRENT_TIMESTAMP ("YYYY-MM-DD HH:MM:SS", UTC) —
    // parseServerTime appends the missing Z before diffing.
    function fmtHistoryTime(s: string): string {
        void now;
        const t = parseServerTime(s);
        return t ? fmtAgo(t) : s;
    }

    async function selectHistoryKind(kind: string) {
        const req = ++historyReq;
        historyKind = kind;
        historyEntries = [];
        loadingHistory = true;
        try {
            const entries = await cloudHistoryList(kind);
            if (req !== historyReq) return; // a newer pill won the race
            historyEntries = entries;
        } catch (e) {
            if (req !== historyReq) return;
            showToast(friendlyError(e), "error");
        } finally {
            // Only the latest request may flip the spinner off — otherwise a
            // stale response would enable the restore buttons early.
            if (req === historyReq) loadingHistory = false;
        }
    }

    async function restoreHistoryVersion() {
        const entry = historyToRestore;
        const kind = historyKind;
        historyToRestore = null;
        if (!entry || !kind || restoringHistory) return;
        restoringHistory = entry.contentHash;
        try {
            await cloudHistoryRestore(kind, entry.contentHash);
            showToast("Version restored", "success");
            await reloadSyncedStores();
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            restoringHistory = null;
            // The restore force-pushed a new cloud blob (archiving the old
            // one) AND wrote a pre-history-restore local snapshot — refresh
            // both lists so the UI reflects that.
            if (historyKind === kind) selectHistoryKind(kind);
            loadSnapshots();
        }
    }

    async function wipeRemote() {
        if (wiping) return; // guard against double-click
        wiping = true;
        try {
            await cloudWipeRemote();
            // Backend no longer logs the user out on wipe (intentional —
            // matches the "your account stays" UI promise). We refresh the
            // status so the `lastSyncedByKind` row in the UI clears to
            // reflect the empty cloud state.
            await refreshStatus();
            showToast("Cloud data wiped — local data intact", "success");
            confirmingWipe = false;
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            wiping = false;
        }
    }

    async function deleteAccount() {
        if (deleting) return; // guard against double-click
        if (!slugMatches) {
            showToast("Type your handle exactly to confirm", "error");
            return;
        }
        // Snapshot BEFORE the async block — `setDisconnected()` clears
        // `$cloudUser` and `$cloudSub`, which the derived flags below
        // read from. Without these snapshots, the success toast would
        // pick the wrong copy for Pro users because the derived values
        // would read post-disconnect (free) state.
        const wasProRecurring = isProRecurring;
        const wasProLifetime = isProLifetime;
        deleting = true;
        try {
            await cloudDeleteAccount(deleteSlugInput.trim());
            setDisconnected();
            showToast(
                wasProRecurring
                    ? "Account deleted and subscription cancelled"
                    : wasProLifetime
                      ? "Account and lifetime Pro purchase deleted"
                      : "Account deleted — local data intact",
                "success",
            );
            confirmingDelete = false;
            deleteSlugInput = "";
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            deleting = false;
        }
    }

    function providerLinked(p: Provider): boolean {
        return $cloudProviders.some((x) => x.provider === p);
    }

    // SQLite CURRENT_TIMESTAMP is "YYYY-MM-DD HH:MM:SS" with no timezone — actually
    // UTC, but JS treats unmarked strings as local. Append Z so the diff is right.
    function parseServerTime(s: string | undefined): number {
        if (!s) return 0;
        const hasTz = /Z$|[+-]\d{2}:?\d{2}$/.test(s);
        return new Date(hasTz ? s : s.replace(" ", "T") + "Z").getTime();
    }

    function fmtAgo(t: number): string {
        const diff = Math.max(0, Date.now() - t);
        if (diff < 60_000) return "just now";
        if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
        if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
        return new Date(t).toLocaleDateString();
    }

    let lastSyncOverall = $derived.by(() => {
        void now;
        let max = 0;
        for (const v of Object.values($lastSyncedByKind)) {
            const t = parseServerTime(v as string | undefined);
            if (t > max) max = t;
        }
        if (!max) return null;
        return fmtAgo(max);
    });

    // Remote per-kind state (last writing device) — lazy-loaded, decorative.
    let remoteState = $state<SyncStateRow[]>([]);
    async function loadRemoteState() {
        try {
            remoteState = await cloudRemoteState();
        } catch {
            /* silent — device labels are optional decoration */
        }
        // The push path writes `cloud:too_large:<kind>` flags from Rust,
        // bypassing the settings store — re-pull so the chips are fresh.
        loadSettings().catch(() => {});
    }

    // Kinds the server currently holds — drives the history kind selector.
    let historyKinds = $derived(remoteState.map((r) => r.kind));

    // Kinds whose last export exceeded the worker's payload limit — the
    // push path parks a `cloud:too_large:<kind>` setting (value = gzipped
    // byte estimate) instead of pushing. Map kind → KB for the chips.
    let tooLargeKinds = $derived.by(() => {
        const out: Record<string, number> = {};
        const prefix = "cloud:too_large:";
        for (const [k, v] of Object.entries($settings)) {
            if (k.startsWith(prefix)) {
                out[k.slice(prefix.length)] = Math.round(
                    (parseInt(v, 10) || 0) / 1024,
                );
            }
        }
        return out;
    });

    let lastSyncDevice = $derived.by(() => {
        let best: SyncStateRow | null = null;
        let bestT = 0;
        for (const r of remoteState) {
            const t = parseServerTime(r.updatedAt);
            if (t > bestT) {
                bestT = t;
                best = r;
            }
        }
        return best?.deviceName ?? null;
    });

    let syncBreakdownTitle = $derived.by(() => {
        void now;
        const lines = remoteState.map((r) => {
            const t = parseServerTime(r.updatedAt);
            const from = r.deviceName ? ` from ${r.deviceName}` : "";
            const big =
                tooLargeKinds[r.kind] !== undefined
                    ? ` — too large to sync (~${tooLargeKinds[r.kind]} KB)`
                    : "";
            return `${kindLabel(r.kind)} — ${t ? fmtAgo(t) : r.updatedAt}${from}${big}`;
        });
        // Oversize kinds the server has never seen still deserve a line.
        for (const [k, kb] of Object.entries(tooLargeKinds)) {
            if (!remoteState.some((r) => r.kind === k)) {
                lines.push(`${kindLabel(k)} — too large to sync (~${kb} KB)`);
            }
        }
        return lines.join("\n");
    });

    // Device name shown on other devices next to data this one pushed.
    let deviceNameInput = $state("");
    async function initDeviceName() {
        const cur = (get(settings)["cloud:device_name"] ?? "").trim();
        if (cur) {
            deviceNameInput = cur;
            return;
        }
        let name = "This device";
        try {
            const { hostname } = await import("@tauri-apps/plugin-os");
            name = ((await hostname()) ?? "").trim() || name;
        } catch {
            /* fall back to generic label */
        }
        deviceNameInput = name;
        try {
            await setSetting("cloud:device_name", name);
        } catch {
            /* non-fatal — retried on next edit */
        }
    }
    async function saveDeviceName() {
        const v = deviceNameInput.trim().slice(0, 64);
        if (!v) {
            deviceNameInput = (get(settings)["cloud:device_name"] ?? "").trim();
            return;
        }
        deviceNameInput = v;
        if (v === get(settings)["cloud:device_name"]) return;
        try {
            await setSetting("cloud:device_name", v);
        } catch (e) {
            showToast(friendlyError(e), "error");
        }
    }

    function copyHandle() {
        const slug = $cloudUser?.slug ?? "";
        if (!slug) return;
        try {
            navigator.clipboard.writeText(`@${slug}`);
            showToast("Handle copied", "info");
        } catch {
            showToast("Could not copy", "error");
        }
    }

    function openUpgrade() {
        upgradeModalOpen.set(true);
    }

    let openingPortal = $state(false);
    async function openManageSubscription() {
        if (openingPortal) return;
        openingPortal = true;
        try {
            const url = await invoke<string>("cloud_open_portal");
            const { openUrl } = await import("@tauri-apps/plugin-opener");
            await openUrl(url);
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            openingPortal = false;
        }
    }

    // Auto-load AI balance for Pro users so the Profile-card strip shows
    // credits without requiring the user to send a chat first (the SSE
    // balance event is the only other writer to cloudCredits).
    let balanceLoadedFor = $state<string | null>(null);
    async function refreshBalance() {
        try {
            // The Rust `cloud_ai_balance` command patches the ProStateManager
            // with the fresh remaining/allowance, which emits cloud:pro-state
            // → updates derived cloudCredits reactively. We just fire and
            // forget; the response is consumed for error detection only.
            await invoke("cloud_ai_balance");
        } catch {
            /* silent — strip falls back to plan affirmation only */
        }
    }
    $effect(() => {
        const key = `${$cloudConnected}:${$cloudPlan}`;
        if (!$cloudConnected || $cloudPlan !== "pro") return;
        if (balanceLoadedFor === key) return;
        balanceLoadedFor = key;
        refreshBalance();
    });

    function formatResetCountdown(resetsAt: string | null): string {
        if (!resetsAt) return "";
        const reset = new Date(resetsAt).getTime();
        const days = Math.max(0, Math.ceil((reset - Date.now()) / 86400000));
        if (days === 0) return "Resets today";
        if (days === 1) return "Resets tomorrow";
        return `Resets in ${days} days`;
    }

    // Absolute date for the subscription card (e.g. "Jun 17, 2026").
    function fmtAbsDate(iso: string | null | undefined): string {
        if (!iso) return "";
        try {
            return new Date(iso).toLocaleDateString(undefined, {
                month: "short",
                day: "numeric",
                year: "numeric",
            });
        } catch {
            return "";
        }
    }
    function daysUntil(iso: string | null | undefined): number | null {
        if (!iso) return null;
        const t = new Date(iso).getTime();
        if (Number.isNaN(t)) return null;
        return Math.max(0, Math.ceil((t - Date.now()) / 86400000));
    }
    function fmtDaysCount(n: number): string {
        if (n === 0) return "today";
        if (n === 1) return "tomorrow";
        return `in ${n} days`;
    }
    // Coarse-grained "how long ago" for Member Since. ISO → "8 months" / "2 years".
    function fmtRelativeSince(iso: string | null | undefined): string {
        if (!iso) return "";
        const then = new Date(iso).getTime();
        if (Number.isNaN(then)) return "";
        const days = Math.max(0, Math.floor((Date.now() - then) / 86400000));
        if (days < 30) return days <= 1 ? "today" : `${days} days`;
        const months = Math.floor(days / 30);
        if (months < 12) return months === 1 ? "1 month" : `${months} months`;
        const years = Math.floor(days / 365);
        return years === 1 ? "1 year" : `${years} years`;
    }
    function capitalize(s: string): string {
        return s ? s[0].toUpperCase() + s.slice(1) : s;
    }

    function openMenu(e: MouseEvent) {
        menuAnchor = e.currentTarget as HTMLElement;
        menuOpen = true;
    }

    let menuItems = $derived([
        {
            label: refreshing ? "Refreshing…" : "Refresh status",
            action: () => {
                refreshStatus();
            },
        },
        {
            label: "Pull from cloud",
            action: () => {
                confirmPull = true;
            },
        },
        {
            label: "Copy handle",
            action: () => {
                copyHandle();
            },
        },
        { separator: true, label: "", action: () => {} },
        {
            label: "Sign out",
            action: () => {
                signOut();
            },
            danger: true,
        },
    ]);

    // Profile avatar can 404 / time out (network down, image expired,
    // host blocked). Without this guard the <img> renders as the broken-
    // image glyph; we'd rather fall back to the initial-letter avatar.
    // Reset on URL change so a retry works after the network recovers.
    let acc_avatar_failed = $state(false);
    $effect(() => {
        const _ = $cloudUser?.avatarUrl;
        acc_avatar_failed = false;
    });
</script>

<div class="acc-pane">
    <header class="acc-page-head">
        <h1>Account settings</h1>
        <p>Manage your profile and preferences</p>
    </header>

    {#if !$cloudConnected}
        <!-- Signed-out: cloud sign-in card -->
        <section class="acc-signin">
            <div class="acc-signin-icon" aria-hidden="true">
                <svg
                    viewBox="0 0 24 24"
                    width="32"
                    height="32"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.7"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                >
                    <path d="M20 16.58A5 5 0 0018 7h-1.26A8 8 0 104 15.25" />
                    <polyline points="16 16 12 12 8 16" />
                    <line x1="12" y1="12" x2="12" y2="21" />
                </svg>
            </div>
            <h2>Sign in to Clauge cloud</h2>
            <p class="acc-signin-sub">
                Sync your work across every device, automatically.
            </p>

            <!-- <ul class="acc-signin-features">
        <li><span class="acc-bullet"></span>REST collections &amp; saved queries</li>
        <li><span class="acc-bullet"></span>SQL &amp; NoSQL connections</li>
        <li><span class="acc-bullet"></span>SSH profiles &amp; agents</li>
      </ul> -->

            <div class="acc-signin-buttons">
                <button
                    class="acc-oauth-btn"
                    onclick={() => signIn("github")}
                    disabled={!!signingIn}
                >
                    {#if signingIn === "github"}
                        <span class="acc-spinner acc-spinner-light"></span>
                    {:else}
                        <svg
                            viewBox="0 0 16 16"
                            fill="currentColor"
                            width="15"
                            height="15"
                            ><path
                                d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.22 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"
                            /></svg
                        >
                    {/if}
                    <span>Continue with GitHub</span>
                </button>
                <button
                    class="acc-oauth-btn"
                    onclick={() => signIn("google")}
                    disabled={!!signingIn}
                >
                    {#if signingIn === "google"}
                        <span class="acc-spinner acc-spinner-light"></span>
                    {:else}
                        <svg viewBox="0 0 24 24" width="15" height="15"
                            ><path
                                d="M22.5 12.27c0-.79-.07-1.54-.2-2.27H12v4.51h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.32z"
                                fill="#4285F4"
                            /><path
                                d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
                                fill="#34A853"
                            /><path
                                d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09 0-.73.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
                                fill="#FBBC05"
                            /><path
                                d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
                                fill="#EA4335"
                            /></svg
                        >
                    {/if}
                    <span>Continue with Google</span>
                </button>
            </div>

            {#if signingIn}
                <div class="acc-waiting">
                    <span class="acc-spinner acc-spinner-light"></span>
                    <span class="acc-waiting-text"
                        >Waiting for {signingIn === "github"
                            ? "GitHub"
                            : "Google"} authorization in your browser…</span
                    >
                    <button
                        class="acc-waiting-cancel"
                        onclick={() => (signingIn = null)}>Cancel</button
                    >
                </div>
            {/if}

            <hr class="acc-signin-sep" />
            <p class="acc-signin-fine">
                We only request your basic profile — no access to your repos,
                files, or email content.
            </p>
        </section>
    {:else if $cloudUser}
        <div class="acc-stack">
            <!-- Profile card -->
            <section class="acc-card">
                <div class="acc-card-head">
                    <h3 class="acc-card-title">Profile</h3>
                    <div class="acc-sync-controls">
                        <div
                            class="acc-sync-status"
                            title={syncBreakdownTitle || undefined}
                        >
                            <span class="acc-sync-label">Last sync</span>
                            <span class="acc-sync-value"
                                >{lastSyncOverall ?? "Never"}</span
                            >
                            {#if lastSyncOverall && lastSyncDevice}
                                <span class="acc-sync-device"
                                    >from {lastSyncDevice}</span
                                >
                            {/if}
                        </div>
                        {#if $cloudConflicts.length > 0}
                            <!-- Conflict-mode replacement for the Sync
                                 button — same slot, accent-tinted, opens
                                 the resolver instead of pushing. -->
                            <button
                                class="acc-sync-btn acc-sync-btn-action"
                                onclick={() => (conflictResolverOpen = true)}
                                title="Resolve sync conflicts"
                            >
                                <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
                                    <line x1="12" y1="9" x2="12" y2="13"/>
                                    <line x1="12" y1="17" x2="12.01" y2="17"/>
                                </svg>
                                <span>Action Required ({$cloudConflicts.length})</span>
                            </button>
                        {:else}
                            <button
                                class="acc-sync-btn"
                                onclick={syncNow}
                                disabled={$syncing}
                                title="Sync now"
                            >
                                {#if $syncing}
                                    <span
                                        class="acc-spinner acc-spinner-light acc-spinner-tiny"
                                    ></span>
                                {:else}
                                    <svg
                                        viewBox="0 0 24 24"
                                        width="13"
                                        height="13"
                                        fill="none"
                                        stroke="currentColor"
                                        stroke-width="2"
                                        stroke-linecap="round"
                                        stroke-linejoin="round"
                                    >
                                        <path d="M23 4v6h-6" />
                                        <path d="M1 20v-6h6" />
                                        <path
                                            d="M3.51 9a9 9 0 0114.85-3.36L23 10"
                                        />
                                        <path
                                            d="M1 14l4.64 4.36A9 9 0 0020.49 15"
                                        />
                                    </svg>
                                {/if}
                                <span>{$syncing ? "Syncing…" : "Sync"}</span>
                            </button>
                        {/if}
                        <button
                            class="acc-kebab-btn"
                            onclick={openMenu}
                            title="More options"
                            aria-label="Account menu"
                        >
                            <svg
                                viewBox="0 0 24 24"
                                width="14"
                                height="14"
                                fill="currentColor"
                                ><circle cx="5" cy="12" r="1.7" /><circle
                                    cx="12"
                                    cy="12"
                                    r="1.7"
                                /><circle cx="19" cy="12" r="1.7" /></svg
                            >
                        </button>
                    </div>
                </div>

                <div class="acc-profile-row">
                    {#if $cloudUser.avatarUrl && !acc_avatar_failed}
                        <img
                            class="acc-avatar"
                            src={$cloudUser.avatarUrl}
                            alt=""
                            referrerpolicy="no-referrer"
                            onerror={() => (acc_avatar_failed = true)}
                        />
                    {:else}
                        <div class="acc-avatar acc-avatar-fallback">
                            {($cloudUser.displayName ?? $cloudUser.slug)
                                .charAt(0)
                                .toUpperCase()}
                        </div>
                    {/if}
                    <div class="acc-profile-text">
                        <div class="acc-profile-name">
                            {$cloudUser.displayName ?? $cloudUser.slug}
                        </div>
                        <div class="acc-profile-email">@{$cloudUser.slug}</div>
                        <div class="acc-profile-meta">
                            <span
                                class="acc-plan-pill"
                                class:is-pro={$cloudPlan === "pro"}
                            >
                                {$cloudPlan === "pro" ? "Pro" : "Free"}
                            </span>
                            {#if $cloudUser.email}
                                <span
                                    class="acc-handle-pill"
                                    title="Provider handle"
                                    >@{$cloudUser.slug}</span
                                >
                            {/if}
                        </div>
                    </div>
                </div>

                <div class="acc-fields">
                    <label class="acc-field">
                        <span class="acc-field-label">Display name</span>
                        <input
                            type="text"
                            bind:value={displayNameInput}
                            maxlength="120"
                        />
                    </label>
                    <div class="acc-field-row">
                        <label class="acc-field">
                            <span class="acc-field-label">First name</span>
                            <input
                                type="text"
                                bind:value={firstNameInput}
                                maxlength="80"
                            />
                        </label>
                        <label class="acc-field">
                            <span class="acc-field-label">Last name</span>
                            <input
                                type="text"
                                bind:value={lastNameInput}
                                maxlength="80"
                            />
                        </label>
                    </div>
                    <label class="acc-field">
                        <span class="acc-field-label">Device name</span>
                        <input
                            type="text"
                            bind:value={deviceNameInput}
                            maxlength="64"
                            onblur={saveDeviceName}
                            onchange={saveDeviceName}
                        />
                    </label>
                    <div class="acc-fields-footer">
                        <p class="acc-fine"></p>
                        <button
                            class="acc-btn acc-btn-primary"
                            onclick={saveProfile}
                            disabled={!profileChanged() || savingProfile}
                        >
                            {#if savingProfile}<span
                                    class="acc-spinner acc-spinner-light acc-spinner-tiny"
                                ></span>{/if}
                            <span
                                >{savingProfile
                                    ? "Saving…"
                                    : "Save changes"}</span
                            >
                        </button>
                    </div>
                </div>

                <!-- Subscription card: three horizontal sections.
                     Row 1: title + status + "Billed X · renews Y" subtitle, manage button on right.
                     Row 2: Clauge AI credits balance with a progress bar.
                     Row 3: 3-column meta grid (Plan / Next Renewal / Member Since).
                     Free users get a compact upsell variant of just row 1. -->
                <section class="acc-sub-card">
                    {#if $cloudPlan === "pro"}
                        {@const interval = $cloudSub?.interval ?? null}
                        {@const isLifetime = $cloudSub?.isLifetime === true || interval === "lifetime"}
                        {@const periodEnd = isLifetime
                            ? null
                            : ($cloudSub?.currentPeriodEnd ??
                              $cloudCredits?.resetsAt ??
                              null)}
                        {@const periodStart =
                            $cloudSub?.currentPeriodStart ?? null}
                        {@const used = $cloudCredits
                            ? Math.max(
                                  0,
                                  $cloudCredits.allowance -
                                      $cloudCredits.remaining,
                              )
                            : 0}
                        {@const renewalDays = daysUntil(periodEnd)}
                        {@const pctRemaining = $cloudCredits
                            ? Math.min(
                                  100,
                                  Math.round(
                                      ($cloudCredits.remaining /
                                          Math.max(
                                              1,
                                              $cloudCredits.allowance,
                                          )) *
                                          100,
                                  ),
                              )
                            : 0}
                        {@const cancelling = $cloudSub?.cancelAtPeriodEnd === true}

                        <!-- Row 1: header -->
                        <header class="acc-sub-head">
                            <div class="acc-sub-head-left">
                                <div class="acc-sub-title-row">
                                    <strong class="acc-sub-title"
                                        >Clauge Pro</strong
                                    >
                                    <span
                                        class="acc-sub-status"
                                        class:is-warn={cancelling ||
                                            $cloudSub?.status === "past_due"}
                                        class:is-lifetime={isLifetime &&
                                            !cancelling &&
                                            $cloudSub?.status !== "past_due"}
                                    >
                                        {#if isLifetime && !cancelling && $cloudSub?.status !== "past_due"}
                                            Lifetime
                                        {:else if cancelling}
                                            Cancelling
                                        {:else}
                                            {capitalize(
                                                $cloudSub?.status ?? "active",
                                            )}
                                        {/if}
                                    </span>
                                </div>
                                {#if periodEnd}
                                    <p class="acc-sub-sub">
                                        {#if isLifetime}
                                            One-time purchase
                                            <span class="acc-sub-dot">·</span>
                                            no renewal
                                        {:else}
                                            {#if interval}Billed {interval}
                                                <span class="acc-sub-dot">·</span>
                                            {/if}
                                            {cancelling ? "ends" : "renews"}
                                            {fmtAbsDate(periodEnd)}
                                        {/if}
                                    </p>
                                {/if}
                            </div>
                            <button
                                class="acc-btn acc-btn-manage"
                                onclick={openManageSubscription}
                                disabled={openingPortal}
                            >
                                {openingPortal
                                    ? "Opening…"
                                    : "Manage subscription"}
                            </button>
                        </header>

                        <div class="acc-sub-divider"></div>

                        <!-- Row 2: credits -->
                        <div class="acc-sub-credits">
                            <div class="acc-sub-credits-row">
                                <span class="acc-sub-credits-label">
                                    <svg
                                        class="acc-sub-credits-icon"
                                        viewBox="0 0 24 24"
                                        width="14"
                                        height="14"
                                        fill="currentColor"
                                        aria-hidden="true"
                                    >
                                        <path
                                            d="M13 2L3 14h7l-1 8 10-12h-7l1-8z"
                                        />
                                    </svg>
                                    Clauge AI credits
                                </span>
                                <span class="acc-sub-credits-val">
                                    {#if $cloudCredits}
                                        <strong
                                            >{$cloudCredits.remaining.toLocaleString()}</strong
                                        >
                                        / {$cloudCredits.allowance.toLocaleString()}
                                        remaining
                                    {:else}
                                        Loading…
                                    {/if}
                                </span>
                            </div>
                            <div class="acc-sub-bar-wrap">
                                <div
                                    class="acc-sub-bar"
                                    style="width: {pctRemaining}%"
                                ></div>
                            </div>
                            {#if $cloudCredits}
                                <p class="acc-sub-credits-meta">
                                    {used.toLocaleString()} credits used{#if periodEnd}
                                        <span class="acc-sub-dot">·</span> resets {fmtAbsDate(
                                            periodEnd,
                                        )}
                                    {/if}
                                </p>
                            {/if}
                        </div>

                        <div class="acc-sub-divider"></div>

                        <!-- Row 3: meta grid -->
                        <div class="acc-sub-meta">
                            <div class="acc-sub-meta-col">
                                <span class="acc-sub-meta-label">PLAN</span>
                                <strong class="acc-sub-meta-val">
                                    {interval
                                        ? capitalize(interval)
                                        : "Pro"}
                                </strong>
                                {#if $cloudSub?.priceUsd && interval}
                                    {@const priceUnit =
                                        interval === "lifetime"
                                            ? "once"
                                            : interval === "yearly"
                                              ? "/ year"
                                              : "/ month"}
                                    <span class="acc-sub-meta-sub">
                                        ${$cloudSub.priceUsd} {priceUnit}
                                    </span>
                                {:else}
                                    <span class="acc-sub-meta-sub"
                                        >Managed AI</span
                                    >
                                {/if}
                            </div>
                            {#if isLifetime ? periodStart : (periodEnd && renewalDays !== null)}
                                <div class="acc-sub-meta-col">
                                    <span class="acc-sub-meta-label">
                                        {#if isLifetime}
                                            PURCHASED
                                        {:else if cancelling}
                                            ENDS
                                        {:else}
                                            NEXT RENEWAL
                                        {/if}
                                    </span>
                                    <strong class="acc-sub-meta-val"
                                        >{fmtAbsDate(isLifetime ? periodStart : periodEnd)}</strong
                                    >
                                    <span class="acc-sub-meta-sub">
                                        {#if isLifetime}
                                            {fmtRelativeSince(periodStart)}
                                        {:else}
                                            {fmtDaysCount(renewalDays!)}
                                        {/if}
                                    </span>
                                </div>
                            {/if}
                            {#if $cloudUser?.createdAt}
                                <div class="acc-sub-meta-col">
                                    <span class="acc-sub-meta-label"
                                        >MEMBER SINCE</span
                                    >
                                    <strong class="acc-sub-meta-val"
                                        >{fmtAbsDate(
                                            $cloudUser.createdAt,
                                        )}</strong
                                    >
                                    <span class="acc-sub-meta-sub"
                                        >{fmtRelativeSince(
                                            $cloudUser.createdAt,
                                        )}</span
                                    >
                                </div>
                            {/if}
                        </div>
                    {:else}
                        <!-- Free state — single row, upsell + Upgrade CTA -->
                        <header class="acc-sub-head">
                            <div class="acc-sub-head-left">
                                <strong class="acc-sub-title"
                                    >Free plan</strong
                                >
                                <p class="acc-sub-sub">
                                    Upgrade for managed AI assistance and
                                    premium features.
                                </p>
                            </div>
                            <button
                                class="acc-btn acc-btn-primary"
                                onclick={openUpgrade}
                            >
                                <svg
                                    viewBox="0 0 24 24"
                                    width="13"
                                    height="13"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2.2"
                                    stroke-linejoin="round"
                                    ><path
                                        d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z"
                                    /></svg
                                >
                                <span>Upgrade to Pro</span>
                            </button>
                        </header>
                    {/if}
                </section>
            </section>

            <!-- Linked accounts -->
            <section class="acc-card">
                <h3 class="acc-card-title acc-card-title-solo">
                    Linked accounts
                </h3>
                {#each ["github", "google"] as p}
                    {@const linked = providerLinked(p as Provider)}
                    {@const meta = linked
                        ? $cloudProviders.find((x) => x.provider === p)
                        : null}
                    <div class="acc-prov-row">
                        <span
                            class="acc-prov-icon"
                            class:gh={p === "github"}
                            class:gg={p === "google"}
                        >
                            {#if p === "github"}
                                <svg
                                    viewBox="0 0 16 16"
                                    fill="currentColor"
                                    width="16"
                                    height="16"
                                    ><path
                                        d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.22 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"
                                    /></svg
                                >
                            {:else}
                                <svg viewBox="0 0 24 24" width="16" height="16"
                                    ><path
                                        d="M22.5 12.27c0-.79-.07-1.54-.2-2.27H12v4.51h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.32z"
                                        fill="#4285F4"
                                    /><path
                                        d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
                                        fill="#34A853"
                                    /><path
                                        d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09 0-.73.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
                                        fill="#FBBC05"
                                    /><path
                                        d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
                                        fill="#EA4335"
                                    /></svg
                                >
                            {/if}
                        </span>
                        <div class="acc-prov-text">
                            <span class="acc-prov-name"
                                >{p === "github" ? "GitHub" : "Google"}</span
                            >
                            {#if linked}
                                <span class="acc-prov-sub"
                                    >{meta?.providerLogin ??
                                        meta?.email ??
                                        ""}</span
                                >
                            {:else}
                                <span class="acc-prov-sub acc-prov-empty"
                                    >Not linked</span
                                >
                            {/if}
                        </div>
                        {#if linked}
                            {#if $cloudProviders.length > 1}
                                <button
                                    class="acc-mini-btn"
                                    onclick={() => unlink(p as Provider)}
                                    >Unlink</button
                                >
                            {:else}
                                <span class="acc-prov-primary"
                                    ><span class="acc-primary-dot"
                                    ></span>primary</span
                                >
                            {/if}
                        {:else}
                            <button
                                class="acc-mini-btn acc-mini-link"
                                onclick={() => linkAdditional(p as Provider)}
                                disabled={linking === p}
                            >
                                {#if linking === p}<span
                                        class="acc-spinner acc-spinner-light acc-spinner-tiny"
                                    ></span>{/if}
                                <span
                                    >{linking === p ? "Opening…" : "Link"}</span
                                >
                            </button>
                        {/if}
                    </div>
                {/each}
            </section>

            <!-- Local snapshots -->
            <section class="acc-card">
                <h3 class="acc-card-title acc-card-title-solo">
                    Local snapshots
                </h3>
                {#if snapshots.length === 0}
                    <p class="acc-snap-empty">
                        No snapshots yet — they're created automatically
                        before any sync restore.
                    </p>
                {:else}
                    {#each snapshots as s (s.fileName)}
                        <div class="acc-snap-row">
                            <div class="acc-snap-text">
                                <span class="acc-snap-name"
                                    >{kindLabel(s.kind)}</span
                                >
                                <span class="acc-snap-sub"
                                    >{s.reason}
                                    <span class="acc-sub-dot">·</span>
                                    {fmtSnapshotTime(s.createdAt)}
                                    <span class="acc-sub-dot">·</span>
                                    {fmtSnapshotSize(s.sizeBytes)}</span
                                >
                            </div>
                            <button
                                class="acc-mini-btn"
                                disabled={restoringSnapshot !== null}
                                onclick={() => {
                                    snapshotToRestore = s;
                                    confirmRestoreSnapshot = true;
                                }}
                            >
                                {#if restoringSnapshot === s.fileName}<span
                                        class="acc-spinner acc-spinner-light acc-spinner-tiny"
                                    ></span>{/if}
                                <span
                                    >{restoringSnapshot === s.fileName
                                        ? "Restoring…"
                                        : "Restore"}</span
                                >
                            </button>
                        </div>
                    {/each}
                {/if}
            </section>

            <!-- Cloud version history -->
            <section class="acc-card">
                <h3 class="acc-card-title acc-card-title-solo">
                    Cloud version history
                </h3>
                {#if historyKinds.length === 0}
                    <p class="acc-snap-empty">
                        Nothing synced to the cloud yet.
                    </p>
                {:else}
                    <div class="acc-hist-kinds">
                        {#each historyKinds as k (k)}
                            <button
                                class="acc-hist-kind-btn"
                                class:acc-hist-kind-active={historyKind === k}
                                onclick={() => selectHistoryKind(k)}
                            >
                                {kindLabel(k)}
                                {#if tooLargeKinds[k] !== undefined}
                                    <span
                                        class="acc-too-large"
                                        title={`Last export was ~${tooLargeKinds[k]} KB gzipped — over the sync limit. New changes for this kind stay local.`}
                                        >Too large to sync</span
                                    >
                                {/if}
                            </button>
                        {/each}
                    </div>
                    {#if historyKind !== null}
                        {#if loadingHistory}
                            <p class="acc-snap-empty">Loading…</p>
                        {:else if historyEntries.length === 0}
                            <p class="acc-snap-empty">
                                No older versions yet — history is written
                                when a device overwrites a synced kind.
                            </p>
                        {:else}
                            {#each historyEntries as h (h.contentHash)}
                                <div class="acc-snap-row">
                                    <div class="acc-snap-text">
                                        <span class="acc-snap-name"
                                            >{h.deviceName ??
                                                "unknown device"}</span
                                        >
                                        <span class="acc-snap-sub"
                                            >{fmtHistoryTime(h.replacedAt)}
                                            <span class="acc-sub-dot">·</span>
                                            {h.contentHash.slice(0, 8)}</span
                                        >
                                    </div>
                                    <button
                                        class="acc-mini-btn"
                                        disabled={restoringHistory !== null}
                                        onclick={() => {
                                            historyToRestore = h;
                                            confirmRestoreHistory = true;
                                        }}
                                    >
                                        {#if restoringHistory === h.contentHash}<span
                                                class="acc-spinner acc-spinner-light acc-spinner-tiny"
                                            ></span>{/if}
                                        <span
                                            >{restoringHistory ===
                                            h.contentHash
                                                ? "Restoring…"
                                                : "Restore"}</span
                                        >
                                    </button>
                                </div>
                            {/each}
                        {/if}
                    {:else}
                        <p class="acc-snap-empty">
                            Pick a kind to see its archived cloud versions.
                        </p>
                    {/if}
                {/if}
            </section>

            <!-- Danger zone -->
            <section class="acc-card acc-card-danger">
                <h3 class="acc-card-title acc-card-title-solo acc-danger-title">
                    Danger zone
                </h3>
                {#if !confirmingWipe && !confirmingDelete}
                    <div class="acc-danger-row">
                        <div>
                            <strong>Wipe cloud data</strong>
                            <p>
                                Removes synced data from our servers. Your
                                account stays, local data stays — you can
                                re-push anytime.
                            </p>
                        </div>
                        <button
                            class="acc-danger-btn"
                            onclick={() => (confirmingWipe = true)}
                            >Wipe data</button
                        >
                    </div>
                    <div class="acc-danger-row">
                        <div>
                            <strong>Delete account</strong>
                            <p>
                                Permanently removes your Clauge account, all
                                linked providers, and cloud data. Cannot be
                                undone.
                            </p>
                        </div>
                        <button
                            class="acc-danger-btn acc-danger-strong"
                            onclick={() => (confirmingDelete = true)}
                            >Delete account</button
                        >
                    </div>
                {/if}
                {#if confirmingWipe}
                    <div
                        class="acc-confirm"
                        role="alertdialog"
                        aria-label="Confirm wipe cloud data"
                        onkeydown={(e) => {
                            if (e.key === "Escape" && !wiping) {
                                confirmingWipe = false;
                            }
                        }}
                    >
                        <p>
                            This deletes synced data from our servers. Your
                            local data stays — your account stays. You can
                            re-push anytime.
                        </p>
                        <div class="acc-confirm-row">
                            <button
                                class="acc-btn acc-btn-ghost"
                                disabled={wiping}
                                onclick={() => (confirmingWipe = false)}
                                >Cancel</button
                            >
                            <button
                                class="acc-danger-btn"
                                disabled={wiping}
                                onclick={wipeRemote}
                                >{wiping
                                    ? "Wiping…"
                                    : "Yes, wipe cloud data"}</button
                            >
                        </div>
                    </div>
                {/if}
                {#if confirmingDelete}
                    <div
                        class="acc-confirm"
                        role="alertdialog"
                        aria-label="Confirm delete account"
                        onkeydown={(e) => {
                            if (e.key === "Escape" && !deleting) {
                                confirmingDelete = false;
                                deleteSlugInput = "";
                            } else if (
                                e.key === "Enter" &&
                                slugMatches &&
                                !deleting
                            ) {
                                deleteAccount();
                            }
                        }}
                    >
                        {#if isProRecurring}
                            <div class="acc-confirm-warn" role="alert">
                                <strong>⚠ Active subscription</strong>
                                <p>
                                    Deleting your account will also cancel
                                    your Clauge Pro
                                    {$cloudSub?.interval}
                                    subscription{$cloudSub?.currentPeriodEnd
                                        ? ` (next renewal: ${new Date($cloudSub.currentPeriodEnd).toLocaleDateString()})`
                                        : ""}.
                                    You will not be charged again. Access ends
                                    immediately. This cannot be undone.
                                </p>
                            </div>
                        {:else if isProLifetime}
                            <div class="acc-confirm-warn" role="alert">
                                <strong>⚠ Lifetime Pro purchase</strong>
                                <p>
                                    Deleting your account will permanently
                                    remove your one-time Clauge Pro Lifetime
                                    purchase and any remaining credits. The
                                    purchase amount is non-refundable. You
                                    will lose access to Pro features
                                    immediately. This cannot be undone.
                                </p>
                            </div>
                        {/if}
                        <p>
                            This permanently removes:
                        </p>
                        <ul class="acc-confirm-bullets">
                            <li>
                                Your Clauge account and handle
                                <code>@{slugExpected}</code>
                            </li>
                            <li>All linked sign-in providers</li>
                            <li>All synced cloud data</li>
                            {#if isPro}
                                <li>
                                    Your Pro plan{$cloudCredits?.remaining
                                        ? ` and ${$cloudCredits.remaining.toLocaleString()} remaining Clauge AI credit${$cloudCredits.remaining === 1 ? "" : "s"}`
                                        : ""}
                                </li>
                            {/if}
                            {#if isProRecurring}
                                <li>
                                    Your active subscription (cancelled at
                                    Polar — no further charges)
                                </li>
                            {:else if isProLifetime}
                                <li>
                                    Your lifetime entitlement (non-refundable)
                                </li>
                            {/if}
                        </ul>
                        <p>
                            Type your handle <code>{slugExpected}</code>
                            to confirm:
                        </p>
                        <input
                            class="acc-confirm-input"
                            bind:value={deleteSlugInput}
                            placeholder="Type your handle exactly"
                            aria-label="Type your handle to confirm deletion"
                            autocomplete="off"
                            autocapitalize="off"
                            spellcheck="false"
                            disabled={deleting}
                            {@attach (node) => {
                                // Autofocus when the confirm panel opens so the
                                // user can start typing immediately.
                                node.focus();
                            }}
                        />
                        {#if deleteSlugInput.trim().length > 0}
                            <p
                                class="acc-confirm-hint"
                                class:acc-confirm-hint-ok={slugMatches}
                            >
                                {slugMatches
                                    ? "✓ Handle matches"
                                    : "Doesn't match — keep typing"}
                            </p>
                        {/if}
                        <div class="acc-confirm-row">
                            <button
                                class="acc-btn acc-btn-ghost"
                                disabled={deleting}
                                onclick={() => {
                                    confirmingDelete = false;
                                    deleteSlugInput = "";
                                }}>Cancel</button
                            >
                            <button
                                class="acc-danger-btn acc-danger-strong"
                                onclick={deleteAccount}
                                disabled={!slugMatches || deleting}
                                >{deleting
                                    ? "Deleting…"
                                    : "Delete account"}</button
                            >
                        </div>
                    </div>
                {/if}
            </section>
        </div>
    {/if}
</div>

<Dropdown
    bind:show={menuOpen}
    anchor={menuAnchor}
    items={menuItems}
    onclose={() => (menuOpen = false)}
/>

<ConfirmDialog
    bind:show={confirmPull}
    title="Pull from cloud?"
    message="This overwrites local collections, connections, queries, agents, SSH profiles, explorer paths, and workspace coworkers with the latest from cloud. Local-only items that haven't been pushed yet will be lost."
    confirmText="Pull from cloud"
    confirmColor="var(--acc)"
    onconfirm={() => {
        confirmPull = false;
        pullFromCloud();
    }}
    oncancel={() => {
        confirmPull = false;
    }}
/>

<ConfirmDialog
    bind:show={confirmRestoreSnapshot}
    title="Restore snapshot?"
    message={`Restore this snapshot? Current data for ${snapshotToRestore ? kindLabel(snapshotToRestore.kind) : "this kind"} is snapshotted first.`}
    confirmText="Restore"
    confirmColor="var(--acc)"
    onconfirm={() => {
        confirmRestoreSnapshot = false;
        restoreSnapshot();
    }}
    oncancel={() => {
        confirmRestoreSnapshot = false;
        snapshotToRestore = null;
    }}
/>

<ConfirmDialog
    bind:show={confirmRestoreHistory}
    title="Restore version?"
    message={`Restore this version of ${historyKind ? kindLabel(historyKind) : "this kind"}? Current data is snapshotted locally and the current cloud copy is archived to history first.`}
    confirmText="Restore"
    confirmColor="var(--acc)"
    onconfirm={() => {
        confirmRestoreHistory = false;
        restoreHistoryVersion();
    }}
    oncancel={() => {
        confirmRestoreHistory = false;
        historyToRestore = null;
    }}
/>

<ConflictResolverModal bind:show={conflictResolverOpen} />

<style>
    .acc-pane {
        padding: 4px 2px 12px;
        font-family: var(--ui);
    }

    /* ── Page header ──────────────────────────────────────────────── */
    .acc-page-head {
        margin: 4px 0 22px;
    }
    .acc-page-head h1 {
        font-size: 20px;
        font-weight: 600;
        color: var(--t1);
        margin: 0 0 4px;
        letter-spacing: -0.01em;
    }
    .acc-page-head p {
        font-size: 12.5px;
        color: var(--t3);
        margin: 0;
    }

    /* ── Signed-out sign-in card ──────────────────────────────────── */
    .acc-signin {
        max-width: 500px;
        margin: 18px auto 24px;
        padding: 32px 36px 28px;
        text-align: center;
        border: 1px solid var(--b1);
        border-radius: 14px;
        background: linear-gradient(
            180deg,
            rgba(255, 255, 255, 0.025),
            rgba(255, 255, 255, 0.005)
        );
    }
    .acc-signin-icon {
        width: 64px;
        height: 64px;
        margin: 0 auto 18px;
        display: flex;
        align-items: center;
        justify-content: center;
        border-radius: 14px;
        background: linear-gradient(135deg, #2dd4bf, #3b82f6);
        color: #fff;
        box-shadow: 0 8px 22px -6px rgba(59, 130, 246, 0.5);
    }
    .acc-signin h2 {
        font-size: 19px;
        font-weight: 600;
        color: var(--t1);
        margin: 0 0 6px;
        letter-spacing: -0.01em;
    }
    .acc-signin-sub {
        font-size: 13px;
        color: var(--t3);
        line-height: 1.55;
        margin: 0 0 22px;
    }
    .acc-signin-features {
        list-style: none;
        padding: 0;
        margin: 0 auto 24px;
        display: flex;
        flex-direction: column;
        gap: 8px;
        text-align: left;
        max-width: 280px;
        font-size: 12.5px;
        color: var(--t2);
    }
    .acc-signin-features li {
        display: flex;
        align-items: center;
        gap: 10px;
    }
    .acc-bullet {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: var(--acc);
        flex-shrink: 0;
    }
    .acc-signin-buttons {
        display: flex;
        flex-direction: column;
        gap: 8px;
        max-width: 320px;
        margin: 0 auto 14px;
    }
    .acc-oauth-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 10px;
        padding: 11px 16px;
        border-radius: 8px;
        font-family: var(--ui);
        font-size: 13px;
        font-weight: 500;
        cursor: default;
        border: 1px solid var(--b1);
        background: var(--surface-hover);
        color: var(--t1);
        transition:
            background 0.14s,
            border-color 0.14s,
            opacity 0.14s,
            transform 0.08s;
        min-height: 42px;
    }
    .acc-oauth-btn:active:not(:disabled) {
        transform: translateY(1px);
    }
    .acc-oauth-btn:hover:not(:disabled) {
        background: var(--surface-hover);
        border-color: var(--b2);
    }
    .acc-oauth-btn:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    .acc-waiting {
        display: flex;
        align-items: center;
        gap: 10px;
        margin: 4px auto 0;
        padding: 9px 14px;
        border-radius: 8px;
        background: var(--surface-hover);
        border: 1px solid var(--b1);
        color: var(--t2);
        font-size: 12px;
        max-width: 320px;
    }
    .acc-waiting-text {
        flex: 1;
        text-align: left;
        line-height: 1.4;
    }
    .acc-waiting-cancel {
        border: none;
        background: transparent;
        cursor: default;
        color: var(--acc);
        font-size: 12px;
        font-weight: 500;
        padding: 2px 6px;
        border-radius: 4px;
        font-family: var(--ui);
    }
    .acc-waiting-cancel:hover {
        background: rgba(29, 200, 128, 0.12);
    }

    .acc-signin-sep {
        margin: 22px 0 14px;
        border: none;
        border-top: 1px solid var(--b1);
    }
    .acc-signin-fine {
        font-size: 11.5px;
        color: var(--t3);
        line-height: 1.55;
        margin: 0;
    }

    /* ── Signed-in stack ──────────────────────────────────────────── */
    .acc-stack {
        display: flex;
        flex-direction: column;
        gap: 14px;
    }
    .acc-card {
        padding: 16px 18px;
        border: 1px solid var(--b1);
        border-radius: 10px;
        background: var(--surface-hover);
    }
    .acc-card-head {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 12px;
        margin-bottom: 16px;
    }
    .acc-card-title {
        font-size: 10.5px;
        text-transform: uppercase;
        letter-spacing: 0.14em;
        font-weight: 600;
        color: var(--t3);
        margin: 0;
        padding-top: 4px;
    }
    .acc-card-title-solo {
        margin-bottom: 12px;
    }
    .acc-danger-title {
        color: #f04444;
    }

    /* Sync controls on Profile card header */
    .acc-sync-controls {
        display: flex;
        align-items: center;
        gap: 8px;
        flex-shrink: 0;
    }
    .acc-sync-status {
        display: flex;
        flex-direction: column;
        align-items: flex-end;
        gap: 1px;
        line-height: 1.25;
        margin-right: 4px;
    }
    .acc-sync-label {
        font-size: 9.5px;
        text-transform: uppercase;
        letter-spacing: 0.1em;
        color: var(--t3);
        font-weight: 500;
    }
    .acc-sync-value {
        font-size: 11.5px;
        color: var(--t2);
        font-variant-numeric: tabular-nums;
    }
    .acc-sync-device {
        font-size: 10px;
        color: var(--t3);
        max-width: 140px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .acc-sync-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 6px 11px;
        font-size: 12px;
        font-weight: 500;
        font-family: var(--ui);
        border-radius: 6px;
        border: 1px solid var(--b1);
        background: var(--surface-hover);
        color: var(--t1);
        cursor: default;
        transition:
            background 0.14s,
            border-color 0.14s,
            opacity 0.14s;
    }
    .acc-sync-btn:hover:not(:disabled) {
        background: var(--surface-hover);
        border-color: var(--b2);
    }
    .acc-sync-btn:disabled {
        opacity: 0.55;
    }
    .acc-sync-btn svg {
        flex-shrink: 0;
    }
    /* "Action Required" variant — accent-tinted, opens the resolver. */
    .acc-sync-btn-action {
        background: color-mix(in srgb, var(--acc) 14%, transparent);
        border-color: color-mix(in srgb, var(--acc) 35%, transparent);
        color: var(--acc);
        font-weight: 600;
    }
    .acc-sync-btn-action:hover {
        background: color-mix(in srgb, var(--acc) 22%, transparent);
        border-color: color-mix(in srgb, var(--acc) 50%, transparent);
    }
    .acc-sync-btn-action svg { stroke: var(--acc); }
    .acc-kebab-btn {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 28px;
        border-radius: 6px;
        border: 1px solid var(--b1);
        background: var(--surface-hover);
        color: var(--t3);
        cursor: default;
        transition:
            background 0.14s,
            color 0.14s,
            border-color 0.14s;
    }
    .acc-kebab-btn:hover {
        color: var(--t1);
        background: var(--surface-hover);
        border-color: var(--b2);
    }

    /* Profile body */
    .acc-profile-row {
        display: flex;
        gap: 16px;
        align-items: center;
        margin-bottom: 22px;
    }
    .acc-avatar {
        width: 64px;
        height: 64px;
        border-radius: 50%;
        flex-shrink: 0;
    }
    .acc-avatar-fallback {
        background: linear-gradient(135deg, #f97316, #ec4899);
        display: flex;
        align-items: center;
        justify-content: center;
        color: #fff;
        font-size: 24px;
        font-weight: 600;
    }
    .acc-profile-text {
        display: flex;
        flex-direction: column;
        gap: 4px;
        flex: 1;
        min-width: 0;
    }
    .acc-profile-name {
        font-size: 17px;
        font-weight: 600;
        color: var(--t1);
        letter-spacing: -0.01em;
        line-height: 1.2;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .acc-profile-email {
        font-size: 12.5px;
        color: var(--t3);
        line-height: 1.2;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .acc-profile-meta {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-top: 6px;
    }
    .acc-plan-pill {
        font-size: 10px;
        padding: 3px 9px;
        border-radius: 999px;
        background: var(--surface-hover);
        color: var(--t2);
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.08em;
    }
    .acc-plan-pill.is-pro {
        background: linear-gradient(120deg, var(--acc), #1dc880);
        color: #fff;
    }
    .acc-handle-pill {
        font-size: 10.5px;
        padding: 3px 9px;
        border-radius: 999px;
        background: rgba(45, 212, 191, 0.1);
        color: #2dd4bf;
        font-family: var(--mono, ui-monospace);
        border: 1px solid rgba(45, 212, 191, 0.18);
    }

    /* Fields */
    .acc-fields {
        display: flex;
        flex-direction: column;
        gap: 12px;
    }
    .acc-field {
        display: flex;
        flex-direction: column;
        gap: 5px;
        flex: 1;
    }
    .acc-field-label {
        font-size: 11px;
        color: var(--t3);
        font-weight: 500;
        letter-spacing: 0.04em;
    }
    .acc-field input {
        padding: 9px 12px;
        font-size: 13px;
        border: 1px solid var(--b1);
        border-radius: 7px;
        background: rgba(0, 0, 0, 0.25);
        color: var(--t1);
        font-family: var(--ui);
    }
    .acc-field input:focus {
        outline: none;
        border-color: var(--acc);
    }
    .acc-field-row {
        display: flex;
        gap: 12px;
    }
    .acc-fields-footer {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        margin-top: 6px;
    }
    .acc-fields-footer .acc-fine {
        margin: 0;
        flex: 1;
        max-width: 380px;
    }

    /* Generic buttons */
    .acc-btn {
        padding: 9px 16px;
        font-size: 12.5px;
        font-weight: 500;
        border-radius: 7px;
        border: 1px solid var(--b1);
        background: transparent;
        color: var(--t1);
        cursor: default;
        transition:
            background 0.14s,
            border-color 0.14s,
            opacity 0.14s;
        font-family: var(--ui);
        display: inline-flex;
        align-items: center;
        gap: 6px;
        white-space: nowrap;
    }
    .acc-btn:hover:not(:disabled) {
        background: var(--surface-hover);
        border-color: var(--b2);
    }
    .acc-btn:disabled {
        opacity: 0.5;
    }
    .acc-btn-primary {
        background: var(--acc);
        border-color: transparent;
        color: #fff;
    }
    .acc-btn-primary:hover:not(:disabled) {
        opacity: 0.92;
        background: var(--acc);
    }
    .acc-btn-ghost {
        border-color: transparent;
        color: var(--t3);
    }
    .acc-btn-ghost:hover:not(:disabled) {
        color: var(--t1);
        background: var(--surface-hover);
    }

    .acc-fine {
        font-size: 11.5px;
        color: var(--t3);
        line-height: 1.55;
        margin: 6px 0 0;
    }
    .acc-fine-quiet {
        margin-top: 10px;
    }

    /* Linked accounts */
    .acc-prov-row {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 12px;
        border-radius: 8px;
        border: 1px solid var(--b1);
        margin-bottom: 8px;
        font-size: 12.5px;
        background: var(--surface-hover);
    }
    .acc-prov-row:last-child {
        margin-bottom: 0;
    }
    .acc-prov-icon {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 36px;
        height: 36px;
        border-radius: 8px;
        flex-shrink: 0;
    }
    .acc-prov-icon.gh {
        background: #1a1c20;
        color: #fff;
    }
    .acc-prov-icon.gg {
        background: #fff;
    }
    .acc-prov-text {
        display: flex;
        flex-direction: column;
        gap: 2px;
        flex: 1;
        min-width: 0;
    }
    .acc-prov-name {
        font-weight: 600;
        color: var(--t1);
        font-size: 13px;
        line-height: 1.2;
    }
    .acc-prov-sub {
        font-size: 11.5px;
        color: var(--t3);
        font-family: var(--mono, ui-monospace);
        line-height: 1.3;
    }
    .acc-prov-empty {
        font-style: italic;
        font-family: var(--ui);
    }
    .acc-prov-primary {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        font-size: 11.5px;
        color: var(--t3);
        font-style: italic;
    }
    .acc-primary-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: #22c55e;
        flex-shrink: 0;
    }
    .acc-mini-btn {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        padding: 5px 12px;
        font-size: 11.5px;
        border-radius: 6px;
        border: 1px solid var(--b1);
        background: var(--surface-hover);
        color: var(--t2);
        cursor: default;
        font-family: var(--ui);
    }
    .acc-mini-btn:hover:not(:disabled) {
        background: var(--surface-hover);
        color: var(--t1);
        border-color: var(--b2);
    }
    .acc-mini-link {
        color: var(--t1);
    }

    /* Subscription card — three horizontal sections separated by hairlines.
       Sits below the existing Profile form. No card background of its own;
       the surrounding profile card already provides the chrome. */
    .acc-sub-card {
        margin-top: 18px;
        padding-top: 16px;
        border-top: 1px solid var(--b1);
        display: flex;
        flex-direction: column;
    }

    /* Row 1 — title + status pill on left, manage button on right. */
    .acc-sub-head {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 16px;
    }
    .acc-sub-head-left {
        flex: 1;
        min-width: 0;
    }
    .acc-sub-title-row {
        display: flex;
        align-items: center;
        gap: 10px;
        margin-bottom: 4px;
    }
    .acc-sub-title {
        font-size: 16px;
        font-weight: 700;
        color: var(--t1);
        letter-spacing: -0.01em;
    }
    .acc-sub-status {
        font-size: 10px;
        font-weight: 600;
        text-transform: capitalize;
        padding: 2px 9px;
        border-radius: 999px;
        background: color-mix(in srgb, #22c55e 14%, transparent);
        color: #22c55e;
        border: 1px solid color-mix(in srgb, #22c55e 30%, transparent);
        line-height: 1.5;
    }
    .acc-sub-status.is-warn {
        background: color-mix(in srgb, #f59e0b 14%, transparent);
        color: #f59e0b;
        border-color: color-mix(in srgb, #f59e0b 35%, transparent);
    }
    /* Lifetime gets a gold/amber accent — visually distinct from "Active"
       green so users see at a glance they're on the one-time tier. */
    .acc-sub-status.is-lifetime {
        background: color-mix(in srgb, #d4a017 14%, transparent);
        color: #f0b429;
        border-color: color-mix(in srgb, #d4a017 35%, transparent);
    }
    .acc-sub-sub {
        font-size: 12px;
        color: var(--t3);
        margin: 0;
        line-height: 1.5;
    }

    /* Manage-subscription button — pinned width so the "Opening…" swap
       doesn't collapse the button and jolt the surrounding card. */
    .acc-btn-manage {
        min-width: 175px;
        justify-content: center;
        flex-shrink: 0;
    }
    .acc-btn-manage:disabled {
        opacity: 0.75;
    }

    .acc-sub-divider {
        height: 1px;
        background: var(--b1);
        margin: 18px 0;
    }

    /* Row 2 — Clauge AI credits */
    .acc-sub-credits {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .acc-sub-credits-row {
        display: flex;
        align-items: baseline;
        justify-content: space-between;
        gap: 16px;
    }
    .acc-sub-credits-label {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        font-size: 13px;
        color: var(--t2);
        font-weight: 500;
    }
    .acc-sub-credits-icon {
        color: var(--acc);
        flex-shrink: 0;
    }
    .acc-sub-credits-val {
        font-size: 12.5px;
        color: var(--t3);
        font-variant-numeric: tabular-nums;
    }
    .acc-sub-credits-val strong {
        color: var(--t1);
        font-weight: 700;
        font-size: 14px;
        margin-right: 1px;
    }
    .acc-sub-bar-wrap {
        height: 6px;
        background: var(--b1);
        border-radius: 100px;
        overflow: hidden;
    }
    .acc-sub-bar {
        height: 100%;
        background: var(--acc);
        border-radius: 100px;
        transition: width 0.3s ease;
    }
    .acc-sub-credits-meta {
        font-size: 11.5px;
        color: var(--t3);
        margin: 0;
        line-height: 1.5;
    }
    .acc-sub-dot {
        opacity: 0.5;
        margin: 0 4px;
    }

    /* Row 3 — meta grid (Plan / Next renewal / Account) */
    .acc-sub-meta {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 16px;
    }
    .acc-sub-meta-col {
        display: flex;
        flex-direction: column;
        gap: 4px;
        min-width: 0;
    }
    .acc-sub-meta-label {
        font-size: 10px;
        font-weight: 600;
        letter-spacing: 0.12em;
        color: var(--t3);
        text-transform: uppercase;
    }
    .acc-sub-meta-val {
        font-size: 15px;
        font-weight: 700;
        color: var(--t1);
        letter-spacing: -0.01em;
        line-height: 1.25;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .acc-sub-meta-sub {
        font-size: 11.5px;
        color: var(--t3);
        line-height: 1.4;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    /* Local snapshots */
    .acc-snap-row {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 10px 12px;
        border-radius: 8px;
        border: 1px solid var(--b1);
        margin-bottom: 8px;
        font-size: 12.5px;
        background: var(--surface-hover);
    }
    .acc-snap-row:last-child {
        margin-bottom: 0;
    }
    .acc-snap-text {
        display: flex;
        flex-direction: column;
        gap: 2px;
        flex: 1;
        min-width: 0;
    }
    .acc-snap-name {
        font-weight: 600;
        color: var(--t1);
        font-size: 13px;
        line-height: 1.2;
    }
    .acc-snap-sub {
        font-size: 11.5px;
        color: var(--t3);
        font-family: var(--mono, ui-monospace);
        line-height: 1.3;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .acc-snap-empty {
        font-size: 11.5px;
        color: var(--t3);
        line-height: 1.55;
        margin: 0;
    }

    /* Cloud version history */
    .acc-hist-kinds {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        margin-bottom: 10px;
    }
    .acc-hist-kind-btn {
        padding: 5px 10px;
        font-size: 11.5px;
        border-radius: 6px;
        border: 1px solid var(--b1);
        background: transparent;
        color: var(--t2);
        cursor: default;
        white-space: nowrap;
        font-family: var(--ui);
    }
    .acc-hist-kind-btn:hover {
        background: var(--surface-hover);
        color: var(--t1);
    }
    .acc-hist-kind-active,
    .acc-hist-kind-active:hover {
        border-color: var(--acc);
        color: var(--t1);
        background: var(--surface-hover);
    }
    .acc-too-large {
        margin-left: 5px;
        padding: 1px 6px;
        font-size: 10px;
        border-radius: 5px;
        border: 1px solid rgba(245, 158, 11, 0.35);
        background: rgba(245, 158, 11, 0.12);
        color: #d97706;
        white-space: nowrap;
    }

    /* Danger zone */
    .acc-card-danger {
        border-color: rgba(240, 68, 68, 0.18);
    }
    .acc-danger-row {
        display: flex;
        gap: 16px;
        align-items: center;
        padding: 12px 0;
        border-top: 1px solid var(--b1);
    }
    .acc-danger-row:first-of-type {
        border-top: none;
        padding-top: 4px;
    }
    .acc-danger-row > div {
        flex: 1;
    }
    .acc-danger-row strong {
        font-size: 13px;
        color: var(--t1);
        font-weight: 600;
    }
    .acc-danger-row p {
        font-size: 12px;
        color: var(--t3);
        line-height: 1.55;
        margin: 4px 0 0;
    }
    .acc-danger-btn {
        padding: 8px 14px;
        font-size: 12px;
        border-radius: 7px;
        border: 1px solid rgba(240, 68, 68, 0.3);
        background: transparent;
        color: #f04444;
        cursor: default;
        white-space: nowrap;
        font-family: var(--ui);
        font-weight: 500;
    }
    .acc-danger-btn:hover:not(:disabled) {
        background: rgba(240, 68, 68, 0.08);
    }
    .acc-danger-strong {
        border-color: #f04444;
    }

    .acc-confirm {
        margin-top: 10px;
        padding: 14px;
        border-radius: 8px;
        border: 1px solid rgba(240, 68, 68, 0.3);
        background: rgba(240, 68, 68, 0.04);
        font-size: 12.5px;
        color: var(--t2);
    }
    .acc-confirm p {
        margin: 0 0 10px;
        line-height: 1.55;
    }
    .acc-confirm p code {
        font-family: var(--mono, ui-monospace);
        color: var(--t1);
    }
    .acc-confirm-input {
        width: 100%;
        box-sizing: border-box;
        padding: 8px 10px;
        border-radius: 4px;
        border: 1px solid var(--b1);
        background: rgba(0, 0, 0, 0.25);
        color: var(--t1);
        font-family: var(--ui);
        font-size: 12.5px;
        margin-bottom: 10px;
    }
    .acc-confirm-row {
        display: flex;
        gap: 8px;
        justify-content: flex-end;
    }
    /* Yellow warning block shown to Pro recurring users in the delete-
       account confirm panel — tells them the subscription is part of
       what gets cancelled. Doesn't apply to free or lifetime users. */
    .acc-confirm-warn {
        margin: 0 0 12px;
        padding: 10px 12px;
        border-radius: 6px;
        border: 1px solid rgba(245, 166, 35, 0.45);
        background: rgba(245, 166, 35, 0.08);
        color: var(--t1);
        font-size: 12.5px;
        line-height: 1.5;
    }
    .acc-confirm-warn strong {
        display: block;
        margin-bottom: 4px;
        color: var(--warn);
    }
    .acc-confirm-warn p {
        margin: 0;
        color: var(--t2);
    }
    /* "Here's exactly what gets deleted" bullet list, replaces the old
       prose-only description so the user can scan it. */
    .acc-confirm-bullets {
        margin: 0 0 12px;
        padding: 0 0 0 18px;
        color: var(--t2);
        font-size: 12.5px;
        line-height: 1.7;
    }
    .acc-confirm-bullets code {
        font-family: var(--mono, ui-monospace);
        color: var(--t1);
        background: rgba(0, 0, 0, 0.25);
        padding: 1px 4px;
        border-radius: 3px;
    }
    /* Live validation hint under the slug input. Red until match, green
       when match. Hidden while the input is empty so the user isn't
       scolded before they've typed anything. */
    .acc-confirm-hint {
        margin: -4px 0 10px;
        font-size: 12px;
        color: var(--err);
    }
    .acc-confirm-hint-ok {
        color: var(--ok);
    }

    /* Spinners */
    .acc-spinner {
        display: inline-block;
        width: 13px;
        height: 13px;
        border-radius: 50%;
        border: 2px solid currentColor;
        border-right-color: transparent;
        animation: acc-spin 0.7s linear infinite;
        flex-shrink: 0;
    }
    .acc-spinner-tiny {
        width: 11px;
        height: 11px;
        border-width: 1.8px;
    }
    .acc-spinner-light {
        color: var(--t1);
    }
    @keyframes acc-spin {
        to {
            transform: rotate(360deg);
        }
    }
</style>

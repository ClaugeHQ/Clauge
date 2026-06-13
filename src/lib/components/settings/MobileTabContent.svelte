<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import QRCode from "qrcode";
    import {
        companionStatus,
        companionStart,
        companionStop,
        companionNewPairCode,
        companionListDevices,
        companionRevokeDevice,
        companionDeleteDevice,
        companionPurgeRevoked,
        type CompanionStatus,
        type PairCodeInfo,
        type CompanionDevice,
    } from "$lib/commands/companion";
    import ConfirmDialog from "$lib/shared/primitives/ConfirmDialog.svelte";
    import { showToast } from "$lib/shared/primitives/toast";
    import { friendlyError } from "$lib/utils/errors";

    const ANDROID_RELEASES_URL =
        "https://github.com/ClaugeHQ/clauge-android/releases/latest";
    const IOS_REPO_URL = "https://github.com/ClaugeHQ/clauge-ios";

    let status = $state<CompanionStatus>({ running: false, port: null });
    let toggling = $state(false);
    let devices = $state<CompanionDevice[]>([]);
    let androidQrDataUrl = $state("");

    // Pairing flow state.
    let pairInfo = $state<PairCodeInfo | null>(null);
    let qrDataUrl = $state("");
    let generating = $state(false);
    // 2-min countdown mirroring the server-side code TTL.
    const PAIR_TTL_SECONDS = 120;
    let secondsLeft = $state(0);
    let countdownTimer: ReturnType<typeof setInterval> | null = null;

    let hosts = $derived(pairInfo?.hosts ?? []);
    let primaryHost = $derived(hosts[0] ?? null);

    // Revoke confirm.
    let showRevokeConfirm = $state(false);
    let revokeTarget = $state<CompanionDevice | null>(null);

    // Remove (hard-delete) confirm.
    let showRemoveConfirm = $state(false);
    let removeTarget = $state<CompanionDevice | null>(null);

    // Clear-revoked confirm.
    let showClearConfirm = $state(false);

    let hasRevoked = $derived(devices.some((d) => d.revoked));

    async function refreshStatus() {
        try {
            status = await companionStatus();
        } catch (e) {
            console.warn("[companion] status failed:", e);
        }
    }

    async function refreshDevices() {
        try {
            devices = await companionListDevices();
        } catch (e) {
            console.warn("[companion] list devices failed:", e);
        }
    }

    async function toggleServer() {
        if (toggling) return;
        toggling = true;
        try {
            status = status.running
                ? await companionStop()
                : await companionStart();
            if (!status.running) {
                clearPairing();
            }
            await refreshDevices();
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            toggling = false;
        }
    }

    function clearPairing() {
        pairInfo = null;
        qrDataUrl = "";
        secondsLeft = 0;
        if (countdownTimer) {
            clearInterval(countdownTimer);
            countdownTimer = null;
        }
    }

    async function generatePairCode() {
        if (generating || !status.running) return;
        generating = true;
        try {
            pairInfo = await companionNewPairCode();
            // The QR encodes exactly what the phone needs to dial home.
            const payload = JSON.stringify({
                v: 1,
                hosts: pairInfo.hosts,
                port: pairInfo.port,
                code: pairInfo.code,
            });
            qrDataUrl = await QRCode.toDataURL(payload, {
                margin: 1,
                width: 220,
                color: { dark: "#0b0a16", light: "#ffffff" },
            });
            startCountdown();
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            generating = false;
        }
    }

    function startCountdown() {
        secondsLeft = PAIR_TTL_SECONDS;
        if (countdownTimer) clearInterval(countdownTimer);
        countdownTimer = setInterval(() => {
            secondsLeft -= 1;
            if (secondsLeft <= 0) {
                // Code expired server-side — drop the stale QR so nobody
                // scans a dead code.
                clearPairing();
            }
        }, 1000);
    }

    function countdownLabel(s: number): string {
        const m = Math.floor(s / 60);
        const sec = (s % 60).toString().padStart(2, "0");
        return `${m}:${sec}`;
    }

    function askRevoke(d: CompanionDevice) {
        revokeTarget = d;
        showRevokeConfirm = true;
    }

    async function confirmRevoke() {
        if (!revokeTarget) return;
        try {
            await companionRevokeDevice(revokeTarget.id);
            await refreshDevices();
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            revokeTarget = null;
        }
    }

    function askRemove(d: CompanionDevice) {
        removeTarget = d;
        showRemoveConfirm = true;
    }

    async function confirmRemove() {
        if (!removeTarget) return;
        try {
            await companionDeleteDevice(removeTarget.id);
            await refreshDevices();
        } catch (e) {
            showToast(friendlyError(e), "error");
        } finally {
            removeTarget = null;
        }
    }

    async function confirmClearRevoked() {
        try {
            const n = await companionPurgeRevoked();
            await refreshDevices();
            showToast(
                `Removed ${n} device${n === 1 ? "" : "s"}`,
                "success",
            );
        } catch (e) {
            showToast(friendlyError(e), "error");
        }
    }

    function relativeTime(iso: string | null): string {
        if (!iso) return "never";
        const then = new Date(iso.includes("T") ? iso : iso + "Z").getTime();
        if (Number.isNaN(then)) return iso;
        const diff = Date.now() - then;
        const min = Math.floor(diff / 60000);
        if (min < 1) return "just now";
        if (min < 60) return `${min}m ago`;
        const hr = Math.floor(min / 60);
        if (hr < 24) return `${hr}h ago`;
        const d = Math.floor(hr / 24);
        return `${d}d ago`;
    }

    async function openExternal(url: string) {
        try {
            const { openUrl } = await import("@tauri-apps/plugin-opener");
            await openUrl(url);
        } catch (e) {
            showToast(friendlyError(e), "error");
        }
    }

    onMount(async () => {
        await refreshStatus();
        await refreshDevices();
        try {
            androidQrDataUrl = await QRCode.toDataURL(ANDROID_RELEASES_URL, {
                margin: 1,
                width: 160,
                color: { dark: "#0b0a16", light: "#ffffff" },
            });
        } catch (e) {
            console.warn("[companion] android QR failed:", e);
        }
    });

    onDestroy(() => {
        if (countdownTimer) clearInterval(countdownTimer);
    });
</script>

<div class="stg-card-stack">
    <!-- Get the app -->
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
                    <rect x="5" y="2" width="14" height="20" rx="2" />
                    <line x1="12" y1="18" x2="12" y2="18" />
                </svg>
            </span>
            <div class="stg-card-titles">
                <h3 class="stg-card-title">Get the app</h3>
                <p class="stg-card-sub">
                    Drive your desktop sessions from your phone.
                </p>
            </div>
        </header>
        <div class="stg-card-body">
            <div class="get-app">
                {#if androidQrDataUrl}
                    <img
                        class="get-app-qr"
                        src={androidQrDataUrl}
                        alt="Android releases QR"
                    />
                {/if}
                <div class="get-app-main">
                    <button
                        class="stg-btn primary"
                        onclick={() => openExternal(ANDROID_RELEASES_URL)}
                    >
                        Download for Android
                    </button>
                    <button
                        class="stg-btn"
                        onclick={() => openExternal(IOS_REPO_URL)}
                    >
                        Download for iOS
                    </button>
                </div>
            </div>
        </div>
    </section>

    <!-- Server toggle -->
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
                    <rect x="2" y="3" width="20" height="14" rx="2" />
                    <line x1="8" y1="21" x2="16" y2="21" />
                    <line x1="12" y1="17" x2="12" y2="21" />
                </svg>
            </span>
            <div class="stg-card-titles">
                <h3 class="stg-card-title">Companion server</h3>
                <p class="stg-card-sub">
                    Mirror your terminals to the Clauge mobile app over your
                    local network or tailnet.
                </p>
            </div>
        </header>
        <div class="stg-card-body">
            <div class="stg-card-row">
                <span class="stg-card-row-label">Enable server</span>
                <label class="stg-toggle">
                    <input
                        type="checkbox"
                        checked={status.running}
                        disabled={toggling}
                        onchange={toggleServer}
                        aria-label="Toggle companion server"
                    />
                    <span class="stg-toggle-slider"></span>
                </label>
            </div>
            {#if status.running && status.port != null && primaryHost}
                <div class="stg-card-row">
                    <span class="stg-card-row-label">Running at</span>
                    <span class="hostline">
                        <span class="dot on"></span>
                        <code class="host">{primaryHost}:{status.port}</code>
                    </span>
                </div>
            {/if}
        </div>
    </section>

    <!-- Pairing -->
    {#if status.running}
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
                        <rect x="3" y="3" width="7" height="7" rx="1" />
                        <rect x="14" y="3" width="7" height="7" rx="1" />
                        <rect x="3" y="14" width="7" height="7" rx="1" />
                        <line x1="14" y1="14" x2="21" y2="14" />
                        <line x1="14" y1="21" x2="21" y2="21" />
                    </svg>
                </span>
                <div class="stg-card-titles">
                    <h3 class="stg-card-title">Add device</h3>
                    <p class="stg-card-sub">
                        {#if pairInfo}
                            Scan the QR in the Clauge mobile app, then approve
                            the request here.
                        {:else}
                            Generate a one-time code, then scan it in the
                            Clauge mobile app. The code expires after two
                            minutes.
                        {/if}
                    </p>
                </div>
                <button
                    class="stg-btn"
                    disabled={generating}
                    onclick={generatePairCode}
                >
                    {pairInfo ? "Regenerate" : "Generate code"}
                </button>
            </header>

            {#if pairInfo}
                <div class="stg-card-body">
                    <div class="pair">
                        {#if qrDataUrl}
                            <img class="pair-qr" src={qrDataUrl} alt="Pairing QR" />
                        {/if}
                        <div class="pair-meta">
                            <span class="pair-label">Pairing code</span>
                            <span class="pair-code">{pairInfo.code}</span>
                            <span class="pair-label">Expires in</span>
                            <span
                                class="pair-countdown"
                                class:warn={secondsLeft <= 30}
                                >{countdownLabel(secondsLeft)}</span
                            >
                            {#if hosts.length > 0}
                                <span class="pair-label">Reachable at</span>
                                <span class="pair-hosts">
                                    {#each hosts as h}
                                        <code>{h}:{pairInfo.port}</code>
                                    {/each}
                                </span>
                            {/if}
                        </div>
                    </div>
                </div>
            {/if}
        </section>
    {/if}

    <!-- Paired devices -->
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
                    <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" />
                    <circle cx="9" cy="7" r="4" />
                    <path d="M23 21v-2a4 4 0 0 0-3-3.87" />
                </svg>
            </span>
            <div class="stg-card-titles">
                <h3 class="stg-card-title">Paired devices</h3>
                <p class="stg-card-sub">
                    Phones allowed to connect to this desktop.
                </p>
            </div>
            {#if hasRevoked}
                <button
                    class="stg-btn danger"
                    onclick={() => (showClearConfirm = true)}
                    >Clear revoked</button
                >
            {/if}
        </header>
        <div class="stg-card-body">
            {#if devices.length === 0}
                <p class="empty">No devices paired yet.</p>
            {:else}
                <ul class="devices">
                    {#each devices as d (d.id)}
                        <li class="device" class:revoked={d.revoked}>
                            <div class="device-info">
                                <span class="device-name">{d.name}</span>
                                <span class="device-meta">
                                    {d.platform} · last seen {relativeTime(
                                        d.lastSeenAt,
                                    )}
                                    {#if d.revoked}· revoked{/if}
                                </span>
                            </div>
                            {#if d.revoked}
                                <button
                                    class="stg-btn danger"
                                    onclick={() => askRemove(d)}>Remove</button
                                >
                            {:else}
                                <button
                                    class="stg-btn danger"
                                    onclick={() => askRevoke(d)}>Revoke</button
                                >
                            {/if}
                        </li>
                    {/each}
                </ul>
            {/if}
        </div>
    </section>
</div>

<ConfirmDialog
    bind:show={showRevokeConfirm}
    title="Revoke device"
    message={`Revoke "${revokeTarget?.name ?? ""}"? It will no longer be able to connect until re-paired.`}
    confirmText="Revoke"
    onconfirm={confirmRevoke}
    oncancel={() => (revokeTarget = null)}
/>

<ConfirmDialog
    bind:show={showRemoveConfirm}
    title="Remove device"
    message={`Permanently remove "${removeTarget?.name ?? ""}" from this list? This cannot be undone.`}
    confirmText="Remove"
    onconfirm={confirmRemove}
    oncancel={() => (removeTarget = null)}
/>

<ConfirmDialog
    bind:show={showClearConfirm}
    title="Clear revoked devices"
    message="Permanently remove all revoked devices from this list? This cannot be undone."
    confirmText="Clear revoked"
    onconfirm={confirmClearRevoked}
/>

<style>
    /* ------- Shared settings card language ------- */
    /* Mirrors the .stg-card / .stg-toggle / .stg-btn styles used across the
       other settings tabs. Redefined locally because those rules are scoped
       to SettingsModal.svelte and don't reach this child component. */

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

    .stg-card-hd {
        display: flex;
        align-items: flex-start;
        gap: 12px;
        padding: 14px 16px 12px;
    }
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
    }

    .stg-card-sub {
        margin: 0;
        font-size: 11.5px;
        line-height: 1.5;
        color: var(--t3);
        font-family: var(--ui);
        max-width: 380px;
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

    /* -- Toggle switch (shared) -- */
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
        inset: 0;
        background: var(--b1);
        border-radius: 10px;
        transition: background 0.2s;
    }
    .stg-toggle-slider::after {
        content: "";
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
    }
    .stg-toggle input:disabled + .stg-toggle-slider {
        opacity: 0.5;
    }

    /* -- Buttons (shared action-button language) -- */
    .stg-btn {
        padding: 7px 16px;
        border-radius: var(--radius-md);
        border: 1px solid var(--b1);
        background: var(--surface-hover);
        color: var(--t2);
        font-family: var(--ui);
        font-size: 12px;
        font-weight: 500;
        cursor: default;
        transition: border-color 0.12s, color 0.12s, background 0.12s,
            opacity 0.12s;
    }
    .stg-btn:hover:not(:disabled) {
        border-color: var(--b2);
        color: var(--t1);
    }
    .stg-btn:disabled {
        opacity: 0.4;
        cursor: default;
    }
    .stg-btn.primary {
        background: var(--acc);
        color: #fff;
        border-color: var(--acc);
    }
    .stg-btn.primary:hover:not(:disabled) {
        opacity: 0.85;
        color: #fff;
    }
    .stg-btn.danger {
        color: var(--t3);
    }
    .stg-btn.danger:hover:not(:disabled) {
        color: var(--err);
        border-color: var(--err);
    }

    /* ------- Get the app ------- */
    .get-app {
        display: flex;
        align-items: center;
        gap: 16px;
        padding-top: 6px;
    }
    .get-app-qr {
        width: 96px;
        height: 96px;
        border-radius: 8px;
        background: #fff;
        padding: 6px;
        flex-shrink: 0;
    }
    .get-app-main {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 8px;
    }

    /* ------- Server host line ------- */
    .hostline {
        display: flex;
        align-items: center;
        gap: 8px;
    }
    .dot {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--t3);
    }
    .dot.on {
        background: #34d399;
    }
    .host {
        font-family: var(--mono);
        font-size: 12px;
        color: var(--t2);
    }

    /* ------- Pairing ------- */
    .pair {
        display: flex;
        gap: 18px;
        align-items: center;
        padding-top: 6px;
    }
    .pair-qr {
        width: 140px;
        height: 140px;
        border-radius: 10px;
        background: #fff;
        padding: 6px;
        flex-shrink: 0;
    }
    .pair-meta {
        display: grid;
        grid-template-columns: auto 1fr;
        gap: 4px 14px;
        align-items: baseline;
    }
    .pair-label {
        font-size: 10px;
        color: var(--t3);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        font-family: var(--ui);
        font-weight: 600;
    }
    .pair-code {
        font-family: var(--mono);
        font-size: 20px;
        font-weight: 600;
        letter-spacing: 0.18em;
        color: var(--acc);
    }
    .pair-countdown {
        font-family: var(--mono);
        font-size: 13px;
        color: var(--t2);
    }
    .pair-countdown.warn {
        color: var(--err);
    }
    .pair-hosts {
        display: flex;
        flex-direction: column;
        gap: 2px;
    }
    .pair-hosts code {
        font-family: var(--mono);
        font-size: 11px;
        color: var(--t2);
    }

    /* ------- Paired devices ------- */
    .empty {
        margin: 6px 0 0;
        font-size: 12px;
        color: var(--t3);
        font-family: var(--ui);
        line-height: 1.5;
    }
    .devices {
        list-style: none;
        margin: 6px 0 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .device {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        padding: 10px 12px;
        border: 1px solid var(--b1);
        border-radius: 8px;
        background: var(--surface-hover);
    }
    .device.revoked {
        opacity: 0.5;
    }
    .device-info {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
    }
    .device-name {
        font-size: 12.5px;
        color: var(--t1);
        font-weight: 600;
        font-family: var(--ui);
    }
    .device-meta {
        font-size: 11px;
        color: var(--t3);
        font-family: var(--ui);
    }
</style>

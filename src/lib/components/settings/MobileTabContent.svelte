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

    let status = $state<CompanionStatus>({ running: false, port: null });
    let toggling = $state(false);
    let devices = $state<CompanionDevice[]>([]);

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

    onMount(async () => {
        await refreshStatus();
        await refreshDevices();
    });

    onDestroy(() => {
        if (countdownTimer) clearInterval(countdownTimer);
    });
</script>

<div class="stg-card-stack">
    <!-- Server toggle -->
    <section class="stg-card">
        <div class="mob-row">
            <div class="mob-row-text">
                <span class="mob-title">Companion server</span>
                <span class="mob-sub">
                    Mirror your terminals to the Clauge mobile app over your
                    local network or tailnet.
                </span>
            </div>
            <button
                class="mob-switch"
                class:on={status.running}
                disabled={toggling}
                onclick={toggleServer}
                aria-label="Toggle companion server"
            >
                <span class="mob-knob"></span>
            </button>
        </div>
        {#if status.running && status.port != null}
            <div class="mob-hostline">
                <span class="mob-dot on"></span>
                <span class="mob-host"
                    >{primaryHost ?? "0.0.0.0"}:{status.port}</span
                >
            </div>
        {/if}
    </section>

    <!-- Pairing -->
    {#if status.running}
        <section class="stg-card">
            <div class="mob-section-hdr">
                <span class="mob-title">Add device</span>
                <button
                    class="mob-btn"
                    disabled={generating}
                    onclick={generatePairCode}
                >
                    {pairInfo ? "Regenerate" : "Generate code"}
                </button>
            </div>

            {#if pairInfo}
                <div class="mob-pair">
                    {#if qrDataUrl}
                        <img class="mob-qr" src={qrDataUrl} alt="Pairing QR" />
                    {/if}
                    <div class="mob-pair-meta">
                        <span class="mob-pair-label">Pairing code</span>
                        <span class="mob-code">{pairInfo.code}</span>
                        <span class="mob-pair-label">Expires in</span>
                        <span
                            class="mob-countdown"
                            class:warn={secondsLeft <= 30}
                            >{countdownLabel(secondsLeft)}</span
                        >
                        {#if hosts.length > 0}
                            <span class="mob-pair-label">Reachable at</span>
                            <span class="mob-hosts">
                                {#each hosts as h}
                                    <code>{h}:{pairInfo.port}</code>
                                {/each}
                            </span>
                        {/if}
                    </div>
                </div>
                <p class="mob-hint">
                    Scan the QR in the Clauge mobile app, then approve the
                    request here.
                </p>
            {:else}
                <p class="mob-hint">
                    Generate a one-time code, then scan it in the Clauge mobile
                    app to pair a phone. The code expires after two minutes.
                </p>
            {/if}
        </section>
    {/if}

    <!-- Paired devices -->
    <section class="stg-card">
        <div class="mob-section-hdr">
            <span class="mob-title">Paired devices</span>
            {#if hasRevoked}
                <button
                    class="mob-btn danger"
                    onclick={() => (showClearConfirm = true)}
                    >Clear revoked</button
                >
            {/if}
        </div>
        {#if devices.length === 0}
            <p class="mob-hint">No devices paired yet.</p>
        {:else}
            <ul class="mob-devices">
                {#each devices as d (d.id)}
                    <li class="mob-device" class:revoked={d.revoked}>
                        <div class="mob-device-info">
                            <span class="mob-device-name">{d.name}</span>
                            <span class="mob-device-meta">
                                {d.platform} · last seen {relativeTime(
                                    d.lastSeenAt,
                                )}
                                {#if d.revoked}· revoked{/if}
                            </span>
                        </div>
                        {#if d.revoked}
                            <button
                                class="mob-btn danger"
                                onclick={() => askRemove(d)}>Remove</button
                            >
                        {:else}
                            <button
                                class="mob-btn danger"
                                onclick={() => askRevoke(d)}>Revoke</button
                            >
                        {/if}
                    </li>
                {/each}
            </ul>
        {/if}
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
    .mob-row {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 16px;
    }

    .mob-row-text {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .mob-title {
        font-size: 13px;
        font-weight: 600;
        color: var(--t1);
        font-family: var(--ui);
    }

    .mob-sub {
        font-size: 12px;
        color: var(--t3);
        line-height: 1.5;
        max-width: 380px;
    }

    .mob-switch {
        flex-shrink: 0;
        width: 40px;
        height: 22px;
        border-radius: 999px;
        border: 1px solid var(--b1);
        background: var(--e);
        position: relative;
        cursor: pointer;
        transition: background 0.15s, border-color 0.15s;
        padding: 0;
    }

    .mob-switch.on {
        background: var(--accent);
        border-color: var(--accent);
    }

    .mob-switch:disabled {
        opacity: 0.5;
        cursor: default;
    }

    .mob-knob {
        position: absolute;
        top: 2px;
        left: 2px;
        width: 16px;
        height: 16px;
        border-radius: 50%;
        background: #fff;
        transition: transform 0.15s;
    }

    .mob-switch.on .mob-knob {
        transform: translateX(18px);
    }

    .mob-hostline {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-top: 12px;
        padding-top: 12px;
        border-top: 1px solid var(--b1);
    }

    .mob-dot {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--t3);
    }

    .mob-dot.on {
        background: #34d399;
    }

    .mob-host {
        font-family: var(--mono, monospace);
        font-size: 12px;
        color: var(--t2);
    }

    .mob-section-hdr {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 12px;
    }

    .mob-btn {
        height: 28px;
        padding: 0 14px;
        border-radius: 8px;
        border: 1px solid var(--b1);
        background: transparent;
        color: var(--t1);
        font-size: 12px;
        font-family: var(--ui);
        cursor: pointer;
        transition: border-color 0.12s, color 0.12s;
    }

    .mob-btn:hover:not(:disabled) {
        border-color: var(--b2);
    }

    .mob-btn:disabled {
        opacity: 0.5;
        cursor: default;
    }

    .mob-btn.danger {
        color: var(--err);
        border-color: color-mix(in srgb, var(--err) 35%, transparent);
    }

    .mob-btn.danger:hover {
        border-color: var(--err);
    }

    .mob-pair {
        display: flex;
        gap: 18px;
        align-items: center;
    }

    .mob-qr {
        width: 140px;
        height: 140px;
        border-radius: 10px;
        background: #fff;
        padding: 6px;
        flex-shrink: 0;
    }

    .mob-pair-meta {
        display: grid;
        grid-template-columns: auto 1fr;
        gap: 4px 14px;
        align-items: baseline;
    }

    .mob-pair-label {
        font-size: 11px;
        color: var(--t3);
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }

    .mob-code {
        font-family: var(--mono, monospace);
        font-size: 20px;
        font-weight: 600;
        letter-spacing: 0.18em;
        color: var(--accent);
    }

    .mob-countdown {
        font-family: var(--mono, monospace);
        font-size: 13px;
        color: var(--t2);
    }

    .mob-countdown.warn {
        color: var(--err);
    }

    .mob-hosts {
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .mob-hosts code {
        font-family: var(--mono, monospace);
        font-size: 11px;
        color: var(--t2);
    }

    .mob-hint {
        margin: 12px 0 0;
        font-size: 12px;
        color: var(--t3);
        line-height: 1.5;
    }

    .mob-devices {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 8px;
    }

    .mob-device {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        padding: 10px 12px;
        border: 1px solid var(--b1);
        border-radius: 10px;
        background: var(--e);
    }

    .mob-device.revoked {
        opacity: 0.5;
    }

    .mob-device-info {
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .mob-device-name {
        font-size: 13px;
        color: var(--t1);
        font-weight: 500;
    }

    .mob-device-meta {
        font-size: 11px;
        color: var(--t3);
    }
</style>

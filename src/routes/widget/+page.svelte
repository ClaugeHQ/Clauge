<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import { listen, emit, type UnlistenFn } from "@tauri-apps/api/event";
    import { getCurrentWindow } from "@tauri-apps/api/window";
    import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
    import { LogicalSize, LogicalPosition } from "@tauri-apps/api/dpi";
    import { MEETING_EVENT } from "$lib/shared/constants/events";
    import { MEETING_MODEL_MISSING } from "$lib/modes/workspace/commands";

    type DetectStatus = {
        enabled: boolean;
        app: string | null;
        active: boolean;
    };
    type RecordingStatus = {
        recording: boolean;
        stopping: boolean;
        meetingId: string | null;
        startedAt: string | null;
        sourceApp: string | null;
        systemAudio: boolean;
        elapsedSecs: number;
    };

    const WINDOW_WIDTH = 480;
    const COLLAPSED_HEIGHT = 80;
    const EXPANDED_HEIGHT = 236;
    // Compact recording pill — sized to hug its content (grip · dot ·
    // timer · stop) with no dead space. ~130px pill + 10px shadow margin
    // each side.
    const COMPACT_WIDTH = 150;
    const COMPACT_HEIGHT = 64;
    // Wider pill while the stop-failed error line is showing.
    const COMPACT_ERROR_WIDTH = 240;

    let phase = $state<"prompt" | "starting" | "model-missing" | "recording">(
        "prompt",
    );
    let sourceApp = $state<string | null>(null);
    let stopping = $state(false);
    let stopError = $state(false);
    let micOnly = $state(false);
    let menuOpen = $state(false);
    let errorMsg = $state<string | null>(null);
    let startedAtMs = $state(0);
    let nowMs = $state(Date.now());
    let unlistens: UnlistenFn[] = [];

    function cap(s: string): string {
        return s.charAt(0).toUpperCase() + s.slice(1);
    }

    function formatElapsed(total: number): string {
        const h = Math.floor(total / 3600);
        const m = Math.floor((total % 3600) / 60);
        const s = total % 60;
        const mm = String(m).padStart(2, "0");
        const ss = String(s).padStart(2, "0");
        return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
    }

    const elapsedLabel = $derived(
        formatElapsed(Math.max(0, Math.floor((nowMs - startedAtMs) / 1000))),
    );
    const promptSubtitle = $derived(
        phase === "model-missing"
            ? "Download a transcription model in Settings first"
            : (errorMsg ??
                  (sourceApp ? `${cap(sourceApp)} call detected` : "Call detected")),
    );
    $effect(() => {
        if (phase !== "recording") return;
        const id = setInterval(() => (nowMs = Date.now()), 1000);
        return () => clearInterval(id);
    });

    // The window is only as tall as the pill; the dropdown needs room, so
    // grow the (transparent) window while the menu is open. Recording
    // shrinks to the compact pill instead and stays there — the window
    // closes when the recording ends.
    let compactApplied = false;
    let preCompactPos: LogicalPosition | null = null;
    $effect(() => {
        if (phase === "recording") {
            const width = stopError ? COMPACT_ERROR_WIDTH : COMPACT_WIDTH;
            if (!compactApplied) {
                compactApplied = true;
                void enterCompact(width);
            } else {
                getCurrentWindow()
                    .setSize(new LogicalSize(width, COMPACT_HEIGHT))
                    .catch(() => {});
            }
            return;
        }
        compactApplied = false;
        // Undo the compact recentering: "close" only hides this window, so
        // a leftover offset would shift it further right every recording
        // cycle until it reappears off-screen.
        if (preCompactPos) {
            getCurrentWindow()
                .setPosition(preCompactPos)
                .catch(() => {});
            preCompactPos = null;
        }
        const height = menuOpen ? EXPANDED_HEIGHT : COLLAPSED_HEIGHT;
        getCurrentWindow()
            .setSize(new LogicalSize(WINDOW_WIDTH, height))
            .catch(() => {});
    });

    async function enterCompact(width: number) {
        const win = getCurrentWindow();
        try {
            const [pos, size, factor] = await Promise.all([
                win.outerPosition(),
                win.outerSize(),
                win.scaleFactor(),
            ]);
            const oldPos = pos.toLogical(factor);
            const oldWidth = size.toLogical(factor).width;
            preCompactPos = new LogicalPosition(oldPos.x, oldPos.y);
            await win.setSize(new LogicalSize(width, COMPACT_HEIGHT));
            await win.setPosition(
                new LogicalPosition(
                    oldPos.x + (oldWidth - width) / 2,
                    oldPos.y,
                ),
            );
        } catch {
            /* full-size widget still works */
        }
    }

    // Self-heal: while showing the recording pill, poll the backend; if
    // the recording ended without the stopped/error event reaching this
    // window (the zombie-widget case), close instead of staying stuck.
    $effect(() => {
        if (phase !== "recording") return;
        const id = setInterval(() => void refreshRecordingStatus(), 3000);
        return () => clearInterval(id);
    });

    // Escape closes the menu. No arrow-key navigation: the menu items are
    // plain focusable buttons, so Tab already cycles through all three.
    $effect(() => {
        if (!menuOpen) return;
        const onKeydown = (e: KeyboardEvent) => {
            if (e.key === "Escape") menuOpen = false;
        };
        window.addEventListener("keydown", onKeydown);
        return () => window.removeEventListener("keydown", onKeydown);
    });

    function applyRecording(rec: RecordingStatus) {
        sourceApp = rec.sourceApp ?? sourceApp;
        startedAtMs = Date.now() - rec.elapsedSecs * 1000;
        nowMs = Date.now();
        // Never downgrade a locally in-flight stop: the backend flips its
        // stopping flag a beat after the invoke leaves this window.
        stopping = stopping || rec.stopping;
        micOnly = !rec.systemAudio;
        menuOpen = false;
        phase = "recording";
    }

    /** Close after a recording finished. "Close" only hides this window
     *  (the app-wide close handler prevents destruction), so also reset
     *  to the idle prompt: a hidden window stuck in the recording phase
     *  keeps the self-heal poller alive, and its queued close() can land
     *  right after the next open_widget show() — hiding the freshly
     *  reopened prompt before the user ever sees it. */
    function closeFinished() {
        getCurrentWindow().close().catch(() => {});
        stopping = false;
        stopError = false;
        menuOpen = false;
        phase = "prompt";
    }

    async function refreshRecordingStatus() {
        try {
            const rec = await invoke<RecordingStatus>(
                "workspace_meeting_recording_status",
            );
            if (rec.recording || rec.stopping) {
                applyRecording(rec);
            } else if (phase === "recording") {
                // The recording ended but no stopped/error event reached
                // this window — close instead of showing a dead pill.
                closeFinished();
            }
        } catch {
            /* status refresh is best-effort */
        }
    }

    async function startTranscribing() {
        if (phase === "starting") return;
        menuOpen = false;
        errorMsg = null;
        phase = "starting";
        try {
            await invoke("workspace_meeting_start", { sourceApp });
        } catch (e) {
            if (String(e) === MEETING_MODEL_MISSING) {
                phase = "model-missing";
            } else {
                errorMsg = String(e);
                phase = "prompt";
            }
        }
    }

    async function stopRecording() {
        if (stopping) return;
        stopping = true;
        stopError = false;
        try {
            await invoke("workspace_meeting_stop");
            // Stop is accepted: the backend keeps draining the final
            // chunks and flushing them to the DB on its own. The stopped
            // event can lag minutes behind that drain, so close now —
            // the in-app surfaces track the rest.
            closeFinished();
        } catch {
            stopping = false;
            stopError = true;
        }
    }

    async function dismiss() {
        menuOpen = false;
        try {
            await invoke("workspace_meeting_detect_dismiss");
        } catch {
            /* closing anyway */
        }
        getCurrentWindow().close().catch(() => {});
    }

    /** Recording-phase escape hatch: hides only this window. The
     *  recording continues in the backend and the in-app surfaces
     *  (nav, statusbar, meeting view) still control it — so no
     *  detect_dismiss here. */
    function hideWidget() {
        getCurrentWindow().close().catch(() => {});
    }

    async function disableDetect() {
        menuOpen = false;
        try {
            await invoke("workspace_meeting_detect_set_enabled", {
                enabled: false,
            });
            // The main window owns the confirmation toast — this window
            // is about to close.
            await emit(MEETING_EVENT.DETECT_DISABLED);
        } catch {
            /* closing anyway */
        }
        getCurrentWindow().close().catch(() => {});
    }

    async function openSettings() {
        menuOpen = false;
        try {
            await emit(MEETING_EVENT.OPEN_SETTINGS);
            const main = await WebviewWindow.getByLabel("main");
            if (main) {
                await main.unminimize();
                await main.show();
                await main.setFocus();
            }
        } catch (e) {
            console.warn("[meeting-widget] focusing main window failed:", e);
        }
    }

    onMount(async () => {
        try {
            await setup();
        } catch (e) {
            // Forwarded to the app log file by the console forwarder — a
            // silent mount failure here is an invisible (transparent)
            // window with no way to debug it.
            console.error("[meeting-widget] mount failed:", e);
        }
    });

    async function setup() {
        unlistens = await Promise.all([
            listen<{ app: string }>(MEETING_EVENT.CALL_DETECTED, async (e) => {
                if (phase === "recording") {
                    // A fresh call while the pill is still up usually means
                    // the previous recording ended without us hearing the
                    // stopped event — re-check instead of ignoring. A live
                    // recording keeps the pill.
                    try {
                        const rec = await invoke<RecordingStatus>(
                            "workspace_meeting_recording_status",
                        );
                        if (rec.recording || rec.stopping) return;
                    } catch {
                        return;
                    }
                    stopping = false;
                    stopError = false;
                }
                sourceApp = e.payload.app;
                errorMsg = null;
                phase = "prompt";
            }),
            listen<{
                meetingId: string;
                startedAt: string;
                sourceApp: string | null;
                systemAudio: boolean;
            }>(MEETING_EVENT.RECORDING_STARTED, (e) => {
                sourceApp = e.payload.sourceApp ?? sourceApp;
                startedAtMs = Date.parse(e.payload.startedAt) || Date.now();
                nowMs = Date.now();
                stopping = false;
                stopError = false;
                micOnly = !e.payload.systemAudio;
                menuOpen = false;
                phase = "recording";
            }),
            listen(MEETING_EVENT.RECORDING_WARNING, () => {
                void refreshRecordingStatus();
            }),
            listen(MEETING_EVENT.RECORDING_STOPPED, () => {
                if (phase !== "recording") return;
                closeFinished();
            }),
            listen(MEETING_EVENT.RECORDING_ERROR, () => {
                if (phase !== "recording") return;
                closeFinished();
            }),
        ]);

        // The window may open before events arrive, and a running recording
        // must survive a widget reopen — resync from the live snapshots.
        try {
            const rec = await invoke<RecordingStatus>(
                "workspace_meeting_recording_status",
            );
            if (rec.recording || rec.stopping) {
                applyRecording(rec);
                return;
            }
        } catch {
            /* fall through to detection state */
        }
        try {
            const det = await invoke<DetectStatus>(
                "workspace_meeting_detect_status",
            );
            if (det.app) sourceApp = det.app;
        } catch {
            /* keep generic subtitle */
        }
    }

    onDestroy(() => {
        for (const unlisten of unlistens) unlisten();
    });
</script>

<svelte:window
    onmousedown={(e) => {
        if (
            menuOpen &&
            !(e.target as HTMLElement).closest(".menu, .menu-toggle")
        ) {
            menuOpen = false;
        }
    }}
/>

<div class="widget-root">
    <div
        class="pill"
        class:compact={phase === "recording"}
        title={phase === "recording"
            ? micOnly
                ? "Recording — microphone only"
                : "Recording"
            : undefined}
        data-tauri-drag-region
    >
        {#if phase === "recording"}
            <svg
                class="grip"
                width="7"
                height="13"
                viewBox="0 0 7 13"
                fill="currentColor"
                aria-hidden="true"
            >
                <circle cx="1.5" cy="1.5" r="1.5" />
                <circle cx="5.5" cy="1.5" r="1.5" />
                <circle cx="1.5" cy="6.5" r="1.5" />
                <circle cx="5.5" cy="6.5" r="1.5" />
                <circle cx="1.5" cy="11.5" r="1.5" />
                <circle cx="5.5" cy="11.5" r="1.5" />
            </svg>
            <span class="rec-dot" aria-hidden="true"></span>
            {#if stopError}
                <span class="stop-error">Couldn't stop — open Clauge</span>
            {:else}
                <span class="elapsed">{elapsedLabel}</span>
            {/if}
            <button
                class="stop-btn"
                onclick={stopRecording}
                disabled={stopping}
                aria-label="Stop recording"
                title="Stop recording"
            >
                {#if stopping}
                    <span class="spinner"></span>
                {:else}
                    <svg
                        width="12"
                        height="12"
                        viewBox="0 0 12 12"
                        fill="currentColor"
                        aria-hidden="true"
                    >
                        <rect x="1" y="1" width="10" height="10" rx="2" />
                    </svg>
                {/if}
            </button>
            <button
                class="close-btn subtle"
                onclick={hideWidget}
                aria-label="Hide widget"
                title="Hide widget — recording continues"
            >
                <svg
                    width="10"
                    height="10"
                    viewBox="0 0 10 10"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    aria-hidden="true"
                >
                    <path d="M2 2l6 6M8 2L2 8" />
                </svg>
            </button>
        {:else}
            <img class="logo" src="/clauge-icon.svg" alt="" />
            <div class="text">
                <span class="title">Start AI Meeting Note</span>
                <span class="subtitle" class:warn={phase === "model-missing"}
                    >{promptSubtitle}</span
                >
            </div>
            {#if phase === "model-missing"}
                <button class="primary solo" onclick={openSettings}>
                    Open Settings
                </button>
            {:else}
                <div class="actions">
                    <button
                        class="primary split-main"
                        onclick={startTranscribing}
                        disabled={phase === "starting"}
                    >
                        {#if phase === "starting"}
                            <span class="spinner light"></span>
                        {/if}
                        Start transcribing
                    </button>
                    <button
                        class="primary split-chevron menu-toggle"
                        onclick={() => (menuOpen = !menuOpen)}
                        aria-label="More options"
                        aria-haspopup="menu"
                        aria-expanded={menuOpen}
                    >
                        <svg
                            width="10"
                            height="10"
                            viewBox="0 0 10 10"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="1.6"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            aria-hidden="true"
                        >
                            <path d="M2 3.5 5 6.5 8 3.5" />
                        </svg>
                    </button>
                </div>
            {/if}
            <button class="close-btn" onclick={dismiss} aria-label="Dismiss">
                <svg
                    width="10"
                    height="10"
                    viewBox="0 0 10 10"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    aria-hidden="true"
                >
                    <path d="M2 2l6 6M8 2L2 8" />
                </svg>
            </button>
        {/if}
    </div>

    {#if menuOpen}
        <div class="menu" role="menu">
            <button class="menu-item" role="menuitem" onclick={dismiss}>
                Dismiss for this call
            </button>
            <button class="menu-item" role="menuitem" onclick={disableDetect}>
                Don't auto-detect calls
            </button>
            <div class="menu-divider"></div>
            <button class="menu-item" role="menuitem" onclick={openSettings}>
                Meeting Notes settings…
            </button>
        </div>
    {/if}
</div>

<style>
    /* app.css paints `body:not(.glass-mode)` opaque with !important; the
       extra `html.widget-window body` selector out-ranks it so the only
       painted pixels in this window are the pill and the menu card. */
    :global(html),
    :global(body),
    :global(html.widget-window body) {
        background: transparent !important;
        margin: 0;
        overflow: hidden;
    }

    .widget-root {
        width: 100vw;
        height: 100vh;
        font-family:
            -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Ubuntu,
            sans-serif;
        -webkit-font-smoothing: antialiased;
        user-select: none;
        -webkit-user-select: none;
    }

    .pill {
        position: relative;
        height: 60px;
        margin: 10px;
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 0 10px 0 12px;
        background: #ffffff;
        border: 1px solid rgba(0, 0, 0, 0.08);
        border-radius: 14px;
        /* offset + blur must stay within the 10px window margin or the
           shadow gets clipped at the (transparent) window edge */
        box-shadow:
            0 3px 7px rgba(0, 0, 0, 0.22),
            0 1px 3px rgba(0, 0, 0, 0.1);
        box-sizing: border-box;
    }

    .pill.compact {
        height: 44px;
        justify-content: center;
        gap: 8px;
        padding: 0 10px;
        cursor: grab;
    }

    /* Drag affordance: anchored at the pill's far left so the timer
       cluster stays visually centered. pointer-events: none so drags
       pass through to the pill's data-tauri-drag-region. */
    .grip {
        position: absolute;
        left: 9px;
        top: 50%;
        transform: translateY(-50%);
        color: #1f1f21;
        opacity: 0.4;
        pointer-events: none;
        transition: opacity 0.12s;
    }

    .pill:hover .grip {
        opacity: 0.6;
    }

    .close-btn {
        position: absolute;
        top: -8px;
        right: -8px;
        width: 22px;
        height: 22px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        padding: 0;
        border: none;
        border-radius: 50%;
        background: rgba(0, 0, 0, 0.45);
        color: #ffffff;
        cursor: pointer;
        transition: background 0.12s;
    }

    .close-btn:hover {
        background: rgba(0, 0, 0, 0.65);
    }

    /* Recording-phase escape hatch: present but unobtrusive until the
       pill is hovered. */
    .close-btn.subtle {
        width: 18px;
        height: 18px;
        top: -6px;
        right: -6px;
        opacity: 0.55;
        transition:
            opacity 0.12s,
            background 0.12s;
    }

    .pill:hover .close-btn.subtle {
        opacity: 1;
    }

    .stop-error {
        font-size: 10.5px;
        font-weight: 500;
        color: #b54708;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        pointer-events: none;
    }

    .logo {
        width: 30px;
        height: 30px;
        flex: none;
        pointer-events: none;
    }

    .pill.compact .rec-dot {
        margin: 0;
    }

    .rec-dot {
        width: 10px;
        height: 10px;
        margin: 0 6px 0 4px;
        flex: none;
        border-radius: 50%;
        background: #e5484d;
        animation: rec-pulse 1.6s ease-in-out infinite;
        pointer-events: none;
    }

    @keyframes rec-pulse {
        0%,
        100% {
            box-shadow: 0 0 0 0 rgba(229, 72, 77, 0.45);
        }
        50% {
            box-shadow: 0 0 0 5px rgba(229, 72, 77, 0);
        }
    }

    .text {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 1px;
        pointer-events: none;
    }

    .title {
        font-size: 13px;
        font-weight: 600;
        color: #1f1f21;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .subtitle {
        font-size: 11.5px;
        color: #6f6f76;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .subtitle.warn {
        color: #b54708;
    }

    .elapsed {
        font-size: 12.5px;
        font-variant-numeric: tabular-nums;
        color: #e5484d;
        font-weight: 600;
        pointer-events: none;
    }

    .actions {
        display: flex;
        flex: none;
    }

    .primary {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 6px;
        height: 30px;
        padding: 0 12px;
        border: none;
        background: #2383e2;
        color: #ffffff;
        font-size: 12.5px;
        font-weight: 500;
        font-family: inherit;
        cursor: pointer;
        transition: background 0.12s;
    }

    .primary:hover {
        background: #1b74cd;
    }

    .primary:disabled {
        opacity: 0.75;
        cursor: default;
    }

    .primary.solo {
        border-radius: 8px;
    }

    .split-main {
        border-radius: 8px 0 0 8px;
    }

    .split-chevron {
        border-radius: 0 8px 8px 0;
        padding: 0 7px;
        border-left: 1px solid rgba(255, 255, 255, 0.3);
    }

    .stop-btn {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 30px;
        height: 30px;
        flex: none;
        border: none;
        border-radius: 8px;
        background: #e5484d;
        color: #ffffff;
        cursor: pointer;
        transition: background 0.12s;
    }

    .stop-btn:hover {
        background: #d23339;
    }

    .stop-btn:disabled {
        opacity: 0.75;
        cursor: default;
    }

    .spinner {
        width: 12px;
        height: 12px;
        flex: none;
        border-radius: 50%;
        border: 2px solid rgba(255, 255, 255, 0.35);
        border-top-color: #ffffff;
        animation: spin 0.7s linear infinite;
    }

    @keyframes spin {
        to {
            transform: rotate(360deg);
        }
    }

    .menu {
        margin: -2px 10px 0 auto;
        width: 220px;
        padding: 4px;
        background: #ffffff;
        border: 1px solid rgba(0, 0, 0, 0.08);
        border-radius: 10px;
        box-shadow:
            0 8px 24px rgba(0, 0, 0, 0.22),
            0 1px 3px rgba(0, 0, 0, 0.1);
        display: flex;
        flex-direction: column;
    }

    .menu-item {
        border: none;
        background: none;
        text-align: left;
        font-size: 12.5px;
        font-family: inherit;
        color: #1f1f21;
        padding: 7px 10px;
        border-radius: 6px;
        cursor: pointer;
        transition: background 0.12s;
    }

    .menu-item:hover {
        background: rgba(0, 0, 0, 0.06);
    }

    .menu-divider {
        height: 1px;
        margin: 4px 6px;
        background: rgba(0, 0, 0, 0.08);
    }

    @media (prefers-color-scheme: dark) {
        .pill,
        .menu {
            background: #28282c;
            border-color: rgba(255, 255, 255, 0.1);
        }
        .title,
        .menu-item {
            color: #f0f0f2;
        }
        .grip {
            color: #f0f0f2;
        }
        .subtitle {
            color: #9d9da6;
        }
        .subtitle.warn {
            color: #f0a05a;
        }
        .stop-error {
            color: #f0a05a;
        }
        .menu-item:hover {
            background: rgba(255, 255, 255, 0.08);
        }
        .menu-divider {
            background: rgba(255, 255, 255, 0.1);
        }
    }
</style>

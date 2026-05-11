<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { settings, setSetting } from "$lib/stores/settings";
    import { get } from "svelte/store";
    import { getCurrentWindow } from "@tauri-apps/api/window";
    import { isMac, isLinux } from "$lib/utils/platform";
    import { APP_EVENT } from "$lib/shared/constants/events";

    const showCustomChrome = isMac() || isLinux();

    async function wcClose() {
        await getCurrentWindow().close();
    }
    async function wcMinimize() {
        await getCurrentWindow().minimize();
    }
    async function wcFullscreen() {
        const win = getCurrentWindow();
        await win.setFullscreen(!(await win.isFullscreen()));
    }

    let show = $state(false);
    let mounted = $state(false);
    let ghConnecting = $state(false);

    async function finish() {
        await setSetting("onboarding_complete", "true");
        show = false;
    }

    async function handleOAuthCallback(e: Event) {
        if (get(settings)["onboarding_complete"]) return;
        const detail = (e as CustomEvent<{ provider: "github" | "google"; code: string }>).detail;
        if (!detail?.code || !detail?.provider) return;
        ghConnecting = true;
        try {
            const { cloudExchangeCode, cloudCheckRemoteExists } =
                await import("$lib/commands/cloud");
            const { setConnected, markSynced, showSyncRestorePrompt, setLastSyncedForKinds } =
                await import("$lib/stores/cloud");
            const { showToast } = await import("$lib/shared/primitives/toast");
            const status = await cloudExchangeCode(detail.provider, detail.code);
            if (status.user) {
                setConnected(status.user, status.providers, status.activeProvider, status.plan);
                setLastSyncedForKinds(status.lastSynced);
                showToast(`Connected as ${status.user.displayName || status.user.slug}`, "success");
            }
            const { collections } = await import("$lib/modes/rest/stores");
            const { connections: sqlConns } =
                await import("$lib/modes/sql/stores");
            const { nosqlConnections } =
                await import("$lib/modes/nosql/stores");
            const localEmpty =
                get(collections).length === 0 &&
                get(sqlConns).length === 0 &&
                get(nosqlConnections).length === 0;
            if (localEmpty) {
                try {
                    const remoteHas = await cloudCheckRemoteExists();
                    if (remoteHas) showSyncRestorePrompt.set(true);
                    else markSynced();
                } catch {
                    markSynced();
                }
            } else {
                markSynced();
            }
            await finish();
        } catch (e: any) {
            const { showToast } = await import("$lib/shared/primitives/toast");
            const { friendlyError } = await import("$lib/utils/errors");
            showToast(friendlyError(e), "error");
        } finally {
            ghConnecting = false;
        }
    }

    onMount(() => {
        if (!get(settings)["onboarding_complete"]) {
            show = true;
            setTimeout(() => {
                mounted = true;
            }, 50);
        }
        window.addEventListener(APP_EVENT.OAUTH_CALLBACK, handleOAuthCallback);
    });

    onDestroy(() => {
        window.removeEventListener(
            APP_EVENT.OAUTH_CALLBACK,
            handleOAuthCallback,
        );
    });

    $effect(() => {
        if ($settings["onboarding_complete"] === "true") show = false;
    });

    async function handleOverlayMousedown(e: MouseEvent) {
        if (e.buttons !== 1) return;
        const target = e.target as HTMLElement;
        if (target.closest('button, input, a, [role="button"]')) return;
        const win = getCurrentWindow();
        if (e.detail === 2) {
            win.toggleMaximize();
        } else {
            win.startDragging();
        }
    }

    async function handleConnect(provider: "github" | "google") {
        ghConnecting = true;
        try {
            const { cloudGithubLoginUrl, cloudGoogleLoginUrl } = await import("$lib/commands/cloud");
            const url = provider === "github"
                ? await cloudGithubLoginUrl()
                : await cloudGoogleLoginUrl();
            try {
                const { openUrl } = await import("@tauri-apps/plugin-opener");
                await openUrl(url);
            } catch {
                window.open(url, "_blank");
            }
        } catch (e: any) {
            ghConnecting = false;
            const { showToast } = await import("$lib/shared/primitives/toast");
            const { friendlyError } = await import("$lib/utils/errors");
            showToast(friendlyError(e), "error");
        }
    }

    const handleGitHubConnect = () => handleConnect("github");
    const handleGoogleConnect = () => handleConnect("google");

    /** Skip sign-in — marks onboarding complete without connecting an
     *  account. The user can connect later from Settings → About; their
     *  data stays local-only until they do. */
    async function handleSkip() {
        if (ghConnecting) return;
        await finish();
    }
</script>

{#if show}
    <div
        class="ob-overlay"
        class:visible={mounted}
        onmousedown={handleOverlayMousedown}
    >
        {#if showCustomChrome}
            <div class="ob-wc" data-drag-region>
                <button
                    class="ob-dot ob-close"
                    onclick={wcClose}
                    aria-label="Close"
                ></button>
                <button
                    class="ob-dot ob-min"
                    onclick={wcMinimize}
                    aria-label="Minimize"
                ></button>
                <button
                    class="ob-dot ob-max"
                    onclick={wcFullscreen}
                    aria-label="Fullscreen"
                ></button>
            </div>
        {/if}

        <div class="ob-card">
            <img
                src="/clauge-icon-animated.svg"
                alt="Clauge"
                class="ob-logo"
                width="72"
                height="72"
            />
            <h1 class="ob-title">Welcome to Clauge</h1>
            <p class="ob-sub">
                Sign in to sync across devices — or skip and stay local
            </p>

            <div class="ob-btns">
                {#if ghConnecting}
                    <div class="ob-waiting">
                        <span class="ob-spinner"></span>
                        <span class="ob-waiting-txt"
                            >Waiting for authorization…</span
                        >
                        <button
                            class="ob-cancel"
                            onclick={() => (ghConnecting = false)}
                            >Cancel</button
                        >
                    </div>
                {:else}
                    <button class="ob-btn-gh" onclick={handleGitHubConnect}>
                        <svg
                            width="18"
                            height="18"
                            viewBox="0 0 24 24"
                            fill="currentColor"
                            aria-hidden="true"
                        >
                            <path
                                d="M12 2C6.477 2 2 6.477 2 12c0 4.42 2.865 8.166 6.839 9.489.5.092.682-.217.682-.482 0-.237-.009-.866-.013-1.7-2.782.604-3.369-1.34-3.369-1.34-.454-1.156-1.11-1.464-1.11-1.464-.908-.62.069-.608.069-.608 1.003.07 1.531 1.03 1.531 1.03.892 1.529 2.341 1.087 2.91.831.092-.646.35-1.086.636-1.337-2.22-.253-4.555-1.11-4.555-4.943 0-1.091.39-1.984 1.029-2.683-.103-.253-.446-1.27.098-2.647 0 0 .84-.269 2.75 1.025A9.578 9.578 0 0112 6.836a9.59 9.59 0 012.504.337c1.909-1.294 2.747-1.025 2.747-1.025.546 1.377.203 2.394.1 2.647.64.699 1.028 1.592 1.028 2.683 0 3.842-2.339 4.687-4.566 4.935.359.309.678.919.678 1.852 0 1.336-.012 2.415-.012 2.743 0 .267.18.578.688.48C19.138 20.161 22 16.416 22 12c0-5.523-4.477-10-10-10z"
                            />
                        </svg>
                        Continue with GitHub
                    </button>

                    <button class="ob-btn-google" onclick={handleGoogleConnect} disabled={ghConnecting}>
                        <svg
                            width="18"
                            height="18"
                            viewBox="0 0 24 24"
                            aria-hidden="true"
                        >
                            <path
                                d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"
                                fill="#4285F4"
                            />
                            <path
                                d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
                                fill="#34A853"
                            />
                            <path
                                d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l3.66-2.84z"
                                fill="#FBBC05"
                            />
                            <path
                                d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
                                fill="#EA4335"
                            />
                        </svg>
                        Continue with Google
                    </button>

                    <!-- "or" divider — quiet, frames the skip option as a real
             alternative rather than a hidden escape hatch. -->
                    <div class="ob-divider" aria-hidden="true">
                        <span>or</span>
                    </div>

                    <button class="ob-btn-skip" onclick={handleSkip}>
                        Continue without signing in
                    </button>
                    <!-- <p class="ob-skip-hint">
          Your data stays on this device. You can connect later from
          <strong>Settings → About</strong>.
        </p> -->
                {/if}
            </div>
        </div>
    </div>
{/if}

<style>
    .ob-overlay {
        position: fixed;
        inset: 0;
        z-index: 9999;
        background: rgba(4, 4, 12, 0.96);
        backdrop-filter: blur(24px);
        -webkit-backdrop-filter: blur(24px);
        display: flex;
        align-items: center;
        justify-content: center;
        opacity: 0;
        transition: opacity 0.4s ease;
    }
    .ob-overlay.visible {
        opacity: 1;
    }

    .ob-wc {
        position: absolute;
        top: 0;
        left: 0;
        width: 72px;
        height: 46px;
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 8px;
        z-index: 1;
    }
    .ob-dot {
        width: 12px;
        height: 12px;
        border-radius: 50%;
        border: none;
        cursor: default;
        padding: 0;
        transition: filter 0.1s;
    }
    .ob-dot:hover {
        filter: brightness(0.85);
    }
    .ob-close {
        background: #ff5f57;
    }
    .ob-min {
        background: #febc2e;
    }
    .ob-max {
        background: #28c840;
    }
    :global(body.window-blurred) .ob-dot {
        background: var(--t4) !important;
    }

    .ob-card {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 12px;
        text-align: center;
    }

    .ob-logo {
        margin-bottom: 8px;
        animation: fadeUp 0.5s ease 0.1s both;
    }

    .ob-title {
        font-size: 26px;
        font-weight: 600;
        color: var(--t1);
        font-family: var(--ui);
        letter-spacing: -0.02em;
        margin: 0;
        animation: fadeUp 0.5s ease 0.18s both;
    }

    .ob-sub {
        font-size: 14px;
        color: var(--t3);
        font-family: var(--ui);
        margin: 0 0 12px;
        animation: fadeUp 0.5s ease 0.24s both;
    }

    .ob-btns {
        display: flex;
        flex-direction: column;
        gap: 10px;
        width: 280px;
        animation: fadeUp 0.5s ease 0.32s both;
    }

    .ob-btn-gh {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 10px;
        height: 44px;
        border-radius: 10px;
        background: #fff;
        color: #24292e;
        border: none;
        font-size: 13px;
        font-weight: 600;
        font-family: var(--ui);
        cursor: pointer;
        transition:
            background 0.15s,
            transform 0.1s;
        width: 100%;
    }
    .ob-btn-gh:hover {
        background: #f0f0f0;
    }
    .ob-btn-gh:active {
        transform: scale(0.98);
    }

    .ob-btn-google {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 10px;
        height: 44px;
        border-radius: 10px;
        background: transparent;
        color: var(--t3);
        border: 1px solid var(--b1);
        font-size: 13px;
        font-weight: 500;
        font-family: var(--ui);
        cursor: not-allowed;
        opacity: 0.5;
        width: 100%;
        position: relative;
    }

    .ob-soon {
        position: absolute;
        right: 14px;
        font-size: 10px;
        font-weight: 600;
        color: var(--t4);
        background: var(--b1);
        padding: 2px 6px;
        border-radius: 4px;
        letter-spacing: 0.04em;
        text-transform: uppercase;
    }

    .ob-waiting {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 10px;
    }
    .ob-spinner {
        width: 22px;
        height: 22px;
        border: 2px solid var(--b1);
        border-top-color: var(--acc);
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
    }
    .ob-waiting-txt {
        font-size: 13px;
        color: var(--t3);
        font-family: var(--ui);
    }
    .ob-cancel {
        height: 30px;
        padding: 0 16px;
        border-radius: 6px;
        border: 1px solid var(--b1);
        background: transparent;
        color: var(--t2);
        font-size: 12px;
        font-family: var(--ui);
        cursor: pointer;
        transition: background 0.1s;
    }
    .ob-cancel:hover {
        background: var(--b1);
    }

    /* "or" divider between the auth buttons and the skip option.
     Hairline + centered label — frames skip as a peer to sign-in,
     not a hidden escape. */
    .ob-divider {
        display: flex;
        align-items: center;
        gap: 10px;
        margin: 6px 0 2px;
        color: var(--t4);
        font-family: var(--ui);
        font-size: 10.5px;
        font-weight: 600;
        letter-spacing: 0.12em;
        text-transform: uppercase;
    }
    .ob-divider::before,
    .ob-divider::after {
        content: "";
        flex: 1;
        height: 1px;
        background: var(--b1);
    }
    .ob-divider span {
        line-height: 1;
    }

    /* Skip — quieter than the primary CTA but a real button (not a tiny
     text link), so the user knows it's a fully supported path. Same
     height as the auth buttons so the column reads as one unit. */
    .ob-btn-skip {
        display: flex;
        align-items: center;
        justify-content: center;
        height: 44px;
        border-radius: 10px;
        background: transparent;
        color: var(--t2);
        border: 1px solid var(--b1);
        font-size: 13px;
        font-weight: 500;
        font-family: var(--ui);
        cursor: pointer;
        transition:
            background 0.15s,
            border-color 0.15s,
            color 0.15s,
            transform 0.1s;
        width: 100%;
    }
    .ob-btn-skip:hover {
        background: rgba(255, 255, 255, 0.04);
        border-color: var(--b2);
        color: var(--t1);
    }
    .ob-btn-skip:active {
        transform: scale(0.98);
    }

    .ob-skip-hint {
        margin: 4px 0 0;
        font-size: 11px;
        line-height: 1.5;
        color: var(--t4);
        font-family: var(--ui);
        text-align: center;
    }
    .ob-skip-hint strong {
        color: var(--t3);
        font-weight: 600;
    }

    @keyframes fadeUp {
        from {
            opacity: 0;
            transform: translateY(10px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }
    @keyframes spin {
        to {
            transform: rotate(360deg);
        }
    }
</style>

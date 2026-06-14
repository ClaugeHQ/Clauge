import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { STORAGE_KEYS } from '$lib/shared/constants/storage';
import { installType, supportsSelfUpdate } from '$lib/utils/platform';

/** Toast payload for the update notification. `infoOnly` flips the toast
 *  from auto-install ("Restart to Update") to an awareness toast that
 *  sends the user to the public changelog page (with download buttons).
 *  Used on Linux .deb / .rpm installs where Tauri can't replace files in
 *  /usr/bin. */
export interface UpdateNoticeInfo {
  version: string;
  body: string;
  infoOnly?: boolean;
}

// User-facing changelog page (with download buttons). Preferred over the
// GitHub releases UI for non-developer users.
const CHANGELOG_URL = 'https://clauge.in/changelog';

// All users now follow the stable channel. The previous Settings → About
// "Receive pre-release updates" toggle was removed; any user who had
// previously opted into `pre` is migrated to `stable` here on module load
// so their next update check pulls from the stable feed. The localStorage
// key is also cleared so this migration is idempotent (no-op on subsequent
// loads) — and the rest of this file always passes 'stable' downstream.
if (typeof localStorage !== 'undefined') {
  try { localStorage.removeItem(STORAGE_KEYS.UPDATE_CHANNEL); } catch { /* ignore quota / privacy mode */ }
}

let updateReadyData: UpdateNoticeInfo | null = null;
// Sentinel — true means "an Update is staged in Rust state, ready to install".
// The actual Update object lives in tauri::State (PendingUpdate); we never see
// it from JS to avoid round-tripping a non-Cloneable type. Always false on the
// info-only path (.deb / .rpm) — there's nothing pre-downloaded.
let pendingUpdate: boolean = false;

/** Reactive store: set when a new version is detected. For installable
 *  install types the binary is already pre-downloaded; for info-only
 *  installs (.deb / .rpm) only the version metadata is populated. */
export const updateAvailable = writable<UpdateNoticeInfo | null>(null);

/** Reactive store: controls visibility of the What's New modal */
export const showWhatsNewModal = writable(false);

/** Reactive store: holds changelog content for What's New display */
export const whatsNewContent = writable<{ version: string; body: string } | null>(null);

/**
 * Check for updates and set the `updateAvailable` store.
 *
 * Routes through the Rust-side channel-aware updater so that pre-release
 * users see the latest pre-release and stable users only see stable. Both
 * paths verify the `latest.json` signature against the baked-in pubkey.
 *
 * Two paths based on install type:
 *  - **Installable** (macOS / Windows / AppImage): pre-downloads the binary
 *    via `check_for_update_in_channel`, stashes it in Rust state, sets
 *    `pendingUpdate=true`. Toast shows "Restart to Update".
 *  - **Info-only** (Linux .deb / .rpm): calls `check_latest_version`
 *    instead — same signed channel, no download, no install. Toast shows
 *    "Download .deb" linking to the GitHub releases page.
 *  - **Dev / unknown**: skipped silently.
 */
export async function checkAndDownloadUpdate(): Promise<UpdateNoticeInfo | null> {
  try {
    // Stable-only after the pre-release channel was removed from Settings.
    // Kept as a local const so the Rust signature (which still accepts a
    // channel arg) stays untouched — flipping back is a one-line change
    // if pre-release ever returns.
    const channel: 'stable' | 'pre' = 'stable';
    const kind = await installType();

    if (kind === 'linux-package') {
      // Info-only path — no download, no PendingUpdate state.
      const info = await invoke<UpdateNoticeInfo | null>(
        'check_latest_version',
        { channel }
      );
      if (!info) return null;
      const notice: UpdateNoticeInfo = { ...info, infoOnly: true };
      updateReadyData = notice;
      updateAvailable.set(notice);
      return notice;
    }

    if (!(await supportsSelfUpdate())) {
      // Dev / unknown — nothing meaningful to do.
      return null;
    }

    const info = await invoke<UpdateNoticeInfo | null>(
      'check_for_update_in_channel',
      { channel }
    );
    if (!info) return null;

    pendingUpdate = true; // sentinel — actual Update object lives in Rust state
    updateReadyData = info;
    updateAvailable.set(info);
    return info;
  } catch (e) {
    console.warn('Update check failed:', e);
  }
  return null;
}

/**
 * Open the public changelog page (with download buttons) in the user's
 * default browser. Used by the info-only update toast on .deb / .rpm
 * installs where in-app install is not possible.
 */
export async function openChangelogPage(): Promise<void> {
  try {
    const { openUrl } = await import('@tauri-apps/plugin-opener');
    await openUrl(CHANGELOG_URL);
  } catch {
    // Last-ditch fallback if the opener plugin failed.
    window.open(CHANGELOG_URL, '_blank');
  }
}

/**
 * Install the pending update and hand off to the OS.
 *
 * `install_pending_update` does NOT return on success — the Rust side
 * either calls `std::process::exit(0)` (Windows; NSIS passive-mode
 * auto-launches the new binary) or `app.restart()` (macOS / AppImage).
 * Calling a JS-side relaunch here races the in-progress installer; that
 * race was the "Restart stuck" symptom on Windows.
 *
 * If no pending update is loaded (e.g. user closed the app between check
 * and install), re-runs the check on the current channel before installing.
 */
export async function restartToUpdate(): Promise<void> {
  // Info-only path (.deb / .rpm): there's no pre-downloaded binary; the
  // notification's button uses `openChangelogPage` directly. Bail safely
  // in case this is invoked anyway.
  if (updateReadyData?.infoOnly) {
    await openChangelogPage();
    return;
  }
  if (!pendingUpdate) {
    try {
      await checkAndDownloadUpdate();
    } catch (_) { /* ignore */ }
  }
  if (!pendingUpdate) return;
  try {
    await invoke('install_pending_update');
    // Reaching this line means Rust returned an error before the install
    // could hand off — surface it (the catch below logs).
  } catch (e) {
    console.error('Update restart failed:', e);
  }
}

/**
 * Get the current update-ready data (non-reactive).
 */
export function getUpdateReady(): UpdateNoticeInfo | null {
  return updateReadyData;
}

/**
 * Check if this version is new since last launch and fetch release notes.
 * Shows the What's New modal if the version changed.
 */
export async function checkWhatsNew(currentVersion: string): Promise<{ version: string; body: string } | null> {
  const lastSeen = typeof localStorage !== 'undefined'
    ? localStorage.getItem(STORAGE_KEYS.LAST_SEEN_VERSION)
    : null;

  if (lastSeen && lastSeen !== currentVersion) {
    try {
      const res = await fetch(
        `https://api.github.com/repos/ClaugeHQ/Clauge/releases/tags/v${currentVersion}`
      );
      if (res.ok) {
        const data = await res.json();
        if (data?.body) {
          const info = { version: currentVersion, body: data.body };
          whatsNewContent.set(info);
          showWhatsNewModal.set(true);
          if (typeof localStorage !== 'undefined') {
            localStorage.setItem(STORAGE_KEYS.LAST_SEEN_VERSION, currentVersion);
          }
          return info;
        }
      }
    } catch { /* ignore */ }
  }

  if (typeof localStorage !== 'undefined') {
    localStorage.setItem(STORAGE_KEYS.LAST_SEEN_VERSION, currentVersion);
  }
  return null;
}

/**
 * Convert GitHub release markdown to simple HTML.
 */
export function renderReleaseMarkdown(md: string): string {
  return md
    .replace(/\r\n/g, '\n')
    .replace(/^\s*### (.+)$/gm, '<h4>$1</h4>')
    .replace(/^\s*## (.+)$/gm, '<h3>$1</h3>')
    .replace(/^\s*# (.+)$/gm, '<h2>$1</h2>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/^\s*[-*] (.+)$/gm, '<li>$1</li>')
    .replace(/((?:<li>.*<\/li>\n?)+)/g, '<ul>$1</ul>')
    .replace(/\n\n+/g, '<br>')
    .replace(/\n/g, '');
}

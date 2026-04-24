import { writable } from 'svelte/store';

let updateReadyData: { version: string; body: string } | null = null;
let pendingUpdate: any = null;

/** Reactive store: set when an update has been downloaded and is ready to install */
export const updateAvailable = writable<{ version: string; body: string } | null>(null);

/** Reactive store: controls visibility of the What's New modal */
export const showWhatsNewModal = writable(false);

/** Reactive store: holds changelog content for What's New display */
export const whatsNewContent = writable<{ version: string; body: string } | null>(null);

/**
 * Check for updates, download if available, and set the updateAvailable store.
 * Returns update info if an update was found, null otherwise.
 */
export async function checkAndDownloadUpdate(): Promise<{ version: string; body: string } | null> {
  try {
    const { check } = await import('@tauri-apps/plugin-updater');
    const update = await check();
    if (!update) return null;

    await update.download();
    pendingUpdate = update;
    updateReadyData = { version: update.version, body: update.body || '' };
    updateAvailable.set(updateReadyData);
    return updateReadyData;
  } catch (e) {
    console.warn('Update check failed:', e);
  }
  return null;
}

/**
 * Install the pending update and relaunch the app.
 */
export async function restartToUpdate(): Promise<void> {
  if (!pendingUpdate) {
    // Re-check and download if pendingUpdate was lost
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const update = await check();
      if (update) {
        await update.download();
        pendingUpdate = update;
      }
    } catch (_) { /* ignore */ }
  }
  if (!pendingUpdate) return;
  try {
    await pendingUpdate.install();
    const { relaunch } = await import('@tauri-apps/plugin-process');
    await relaunch();
  } catch (e) {
    console.error('Update restart failed:', e);
  }
}

/**
 * Get the current update-ready data (non-reactive).
 */
export function getUpdateReady(): { version: string; body: string } | null {
  return updateReadyData;
}

/**
 * Check if this version is new since last launch and fetch release notes.
 * Shows the What's New modal if the version changed.
 */
export async function checkWhatsNew(currentVersion: string): Promise<{ version: string; body: string } | null> {
  const lastSeen = typeof localStorage !== 'undefined'
    ? localStorage.getItem('clauge_last_seen_version')
    : null;

  if (lastSeen && lastSeen !== currentVersion) {
    try {
      const res = await fetch(
        `https://api.github.com/repos/ansxuman/Clauge/releases/tags/v${currentVersion}`
      );
      if (res.ok) {
        const data = await res.json();
        if (data?.body) {
          const info = { version: currentVersion, body: data.body };
          whatsNewContent.set(info);
          showWhatsNewModal.set(true);
          if (typeof localStorage !== 'undefined') {
            localStorage.setItem('clauge_last_seen_version', currentVersion);
          }
          return info;
        }
      }
    } catch { /* ignore */ }
  }

  if (typeof localStorage !== 'undefined') {
    localStorage.setItem('clauge_last_seen_version', currentVersion);
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

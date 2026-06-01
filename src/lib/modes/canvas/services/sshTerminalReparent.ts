import { get } from 'svelte/store';
import { sshTerminalIds, sshTerminalMap, sshProfiles } from '$lib/modes/ssh/stores';
import { profileIdFromTabKey } from '$lib/modes/ssh/tabkey';

/**
 * Reparent an SSH xterm container into a Canvas tile slot.
 * The entry's container HTMLDivElement is moved via appendChild so xterm
 * continues rendering without a full remount.
 */
export function attachSshTerminal(tabKey: string, slot: HTMLElement): void {
  const entry = get(sshTerminalMap).get(tabKey);
  if (!entry) return;
  slot.appendChild(entry.container);
  try {
    entry.fitAddon.fit();
  } catch {
    // First ResizeObserver tick will fit.
  }
}

/**
 * Remove the SSH xterm container from a Canvas tile slot, returning it to
 * the SshPanel's terminalEl wrapper on next activateProfile call.
 */
export function detachSshTerminal(tabKey: string, slot: HTMLElement): void {
  const entry = get(sshTerminalMap).get(tabKey);
  if (!entry) return;
  if (entry.container.parentElement === slot) {
    slot.removeChild(entry.container);
  }
}

/**
 * Return SSH tabs that have an active backend terminal id (i.e. have been
 * connected at least once). Source of truth is `sshTerminalIds`, which is
 * populated by SshPanel when a PTY is established.
 *
 * Title is derived from the profile name where available, falling back to
 * the raw tabKey. Multi-tab titles include a disambiguating suffix derived
 * from the timestamp-counter portion of the key.
 */
export function listOpenSshTerminals(): { id: string; title: string }[] {
  const termIds = get(sshTerminalIds);
  const profiles = get(sshProfiles);

  const out: { id: string; title: string }[] = [];
  for (const [tabKey] of termIds) {
    const profileId = profileIdFromTabKey(tabKey);
    const profile = profiles.find((p) => p.id === profileId);
    const baseName = profile?.name ?? profileId;

    // Extract the counter suffix from `profileId#timestamp-N` → "N".
    const hashIdx = tabKey.indexOf('#');
    const suffix = hashIdx !== -1 ? tabKey.slice(hashIdx + 1) : null;
    const title = suffix ? `${baseName} — ${suffix}` : baseName;

    out.push({ id: tabKey, title });
  }
  return out;
}

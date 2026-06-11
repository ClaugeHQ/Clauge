import { get } from 'svelte/store';
import {
  hasSyncedOnce,
  markSynced,
  showSyncRestorePrompt,
  showDeviceSetup,
} from '$lib/stores/cloud';
import { cloudCheckRemoteExists, cloudLocalHasData, cloudSyncPushNow } from '$lib/commands/cloud';
import { showToast } from '$lib/shared/primitives/toast';

/**
 * First-sync decision for a signed-in device that has never synced.
 * Runs at boot (when already signed in) and right after an in-app login.
 *
 * The four cases:
 *   - neither side has data  → nothing to do, mark synced
 *   - only cloud has data    → offer the restore prompt
 *   - only local has data    → push, mark synced once the server confirms
 *   - both sides have data   → the user decides (DeviceSetupModal)
 *
 * On a transient remote-check failure nothing is marked — the next boot
 * retries rather than permanently dismissing the decision.
 */
let inFlight: Promise<void> | null = null;
export function decideFirstSync(): Promise<void> {
  inFlight ??= run().finally(() => { inFlight = null; });
  return inFlight;
}
async function run(): Promise<void> {
  if (get(hasSyncedOnce)) return;
  const localEmpty = !(await cloudLocalHasData());
  try {
    const remoteHas = await cloudCheckRemoteExists();
    if (!remoteHas && localEmpty) {
      markSynced();
    } else if (remoteHas && localEmpty) {
      showSyncRestorePrompt.set(true);
    } else if (!remoteHas && !localEmpty) {
      // Awaited so decideFirstSync() doesn't resolve mid-push — the
      // inFlight guard must cover the whole decision, push included.
      await cloudSyncPushNow()
        .then(() => markSynced())
        .catch((e) => {
          console.warn('[Cloud] initial push failed:', e);
          showToast(
            'Cloud backup failed — use the sync button in the sidebar to retry',
            'error',
          );
        });
    } else {
      // Both sides have data — the user decides.
      showDeviceSetup.set(true);
    }
  } catch (e) {
    console.warn('[Cloud] remote check failed:', e);
  }
}

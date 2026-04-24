<script lang="ts">
  import { updateAvailable, restartToUpdate } from '$lib/utils/updater';

  let dismissed = $state(false);
  let installing = $state(false);

  async function handleInstall() {
    installing = true;
    await restartToUpdate();
  }

  async function handleWhatsNew() {
    try {
      const { openUrl } = await import('@tauri-apps/plugin-opener');
      await openUrl('https://clauge.dev/changelog.html');
    } catch {
      window.open('https://clauge.dev/changelog.html', '_blank');
    }
  }

  function dismiss() {
    dismissed = true;
  }
</script>

{#if $updateAvailable && !dismissed}
  <div class="update-notif">
    <div class="un-header">
      <svg class="un-icon" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/></svg>
      <div class="un-text">
        <span class="un-title">Clauge v{$updateAvailable.version} is available</span>
        <span class="un-desc">A new version has been downloaded. Restart to apply.</span>
      </div>
      <button class="un-close" onclick={dismiss}>&times;</button>
    </div>
    <div class="un-actions">
      <button class="un-btn primary" onclick={handleInstall} disabled={installing}>
        {installing ? 'Restarting...' : 'Restart to Update'}
      </button>
      <button class="un-btn secondary" onclick={handleWhatsNew}>
        What's New
        <svg viewBox="0 0 24 24"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
      </button>
    </div>
  </div>
{/if}

<style>
  .update-notif {
    position: fixed;
    bottom: 40px;
    right: 16px;
    width: 320px;
    background: var(--modal-bg, #101016);
    border: 1px solid var(--b1);
    border-radius: 10px;
    box-shadow: 0 8px 32px rgba(0,0,0,0.5);
    padding: 14px;
    z-index: 900;
    animation: slideUp 0.25s cubic-bezier(0.4, 0, 0.2, 1);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  @keyframes slideUp {
    from { opacity: 0; transform: translateY(12px); }
    to { opacity: 1; transform: none; }
  }
  .un-header {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .un-icon {
    width: 18px;
    height: 18px;
    stroke: var(--acc);
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
    flex-shrink: 0;
    margin-top: 1px;
  }
  .un-text {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .un-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--t1);
    font-family: var(--ui);
  }
  .un-desc {
    font-size: 11px;
    color: var(--t3);
    font-family: var(--ui);
  }
  .un-close {
    width: 20px;
    height: 20px;
    border: none;
    background: transparent;
    color: var(--t3);
    font-size: 16px;
    cursor: default;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    flex-shrink: 0;
    line-height: 1;
    transition: color 0.1s;
  }
  .un-close:hover {
    color: var(--t1);
  }
  .un-actions {
    display: flex;
    gap: 8px;
  }
  .un-btn {
    height: 30px;
    padding: 0 14px;
    border-radius: 6px;
    font-size: 12px;
    font-family: var(--ui);
    font-weight: 600;
    cursor: default;
    display: flex;
    align-items: center;
    gap: 5px;
    transition: opacity 0.12s;
  }
  .un-btn.primary {
    border: none;
    background: var(--acc);
    color: #fff;
  }
  .un-btn.primary:hover:not(:disabled) { opacity: 0.85; }
  .un-btn.primary:disabled { opacity: 0.5; }
  .un-btn.secondary {
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
  }
  .un-btn.secondary:hover {
    border-color: var(--b2);
    color: var(--t1);
  }
  .un-btn.secondary svg {
    width: 11px;
    height: 11px;
    stroke: currentColor;
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>

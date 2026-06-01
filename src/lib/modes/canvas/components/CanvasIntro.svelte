<script lang="ts">
  import { openSettingsTab } from '$lib/shared/stores/tabs';

  function openCanvasSettings() {
    openSettingsTab('canvas');
  }
</script>

<div class="cv-intro">
  <div class="cv-intro-content">
    <div class="cv-intro-icon">
      <svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 7V3h4M21 7V3h-4M3 17v4h4M21 17v4h-4"></path>
        <rect x="7" y="7" width="10" height="10" rx="1.5"></rect>
      </svg>
    </div>
    <h2 class="cv-intro-title">Canvas mode</h2>
    <p class="cv-intro-desc">
      Spread all your open terminals — Agent sessions, SSH connections, quick shells — across an infinite spatial workspace.
      Drag windows around, zoom in and out, snap to neighbors, and stop alt-tabbing.
    </p>
    <ul class="cv-intro-list">
      <li>See every terminal session at once</li>
      <li>Drag and resize windows like a real workspace</li>
      <li>Spawn shells anywhere with one click</li>
      <li>State preserved across mode switches</li>
    </ul>
    <button class="cv-intro-cta" onclick={openCanvasSettings}>Open Canvas Settings</button>
    <p class="cv-intro-foot">Canvas is experimental. Enable in Settings → Canvas.</p>
  </div>
</div>

<style>
  .cv-intro {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 32px;
    background: var(--cv-bg, var(--n));
    overflow: auto;
  }
  .cv-intro-content {
    max-width: 520px;
    text-align: center;
    color: var(--t1);
  }
  .cv-intro-icon {
    color: var(--acc);
    margin-bottom: 24px;
    display: flex;
    justify-content: center;
  }
  .cv-intro-title {
    font-size: 28px;
    font-weight: 600;
    margin: 0 0 12px;
    color: var(--t1);
  }
  .cv-intro-desc {
    font-size: 14px;
    line-height: 1.6;
    margin: 0 0 20px;
    color: var(--t2);
  }
  .cv-intro-list {
    text-align: left;
    margin: 0 auto 28px;
    padding: 0 0 0 20px;
    color: var(--t2);
    font-size: 13px;
    line-height: 1.8;
    max-width: 360px;
  }
  .cv-intro-cta {
    background: var(--acc);
    color: var(--t1);
    border: 0;
    padding: 10px 24px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 14px;
    font-weight: 500;
  }
  .cv-intro-cta:hover {
    background: color-mix(in srgb, var(--acc) 85%, white);
  }
  .cv-intro-foot {
    margin: 16px 0 0;
    font-size: 12px;
    color: var(--t4);
  }
</style>

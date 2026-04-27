<script lang="ts">
  import { onMount } from 'svelte';
  import { githubConnected, syncing, lastSyncedAt } from '$lib/stores/github';
  import { updateAvailable, showWhatsNewModal } from '$lib/utils/updater';
  import { mode } from '$lib/stores/app';
  import { agentGitBranchName, agentGitFiles, agentGitAhead, agentGitBehind, activeAgentSession, agentUsageLimits, agentShellOpen, agentSessionKey } from '$lib/stores/agent';
  import { activeModal } from '$lib/stores/app';
  import AgentGitPanel from '$lib/components/agent/AgentGitPanel.svelte';

  let gitPanelOpen = $state(false);

  let appVersion = $state('');
  onMount(async () => {
    try {
      const { getVersion } = await import('@tauri-apps/api/app');
      appVersion = await getVersion();
    } catch {
      appVersion = '';
    }
  });

  let syncLabel = $derived(
    $syncing ? 'Syncing...' :
    $githubConnected ? ($lastSyncedAt ? 'Gist synced' : 'Connected') :
    'Not connected'
  );

  let syncColor = $derived(
    $syncing ? 'var(--warn)' :
    $githubConnected ? 'var(--ok)' :
    'var(--t3)'
  );

  interface UsageChip { label: string; pct: number; color: string; }

  function usageColor(pct: number): string {
    if (pct > 80) return '#f85149';
    if (pct > 50) return '#d29922';
    return 'var(--acc)';
  }

  let usageChips = $derived.by((): UsageChip[] => {
    const limits = $agentUsageLimits;
    if (!limits) return [];
    const chips: UsageChip[] = [];
    // Claude API returns { five_hour: { utilization }, seven_day: { utilization }, seven_day_sonnet: { utilization } }
    // Also handle alternate shape: { standard: { percentUsed }, extended: { percentUsed } }
    const sessionPct = limits.five_hour?.utilization ?? limits.standard?.percentUsed;
    const weeklyPct = limits.seven_day?.utilization ?? limits.extended?.percentUsed;
    const sonnetPct = limits.seven_day_sonnet?.utilization ?? null;
    if (sessionPct != null) {
      const pct = Math.round(sessionPct);
      chips.push({ label: 'Session', pct, color: usageColor(pct) });
    }
    if (weeklyPct != null) {
      const pct = Math.round(weeklyPct);
      chips.push({ label: 'Weekly', pct, color: usageColor(pct) });
    }
    if (sonnetPct != null) {
      const pct = Math.round(sonnetPct);
      chips.push({ label: 'Sonnet', pct, color: usageColor(pct) });
    }
    return chips;
  });

  function showUsageDashboard() {
    activeModal.set('settings:agent:usage');
  }

  function openUpdateModal() {
    showWhatsNewModal.set(true);
  }
</script>

{#if $mode === 'agent'}
<footer class="statusbar glass-surface">
  {#if $agentGitBranchName}
  <div class="sl">
    <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
    <div class="si git-clickable" onclick={(e) => { e.stopPropagation(); gitPanelOpen = !gitPanelOpen; }}>
      <svg style="width:10px;height:10px;stroke:var(--t3);fill:none;stroke-width:1.7;stroke-linecap:round;stroke-linejoin:round" viewBox="0 0 24 24"><line x1="6" y1="3" x2="6" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 01-9 9"/></svg>
      <span>{$agentGitBranchName}</span>
      {#if $agentGitAhead > 0}<span class="git-ahead">↑{$agentGitAhead}</span>{/if}
      {#if $agentGitBehind > 0}<span class="git-behind">↓{$agentGitBehind}</span>{/if}
      {#if $agentGitFiles.length > 0}<span class="git-changes">{$agentGitFiles.length}</span>{/if}
    </div>
  </div>
  <AgentGitPanel bind:open={gitPanelOpen} />
  {/if}
  <div class="sc">
    {#if usageChips.length > 0}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="usage-chips-clickable" onclick={showUsageDashboard}>
        {#each usageChips as chip, i}
          {#if i > 0}<div class="usage-sep"></div>{/if}
          <div class="usage-chip-item">
            <span class="usage-dot" style="background:{chip.color};box-shadow:0 0 6px {chip.color}44;"></span>
            <span class="usage-lbl">{chip.label}</span>
            <span class="usage-val" style="color:{chip.color}">{chip.pct}%</span>
          </div>
        {/each}
      </div>
    {:else if !$agentSessionKey}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="si setup-usage" onclick={() => activeModal.set('settings:agent')}>
        <svg style="width:10px;height:10px;stroke:var(--t4);fill:none;stroke-width:1.7;stroke-linecap:round" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/></svg>
        <span>Set up usage tracking</span>
      </div>
    {/if}
  </div>
  <div class="sr">
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="si shell-toggle" onclick={() => { if ($activeAgentSession) agentShellOpen.update(v => !v); }}>
      <svg style="width:10px;height:10px;stroke:{$agentShellOpen ? 'var(--acc)' : 'var(--t3)'};fill:none;stroke-width:1.7;stroke-linecap:round" viewBox="0 0 24 24"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>
      <span style="color:{$agentShellOpen ? 'var(--acc)' : ''}">Shell</span>
    </div>
    {#if appVersion}<div class="si">Clauge v{appVersion}</div>{/if}
  </div>
</footer>
{:else}
<footer class="statusbar glass-surface">
  <div class="sr">
    <div class="si">
      <span class="sled" style="background:{syncColor}"></span>
      <svg style="width:10px;height:10px;stroke:var(--t3);fill:none;stroke-width:1.7;stroke-linecap:round" viewBox="0 0 24 24"><path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 00-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0020 4.77 5.07 5.07 0 0019.91 1S18.73.65 16 2.48a13.38 13.38 0 00-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 005 4.77a5.44 5.44 0 00-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 009 18.13V22"/></svg>
      <span>{syncLabel}</span>
    </div>
    {#if $updateAvailable}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="si update-hint" onclick={openUpdateModal}>
        <span class="sled update-dot"></span>
        <span>Update available · v{$updateAvailable.version}</span>
      </div>
    {/if}
    {#if appVersion}<div class="si">Clauge v{appVersion}</div>{/if}
  </div>
</footer>
{/if}

<style>
  .statusbar {
    height: 24px;
    background: var(--s);
    border-top: 1px solid var(--b1);
    display: flex;
    align-items: center;
    padding: 0 12px;
    gap: 16px;
    flex-shrink: 0;
  }
  .si {
    font-size: 10px;
    color: var(--t3);
    display: flex;
    align-items: center;
    gap: 4px;
    font-family: var(--mono);
  }
  .sled {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .sl {
    display: flex;
    gap: 16px;
  }
  .sc {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    gap: 16px;
  }
  .sr {
    margin-left: auto;
    display: flex;
    gap: 16px;
  }
  .git-ahead {
    color: var(--ok, #4c8);
  }
  .git-behind {
    color: var(--warn, #fa0);
  }
  .git-changes {
    font-size: 9px;
    font-weight: 700;
    color: #fff;
    background: var(--acc, #7c5cf8);
    padding: 0 5px;
    border-radius: 8px;
    min-width: 16px;
    text-align: center;
    line-height: 16px;
  }
  .git-clickable {
    cursor: pointer;
    padding: 2px 8px;
    border-radius: 4px;
    transition: background 0.1s;
  }
  .git-clickable:hover {
    background: rgba(255,255,255,0.06);
  }
  .update-hint {
    cursor: default;
    color: var(--acc, #7c5cf8);
    transition: opacity 0.12s;
  }
  .update-hint:hover {
    opacity: 0.8;
  }
  .update-dot {
    background: var(--acc, #7c5cf8) !important;
    animation: updatePulse 2s ease-in-out infinite;
  }
  @keyframes updatePulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }
  .usage-chips-clickable {
    display: flex;
    align-items: center;
    gap: 12px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 6px;
    transition: background 0.15s;
  }
  .usage-chips-clickable:hover {
    background: rgba(255,255,255,0.04);
  }
  .usage-chip-item {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .usage-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .usage-lbl {
    font-size: 10px;
    color: var(--t3);
    font-weight: 500;
    font-family: var(--mono);
  }
  .usage-val {
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    font-family: var(--mono);
  }
  .usage-sep {
    width: 1px;
    height: 10px;
    background: var(--b1);
    opacity: 0.5;
  }
  .setup-usage {
    cursor: pointer;
    color: var(--t4);
    padding: 1px 6px;
    border-radius: 4px;
    transition: color 0.1s;
  }
  .setup-usage:hover {
    color: var(--acc);
  }
  .shell-toggle {
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 4px;
    transition: background 0.1s;
  }
  .shell-toggle:hover {
    background: rgba(255,255,255,0.06);
  }
</style>

<script lang="ts">
  import { onMount } from 'svelte';
  import { githubConnected, syncing, lastSyncedAt } from '$lib/stores/github';
  import { updateAvailable, showWhatsNewModal } from '$lib/utils/updater';
  import { mode } from '$lib/stores/app';
  import { agentGitBranchName, agentGitFiles, agentGitAhead, agentGitBehind, agentContextUsage, activeAgentSession } from '$lib/stores/agent';

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

  let contextPct = $derived.by(() => {
    const session = $activeAgentSession;
    if (!session) return 0;
    const usage = $agentContextUsage.get(session.id);
    return usage ? Math.round(usage.percent) : 0;
  });

  let contextColor = $derived(
    contextPct >= 85 ? 'var(--err, #f44)' :
    contextPct >= 70 ? 'var(--warn, #fa0)' :
    'var(--ok, #4c8)'
  );

  function openUpdateModal() {
    showWhatsNewModal.set(true);
  }
</script>

{#if $mode === 'agent'}
<footer class="statusbar glass-surface">
  <div class="sl">
    <div class="si">
      <svg style="width:10px;height:10px;stroke:var(--t3);fill:none;stroke-width:1.7;stroke-linecap:round;stroke-linejoin:round" viewBox="0 0 24 24"><line x1="6" y1="3" x2="6" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 01-9 9"/></svg>
      <span>{$agentGitBranchName || '—'}</span>
      {#if $agentGitAhead > 0}<span class="git-ahead">↑{$agentGitAhead}</span>{/if}
      {#if $agentGitBehind > 0}<span class="git-behind">↓{$agentGitBehind}</span>{/if}
      {#if $agentGitFiles.length > 0}<span class="git-changes">{$agentGitFiles.length} changed</span>{/if}
    </div>
  </div>
  <div class="sc">
    <div class="si">
      <span class="sled" style="background:{contextColor}"></span>
      <span style="color:{contextColor}">Context: {contextPct}%</span>
    </div>
  </div>
  <div class="sr">
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
    color: var(--t2);
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
</style>

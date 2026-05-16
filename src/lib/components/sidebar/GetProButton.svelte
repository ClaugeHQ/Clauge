<script lang="ts">
  import { cloudPlan, upgradeModalOpen } from '$lib/stores/cloud';

  const visible = $derived($cloudPlan !== 'pro');
</script>

{#if visible}
  <button
    class="get-pro-pill"
    onclick={() => upgradeModalOpen.set(true)}
    title="Upgrade to Clauge Pro"
  >
    ✨ Get Pro
  </button>
{/if}

<style>
  .get-pro-pill {
    padding: 0.25rem 0.75rem;
    border-radius: 999px;
    border: 1px solid var(--acc, #4a90e2);
    background: linear-gradient(90deg, rgba(74, 144, 226, 0.15), rgba(74, 144, 226, 0.05));
    color: var(--acc, #4a90e2);
    cursor: pointer;
    font-size: 0.75rem;
    font-weight: 500;
    font-family: var(--ui);
    white-space: nowrap;
    transition: background 0.15s;
  }
  .get-pro-pill:hover {
    background: linear-gradient(90deg, rgba(74, 144, 226, 0.3), rgba(74, 144, 226, 0.1));
  }
</style>

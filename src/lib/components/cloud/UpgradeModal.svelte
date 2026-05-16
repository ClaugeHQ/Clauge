<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { upgradeModalOpen } from '$lib/stores/cloud';

  type Discount = { percent: number; code: string | null };
  type Plan = { id: string; price_usd: number; discount: Discount | null };
  type Pricing = { schema_version: number; plans: Plan[] };

  let pricing = $state<Pricing | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let busyPlan = $state<string | null>(null);

  $effect(() => {
    if ($upgradeModalOpen && pricing === null && !loading) {
      loadPricing();
    }
  });

  async function loadPricing() {
    loading = true;
    error = null;
    try {
      pricing = await invoke<Pricing>('cloud_get_pricing');
    } catch (e: unknown) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function startCheckout(planId: string) {
    busyPlan = planId;
    error = null;
    try {
      const url = await invoke<string>('cloud_create_checkout', { plan: planId });
      const opener = await import('@tauri-apps/plugin-opener').catch(() => null);
      if (opener) {
        await opener.openUrl(url);
      } else {
        window.open(url, '_blank');
      }
    } catch (e: unknown) {
      error = String(e);
    } finally {
      busyPlan = null;
    }
  }

  function close() {
    upgradeModalOpen.set(false);
    pricing = null;
    error = null;
  }

  function teleportToBody(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentElement === document.body) node.remove();
      },
    };
  }

  function effectivePrice(p: Plan): number {
    if (!p.discount) return p.price_usd;
    return Math.round(p.price_usd * (1 - p.discount.percent / 100) * 100) / 100;
  }

  function perMonth(p: Plan): number {
    return p.id === 'yearly' ? Math.round((p.price_usd / 12) * 100) / 100 : p.price_usd;
  }

  function savingsVsMonthly(yearly: Plan, monthly: Plan | undefined): number | null {
    if (!monthly) return null;
    const yearlyPerMonth = yearly.price_usd / 12;
    if (yearlyPerMonth >= monthly.price_usd) return null;
    const savings = monthly.price_usd * 12 - yearly.price_usd;
    return Math.round((savings / (monthly.price_usd * 12)) * 100);
  }
</script>

{#if $upgradeModalOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={close} use:teleportToBody>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
      <button class="close-btn" onclick={close} aria-label="Close">×</button>

      <div class="head">
        <h2>Upgrade to Clauge Pro</h2>
        <p class="sub">Unlock Clauge AI credits, unlimited coworkers, and premium themes.</p>
      </div>

      {#if loading}
        <p class="muted">Loading pricing…</p>
      {:else if error}
        <p class="err-msg">{error}</p>
      {:else if pricing}
        {@const monthly = pricing.plans.find((p) => p.id === 'monthly')}
        {@const yearly = pricing.plans.find((p) => p.id === 'yearly')}
        <div class="plans">
          {#if monthly}
            <div class="plan-card">
              <h3>Monthly</h3>
              <div class="price">
                {#if monthly.discount}
                  <span class="strike">${monthly.price_usd}</span>
                  <span class="amount">${effectivePrice(monthly).toFixed(2)}</span>
                {:else}
                  <span class="amount">${monthly.price_usd}</span>
                {/if}
                <span class="period">/month</span>
              </div>
              {#if monthly.discount}
                <p class="discount-line">
                  {monthly.discount.percent}% off{#if monthly.discount.code} — use code <strong>{monthly.discount.code}</strong> at checkout{/if}
                </p>
              {/if}
              <button
                class="btn btn-primary"
                onclick={() => startCheckout('monthly')}
                disabled={busyPlan !== null}
              >
                {busyPlan === 'monthly' ? 'Opening…' : 'Upgrade Monthly'}
              </button>
            </div>
          {/if}

          {#if yearly}
            {@const pct = savingsVsMonthly(yearly, monthly)}
            <div class="plan-card highlight">
              <h3>Yearly{pct ? ` · Save ${pct}%` : ''}</h3>
              <div class="price">
                {#if yearly.discount}
                  <span class="strike">${yearly.price_usd}</span>
                  <span class="amount">${effectivePrice(yearly).toFixed(2)}</span>
                {:else}
                  <span class="amount">${yearly.price_usd}</span>
                {/if}
                <span class="period">/year</span>
              </div>
              <p class="muted">${perMonth(yearly).toFixed(2)}/month equivalent</p>
              {#if yearly.discount}
                <p class="discount-line">
                  {yearly.discount.percent}% off{#if yearly.discount.code} — use code <strong>{yearly.discount.code}</strong> at checkout{/if}
                </p>
              {/if}
              <button
                class="btn btn-primary"
                onclick={() => startCheckout('yearly')}
                disabled={busyPlan !== null}
              >
                {busyPlan === 'yearly' ? 'Opening…' : 'Upgrade Yearly'}
              </button>
            </div>
          {/if}
        </div>

        <p class="footer-note muted">
          You'll complete checkout in your browser. We use Polar.sh as our payment processor.
        </p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim-strong, rgba(0, 0, 0, 0.6));
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: var(--z-drawer, 1000);
  }
  .modal {
    background: var(--surface-hover, #1a1a1a);
    border-radius: var(--radius-lg, 10px);
    padding: 2rem;
    min-width: 560px;
    max-width: 90vw;
    color: var(--t1, #ddd);
    font-family: var(--ui);
    position: relative;
  }
  .close-btn {
    position: absolute;
    top: 0.75rem;
    right: 1rem;
    background: transparent;
    border: 0;
    color: var(--t3, #888);
    font-size: 1.5rem;
    cursor: pointer;
    line-height: 1;
  }
  .close-btn:hover {
    color: var(--t1, #fff);
  }
  .head {
    margin-bottom: 1.5rem;
  }
  .head h2 {
    margin: 0 0 0.25rem;
    font-size: 1.5rem;
    font-family: var(--ui);
  }
  .sub {
    margin: 0;
    color: var(--t3);
    font-size: 0.9rem;
  }
  .plans {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
    margin-bottom: 1rem;
  }
  .plan-card {
    padding: 1.25rem;
    border-radius: var(--radius-md, 8px);
    border: 1px solid var(--b1, #2a2a2a);
    background: var(--n2, #0e0e0e);
  }
  .plan-card.highlight {
    border-color: var(--acc, #4a90e2);
  }
  .plan-card h3 {
    margin: 0 0 0.75rem;
    font-size: 1rem;
    font-family: var(--ui);
  }
  .price {
    font-size: 0.9rem;
    margin-bottom: 0.5rem;
    display: flex;
    align-items: baseline;
    gap: 0.25rem;
    flex-wrap: wrap;
  }
  .price .amount {
    font-size: 2rem;
    font-weight: 600;
    color: var(--t1);
  }
  .price .strike {
    text-decoration: line-through;
    color: var(--t3);
    margin-right: 0.25rem;
  }
  .price .period {
    color: var(--t3);
  }
  .discount-line {
    font-size: 0.8rem;
    color: var(--acc, #4a90e2);
    margin: 0.5rem 0 0;
  }
  .btn {
    width: 100%;
    margin-top: 0.75rem;
    padding: 0.625rem 1rem;
    border-radius: var(--radius-md, 6px);
    border: 0;
    cursor: pointer;
    font-size: 0.9rem;
    font-family: var(--ui);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-primary {
    background: var(--acc, #4a90e2);
    color: #fff;
  }
  .muted {
    color: var(--t3, #888);
    font-size: 0.875rem;
    margin: 0.25rem 0;
  }
  .err-msg {
    color: var(--err, #ff6b6b);
    font-size: 0.875rem;
  }
  .footer-note {
    text-align: center;
    margin-top: 1rem;
  }
</style>

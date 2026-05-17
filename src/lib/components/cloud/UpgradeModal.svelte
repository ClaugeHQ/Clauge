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
  let copiedCode = $state(false);

  async function copyDiscountCode(code: string) {
    try {
      await navigator.clipboard.writeText(code);
      copiedCode = true;
      setTimeout(() => { copiedCode = false; }, 2000);
    } catch {
      // Clipboard API may be restricted — silently fail; the code is still visible
    }
  }

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

  // Marketing-positive savings badge:
  // Compare yearly's *effective* price (post any discount) to a full year
  // of monthly *sticker* (no discount on the comparison baseline). This
  // concentrates the discount benefit onto the yearly choice, which is
  // the desired upsell framing.
  function savingsVsMonthly(yearly: Plan, monthly: Plan | undefined): number | null {
    if (!monthly) return null;
    const yearlyEff = effectivePrice(yearly);
    const fullMonthlyYear = monthly.price_usd * 12;
    if (yearlyEff >= fullMonthlyYear) return null;
    const savings = fullMonthlyYear - yearlyEff;
    return Math.round((savings / fullMonthlyYear) * 100);
  }

  // Per-month equivalent of yearly's *discounted* price
  function discountedPerMonth(p: Plan): number {
    return Math.round((effectivePrice(p) / 12) * 100) / 100;
  }
</script>

{#if $upgradeModalOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={close} use:teleportToBody>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modal-wrap" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
      {#if pricing}
        {@const firstDiscount = pricing.plans.find((p) => p.discount)?.discount ?? null}
        {#if firstDiscount}
          <div class="discount-banner">
            <div class="bn-left">
              <svg class="bn-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M20.59 13.41L13.42 20.58a2 2 0 01-2.83 0L2 12V2h10l8.59 8.59a2 2 0 010 2.82z"/>
                <line x1="7" y1="7" x2="7.01" y2="7"/>
              </svg>
              <span>Save <strong>{firstDiscount.percent}%</strong>{#if firstDiscount.code} with code{/if}</span>
            </div>
            {#if firstDiscount.code}
              <button class="code-copy" onclick={() => copyDiscountCode(firstDiscount.code!)} title="Click to copy">
                <span class="code-text">{firstDiscount.code}</span>
                {#if copiedCode}
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <polyline points="20 6 9 17 4 12"/>
                  </svg>
                {:else}
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <rect x="9" y="9" width="13" height="13" rx="2"/>
                    <path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/>
                  </svg>
                {/if}
              </button>
            {/if}
          </div>
        {/if}
      {/if}

      <div class="modal">
      <button class="close-btn" onclick={close} aria-label="Close">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M1 1l12 12M13 1L1 13" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
        </svg>
      </button>

      <div class="head">
        <span class="plan-pill">
          <svg class="pill-icon" width="10" height="10" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <path d="M12 2l2.6 7.4L22 12l-7.4 2.6L12 22l-2.6-7.4L2 12l7.4-2.6L12 2z"/>
          </svg>
          Pro plan
        </span>
        <h2>Upgrade to Clauge Pro</h2>
        <p class="sub">Everything you need for serious development work.</p>
      </div>

      <ul class="feature-list">
        <li>
          <span class="feat-icon" aria-hidden="true">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 3l2 5 5 2-5 2-2 5-2-5-5-2 5-2 2-5z"/>
              <path d="M19 14l.9 2.1 2.1.9-2.1.9-.9 2.1-.9-2.1-2.1-.9 2.1-.9.9-2.1z"/>
            </svg>
          </span>
          <span class="feat-text">
            <strong>Managed AI assistance</strong> <span class="feat-mute">— no API key setup</span>
          </span>
        </li>
        <li>
          <span class="feat-icon" aria-hidden="true">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="9"/>
              <path d="M12 7v10"/>
              <path d="M15 9.5a2.5 2.5 0 00-2.5-2.5h-1a2.5 2.5 0 000 5h1a2.5 2.5 0 010 5h-1A2.5 2.5 0 019 14.5"/>
            </svg>
          </span>
          <span class="feat-text">
            <strong>1,000 credits / month</strong> <span class="feat-mute">· 12,000 / year on yearly plan</span>
          </span>
        </li>
        <li>
          <span class="feat-icon" aria-hidden="true">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="9" cy="8" r="3"/>
              <path d="M3 21v-1a6 6 0 0112 0v1"/>
              <circle cx="17" cy="7" r="2.5"/>
              <path d="M14 16a4.5 4.5 0 018 2.8V20"/>
            </svg>
          </span>
          <span class="feat-text">
            <strong>Unlimited coworkers</strong> <span class="feat-mute">in workspaces (free is capped at 3)</span>
          </span>
        </li>
        <li>
          <span class="feat-icon" aria-hidden="true">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 22a10 10 0 110-20 8 8 0 018 8c0 2-1.5 3-3 3h-2.5a2 2 0 000 4 2 2 0 01-2.5 2"/>
              <circle cx="7.5" cy="11" r="1" fill="currentColor"/>
              <circle cx="11" cy="6.5" r="1" fill="currentColor"/>
              <circle cx="16" cy="9" r="1" fill="currentColor"/>
            </svg>
          </span>
          <span class="feat-text">
            <strong>Premium themes</strong> <span class="feat-mute">— exclusive visual styles</span>
          </span>
        </li>
      </ul>

      {#if loading}
        <p class="status-line muted">Loading pricing…</p>
      {:else if error}
        <p class="status-line err-msg">{error}</p>
      {:else if pricing}
        {@const monthly = pricing.plans.find((p) => p.id === 'monthly')}
        {@const yearly = pricing.plans.find((p) => p.id === 'yearly')}
        {@const pct = yearly ? savingsVsMonthly(yearly, monthly) : null}
        <div class="plans">
          {#if monthly}
            <div class="plan-card">
              <div class="plan-label">MONTHLY</div>
              <div class="price-row">
                <span class="amount">${monthly.discount ? effectivePrice(monthly).toFixed(2) : monthly.price_usd}</span>
                <span class="period">/month</span>
              </div>
              {#if monthly.discount}
                <p class="was-line"><span class="strike">was ${monthly.price_usd.toFixed(2)}</span></p>
              {/if}
              <button
                class="choose-btn outlined"
                onclick={() => startCheckout('monthly')}
                disabled={busyPlan !== null}
              >
                {busyPlan === 'monthly' ? 'Opening…' : 'Choose monthly'}
              </button>
            </div>
          {/if}

          {#if yearly}
            <div class="plan-card highlight">
              {#if pct}
                <span class="save-badge">Save {pct}%</span>
              {/if}
              <div class="plan-label">YEARLY</div>
              <div class="price-row">
                <span class="amount">${yearly.discount ? effectivePrice(yearly).toFixed(2) : yearly.price_usd}</span>
                <span class="period">/year</span>
              </div>
              <p class="per-month">
                {#if yearly.discount}
                  <span class="strike">${yearly.price_usd}</span> · ${discountedPerMonth(yearly).toFixed(2)}/month
                {:else}
                  ${perMonth(yearly).toFixed(2)} / month
                {/if}
              </p>
              <button
                class="choose-btn filled"
                onclick={() => startCheckout('yearly')}
                disabled={busyPlan !== null}
              >
                {busyPlan === 'yearly' ? 'Opening…' : 'Choose yearly'}
              </button>
            </div>
          {/if}
        </div>

        <p class="footer-note">
          <svg class="foot-icon" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <rect x="5" y="11" width="14" height="10" rx="2"/>
            <path d="M8 11V8a4 4 0 018 0v3"/>
          </svg>
          Checkout opens securely in your browser
          <span class="dot">·</span> Cancel anytime
          <span class="dot">·</span> Credits non-refundable once used
        </p>
      {/if}
      </div>
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
    backdrop-filter: blur(2px);
  }

  .modal-wrap {
    width: 560px;
    max-width: 92vw;
    display: flex;
    flex-direction: column;
    color: var(--t1, #ddd);
    font-family: var(--ui);
    border-radius: var(--radius-lg, 14px);
    overflow: hidden;
    border: 1px solid var(--b1, #2a2a2a);
  }
  .discount-banner {
    background: color-mix(in srgb, #22c55e 14%, var(--n2, #0e0e0e));
    color: #4ade80;
    padding: 0.7rem 1rem 0.7rem 1.25rem;
    font-size: 0.85rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    border-bottom: 1px solid color-mix(in srgb, #22c55e 30%, transparent);
  }
  .bn-left {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .discount-banner strong {
    font-weight: 600;
    color: #4ade80;
  }
  .bn-icon { flex: 0 0 auto; }
  .code-copy {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.3rem 0.65rem;
    background: color-mix(in srgb, #22c55e 22%, transparent);
    border: 1px dashed color-mix(in srgb, #4ade80 65%, transparent);
    border-radius: 6px;
    color: #d4f7dc;
    cursor: pointer;
    font-family: var(--mono, ui-monospace, monospace);
    font-size: 0.8rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    transition: background 0.12s, border-color 0.12s;
  }
  .code-copy:hover {
    background: color-mix(in srgb, #22c55e 35%, transparent);
    border-color: #4ade80;
    border-style: solid;
  }
  .code-text { line-height: 1; }
  .modal {
    background: var(--n2, #0e0e0e);
    padding: 2rem 2rem 1.5rem;
    position: relative;
  }

  .close-btn {
    position: absolute;
    top: 1rem;
    right: 1rem;
    width: 28px;
    height: 28px;
    background: var(--surface-hover, #1a1a1a);
    border: 1px solid var(--b1, #2a2a2a);
    border-radius: 6px;
    color: var(--t3, #888);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
  }
  .close-btn:hover { color: var(--t1); border-color: var(--b2, #3a3a3a); }

  /* Header */
  .head { margin-bottom: 1.25rem; }
  .plan-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.25rem 0.75rem;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--acc, #c2185b) 50%, transparent);
    background: color-mix(in srgb, var(--acc, #c2185b) 12%, transparent);
    color: var(--acc, #c2185b);
    font-size: 0.7rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    margin-bottom: 0.85rem;
  }
  .pill-icon {
    display: inline-block;
    flex: 0 0 auto;
  }
  .head h2 {
    margin: 0 0 0.4rem;
    font-size: 1.6rem;
    font-weight: 600;
    font-family: var(--ui);
    letter-spacing: -0.01em;
  }
  .sub {
    margin: 0;
    color: var(--t3, #888);
    font-size: 0.92rem;
  }

  /* Feature list */
  .feature-list {
    list-style: none;
    padding: 0;
    margin: 1.25rem 0 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  .feature-list li {
    display: flex;
    align-items: center;
    gap: 0.85rem;
    font-size: 0.92rem;
    line-height: 1.3;
  }
  .feat-icon {
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    border-radius: 7px;
    background: color-mix(in srgb, var(--acc, #c2185b) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--acc, #c2185b) 35%, transparent);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--acc, #c2185b);
  }
  .feat-text strong {
    color: var(--t1, #ddd);
    font-weight: 600;
  }
  .feat-mute {
    color: var(--t3, #888);
    font-weight: 400;
  }

  /* Plan cards */
  .plans {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.85rem;
    margin-bottom: 1rem;
  }
  .plan-card {
    position: relative;
    padding: 1.1rem 1.1rem 1.1rem;
    border-radius: var(--radius-md, 10px);
    border: 1px solid var(--b1, #2a2a2a);
    background: var(--surface-hover, #161616);
  }
  .plan-card.highlight {
    border-color: var(--acc, #c2185b);
    background: color-mix(in srgb, var(--acc, #c2185b) 6%, var(--n2, #0e0e0e));
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--acc, #c2185b) 30%, transparent);
  }
  .save-badge {
    position: absolute;
    top: -10px;
    right: 12px;
    padding: 0.18rem 0.6rem;
    border-radius: 999px;
    background: var(--acc, #c2185b);
    color: white;
    font-size: 0.68rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
  }
  .plan-label {
    font-size: 0.7rem;
    font-weight: 600;
    letter-spacing: 0.08em;
    color: var(--t3, #888);
    margin-bottom: 0.5rem;
  }
  .price-row {
    display: flex;
    align-items: baseline;
    gap: 0.25rem;
    margin-bottom: 0.4rem;
  }
  .strike {
    text-decoration: line-through;
    color: var(--t3, #888);
    font-size: 0.95rem;
    margin-right: 0.25rem;
  }
  .amount {
    font-size: 2.3rem;
    font-weight: 600;
    color: var(--t1);
    line-height: 1;
    letter-spacing: -0.02em;
  }
  .period {
    color: var(--t3, #888);
    font-size: 0.85rem;
  }
  .per-month {
    margin: 0 0 0.9rem;
    font-size: 0.8rem;
    color: var(--t3, #888);
  }
  .was-line {
    margin: 0 0 0.9rem;
    font-size: 0.8rem;
    color: var(--t3, #888);
  }
  .was-line .strike {
    text-decoration: line-through;
  }
  .per-month .strike {
    text-decoration: line-through;
    color: var(--t3, #888);
    margin-right: 0.15rem;
  }

  .choose-btn {
    width: 100%;
    margin-top: 0.5rem;
    padding: 0.55rem 1rem;
    border-radius: var(--radius-md, 8px);
    cursor: pointer;
    font-size: 0.85rem;
    font-weight: 500;
    font-family: var(--ui);
    transition: opacity 0.12s;
  }
  .choose-btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .choose-btn.outlined {
    background: transparent;
    color: var(--acc, #c2185b);
    border: 1px solid color-mix(in srgb, var(--acc, #c2185b) 60%, transparent);
  }
  .choose-btn.outlined:hover:not(:disabled) {
    background: color-mix(in srgb, var(--acc, #c2185b) 10%, transparent);
  }
  .choose-btn.filled {
    background: var(--acc, #c2185b);
    color: white;
    border: 1px solid var(--acc, #c2185b);
  }
  .choose-btn.filled:hover:not(:disabled) {
    filter: brightness(1.08);
  }

  /* Footer */
  .footer-note {
    text-align: center;
    margin: 0.75rem 0 0;
    font-size: 0.75rem;
    color: var(--t3, #888);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-wrap: wrap;
    gap: 0.25rem;
  }
  .foot-icon {
    display: inline-block;
    flex: 0 0 auto;
    margin-right: 0.25rem;
    opacity: 0.7;
  }
  .dot { opacity: 0.4; margin: 0 0.15rem; }

  .status-line { text-align: center; margin: 1rem 0; }
  .muted { color: var(--t3, #888); }
  .err-msg { color: var(--err, #ff6b6b); }
</style>

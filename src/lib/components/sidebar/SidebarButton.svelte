<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    tip = '',
    label = '',
    active = false,
    dotColor = '',
    id = '',
    onclick = () => {},
    children,
  }: {
    tip?: string;
    label?: string;
    active?: boolean;
    dotColor?: string;
    id?: string;
    onclick?: () => void;
    children: Snippet;
  } = $props();
</script>

<button
  class="sbi"
  class:on={active}
  data-tip={tip || undefined}
  {id}
  onclick={onclick}
>
  <span class="sbi-icon">
    {@render children()}
  </span>
  {#if label}
    <span class="sbi-label">{label}</span>
  {/if}
</button>

<style>
  .sbi {
    width: 60px;
    min-height: 52px;
    border-radius: 8px;
    border: none;
    background: transparent;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    padding: 6px 2px 5px;
    cursor: pointer;
    position: relative;
    transition: background 0.12s;
  }
  .sbi-icon {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    line-height: 0;
  }
  .sbi :global(svg) {
    width: 20px;
    height: 20px;
    stroke: var(--t3);
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: stroke 0.12s;
  }
  .sbi-label {
    font-family: var(--ui);
    font-size: 10px;
    line-height: 1;
    color: var(--t3);
    font-weight: 500;
    letter-spacing: 0.1px;
    transition: color 0.12s;
    user-select: none;
  }
  .sbi:hover {
    background: var(--b-subtle);
  }
  .sbi:hover :global(svg),
  .sbi:hover .sbi-label {
    color: var(--t1);
    stroke: var(--t1);
  }
  .sbi.on :global(svg) {
    stroke: var(--acc);
  }
  .sbi.on .sbi-label {
    color: var(--acc);
    font-weight: 600;
  }
  .sbi.on {
    background: color-mix(in srgb, var(--acc) 10%, transparent);
  }
  /* tooltip — only shown when there's no label visible */
  .sbi[data-tip]:not(:has(.sbi-label)):hover::after {
    content: attr(data-tip);
    position: absolute;
    left: calc(100% + 10px);
    top: 50%;
    transform: translateY(-50%);
    background: var(--n);
    color: var(--t1);
    font-family: var(--ui);
    font-size: 11px;
    padding: 4px 10px;
    border-radius: 5px;
    border: 1px solid var(--b1);
    white-space: nowrap;
    pointer-events: none;
    z-index: 999;
  }
</style>

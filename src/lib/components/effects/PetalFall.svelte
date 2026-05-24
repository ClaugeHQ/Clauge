<script lang="ts">
  // Premium "Petal" theme decoration — 22 small cherry-blossom petals
  // falling diagonally across the viewport while spinning. Each petal has
  // its own duration / drift / rotation profile so they don't fall in
  // lockstep; staggered start delays keep the sky continuously populated.
  //
  // Visibility is bound to the persisted appearance.theme — non-Pro
  // previewers see the plum palette but no petals (the petals are the
  // upgrade carrot).
  //
  // Respects `prefers-reduced-motion: reduce` (petals hidden entirely).
  import { appearance } from "$lib/stores/settings";

  let active = $derived($appearance.theme === "petal");

  // Deterministic per-petal properties — indexed pseudo-random (not
  // Math.random()) so SSR hydration matches and petal positions stay
  // stable across re-renders. Variety across all axes (size, speed,
  // drift, rotation amount, start position) for an organic, never-
  // repeating feel.
  const COUNT = 22;
  const petals = Array.from({ length: COUNT }, (_, i) => ({
    x: ((i * 470) % 1000) / 10,             // 0-100% horizontal start
    size: 9 + ((i * 3) % 6),                // 9-14 px
    dur: 12 + ((i * 7) % 10),               // 12-21s fall
    delay: (i * 19) % 18,                   // 0-17s stagger
    drift: ((i * 31) % 220) - 110,          // -110 to +110 px lateral drift
    rotation: 240 + ((i * 53) % 360),       // 240-600 degrees of spin
    sway: ((i * 11) % 2) === 0 ? 1 : -1,    // sway direction
    // 1-in-4 petal uses the lighter blush variant for visual variety.
    color: i % 4 === 0 ? "#fbcad6" : "#f4a5b8",
  }));
</script>

{#if active}
  <div class="petal-stage" aria-hidden="true">
    {#each petals as p}
      <svg
        class="petal"
        viewBox="0 0 16 24"
        style="left:{p.x}%; width:{p.size}px; height:{p.size * 1.5}px; --dur:{p.dur}s; --delay:{p.delay}s; --drift:{p.drift}px; --rotation:{p.rotation * p.sway}deg;"
      >
        <!-- Cherry petal silhouette: pointed top, rounded base with a
             subtle inner vein for depth. -->
        <path
          d="M 8 0 Q 14 6 14 13 Q 14 21 8 24 Q 2 21 2 13 Q 2 6 8 0 Z"
          fill={p.color}
          opacity="0.9"
        />
        <path
          d="M 8 4 Q 11 9 11 14"
          fill="none"
          stroke="#c97a8d"
          stroke-width="0.6"
          opacity="0.5"
        />
      </svg>
    {/each}
  </div>
{/if}

<style>
  .petal-stage {
    position: fixed;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
    z-index: 5;
  }
  .petal {
    position: absolute;
    top: -40px;
    transform-origin: center;
    will-change: transform, opacity;
    animation: petal-fall var(--dur) linear var(--delay) infinite;
  }
  @keyframes petal-fall {
    0%   { transform: translate3d(0, 0, 0)               rotate(0);                opacity: 0;   }
    8%   { opacity: 0.85; }
    50%  { transform: translate3d(calc(var(--drift) * 0.55), 55vh, 0) rotate(calc(var(--rotation) * 0.55)); opacity: 0.9; }
    100% { transform: translate3d(var(--drift), 110vh, 0) rotate(var(--rotation)); opacity: 0;   }
  }
  @media (prefers-reduced-motion: reduce) {
    .petal-stage { display: none; }
  }
</style>

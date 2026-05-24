<script lang="ts">
  // Premium "Hearth" theme decoration — 28 tiny glowing embers that rise
  // from below the viewport, drift slightly sideways, and fade out before
  // they reach the top. Pure CSS animation (`mix-blend-mode: screen` adds
  // the warm light over dark chrome without obscuring text).
  //
  // Visibility is bound to the persisted appearance.theme — non-Pro
  // previewers see the warm palette but no embers (the embers are the
  // upgrade carrot).
  //
  // Respects `prefers-reduced-motion: reduce` (embers hidden entirely).
  import { appearance } from "$lib/stores/settings";

  let active = $derived($appearance.theme === "hearth");

  // Deterministic per-ember properties — using indexed pseudo-random math
  // (not Math.random()) so SSR hydration matches and ember positions are
  // stable across renders. Goal is a calm, organic distribution: spread
  // across the full viewport width, different speeds, different drift
  // angles, slightly varied sizes.
  const COUNT = 28;
  const embers = Array.from({ length: COUNT }, (_, i) => {
    // Horizontal start position spread evenly with jitter
    const x = ((i * 360) % 1000) / 10 + (i % 3) * 1.2;
    return {
      x,                                  // % horizontal start
      size: 2 + (i * 7) % 4,              // 2-5 px
      dur: 11 + ((i * 13) % 14),          // 11-24s rise duration
      delay: (i * 17) % 22,               // 0-21s start delay (staggers)
      drift: (((i * 23) % 80) - 40),      // -40 to +40 px lateral drift
      hue: i % 5 === 0 ? "#ffd28a" : "#ff8c42", // 1-in-5 is brighter yellow
    };
  });
</script>

{#if active}
  <div class="ember-stage" aria-hidden="true">
    {#each embers as e}
      <span
        class="ember"
        style="left:{e.x}%; width:{e.size}px; height:{e.size}px; --hue:{e.hue}; --dur:{e.dur}s; --delay:{e.delay}s; --drift:{e.drift}px;"
      ></span>
    {/each}
  </div>
{/if}

<style>
  /* Full-viewport canvas. Above the body's firelight glow (z-index 2 on
     body::after) but below the walking critters (which only run on
     Atelier anyway). `mix-blend-mode: screen` on each ember adds light
     to whatever's underneath — perfect for warm sparks over dark chrome. */
  .ember-stage {
    position: fixed;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
    z-index: 5;
  }
  .ember {
    position: absolute;
    bottom: -10px;
    border-radius: 50%;
    background: radial-gradient(
      circle,
      var(--hue) 0%,
      var(--hue) 25%,
      transparent 75%
    );
    box-shadow: 0 0 10px 2px var(--hue);
    opacity: 0;
    mix-blend-mode: screen;
    animation: ember-rise var(--dur) ease-out var(--delay) infinite;
    will-change: transform, opacity;
  }
  @keyframes ember-rise {
    0%   { transform: translate3d(0, 0, 0)                          scale(0.6); opacity: 0;   }
    8%   { transform: translate3d(0, -8vh, 0)                       scale(1);   opacity: 0.9; }
    50%  { transform: translate3d(calc(var(--drift) * 0.5), -50vh, 0) scale(0.95); opacity: 0.75; }
    85%  { transform: translate3d(var(--drift), -85vh, 0)           scale(0.7); opacity: 0.4; }
    100% { transform: translate3d(var(--drift), -105vh, 0)          scale(0.4); opacity: 0;   }
  }
  @media (prefers-reduced-motion: reduce) {
    .ember-stage { display: none; }
  }
</style>

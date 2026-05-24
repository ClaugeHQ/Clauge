<script lang="ts">
  // Premium "Celeste" theme decoration — a cosmic starfield with three
  // distinct layers:
  //   1. 70 ambient pinpoint stars — each independently pulsing opacity
  //      (twinkle) at a different speed
  //   2. EPHEMERAL constellations — up to 3 visible at once, each runs
  //      its own form → visible → fade lifecycle independently in a
  //      different zone of the upper sky. A new one spawns every 5-9s,
  //      total lifetime per constellation is ~14-20s, so the sky always
  //      has 2-3 constellations layered at different stages. Never
  //      repeats the same shape back-to-back, and never spawns into a
  //      zone that's currently occupied.
  //   3. 2 shooting stars — streak diagonally across the viewport at
  //      long, offset intervals so they feel rare and surprising
  //
  // Constellation lifecycle is a JS state machine driven by an $effect;
  // CSS keyframes per phase handle the line draw-in (stroke-dashoffset)
  // and per-star fade-in / pulse / fade-out.
  //
  // Visibility is bound to the persisted appearance.theme — non-Pro
  // previewers see the cosmic palette but no starfield (the starfield
  // is the upgrade carrot).
  //
  // Respects `prefers-reduced-motion: reduce` (everything hidden).
  import { appearance } from "$lib/stores/settings";

  let themeActive = $derived($appearance.theme === "celeste");

  // ─── Layer 1: ambient stars (deterministic, stable across renders) ─
  const STAR_COUNT = 70;
  const stars = Array.from({ length: STAR_COUNT }, (_, i) => ({
    x: ((i * 137) % 1000) / 10,
    y: ((i * 211) % 1000) / 10,
    size: 1 + ((i * 3) % 3),
    dur: 2 + ((i * 7) % 6),
    delay: ((i * 13) % 50) / 10,
    minOpacity: 0.15 + ((i * 5) % 25) / 100,
    maxOpacity: 0.7 + ((i * 11) % 30) / 100,
    color: i % 7 === 0 ? "#ffe6a8" : "#e8e4f5",
    glow: i % 12 === 0,
  }));

  // ─── Layer 2: ephemeral constellations ──────────────────────────────
  type Constellation = { points: [number, number][]; edges: [number, number][] };
  const SHAPES: Constellation[] = [
    // Big Dipper — 7 stars, bowl + handle.
    {
      points: [[0.05, 0.65], [0.18, 0.55], [0.32, 0.6], [0.42, 0.5], [0.6, 0.4], [0.78, 0.25], [0.95, 0.15]],
      edges: [[0,1],[1,2],[2,3],[3,4],[4,5],[5,6],[2,0]],
    },
    // Cassiopeia — 5 stars, flattened W.
    {
      points: [[0, 0.55], [0.22, 0.1], [0.5, 0.55], [0.78, 0.1], [1, 0.55]],
      edges: [[0,1],[1,2],[2,3],[3,4]],
    },
    // Kite — 4 stars in a diamond.
    {
      points: [[0.5, 0], [1, 0.42], [0.5, 1], [0, 0.42]],
      edges: [[0,1],[1,2],[2,3],[3,0]],
    },
    // Hourglass — 5 stars meeting at centre.
    {
      points: [[0, 0], [1, 0], [0.5, 0.5], [0, 1], [1, 1]],
      edges: [[0,1],[1,2],[2,0],[2,3],[2,4],[3,4]],
    },
    // Triangle — 3 stars, the simplest celestial trio.
    {
      points: [[0.05, 0.15], [0.95, 0.35], [0.4, 0.95]],
      edges: [[0,1],[1,2],[2,0]],
    },
    // Arrow — 4 stars, directional zigzag.
    {
      points: [[0.1, 0.7], [0.5, 0.5], [0.9, 0.3], [0.7, 0.7]],
      edges: [[0,1],[1,2],[2,3]],
    },
    // Crown — 5 stars in a fan.
    {
      points: [[0, 0.9], [0.2, 0.3], [0.5, 0.05], [0.8, 0.3], [1, 0.9]],
      edges: [[0,1],[1,2],[2,3],[3,4],[0,4]],
    },
  ];

  // Position zones — biased to corners + sides + top so constellations
  // don't crowd the main work area. Bottom 35% excluded (StatusBar /
  // footer territory).
  type Zone = { topMin: number; topMax: number; leftMin: number; leftMax: number };
  const ZONES: Zone[] = [
    { topMin: 3,  topMax: 16, leftMin: 4,  leftMax: 22 }, // 0 — top-left
    { topMin: 3,  topMax: 12, leftMin: 38, leftMax: 56 }, // 1 — top-centre
    { topMin: 3,  topMax: 16, leftMin: 68, leftMax: 86 }, // 2 — top-right
    { topMin: 28, topMax: 45, leftMin: 4,  leftMax: 22 }, // 3 — mid-left
    { topMin: 28, topMax: 45, leftMin: 70, leftMax: 88 }, // 4 — mid-right
  ];

  type Instance = {
    id: number;
    shape: Constellation;
    topPct: number;
    leftPct: number;
    size: number;
    pulseDur: number;
    phase: "forming" | "visible" | "fading";
    zoneIdx: number;
  };

  const MAX_CONCURRENT = 3;
  let current = $state<Instance[]>([]);
  let seq = 0;
  let lastShapeIdx = -1;

  function pickPhases() {
    return {
      form:    3.0 + Math.random() * 1.2,  // 3.0-4.2s
      visible: 7.0 + Math.random() * 5.0,  // 7.0-12.0s
      fade:    2.5 + Math.random() * 1.5,  // 2.5-4.0s
      gap:     5.0 + Math.random() * 4.0,  // 5.0-9.0s before next spawn
    };
  }

  function pickShape() {
    let idx: number;
    do {
      idx = Math.floor(Math.random() * SHAPES.length);
    } while (idx === lastShapeIdx && SHAPES.length > 1);
    lastShapeIdx = idx;
    return SHAPES[idx];
  }

  // Pick a zone that's not currently occupied. Falls back gracefully if
  // (somehow) all zones are taken — picks the least-recent.
  function pickZone(occupied: Set<number>): { idx: number; zone: Zone } {
    const free = ZONES.map((_, i) => i).filter((i) => !occupied.has(i));
    const candidates = free.length > 0 ? free : ZONES.map((_, i) => i);
    const idx = candidates[Math.floor(Math.random() * candidates.length)];
    return { idx, zone: ZONES[idx] };
  }

  $effect(() => {
    if (!themeActive) {
      current = [];
      return;
    }
    let cancelled = false;
    const timers = new Set<number>();
    const schedule = (fn: () => void, ms: number) => {
      const t = window.setTimeout(() => {
        timers.delete(t);
        if (!cancelled) fn();
      }, ms);
      timers.add(t);
    };

    function setPhase(id: number, phase: Instance["phase"]) {
      const idx = current.findIndex((x) => x.id === id);
      if (idx < 0) return;
      current[idx] = { ...current[idx], phase };
      current = current; // trigger reactivity (array identity unchanged otherwise)
    }

    function spawn() {
      if (cancelled) return;

      // Cap concurrent; if at the cap, try again shortly.
      if (current.length >= MAX_CONCURRENT) {
        schedule(spawn, 1500);
        return;
      }

      const shape = pickShape();
      const { idx: zoneIdx, zone } = pickZone(new Set(current.map((c) => c.zoneIdx)));
      const topPct = zone.topMin + Math.random() * (zone.topMax - zone.topMin);
      const leftPct = zone.leftMin + Math.random() * (zone.leftMax - zone.leftMin);
      const size = 110 + Math.random() * 60;
      const pulseDur = 3.5 + Math.random() * 2.5;
      const phases = pickPhases();

      const id = ++seq;
      const instance: Instance = { id, shape, topPct, leftPct, size, pulseDur, phase: "forming", zoneIdx };
      current = [...current, instance];

      // forming → visible
      schedule(() => setPhase(id, "visible"), phases.form * 1000);
      // visible → fading
      schedule(() => setPhase(id, "fading"), (phases.form + phases.visible) * 1000);
      // fading → remove
      schedule(() => {
        current = current.filter((x) => x.id !== id);
      }, (phases.form + phases.visible + phases.fade) * 1000);

      // Schedule next spawn — INDEPENDENT of this instance's lifecycle so
      // constellations layer over each other rather than running in series.
      schedule(spawn, phases.gap * 1000);
    }

    // First two spawn quickly so the sky doesn't start empty for too long.
    schedule(spawn, 1200);
    schedule(spawn, 4500);

    return () => {
      cancelled = true;
      for (const t of timers) window.clearTimeout(t);
      current = [];
    };
  });

  // ─── Layer 3: shooting stars ────────────────────────────────────────
  const shootingStars = [
    { topPct: 12, leftPct: -10, angle: 18, dur: 14, delay: 4  },
    { topPct: 28, leftPct: -15, angle: 12, dur: 22, delay: 13 },
  ];
</script>

{#if themeActive}
  <div class="sky-stage" aria-hidden="true">
    <!-- Ambient stars -->
    {#each stars as s}
      <span
        class="star"
        class:glow={s.glow}
        style="left:{s.x}%; top:{s.y}%; width:{s.size}px; height:{s.size}px; --hue:{s.color}; --dur:{s.dur}s; --delay:{s.delay}s; --min:{s.minOpacity}; --max:{s.maxOpacity};"
      ></span>
    {/each}

    <!-- Ephemeral constellations — keyed on .id so each instance has its
         own animation lifecycle and re-mount cleanly. -->
    {#each current as inst (inst.id)}
      <svg
        class="constellation phase-{inst.phase}"
        style="top:{inst.topPct}%; left:{inst.leftPct}%; width:{inst.size}px; height:{inst.size}px; --pulse-dur:{inst.pulseDur}s;"
        viewBox="0 0 1 1"
        preserveAspectRatio="none"
      >
        <g class="lines">
          {#each inst.shape.edges as [a, b], li}
            <line
              x1={inst.shape.points[a][0]}
              y1={inst.shape.points[a][1]}
              x2={inst.shape.points[b][0]}
              y2={inst.shape.points[b][1]}
              vector-effect="non-scaling-stroke"
              style="--line-delay: {li * 0.25}s;"
            />
          {/each}
        </g>
        <g class="nodes">
          {#each inst.shape.points as [px, py], ni}
            <circle
              cx={px}
              cy={py}
              r="0.02"
              style="--node-delay: {ni * 0.18}s; --pulse-stagger: {ni * 0.2}s;"
            />
          {/each}
        </g>
      </svg>
    {/each}

    <!-- Shooting stars -->
    {#each shootingStars as ss, i}
      <span
        class="shoot shoot-{i}"
        style="top:{ss.topPct}%; left:{ss.leftPct}%; --angle:{ss.angle}deg; --dur:{ss.dur}s; --delay:{ss.delay}s;"
      ></span>
    {/each}
  </div>
{/if}

<style>
  .sky-stage {
    position: fixed;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
    z-index: 5;
  }

  /* ─── Ambient stars ──────────────────────────────────────────────── */
  .star {
    position: absolute;
    border-radius: 50%;
    background: var(--hue);
    mix-blend-mode: screen;
    will-change: opacity;
    animation: twinkle var(--dur) ease-in-out var(--delay) infinite alternate;
  }
  .star.glow { box-shadow: 0 0 6px 1px var(--hue); }
  @keyframes twinkle {
    0%   { opacity: var(--min); }
    100% { opacity: var(--max); }
  }

  /* ─── Ephemeral constellation ───────────────────────────────────── */
  .constellation {
    position: absolute;
    pointer-events: none;
    mix-blend-mode: screen;
    overflow: visible;
  }

  /* Lines: thin silver. Draw trick — stroke-dasharray needs to be larger
     than any possible line length so the line renders solid (not dotted)
     once fully drawn. With `vector-effect: non-scaling-stroke`, dash
     values are interpreted in SCREEN PIXELS, so 1000px is comfortably
     larger than any constellation line on screen (max ~240px for a
     170px-wide constellation). dashoffset 1000 → 0 = sliding draw-in. */
  .constellation .lines line {
    stroke: #b8c4f0;
    stroke-width: 1;
    stroke-linecap: round;
    fill: none;
    stroke-dasharray: 1000;
    stroke-dashoffset: 1000;
    opacity: 0.32;
  }
  .constellation.phase-forming .lines line {
    animation: line-draw 0.7s ease-out var(--line-delay) forwards;
  }
  .constellation.phase-visible .lines line {
    stroke-dashoffset: 0;
  }
  .constellation.phase-fading .lines line {
    stroke-dashoffset: 0;
    animation: line-fade 2.8s ease-out forwards;
  }
  @keyframes line-draw {
    from { stroke-dashoffset: 1000; }
    to   { stroke-dashoffset: 0; }
  }
  @keyframes line-fade {
    from { opacity: 0.32; }
    to   { opacity: 0; }
  }

  /* Star nodes — brighter than ambient, with halo. Fade in staggered to
     sync with line draw; in `visible` they pulse together on a per-shape
     rhythm; in `fading` they dim out. */
  .constellation .nodes circle {
    fill: #f0eaff;
    filter: drop-shadow(0 0 4px rgba(184, 196, 240, 0.9));
    opacity: 0;
  }
  .constellation.phase-forming .nodes circle {
    animation: node-fadein 0.7s ease-out var(--node-delay) forwards;
  }
  .constellation.phase-visible .nodes circle {
    opacity: 1;
    animation: node-pulse var(--pulse-dur) ease-in-out var(--pulse-stagger) infinite alternate;
  }
  .constellation.phase-fading .nodes circle {
    opacity: 1;
    animation: node-fadeout 2.8s ease-out forwards;
  }
  @keyframes node-fadein {
    from { opacity: 0; transform: scale(0.4); transform-origin: center; }
    to   { opacity: 1; transform: scale(1);   }
  }
  @keyframes node-pulse {
    0%   { opacity: 0.6; }
    100% { opacity: 1;   }
  }
  @keyframes node-fadeout {
    to { opacity: 0; }
  }

  /* ─── Shooting stars ─────────────────────────────────────────────── */
  .shoot {
    position: absolute;
    width: 110px;
    height: 1.5px;
    background: linear-gradient(90deg, transparent 0%, rgba(232, 228, 245, 0) 30%, rgba(232, 228, 245, 0.95) 95%, white 100%);
    border-radius: 1px;
    opacity: 0;
    transform: rotate(var(--angle));
    transform-origin: 100% 50%;
    mix-blend-mode: screen;
    will-change: transform, opacity;
    animation: shoot var(--dur) ease-in var(--delay) infinite;
  }
  @keyframes shoot {
    0%, 88%   { opacity: 0; transform: translate3d(0, 0, 0) rotate(var(--angle)); }
    90%       { opacity: 1; transform: translate3d(0, 0, 0) rotate(var(--angle)); }
    99%       { opacity: 1; transform: translate3d(115vw, 35vh, 0) rotate(var(--angle)); }
    100%      { opacity: 0; transform: translate3d(115vw, 35vh, 0) rotate(var(--angle)); }
  }

  @media (prefers-reduced-motion: reduce) {
    .sky-stage { display: none; }
  }
</style>

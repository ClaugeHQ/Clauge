<script lang="ts">
  import { snapGuides } from '$lib/modes/canvas/stores/snapGuidesStore';
  import { viewport } from '$lib/modes/canvas/stores/canvasStore';

  // Convert canvas-space guide positions to screen-space inline style.
  const positioned = $derived(
    $snapGuides.map((g) => {
      if (g.axis === 'x') {
        const px = g.position * $viewport.zoom + $viewport.offsetX;
        return { axis: g.axis, style: `left: ${px}px;` };
      }
      const py = g.position * $viewport.zoom + $viewport.offsetY;
      return { axis: g.axis, style: `top: ${py}px;` };
    }),
  );
</script>

<div class="cv-overlay">
  {#each positioned as p, i (i)}
    <div class="cv-guide cv-guide-{p.axis}" style={p.style}></div>
  {/each}
</div>

<style>
  .cv-overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
  }
  .cv-guide {
    position: absolute;
    background: var(--cv-guide-color, color-mix(in srgb, var(--acc) 70%, transparent));
  }
  .cv-guide-x {
    top: 0;
    bottom: 0;
    width: 1px;
  }
  .cv-guide-y {
    left: 0;
    right: 0;
    height: 1px;
  }
</style>

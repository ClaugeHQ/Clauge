<script lang="ts">
  import { viewport } from '$lib/modes/canvas/stores/canvasStore';
  const dotSize = 1;
  const baseSpacing = 24;
  // Clamp to [4, 64]px so dots stay visible at min zoom and dense enough at max.
  const spacing = $derived(Math.max(4, Math.min(64, baseSpacing * $viewport.zoom)));
  const offsetX = $derived(((($viewport.offsetX % spacing) + spacing) % spacing));
  const offsetY = $derived(((($viewport.offsetY % spacing) + spacing) % spacing));
</script>

<div
  class="cv-grid"
  style="
    background-image: radial-gradient(circle, var(--cv-grid-dot, rgba(255,255,255,0.05)) {dotSize}px, transparent {dotSize + 0.5}px);
    background-size: {spacing}px {spacing}px;
    background-position: {offsetX}px {offsetY}px;
  "
></div>

<style>
  .cv-grid {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
</style>

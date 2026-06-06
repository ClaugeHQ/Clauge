<script lang="ts">
  import { regionPreview } from '$lib/modes/canvas/stores/canvasStore';
</script>

{#if $regionPreview}
  <div
    class="cv-region-preview"
    style="left: {$regionPreview.x}px; top: {$regionPreview.y}px; width: {$regionPreview.width}px; height: {$regionPreview.height}px; background: color-mix(in srgb, {$regionPreview.color} 18%, transparent); border-color: {$regionPreview.color};"
  ></div>
{/if}

<style>
  .cv-region-preview {
    position: absolute;
    border: 1px dashed;
    border-radius: 10px;
    pointer-events: none;
  }
</style>

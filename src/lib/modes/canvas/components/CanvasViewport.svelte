<script lang="ts">
  import CanvasGrid from './CanvasGrid.svelte';
  import CanvasWorld from './CanvasWorld.svelte';
  import CanvasOverlay from './CanvasOverlay.svelte';
  import CanvasToolbar from './CanvasToolbar.svelte';
  import { pannable } from '$lib/modes/canvas/actions/pannable';
  import { zoomable } from '$lib/modes/canvas/actions/zoomable';
</script>

<!-- Phase 3 TODO: restore per-tile context menus once tile content can host them. -->
<div
  class="cv-viewport"
  use:pannable
  use:zoomable
  oncontextmenu={(e: MouseEvent) => e.preventDefault()}
>
  <CanvasGrid />
  <CanvasWorld />
  <CanvasOverlay />
  <CanvasToolbar />
</div>

<style>
  .cv-viewport {
    position: relative;
    flex: 1;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--cv-bg, var(--n));
    overscroll-behavior: none;
    user-select: none;
  }
</style>

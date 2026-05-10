<script lang="ts">
  // Avatar for a coworker — dicebear-generated SVG, deterministic from
  // (seed, style). Caches the rendered SVG string for the lifetime of
  // the component so re-render with the same props is cheap.
  //
  // We import the styles dynamically by name so the bundle only pulls
  // the styles actually requested (`personas` is the default; user can
  // pick others when we expose the style switcher).

  import { createAvatar } from '@dicebear/core';
  import * as collection from '@dicebear/collection';

  interface Props {
    seed: string;
    style?: string;
    size?: number;
    /** Show a thin ring matching the avatar background — keeps small
     *  avatars legible against any drawer / list background. */
    ring?: boolean;
  }

  let { seed, style = 'personas', size = 32, ring = false }: Props = $props();

  /** Resolve the dicebear style by string. Unknown styles fall back to
   *  `personas` so a stale stored style doesn't crash the UI. */
  function resolveStyle(name: string): unknown {
    const map: Record<string, unknown> = {
      personas: collection.personas,
      bottts: collection.bottts,
      avataaars: collection.avataaars,
      adventurer: collection.adventurer,
      'big-smile': collection.bigSmile,
      identicon: collection.identicon,
      initials: collection.initials,
      lorelei: collection.lorelei,
      micah: collection.micah,
      thumbs: collection.thumbs,
    };
    return map[name] ?? collection.personas;
  }

  const svg = $derived.by(() => {
    try {
      // dicebear's createAvatar takes the style + options; toString()
      // returns inline SVG. Safe to {@html} — we control the input.
      return createAvatar(resolveStyle(style) as never, {
        seed: seed || 'untitled',
        size,
      }).toString();
    } catch (e) {
      console.warn('CoworkerAvatar render failed:', e);
      return '';
    }
  });
</script>

<span
  class="ca"
  class:ca-ring={ring}
  style="width: {size}px; height: {size}px;"
  aria-hidden="true"
>
  {#if svg}
    <!-- eslint-disable-next-line svelte/no-at-html-tags -->
    {@html svg}
  {/if}
</span>

<style>
  .ca {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    flex-shrink: 0;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.05);
    contain: layout paint;
  }
  .ca-ring {
    box-shadow: 0 0 0 1px var(--b1);
  }
  .ca :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }
</style>

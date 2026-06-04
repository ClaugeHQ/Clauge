<script lang="ts">
  // Milkdown Crepe wrapper. Crepe is the "batteries-included" preset
  // that ships a Notion-style editing experience: slash menu, inline
  // formatting on `**` / `*` / headings, code blocks, lists, tables,
  // images, links. We mount it imperatively on a div and forward
  // change events through `onChange`.
  //
  // Crepe owns its own DOM; Svelte just provides the host element.

  import { onMount, onDestroy } from 'svelte';
  import { Crepe } from '@milkdown/crepe';

  // Crepe ships its own theme stylesheets. We import them once at the
  // module level so any number of editor instances on a page share the
  // same styles. The "frame" theme is the closest to Notion's clean
  // light-on-dark look once we layer Clauge tokens on top.
  import '@milkdown/crepe/theme/common/style.css';
  import '@milkdown/crepe/theme/frame-dark.css';

  interface Props {
    value: string;
    onChange?: (markdown: string) => void;
    placeholder?: string;
  }

  let { value, onChange, placeholder = "Type '/' for commands" }: Props = $props();

  let host: HTMLDivElement;
  let crepe: Crepe | null = null;

  /** Persist pasted/dragged images by embedding them as base64 data
   *  URLs in the markdown. Crepe's default upload handler produces
   *  `blob:` URLs which are page-session-scoped — they evaporate on
   *  app restart, leaving broken images. Inlining as a data URL keeps
   *  the image in the same column as the rest of the note content, so
   *  whatever persistence the note has, the image inherits. Trade-off:
   *  bloats the row for large images. Re-evaluate with a file-on-disk
   *  scheme once a note shows real growth pressure. */
  function fileToDataUrl(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as string);
      reader.onerror = () => reject(reader.error ?? new Error('image read failed'));
      reader.readAsDataURL(file);
    });
  }

  onMount(() => {
    crepe = new Crepe({
      root: host,
      defaultValue: value,
      featureConfigs: {
        [Crepe.Feature.Placeholder]: { text: placeholder, mode: 'block' },
        [Crepe.Feature.ImageBlock]: {
          onUpload: fileToDataUrl,
          blockOnUpload: fileToDataUrl,
          inlineOnUpload: fileToDataUrl,
        },
      },
    });
    crepe.create().then(() => {
      // Listen for content changes — Crepe surfaces a Markdown listener
      // through its underlying ctx pipe.
      crepe?.on((listener) => {
        listener.markdownUpdated((_ctx, markdown) => {
          onChange?.(markdown);
        });
      });
    });
  });

  onDestroy(() => {
    crepe?.destroy();
    crepe = null;
  });
</script>

<div class="md-host" bind:this={host}></div>

<style>
  .md-host {
    flex: 1;
    min-height: 0;
    overflow: auto;
    /* Crepe's frame-dark theme uses its own background; Clauge's
       theme tokens flow through where we override below. */
  }
  .md-host::-webkit-scrollbar { width: 6px; }
  .md-host::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 3px; }

  /* Re-skin Crepe to match Clauge tokens. We're targeting the editor
     root that Crepe mounts; :global is needed because the milkdown
     subtree isn't owned by this Svelte component scope. */
  :global(.md-host .milkdown) {
    background: transparent;
    color: var(--t1);
    font-family: var(--ui);
  }
  /* Full-width content. Notion's "Full width" mode equivalent — no
     centred 780px column, lets the editor use the entire panel. */
  :global(.md-host .milkdown .ProseMirror) {
    padding: 12px 0 80px;
    max-width: none;
    margin: 0;
    font-size: 14px;
    line-height: 1.7;
    outline: none;
  }
  /* Crepe wraps content in an inner container with its own padding —
     neutralise it so the inputs stretch edge-to-edge. */
  :global(.md-host .milkdown) { padding: 0 !important; }
  :global(.md-host .milkdown h1),
  :global(.md-host .milkdown h2),
  :global(.md-host .milkdown h3) {
    color: var(--t1);
    letter-spacing: -0.01em;
  }
  :global(.md-host .milkdown code) {
    background: var(--surface-hover);
    color: var(--acc);
    font-family: var(--mono);
    font-size: 12.5px;
    padding: 1px 5px;
    border-radius: 4px;
  }
  :global(.md-host .milkdown pre) {
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid var(--b1);
    border-radius: 6px;
  }
  :global(.md-host .milkdown blockquote) {
    border-left: 2px solid var(--acc);
    color: var(--t2);
  }
  :global(.md-host .milkdown a) {
    color: var(--acc);
  }
  :global(.md-host .milkdown hr) {
    border-color: var(--b1);
  }
  /* Popover surfaces (slash menu, toolbar, link editor, etc.) are
     skinned in src/lib/modes/workspace/milkdown-overrides.css —
     imported globally from app.css because Crepe portals them to
     <body> outside this component's scoped CSS. */
</style>

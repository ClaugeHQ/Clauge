<script lang="ts">
  import FilesBrowser from './FilesBrowser.svelte';
  import { connectionIdFromTabKey } from '$lib/modes/explorer/tabkey';

  interface Props {
    tabId: string;
  }

  let { tabId }: Props = $props();

  const connectionId = $derived(connectionIdFromTabKey(tabId));
</script>

<div class="tile-body">
  {#key tabId}
    <FilesBrowser tabKey={tabId} {connectionId} />
  {/key}
</div>

<style>
  .tile-body {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }
</style>

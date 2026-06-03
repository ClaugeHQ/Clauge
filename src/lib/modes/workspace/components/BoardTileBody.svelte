<script lang="ts">
  import BoardView from './BoardView.svelte';

  let { boardId }: { boardId: string } = $props();
</script>

<div class="board-tile">
  <BoardView {boardId} />
</div>

<style>
  .board-tile {
    width: 100%;
    height: 100%;
    overflow: hidden;
  }
</style>

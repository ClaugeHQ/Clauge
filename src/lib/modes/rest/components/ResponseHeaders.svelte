<script lang="ts">
  interface Props {
    headers: [string, string][];
  }

  let { headers }: Props = $props();
</script>

<div class="resp-headers">
  {#if headers.length === 0}
    <div class="resp-headers-empty">No headers</div>
  {:else}
    <table class="resp-headers-table">
      <tbody>
        {#each headers as [name, value] (name)}
          <tr>
            <td class="rh-key">{name}</td>
            <td class="rh-val">{value}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .resp-headers {
    flex: 1;
    overflow-y: auto;
    background: transparent;
    padding: 8px 0;
  }
  .resp-headers::-webkit-scrollbar { width: 4px; }
  .resp-headers::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .resp-headers-empty {
    padding: 24px 14px;
    font-size: 12px;
    color: var(--t3);
    font-family: var(--mono);
  }

  .resp-headers-table {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--mono);
    font-size: 12px;
  }

  .resp-headers-table td {
    padding: 4px 14px;
    border-bottom: 1px solid #111128;
    vertical-align: top;
  }

  .rh-key {
    color: var(--acc);
    white-space: nowrap;
    width: 1%;
    font-weight: 500;
  }

  .rh-val {
    color: var(--t2);
    word-break: break-all;
  }
</style>

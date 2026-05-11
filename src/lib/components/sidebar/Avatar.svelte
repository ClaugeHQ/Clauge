<script lang="ts">
  import { cloudConnected, cloudUser, cloudDisplayHandle } from '$lib/stores/cloud';
</script>

<div class="avatar-connected">
  <button class="avatar" title={$cloudConnected ? `${$cloudDisplayHandle?.handle ?? ''}` : 'Profile'}>
    {#if $cloudConnected && $cloudUser?.avatarUrl}
      <img class="avatar-img" src={$cloudUser.avatarUrl} alt={$cloudDisplayHandle?.handle ?? ''} />
    {:else if $cloudConnected}
      <span class="avatar-letter">{($cloudDisplayHandle?.handle ?? 'U').charAt(0).toUpperCase()}</span>
    {:else}
      <span class="avatar-letter">CL</span>
    {/if}
  </button>
  {#if $cloudConnected}
    <span class="avatar-dot"></span>
  {/if}
</div>

<style>
  .avatar-connected {
    position: relative;
  }
  .avatar {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--acc), var(--acc));
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 9px;
    font-weight: 700;
    color: #fff;
    cursor: default;
    margin-top: 6px;
    border: none;
    transition: opacity 0.15s;
    font-family: var(--ui);
    overflow: hidden;
    padding: 0;
  }
  .avatar:hover {
    opacity: 0.85;
  }
  .avatar-img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: 50%;
  }
  .avatar-letter {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
  }
  .avatar-dot {
    position: absolute;
    bottom: -1px;
    right: -1px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ok);
    border: 1.5px solid var(--s);
  }
</style>

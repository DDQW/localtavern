<script lang="ts">
  import { lightbox } from "#lib/lightbox.svelte.js";

  function close() {
    lightbox.src = "";
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

{#if lightbox.src}
  <div class="backdrop" role="dialog" aria-modal="true" aria-label="Image viewer" tabindex="-1" onclick={close} onkeydown={() => {}}>
    <img src={lightbox.src} alt="" />
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.82);
    cursor: zoom-out;
    padding: 24px;
  }

  img {
    max-width: 100%;
    max-height: 100%;
    border-radius: var(--radius);
    box-shadow: var(--shadow);
  }
</style>

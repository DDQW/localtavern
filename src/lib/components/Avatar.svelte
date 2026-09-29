<script lang="ts">
  import { fileUrl } from "$lib/media";

  let { name, path = null, size = 36 }: { name: string; path?: string | null; size?: number } = $props();

  let failed = $state(false);
  const src = $derived(fileUrl(path));
  // A path that changes (a new expression, a different chat) deserves a fresh attempt.
  $effect(() => {
    path;
    failed = false;
  });

  const initial = $derived((name.trim()[0] ?? "?").toUpperCase());
  // A stable colour per name, so each speaker is recognisable at a glance.
  const hue = $derived([...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 0));
</script>

{#if src && !failed}
  <img class="avatar" style:width="{size}px" style:height="{size}px" {src} alt={name} onerror={() => (failed = true)} />
{:else}
  <div
    class="avatar fallback"
    style:width="{size}px"
    style:height="{size}px"
    style:font-size="{size * 0.44}px"
    style:background="hsl({hue} 45% 38%)"
    aria-hidden="true"
  >
    {initial}
  </div>
{/if}

<style>
  .avatar {
    border-radius: 50%;
    object-fit: cover;
    flex: none;
    display: grid;
    place-items: center;
  }

  .fallback {
    color: white;
    font-weight: 600;
    user-select: none;
  }
</style>

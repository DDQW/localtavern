<script lang="ts">
  import { goto } from "$app/navigation";
  import { chat } from "$lib/chat.svelte";

  // Straight back into whichever roleplay you were last in; with none yet, pick a character first.
  $effect(() => {
    void chat.openLastRoleplay().then((opened) => {
      void goto(opened && chat.conversationId ? `/chat/${chat.conversationId}` : "/characters", { replaceState: true });
    });
  });
</script>

<p class="muted" style="padding: 24px">Loading…</p>

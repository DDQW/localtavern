<script lang="ts">
  import { goto } from "$app/navigation";
  import { chat } from "$lib/chat.svelte";

  // Whichever roleplay you were last in. A default roleplay persona, when set, is selected on it.
  // With none yet, pick a character first.
  $effect(() => {
    void chat.openLastRoleplay().then((opened) => {
      void goto(opened && chat.conversationId ? `/chat/${chat.conversationId}` : "/characters", { replaceState: true });
    });
  });
</script>

<p class="muted" style="padding: 24px">Loading…</p>

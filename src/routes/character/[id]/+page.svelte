<script lang="ts">
  import { page } from "$app/state";
  import { chat } from "#lib/chat.svelte.js";
  import ChatView from "#lib/components/ChatView.svelte";

  // Opens (resuming the latest chat with) this character; reacts when the picker switches character.
  $effect(() => {
    const id = page.params.id;
    if (id && chat.session?.conversation.characterId !== id) void chat.openCharacter(id);
  });
</script>

<ChatView />

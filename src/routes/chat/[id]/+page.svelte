<script lang="ts">
  import { page } from "$app/state";
  import { chat } from "#lib/chat.svelte.js";
  import ChatView from "#lib/components/ChatView.svelte";

  // A specific conversation, e.g. picked from History.
  $effect(() => {
    const id = page.params.id;
    if (id && chat.conversationId !== id) void chat.resume(id);
  });
</script>

<ChatView />

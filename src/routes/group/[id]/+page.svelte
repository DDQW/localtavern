<script lang="ts">
  import { page } from "$app/state";
  import { chat } from "$lib/chat.svelte";
  import ChatView from "$lib/components/ChatView.svelte";

  // Opens (resuming the latest chat of) this group.
  $effect(() => {
    const id = page.params.id;
    if (id && chat.session?.conversation.groupChatId !== id) void chat.openGroup(id);
  });
</script>

<ChatView />

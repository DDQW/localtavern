<script lang="ts">
  import { chat } from "$lib/chat.svelte";
  import ChatView from "$lib/components/ChatView.svelte";

  // The Assistant resumes your latest assistant chat. If one is already open (say, picked from
  // History), keep it rather than jumping to another.
  $effect(() => {
    if (chat.session?.conversation.mode !== "assistant") void chat.openAssistant();
  });
</script>

<ChatView />

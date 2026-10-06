<script lang="ts">
  import { api } from "$lib/api";
  import { chat } from "$lib/chat.svelte";
  import ChatView from "$lib/components/ChatView.svelte";

  // Entering Assistant opens the latest assistant chat, and selects the assistant persona when one
  // is set. A header change after that is left alone until the next visit.
  let opened = false;
  $effect(() => {
    if (opened) return;
    opened = true;
    const current = chat.session?.conversation;
    void (async () => {
      if (current?.mode === "assistant" && current.id) {
        const defaults = await api.getDefaultPersonas().catch(() => null);
        const wanted = defaults?.assistantPersonaId;
        if (wanted && current.personaId !== wanted) await chat.setPersona(wanted);
        return;
      }
      await chat.openAssistant();
    })();
  });
</script>

<ChatView />

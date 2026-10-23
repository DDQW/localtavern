<script lang="ts">
  // A group chat and its private talks as one place: a tab for the group, one per private talk,
  // and a picker to start a talk with a member who has none yet.
  import { goto } from "$app/navigation";
  import { chat } from "#lib/chat.svelte.js";
  import Avatar from "./Avatar.svelte";

  const room = $derived(chat.session?.room ?? null);
  const current = $derived(chat.conversationId);
  const member = (id: string) => room?.members.find((m) => m.id === id);
  const free = $derived(room ? room.members.filter((m) => !room.talks.some((t) => t.characterId === m.id)) : []);

  function open(id: string) {
    if (id !== current) void goto(`/chat/${id}`);
  }
</script>

{#if room}
  <nav class="room" aria-label="Group chat and private talks">
    <button class="tab" class:on={current === room.groupConversationId} onclick={() => open(room.groupConversationId)} title="The group chat">
      <span aria-hidden="true">👥</span> {room.title}
      {#if chat.unseenRooms.includes(room.groupConversationId)}
        <span class="news" role="img" aria-label="something happened in the group" title="The group noticed something"></span>
      {/if}
    </button>
    {#each room.talks as talk (talk.conversationId)}
      {@const who = member(talk.characterId)}
      <button
        class="tab"
        class:on={current === talk.conversationId}
        onclick={() => open(talk.conversationId)}
        title="Private talk with {who?.name ?? 'a former member'}"
      >
        <Avatar name={who?.name ?? "?"} path={who?.avatarPath ?? null} size={18} />
        {who?.name ?? "Former member"}
        <span class="lock" aria-label="private">🔒</span>
      </button>
    {/each}
    {#if free.length > 0}
      <select
        class="add"
        value=""
        aria-label="Start a private talk with"
        title="Talk to one member alone - the others won't hear it"
        onchange={(e) => {
          const id = e.currentTarget.value;
          e.currentTarget.value = "";
          if (id) void chat.openBreakout(id);
        }}
      >
        <option value="">＋ Private talk…</option>
        {#each free as m (m.id)}<option value={m.id}>{m.name}</option>{/each}
      </select>
    {/if}
  </nav>
{/if}

<style>
  .room {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 999px;
    font-size: 13px;
    background: transparent;
    border: 1px solid var(--border);
    color: var(--muted);
  }

  .tab:hover {
    color: inherit;
  }

  .tab.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
  }

  .news {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 2px var(--bg, transparent);
  }

  .tab.on .news {
    background: var(--accent-text);
  }

  .lock {
    font-size: 11px;
    opacity: 0.8;
  }

  .add {
    font-size: 13px;
    padding: 3px 8px;
    border-radius: 999px;
  }
</style>

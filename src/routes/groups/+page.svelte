<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, errorText } from "#lib/api.js";
  import { chat } from "#lib/chat.svelte.js";
  import type { Character, GroupActivationMode, GroupChat } from "#lib/types.js";

  const MODES: Record<GroupActivationMode, string> = {
    manual: "You pick who replies",
    roundRobin: "Take turns",
    triggerWord: "Whoever is named",
  };

  let groups = $state<GroupChat[]>([]);
  let characters = $state<Character[]>([]);
  let error = $state("");
  let loaded = $state(false);

  $effect(() => {
    void (async () => {
      try {
        [groups, characters] = await Promise.all([api.listGroupChats(), api.listCharacters()]);
      } catch (e) {
        error = errorText(e);
      } finally {
        loaded = true;
      }
    })();
  });

  const names = (group: GroupChat) =>
    group.characterIds.map((id) => characters.find((c) => c.id === id)?.name ?? "(deleted)").join(", ") || "no members";

  async function open(group: GroupChat) {
    if (await chat.openGroup(group.id)) await goto(`/chat/${chat.conversationId}`);
  }

  async function remove(group: GroupChat) {
    if (!confirm(`Delete the group “${group.name}”? Its conversations are kept.`)) return;
    try {
      await api.deleteGroupChat(group.id);
      groups = groups.filter((g) => g.id !== group.id);
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<section class="page">
  <header>
    <h1>Group Chats</h1>
    <span class="grow"></span>
    <a class="newbtn" href="/groups/new">＋ New group</a>
  </header>
  <p class="muted intro">Chat with several characters at once. The activation mode decides who replies after each of your messages.</p>

  {#if error}<p class="notice">{error}</p>{/if}
  {#if loaded && groups.length === 0}<p class="muted">No group chats yet.</p>{/if}

  <ul class="list">
    {#each groups as group (group.id)}
      <li class="card listrow">
        <div class="grow">
          <div class="title">{group.name} <span class="pill">{MODES[group.activationMode]}</span></div>
          <div class="muted meta">{names(group)}</div>
        </div>
        <button onclick={() => open(group)}>Open chat</button>
        <a class="linkbtn" href="/groups/{group.id}">Edit</a>
        <button class="ghost danger" onclick={() => remove(group)}>Delete</button>
      </li>
    {/each}
  </ul>
</section>

<style>
  .grow {
    flex: 1;
  }

  .intro {
    margin-top: -8px;
    max-width: 700px;
  }

  .meta {
    font-size: 12px;
  }

  .newbtn,
  .linkbtn {
    display: inline-block;
    text-decoration: none;
    border: 1px solid var(--border);
    background: var(--panel-2);
    color: var(--text);
    border-radius: var(--radius-sm);
    padding: 6px 12px;
  }

  .newbtn {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }
</style>

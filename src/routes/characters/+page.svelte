<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, errorText } from "$lib/api";
  import { chat } from "$lib/chat.svelte";
  import Avatar from "$lib/components/Avatar.svelte";
  import type { Character } from "$lib/types";

  let characters = $state<Character[]>([]);
  let query = $state("");
  let error = $state("");
  let loaded = $state(false);

  $effect(() => {
    api
      .listCharacters()
      .then((list) => (characters = list))
      .catch((e) => (error = errorText(e)))
      .finally(() => (loaded = true));
  });

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return characters;
    return characters.filter(
      (c) => c.name.toLowerCase().includes(q) || c.tags.some((t) => t.toLowerCase().includes(q)),
    );
  });

  async function startFresh(character: Character) {
    if (await chat.openCharacter(character.id, true)) await goto(`/chat/${chat.conversationId}`);
  }
</script>

<section class="page">
  <header>
    <h1>Characters</h1>
    <input type="search" placeholder="Search name or tag…" bind:value={query} aria-label="Search characters" />
    <span class="muted count">{characters.length} characters</span>
  </header>

  {#if error}<p class="err">{error}</p>{/if}

  {#if loaded && characters.length === 0 && !error}
    <p class="muted">No characters yet. Importing and editing characters is being ported — for now, existing ones from the original app appear here.</p>
  {/if}

  <div class="grid">
    {#each filtered as character (character.id)}
      <article class="card">
        <a class="open" href="/character/{character.id}" aria-label="Chat with {character.name}">
          <Avatar name={character.name} path={character.avatarPath} size={64} />
          <div class="info">
            <h2>{character.name}</h2>
            <p class="desc">{character.description || character.scenario || "No description"}</p>
            {#if character.tags.length > 0}
              <div class="tags">
                {#each character.tags.slice(0, 4) as tag (tag)}<span>{tag}</span>{/each}
              </div>
            {/if}
          </div>
        </a>
        <button class="ghost" onclick={() => startFresh(character)} title="Start a new conversation with {character.name}">＋ New chat</button>
      </article>
    {/each}
  </div>
</section>

<style>
  header input {
    width: 260px;
  }

  .count {
    margin-left: auto;
  }

  .err {
    color: var(--danger);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 14px;
  }

  .card {
    display: flex;
    flex-direction: column;
    padding: 14px;
    gap: 6px;
  }

  .open {
    display: flex;
    gap: 14px;
    text-decoration: none;
    color: inherit;
    border-radius: var(--radius-sm);
  }

  .open:hover h2 {
    color: var(--accent);
  }

  .info {
    min-width: 0;
  }

  h2 {
    font-size: 16px;
  }

  .desc {
    margin: 2px 0 6px;
    color: var(--muted);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .tags span {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--panel-3);
    color: var(--muted);
  }

  .card > button {
    align-self: flex-end;
  }
</style>

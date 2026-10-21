<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, errorText } from "$lib/api";
  import { chat } from "$lib/chat.svelte";
  import Avatar from "$lib/components/Avatar.svelte";
  import { pickCard } from "$lib/pick";
  import type { Character } from "$lib/types";

  let characters = $state<Character[]>([]);
  let query = $state("");
  let error = $state("");
  let loaded = $state(false);
  let importing = $state(false);

  async function load() {
    try {
      characters = await api.listCharacters();
    } catch (e) {
      error = errorText(e);
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    void load();
  });

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return characters;
    return characters.filter((c) => c.name.toLowerCase().includes(q) || c.tags.some((t) => t.toLowerCase().includes(q)));
  });

  async function startFresh(character: Character) {
    if (await chat.openCharacter(character.id, true)) await goto(`/chat/${chat.conversationId}`);
  }

  async function importCard() {
    error = "";
    try {
      const path = await pickCard();
      if (!path) return;
      importing = true;
      const imported = await api.importCharacter(path);
      await load();
      await goto(`/characters/${imported.id}`);
    } catch (e) {
      error = `Import failed: ${errorText(e)}`;
    } finally {
      importing = false;
    }
  }

  async function remove(character: Character) {
    if (!confirm(`Delete ${character.name}? Their conversations are kept but lose the character.`)) return;
    try {
      await api.deleteCharacter(character.id);
      characters = characters.filter((c) => c.id !== character.id);
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<section class="page">
  <header>
    <h1>Characters</h1>
    <input type="search" placeholder="Search name or tag…" bind:value={query} aria-label="Search characters" />
    <span class="grow"></span>
    <span class="muted">{characters.length} characters</span>
    <button onclick={importCard} disabled={importing} title="Import a SillyTavern .json or .png character card">
      {importing ? "Importing…" : "⤓ Import card"}
    </button>
    <a class="newbtn" href="/characters/new">＋ New character</a>
  </header>

  {#if error}<p class="notice">{error}</p>{/if}

  {#if loaded && characters.length === 0 && !error}
    <p class="muted">No characters yet — create one, or import a SillyTavern card (.json or .png).</p>
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
        <div class="btnrow tools">
          <a class="linkbtn" href="/characters/{character.id}">Edit</a>
          <button class="ghost" onclick={() => startFresh(character)} title="Start a new conversation with {character.name}">＋ New chat</button>
          <span class="grow"></span>
          <button class="ghost danger" onclick={() => remove(character)}>Delete</button>
        </div>
      </article>
    {/each}
  </div>
</section>

<style>
  header input {
    width: 260px;
    max-width: 100%;
  }

  .grow {
    flex: 1;
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

  .linkbtn {
    padding: 4px 10px;
    font-size: 13px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(300px, 100%), 1fr));
    gap: 14px;
  }

  .card {
    display: flex;
    flex-direction: column;
    padding: 14px;
    gap: 8px;
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

  .tools {
    border-top: 1px solid var(--border);
    padding-top: 8px;
  }
</style>

<script lang="ts">
  import { api, errorText } from "#lib/api.js";
  import type { Lorebook } from "#lib/types.js";

  let books = $state<Lorebook[]>([]);
  let error = $state("");
  let loaded = $state(false);

  $effect(() => {
    api
      .listLorebooks()
      .then((list) => (books = list))
      .catch((e) => (error = errorText(e)))
      .finally(() => (loaded = true));
  });

  async function remove(book: Lorebook) {
    if (!confirm(`Delete the lorebook “${book.name}” and its ${book.entries.length} entries?`)) return;
    try {
      await api.deleteLorebook(book.id);
      books = books.filter((b) => b.id !== book.id);
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<section class="page">
  <header>
    <h1>Lorebooks</h1>
    <span class="grow"></span>
    <a class="newbtn" href="/lorebooks/new">＋ New lorebook</a>
  </header>
  <p class="muted intro">World info: entries that are added to the prompt when their keywords appear in the recent chat. Link a lorebook to a character in the character editor.</p>

  {#if error}<p class="notice">{error}</p>{/if}
  {#if loaded && books.length === 0}<p class="muted">No lorebooks yet.</p>{/if}

  <ul class="list">
    {#each books as book (book.id)}
      <li class="card listrow">
        <div class="grow">
          <div class="title">{book.name}</div>
          <div class="muted meta">{book.entries.length} entries · scans the last {book.scanDepth} messages · {book.tokenBudget} token budget</div>
        </div>
        <a class="linkbtn" href="/lorebooks/{book.id}">Edit</a>
        <button class="ghost danger" onclick={() => remove(book)}>Delete</button>
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

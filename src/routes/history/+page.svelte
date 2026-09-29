<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, errorText } from "$lib/api";
  import type { Conversation, SearchHit } from "$lib/types";

  let conversations = $state<Conversation[]>([]);
  let hits = $state<SearchHit[]>([]);
  let query = $state("");
  let error = $state("");
  let searching = $state(false);

  async function load() {
    try {
      conversations = await api.listConversations();
    } catch (e) {
      error = errorText(e);
    }
  }

  $effect(() => {
    void load();
  });

  // Search as you type, but wait for a pause so each keystroke isn't a query.
  $effect(() => {
    const q = query.trim();
    if (!q) {
      hits = [];
      return;
    }
    searching = true;
    const timer = setTimeout(async () => {
      try {
        hits = await api.searchMessages(q);
        error = "";
      } catch (e) {
        error = errorText(e);
      } finally {
        searching = false;
      }
    }, 250);
    return () => clearTimeout(timer);
  });

  async function remove(conversation: Conversation) {
    if (!confirm(`Delete “${conversation.title || "Untitled chat"}” and all its messages?`)) return;
    try {
      await api.deleteConversation(conversation.id);
      conversations = conversations.filter((c) => c.id !== conversation.id);
    } catch (e) {
      error = errorText(e);
    }
  }

  const when = (iso: string) => new Date(iso).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
  const snippet = (text: string) => (text.length > 220 ? `${text.slice(0, 220)}…` : text);
</script>

<section class="page">
  <header>
    <h1>History</h1>
    <input type="search" placeholder="Search all messages…" bind:value={query} aria-label="Search all messages" />
    <span class="muted count">{conversations.length} conversations</span>
  </header>

  {#if error}<p class="err">{error}</p>{/if}

  {#if query.trim()}
    {#if searching && hits.length === 0}
      <p class="muted">Searching…</p>
    {:else if hits.length === 0}
      <p class="muted">No messages match “{query.trim()}”.</p>
    {/if}
    <ul class="list">
      {#each hits as hit (hit.message.id)}
        <li class="card row">
          <a class="main" href="/chat/{hit.message.conversationId}">
            <div class="title">{hit.conversationTitle || "Untitled chat"} <span class="muted">· {hit.message.role === "user" ? "you" : "reply"} · {when(hit.message.timestamp)}</span></div>
            <div class="muted text">{snippet(hit.message.text)}</div>
          </a>
        </li>
      {/each}
    </ul>
  {:else}
    <ul class="list">
      {#each conversations as conversation (conversation.id)}
        <li class="card row">
          <a class="main" href="/chat/{conversation.id}">
            <div class="title">{conversation.title || "Untitled chat"}</div>
            <div class="muted meta">
              <span class="badge">{conversation.mode}</span>
              {conversation.providerId}{conversation.model ? ` · ${conversation.model}` : ""} · {when(conversation.updatedAt)}
            </div>
          </a>
          <button class="ghost danger" onclick={() => remove(conversation)} aria-label="Delete conversation">Delete</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  header input {
    width: 300px;
  }

  .count {
    margin-left: auto;
  }

  .err {
    color: var(--danger);
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 900px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px 4px 0;
  }

  .main {
    flex: 1;
    min-width: 0;
    padding: 8px 14px;
    color: inherit;
    text-decoration: none;
    border-radius: var(--radius-sm);
  }

  .main:hover .title {
    color: var(--accent);
  }

  .title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    font-size: 12px;
    margin-top: 2px;
  }

  .text {
    font-size: 13px;
    margin-top: 2px;
  }

  .badge {
    text-transform: capitalize;
    padding: 0 7px;
    border-radius: 999px;
    background: var(--panel-3);
    margin-right: 6px;
  }
</style>

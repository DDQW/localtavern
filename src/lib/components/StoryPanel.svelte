<script lang="ts">
  import { chat } from "$lib/chat.svelte";

  const world = $derived(chat.session?.conversation.worldState ?? null);
  const story = $derived(chat.story);
  const cast = $derived(chat.session?.characters ?? []);

  const when = $derived.by(() => {
    if (!world) return "";
    // The in-story clock has no time zone; it parses (and so prints) as the same wall-clock time.
    return new Date(world.fictionalDateTime).toLocaleString(undefined, { dateStyle: "full", timeStyle: "short" });
  });

  const active = $derived(story?.quests.filter((q) => q.status === "active") ?? []);
  const completed = $derived(story?.quests.filter((q) => q.status === "completed") ?? []);

  function notesFor(characterId: string) {
    return story?.notebook.filter((n) => n.characterId === characterId) ?? [];
  }

  const weatherLabel = (w: string) => w.replace(/([a-z])([A-Z])/g, "$1 $2");
</script>

<aside class="panel" aria-label="Story">
  <details open>
    <summary>World</summary>
    {#if world}
      <p class="time">{when}</p>
      <p class="muted">
        {weatherLabel(world.weather)}{#if world.weatherNotes}, {world.weatherNotes}{/if}
      </p>
      <p class="muted small">
        +{world.defaultAdvanceMinutesPerMessage} min per message{world.autoDriftWeatherEnabled ? " · weather drifts" : ""}
      </p>
    {:else}
      <p class="muted small">No in-story clock for this chat.</p>
    {/if}
  </details>

  <details open>
    <summary>Quests</summary>
    {#if active.length === 0 && completed.length === 0}
      <p class="muted small">Nothing tracked yet.</p>
    {/if}
    {#each active as quest (quest.id)}
      <div class="item">
        <strong>{quest.title}</strong>
        <div class="muted small">{quest.description}</div>
      </div>
    {/each}
    {#if completed.length > 0}
      <h3>Completed</h3>
      {#each completed as quest (quest.id)}
        <div class="item done">{quest.title}</div>
      {/each}
    {/if}
  </details>

  <details open>
    <summary>Notebook</summary>
    {#each cast as character (character.id)}
      {@const notes = notesFor(character.id)}
      {#if notes.length > 0}
        <h3>{character.name}</h3>
        {#each notes as note (note.id)}
          <div class="item small">{note.content}</div>
        {/each}
      {/if}
    {/each}
    {#if !story || story.notebook.length === 0}
      <p class="muted small">No notes yet.</p>
    {/if}
  </details>

  <p class="muted small footnote">Read-only for now — quests and notes are filled in automatically once extraction is ported.</p>
</aside>

<style>
  .panel {
    width: 300px;
    flex: none;
    overflow-y: auto;
    padding: 12px 16px 24px;
    border-left: 1px solid var(--border);
    background: var(--panel);
  }

  details {
    margin-bottom: 14px;
  }

  summary {
    cursor: pointer;
    font-weight: 600;
    padding: 4px 0;
    user-select: none;
  }

  h3 {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
    margin: 10px 0 4px;
  }

  p {
    margin: 4px 0;
  }

  .time {
    font-weight: 500;
  }

  .small {
    font-size: 12px;
  }

  .item {
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }

  .item.done {
    color: var(--muted);
    text-decoration: line-through;
  }

  .footnote {
    margin-top: 20px;
  }
</style>

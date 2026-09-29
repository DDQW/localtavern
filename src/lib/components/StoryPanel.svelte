<script lang="ts">
  import { chat } from "$lib/chat.svelte";
  import type { WeatherCondition } from "$lib/types";
  import { addMinutes } from "$lib/util";

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
  const WEATHER: WeatherCondition[] = ["Clear", "PartlyCloudy", "Overcast", "Rain", "Thunderstorm", "Snow", "Fog", "Windy"];
  const JUMPS = [
    { label: "+1 hour", minutes: 60 },
    { label: "+6 hours", minutes: 360 },
    { label: "+1 day", minutes: 1440 },
  ];

  // The clock is frozen while a reply is being written: that reply is about to advance it itself.
  const locked = $derived(chat.busy);

  function setTime(e: Event & { currentTarget: HTMLInputElement }) {
    const value = e.currentTarget.value; // "YYYY-MM-DDTHH:mm", empty while the field is half-typed
    if (value) void chat.updateWorld({ fictionalDateTime: `${value}:00` });
  }

  function setMinutes(e: Event & { currentTarget: HTMLInputElement }) {
    const minutes = Math.round(Number(e.currentTarget.value));
    if (Number.isFinite(minutes) && minutes >= 1) void chat.updateWorld({ defaultAdvanceMinutesPerMessage: minutes });
    else e.currentTarget.value = String(world?.defaultAdvanceMinutesPerMessage ?? 5);
  }
</script>

<aside class="panel" aria-label="Story">
  <details open>
    <summary>World</summary>
    {#if world}
      <p class="time">{when}</p>
      <div class="clock" class:locked>
        <label class="field">
          Date and time
          <input type="datetime-local" value={world.fictionalDateTime.slice(0, 16)} disabled={locked} onchange={setTime} />
        </label>
        <div class="jumps">
          {#each JUMPS as jump (jump.label)}
            <button
              disabled={locked}
              onclick={() => chat.updateWorld({ fictionalDateTime: addMinutes(world.fictionalDateTime, jump.minutes) })}
            >
              {jump.label}
            </button>
          {/each}
        </div>
        <label class="field">
          Weather
          <select value={world.weather} disabled={locked} onchange={(e) => chat.updateWorld({ weather: e.currentTarget.value as WeatherCondition })}>
            {#each WEATHER as w (w)}<option value={w}>{weatherLabel(w)}</option>{/each}
          </select>
        </label>
        <label class="field">
          Weather notes
          <input
            value={world.weatherNotes ?? ""}
            placeholder="light drizzle, cold wind…"
            disabled={locked}
            onchange={(e) => chat.updateWorld({ weatherNotes: e.currentTarget.value })}
          />
        </label>
        <label class="field">
          Minutes per message (when the reply gives no time)
          <input type="number" min="1" step="1" value={world.defaultAdvanceMinutesPerMessage} disabled={locked} onchange={setMinutes} />
        </label>
        <label class="check">
          <input
            type="checkbox"
            checked={world.autoDriftWeatherEnabled}
            disabled={locked}
            onchange={(e) => chat.updateWorld({ autoDriftWeatherEnabled: e.currentTarget.checked })}
          />
          Auto-drift weather over time
        </label>
      </div>
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

  <p class="muted small footnote">Quests and notes are read-only for now — they fill in automatically once extraction is ported.</p>
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

  .clock {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
    font-size: 12px;
  }

  .clock.locked {
    opacity: 0.6;
  }

  .jumps {
    display: flex;
    gap: 6px;
  }

  .jumps button {
    flex: 1;
    font-size: 11px;
    padding: 4px 6px;
  }

  .footnote {
    margin-top: 20px;
  }
</style>

<script lang="ts">
  import { api } from "$lib/api";
  import { chat } from "$lib/chat.svelte";
  import type { Country, HolidayOverview, WeatherCondition } from "$lib/types";
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

  // ---- setting and holidays ----

  let countries = $state<Country[]>([]);
  let overview = $state<HolidayOverview | null>(null);
  let newHoliday = $state({ name: "", date: "", everyYear: true });

  $effect(() => {
    api.holidayCountries().then((list) => (countries = list), () => (countries = []));
  });

  // Reload whenever the place, the story date or the story's own holidays change.
  const overviewKey = $derived(
    world ? JSON.stringify([chat.conversationId, world.countryCode, world.regionCode, world.fictionalDateTime.slice(0, 10), world.customHolidays]) : "",
  );
  $effect(() => {
    const id = chat.conversationId;
    if (!overviewKey || !id) {
      overview = null;
      return;
    }
    let stale = false;
    api.holidayOverview(id).then(
      (o) => !stale && (overview = o),
      () => !stale && (overview = null),
    );
    return () => (stale = true);
  });

  const storyDate = $derived(world?.fictionalDateTime.slice(0, 10) ?? "");
  const todays = $derived(overview?.holidays.filter((h) => h.date === storyDate) ?? []);
  const upcoming = $derived(overview?.holidays.filter((h) => h.date > storyDate).slice(0, 5) ?? []);
  const dayLabel = $derived.by(() => {
    if (!world) return "";
    if (todays.some((h) => h.dayOff)) return `Public holiday: ${todays.map((h) => h.name).join(" / ")}`;
    const weekday = new Date(`${storyDate}T12:00:00`).getDay();
    const base = weekday === 0 || weekday === 6 ? "Weekend" : "Weekday";
    return todays.length ? `${base} · ${todays.map((h) => h.name).join(" / ")}` : base;
  });
  const shortDate = (date: string) => new Date(`${date}T12:00:00`).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });

  function addCustomHoliday() {
    if (!world || !newHoliday.name.trim() || !newHoliday.date) return;
    const [y, m, d] = newHoliday.date.split("-").map(Number);
    const holiday = { name: newHoliday.name.trim(), month: m, day: d, year: newHoliday.everyYear ? null : y };
    void chat.updateWorld({ customHolidays: [...world.customHolidays, holiday] });
    newHoliday = { name: "", date: "", everyYear: true };
  }

  function removeCustomHoliday(index: number) {
    if (!world) return;
    void chat.updateWorld({ customHolidays: world.customHolidays.filter((_, i) => i !== index) });
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

      <h3>Setting</h3>
      <div class="clock" class:locked>
        <p class="day" class:holiday={todays.some((h) => h.dayOff)}>{dayLabel}</p>
        <label class="field">
          Country
          {#if countries.length}
            <select value={world.countryCode ?? ""} disabled={locked} onchange={(e) => chat.updateWorld({ countryCode: e.currentTarget.value || null, regionCode: null })}>
              <option value="">Not set</option>
              {#each countries as c (c.countryCode)}<option value={c.countryCode}>{c.name}</option>{/each}
            </select>
          {:else}
            <input
              maxlength="2"
              placeholder="e.g. DE"
              value={world.countryCode ?? ""}
              disabled={locked}
              onchange={(e) => chat.updateWorld({ countryCode: e.currentTarget.value.trim().toUpperCase() || null, regionCode: null })}
            />
          {/if}
        </label>
        {#if overview?.regions.length}
          <label class="field">
            Region (for regional holidays)
            <select value={world.regionCode ?? ""} disabled={locked} onchange={(e) => chat.updateWorld({ regionCode: e.currentTarget.value || null })}>
              <option value="">Nationwide only</option>
              {#each overview.regions as r (r)}<option value={r}>{r}</option>{/each}
            </select>
          </label>
        {/if}
        {#if upcoming.length}
          <div class="small">
            <span class="muted">Coming up:</span>
            <ul class="holidays">
              {#each upcoming as h (h.date + h.name)}<li><span class="muted">{shortDate(h.date)}</span> {h.name}{h.dayOff ? "" : " (not a day off)"}</li>{/each}
            </ul>
          </div>
        {:else if world.countryCode}
          <p class="muted small">No holidays in the next two weeks.</p>
        {/if}

        <details class="custom">
          <summary class="small">Story holidays ({world.customHolidays.length})</summary>
          {#each world.customHolidays as h, i (i)}
            <div class="customrow small">
              <span>{h.name} <span class="muted">— {h.year ? `${h.year}-` : "every "}{String(h.month).padStart(2, "0")}-{String(h.day).padStart(2, "0")}</span></span>
              <button class="ghost" disabled={locked} onclick={() => removeCustomHoliday(i)} aria-label="Remove {h.name}">✕</button>
            </div>
          {/each}
          <input placeholder="Name (e.g. Founders' Night)" bind:value={newHoliday.name} disabled={locked} />
          <input type="date" bind:value={newHoliday.date} disabled={locked} aria-label="Holiday date" />
          <label class="check small"><input type="checkbox" bind:checked={newHoliday.everyYear} /> Every year</label>
          <button disabled={locked || !newHoliday.name.trim() || !newHoliday.date} onclick={addCustomHoliday}>Add holiday</button>
        </details>
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
    <p class="muted small">What each character has privately learned. Filled in after replies; edit freely.</p>
    {#each cast as character (character.id)}
      {@const notes = notesFor(character.id)}
      <h3>{character.name}</h3>
      {#each notes as note (note.id)}
        <div class="note">
          <textarea
            rows="1"
            value={note.content}
            aria-label="Note for {character.name}"
            onchange={(e) => {
              void chat.saveNote(note, e.currentTarget.value);
              // An emptied note is refused; show the saved text again.
              if (!e.currentTarget.value.trim()) e.currentTarget.value = note.content;
            }}
          ></textarea>
          <button class="ghost" onclick={() => chat.deleteNote(note.id)} aria-label="Delete note">✕</button>
        </div>
      {:else}
        <p class="muted small">No notes yet.</p>
      {/each}
      <form
        class="add"
        onsubmit={async (e) => {
          e.preventDefault();
          const input = e.currentTarget.elements.namedItem("text") as HTMLInputElement;
          if (await chat.addNote(character.id, input.value)) input.value = "";
        }}
      >
        <input name="text" placeholder="Add a note…" aria-label="New note for {character.name}" />
        <button type="submit">Add</button>
      </form>
    {/each}
  </details>

  <p class="muted small footnote">Quests are tracked automatically after each reply.</p>
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

  .note,
  .add {
    display: flex;
    gap: 4px;
    align-items: flex-start;
    margin: 4px 0;
  }

  .note textarea {
    flex: 1;
    min-height: 0;
    resize: vertical;
    font-size: 12px;
    padding: 4px 6px;
    field-sizing: content;
  }

  .add input {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    padding: 4px 6px;
  }

  .add button,
  .note button {
    font-size: 11px;
    padding: 3px 8px;
  }

  .footnote {
    margin-top: 20px;
  }

  .day {
    font-weight: 600;
  }

  .day.holiday {
    color: var(--accent);
  }

  .holidays {
    margin: 2px 0 0;
    padding-left: 16px;
  }

  .custom {
    margin: 0;
  }

  .custom[open] {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .customrow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
</style>

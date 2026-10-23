<script lang="ts">
  import { api } from "$lib/api";
  import { chat } from "$lib/chat.svelte";
  import type { Country, HolidayOverview, Meter, StoryCharacter, WeatherCondition } from "$lib/types";
  import { addMinutes } from "$lib/util";

  const world = $derived(chat.session?.conversation.worldState ?? null);
  const cast = $derived(chat.session?.characters ?? []);
  const dressable = $derived(cast.filter((c) => c.outfits.length > 0));

  /** Puts a character in an outfit for this story ("" = their default clothes). */
  function setOutfit(characterId: string, outfitId: string) {
    if (!world) return;
    const outfits = { ...(world.outfits ?? {}) };
    if (outfitId) outfits[characterId] = outfitId;
    else delete outfits[characterId];
    void chat.updateWorld({ outfits });
  }

  const when = $derived.by(() => {
    if (!world) return "";
    // The in-story clock has no time zone; it parses (and so prints) as the same wall-clock time.
    return new Date(world.fictionalDateTime).toLocaleString(undefined, { dateStyle: "full", timeStyle: "short" });
  });

  const weatherLabel = (w: string) => w.replace(/([a-z])([A-Z])/g, "$1 $2");
  const WEATHER: WeatherCondition[] = ["Clear", "PartlyCloudy", "Overcast", "Rain", "Thunderstorm", "Snow", "Fog", "Windy"];
  const JUMPS = [
    { label: "+1 hour", minutes: 60 },
    { label: "+6 hours", minutes: 360 },
    { label: "+1 day", minutes: 1440 },
  ];

  // The clock is frozen while a reply is being written so an edit mid-stream cannot race the reply.
  const locked = $derived(chat.busy);

  function setTime(e: Event & { currentTarget: HTMLInputElement }) {
    const value = e.currentTarget.value; // "YYYY-MM-DDTHH:mm", empty while the field is half-typed
    if (value) void chat.updateWorld({ fictionalDateTime: `${value}:00` });
  }

  // ---- arousal meters ----

  function setMeter(characterId: string, patch: Partial<Meter>) {
    if (!world) return;
    const current = world.meters?.[characterId] ?? { arousal: 0, climax: 0, orgasms: 0 };
    void chat.updateWorld({ meters: { ...(world.meters ?? {}), [characterId]: { ...current, ...patch } } });
  }

  // ---- people the story introduced ----

  let tracking = $state(false);
  let editing = $state<StoryCharacter | null>(null);
  let adding = $state("");

  $effect(() => {
    api.getTrackStoryCast().then((on) => (tracking = on), () => {});
  });

  function setTracking(on: boolean) {
    tracking = on;
    void api.setTrackStoryCast(on);
  }

  const introduced = (c: StoryCharacter) =>
    c.introducedAt ? new Date(c.introducedAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "";

  function addPerson() {
    const id = chat.conversationId;
    const name = adding.trim();
    if (!id || !name) return;
    editing = {
      id: crypto.randomUUID(),
      conversationId: id,
      name,
      age: null,
      role: "",
      appearance: "",
      adultDetails: "",
      introducedAt: world?.fictionalDateTime ?? null,
      introducedMessageId: null,
      createdAt: new Date().toISOString(),
    };
    adding = "";
  }

  async function saveEdit() {
    if (!editing) return;
    await chat.saveCastMember(editing);
    editing = null;
  }

  function setAge(e: Event & { currentTarget: HTMLInputElement }) {
    const n = Math.round(Number(e.currentTarget.value));
    if (editing) editing.age = e.currentTarget.value.trim() !== "" && Number.isFinite(n) && n >= 0 ? n : null;
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

</script>

{#snippet editor()}
  {#if editing}
    <div class="person edit">
      <label class="field">Name <input bind:value={editing.name} /></label>
      <div class="two">
        <label class="field">Age <input type="number" min="0" value={editing.age ?? ""} oninput={setAge} /></label>
        <label class="field">Role <input bind:value={editing.role} placeholder="who they are" /></label>
      </div>
      <label class="field">Appearance <textarea rows="5" bind:value={editing.appearance}></textarea></label>
      <label class="field">
        <textarea rows="4" bind:value={editing.adultDetails} aria-label="Further details"></textarea>
      </label>
      <div class="row">
        <button disabled={!editing.name.trim()} onclick={saveEdit}>Save</button>
        <button class="ghost" onclick={() => (editing = null)}>Cancel</button>
      </div>
    </div>
  {/if}
{/snippet}

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
        <p class="muted small">The clock and weather only change when you edit them here. Replies do not move them.</p>
      </div>

      {#if dressable.length > 0}
        <h3>Outfits</h3>
        <div class="clock" class:locked>
          {#each dressable as character (character.id)}
            <label class="field">
              {character.name} wears
              <select
                value={world.outfits?.[character.id] ?? ""}
                disabled={locked}
                onchange={(e) => setOutfit(character.id, e.currentTarget.value)}
              >
                <option value="">Default clothes</option>
                {#each character.outfits as outfit (outfit.id)}<option value={outfit.id}>{outfit.name || "Unnamed outfit"}</option>{/each}
              </select>
            </label>
          {/each}
        </div>
      {/if}

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
        {#each overview?.schoolBreaks ?? [] as b (b.start + b.name)}
          <p class="small">
            <span class="muted">School holidays:</span>
            {b.name}, {b.start <= storyDate ? "until" : `${shortDate(b.start)} –`} {shortDate(b.end)}
          </p>
        {/each}
        {#if world.countryCode && !world.regionCode && overview?.regions.length}
          <p class="muted small">Pick a region to include its school holidays.</p>
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

  {#if world}
    <details>
      <summary>Arousal meters</summary>
      <div class="clock" class:locked>
        <label class="check">
          <input type="checkbox" checked={world.metersEnabled} disabled={locked} onchange={(e) => chat.updateWorld({ metersEnabled: e.currentTarget.checked })} />
          Track arousal and climax automatically
        </label>
        {#if world.metersEnabled}
          <p class="muted small">
            After each reply the story is read for anything that raises or lowers them (one extra request), so they follow what was written. An orgasm in the text counts, empties climax and drops arousal. Off by default; you can also set them by hand.
          </p>
          {#each cast as c (c.id)}
            {@const m = world.meters?.[c.id] ?? { arousal: 0, climax: 0, orgasms: 0 }}
            <div class="meter">
              <strong>{c.name}</strong>
              <label>
                <span>Arousal <span class="muted">{m.arousal}</span></span>
                <input type="range" min="0" max="100" value={m.arousal} disabled={locked} onchange={(e) => setMeter(c.id, { arousal: Number(e.currentTarget.value) })} />
              </label>
              <label>
                <span>Climax <span class="muted">{m.climax}</span></span>
                <input type="range" min="0" max="100" value={m.climax} disabled={locked} onchange={(e) => setMeter(c.id, { climax: Number(e.currentTarget.value) })} />
              </label>
              <span class="muted small">Orgasms so far: {m.orgasms}</span>
            </div>
          {/each}
        {/if}
      </div>
    </details>
  {/if}

  <details open>
    <summary>People ({chat.cast.length})</summary>
    <div class="people-tools">
      <button disabled={chat.busy || chat.scanning} onclick={() => chat.scanCast()}>
        {chat.scanning ? "Reading the story…" : "Scan recent story"}
      </button>
      <label class="check small">
        <input type="checkbox" checked={tracking} onchange={(e) => setTracking(e.currentTarget.checked)} />
        Note new people after each reply (one extra request)
      </label>
    </div>

    {#if chat.cast.length === 0 && !editing}
      <p class="muted small">Nobody yet. People who are introduced in the story (apart from the main characters) are listed here with their age and looks.</p>
    {/if}

    {#each chat.cast as person (person.id)}
      {#if editing?.id === person.id}
        {@render editor()}
      {:else}
        <article class="person">
          <header>
            <strong>{person.name}</strong>
            <span class="muted small">
              {person.age !== null ? `${person.age} years` : "age unknown"}{person.role ? ` · ${person.role}` : ""}
            </span>
            {#if introduced(person)}<span class="muted small">Introduced {introduced(person)}</span>{/if}
          </header>
          {#if person.appearance}<p class="small">{person.appearance}</p>{/if}
          {#if person.adultDetails}<p class="small">{person.adultDetails}</p>{/if}
          <div class="row">
            <button class="ghost small" onclick={() => (editing = structuredClone($state.snapshot(person)))}>Edit</button>
            <button class="ghost small" onclick={() => chat.removeCastMember(person.id)} aria-label="Remove {person.name}">Remove</button>
          </div>
        </article>
      {/if}
    {/each}

    {#if editing && !chat.cast.some((p) => p.id === editing?.id)}
      {@render editor()}
    {/if}

    <div class="addrow">
      <input placeholder="Add a person…" bind:value={adding} onkeydown={(e) => e.key === "Enter" && addPerson()} />
      <button disabled={!adding.trim()} onclick={addPerson}>Add</button>
    </div>
  </details>

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

  .meter {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .meter label {
    display: flex;
    flex-direction: column;
    gap: 2px;
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
  .people-tools {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 6px 0 10px;
  }

  .person {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px 10px;
    margin-bottom: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .person header {
    display: flex;
    flex-direction: column;
  }

  .person.edit {
    gap: 8px;
    font-size: 12px;
  }

  .person textarea {
    width: 100%;
    resize: vertical;
  }

  .two {
    display: grid;
    grid-template-columns: 70px 1fr;
    gap: 6px;
  }

  .row {
    display: flex;
    gap: 6px;
  }

  .addrow {
    display: flex;
    gap: 6px;
  }

  .addrow input {
    flex: 1;
    min-width: 0;
  }
</style>

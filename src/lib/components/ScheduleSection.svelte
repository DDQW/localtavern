<script lang="ts">
  // A character's weekly routine (one timetable per weekday plus public holidays) and home
  // country, which new roleplays start in. The prompt is told where the routine has them now.
  import { api } from "#lib/api.js";
  import { DAY_KINDS, TEMPLATES, WEEKDAYS, emptySchedule } from "#lib/scheduleTemplates.js";
  import type { Character, Country, DayPlan, Schedule } from "#lib/types.js";
  import AiDraft from "./AiDraft.svelte";

  let { schedule = $bindable(null), character }: { schedule: Schedule | null; character: () => Character } = $props();

  let countries = $state<Country[]>([]);
  let tab = $state(0); // 0-6 weekdays, 7 holiday
  let undo = $state<Schedule | null>(null);

  $effect(() => {
    api.holidayCountries().then((list) => (countries = list), () => (countries = []));
  });

  const plan = $derived<DayPlan | null>(schedule ? (tab === 7 ? schedule.holiday : schedule.days[tab]) : null);

  function ensure(): Schedule {
    if (!schedule) schedule = emptySchedule();
    while (schedule.days.length < 7) schedule.days.push({ kind: "off", blocks: [] });
    return schedule;
  }

  function applyTemplate(name: string) {
    const template = TEMPLATES.find((t) => t.name === name);
    if (!template) return;
    const s = ensure();
    undo = $state.snapshot(s) as Schedule;
    Object.assign(s, template.build());
  }

  function copyTo(indices: number[]) {
    if (!schedule || !plan) return;
    const source = $state.snapshot(plan) as DayPlan;
    for (const i of indices) if (i !== tab) schedule.days[i] = structuredClone(source);
  }

  function addBlock() {
    plan?.blocks.push({ start: "09:00", end: "10:00", activity: "", place: null });
  }

  async function draft(idea: string) {
    const card = character();
    const result = await api.draftSchedule(card, idea);
    undo = schedule ? ($state.snapshot(schedule) as Schedule) : emptySchedule();
    schedule = result;
  }

  function revert() {
    if (undo) schedule = undo;
    undo = null;
  }
</script>

{#if !schedule}
  <div class="btnrow">
    <button type="button" onclick={() => ensure()}>＋ Add a schedule</button>
    <span class="muted small">or start from</span>
    {#each TEMPLATES as t (t.name)}<button type="button" class="ghost" onclick={() => applyTemplate(t.name)}>{t.name}</button>{/each}
  </div>
{:else}
  <div class="home">
    <label class="field">
      Lives in (country)
      {#if countries.length}
        <select value={schedule.homeCountry ?? ""} onchange={(e) => (schedule!.homeCountry = e.currentTarget.value || null)}>
          <option value="">—</option>
          {#each countries as c (c.countryCode)}<option value={c.countryCode}>{c.name}</option>{/each}
        </select>
      {:else}
        <input maxlength="2" placeholder="e.g. DE" value={schedule.homeCountry ?? ""} oninput={(e) => (schedule!.homeCountry = e.currentTarget.value.toUpperCase() || null)} />
      {/if}
    </label>
    <label class="field">
      Region (optional)
      <input placeholder="e.g. DE-BY" value={schedule.homeRegion ?? ""} oninput={(e) => (schedule!.homeRegion = e.currentTarget.value.toUpperCase() || null)} />
    </label>
    <label class="field">
      Story starts (date and time)
      <input
        type="datetime-local"
        value={schedule.storyStart?.slice(0, 16) ?? ""}
        onchange={(e) => (schedule!.storyStart = e.currentTarget.value ? `${e.currentTarget.value}:00` : null)}
      />
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={schedule.observesHolidays} />
      Takes public holidays off
    </label>
  </div>
  <p class="muted small">New roleplays with this character are set in this country (its public holidays apply) and begin at the start date and time, if you set one.</p>

  <div class="tabs" role="tablist" aria-label="Day">
    {#each [...WEEKDAYS, "Holiday"] as name, i (name)}
      {@const p = i === 7 ? schedule.holiday : schedule.days[i]}
      <button type="button" role="tab" aria-selected={tab === i} class:on={tab === i} onclick={() => (tab = i)}>
        {name.slice(0, 3)}<span class="kind">{p?.kind === "off" ? "" : p?.kind}</span>
      </button>
    {/each}
  </div>

  {#if plan}
    <div class="dayhead">
      <div class="segmented" role="radiogroup" aria-label="Kind of day">
        {#each DAY_KINDS as k (k.value)}
          <button type="button" role="radio" aria-checked={plan.kind === k.value} class:on={plan.kind === k.value} onclick={() => (plan.kind = k.value)}>{k.label}</button>
        {/each}
      </div>
      <span class="spacer"></span>
      {#if tab < 7}
        <button type="button" class="ghost" onclick={() => copyTo([0, 1, 2, 3, 4])}>Copy to Mon–Fri</button>
        <button type="button" class="ghost" onclick={() => copyTo([5, 6])}>Copy to weekend</button>
      {/if}
    </div>

    {#each plan.blocks as block, i (i)}
      <div class="block">
        <input type="time" bind:value={block.start} aria-label="Start" />
        <span class="muted">–</span>
        <input type="time" bind:value={block.end} aria-label="End" />
        <input class="grow" placeholder="Activity (work, gym, classes…)" bind:value={block.activity} aria-label="Activity" />
        <input class="place" placeholder="Place" value={block.place ?? ""} oninput={(e) => (block.place = e.currentTarget.value || null)} aria-label="Place" />
        <button type="button" class="ghost danger" onclick={() => plan.blocks.splice(i, 1)} aria-label="Remove block">✕</button>
      </div>
    {:else}
      <p class="muted small">{tab === 7 ? "Nothing planned on holidays — free time." : "Nothing planned — free all day."}</p>
    {/each}
    <div class="btnrow">
      <button type="button" onclick={addBlock}>＋ Add time block</button>
      <span class="muted small">A block that ends before it starts runs past midnight.</span>
    </div>
  {/if}

  <div class="btnrow">
    <label class="field inline">
      Start from template
      <select value="" onchange={(e) => { applyTemplate(e.currentTarget.value); e.currentTarget.value = ""; }}>
        <option value="">Choose…</option>
        {#each TEMPLATES as t (t.name)}<option value={t.name}>{t.name}</option>{/each}
      </select>
    </label>
    <span class="spacer"></span>
    <button type="button" class="ghost danger" onclick={() => (schedule = null)}>Remove schedule</button>
  </div>
{/if}

<AiDraft generate={draft} placeholder="Optional: e.g. nurse on rotating shifts, plays football on Sundays">
  {#if undo}
    <div class="btnrow">
      <span class="muted small">Schedule filled in — check each day.</span>
      <button type="button" class="ghost" onclick={revert}>Undo</button>
    </div>
  {/if}
</AiDraft>

<style>
  .home {
    display: flex;
    gap: 16px;
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .tabs {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .tabs button {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-width: 58px;
    padding: 4px 8px;
    line-height: 1.2;
  }

  .tabs button.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 22%, var(--panel-2));
  }

  .kind {
    font-size: 10px;
    color: var(--muted);
    min-height: 12px;
  }

  .dayhead {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .spacer {
    flex: 1;
  }

  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .segmented button {
    border: none;
    border-radius: 0;
    background: var(--panel-2);
  }

  .segmented button + button {
    border-left: 1px solid var(--border);
  }

  .segmented button.on {
    background: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }

  .block {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }

  .block input[type="time"] {
    width: 104px;
  }

  .grow {
    flex: 1 1 160px;
  }

  .place {
    flex: 0 1 150px;
    min-width: 100px;
  }

  .inline {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }

  .small {
    font-size: 12px;
  }
</style>

<script lang="ts">
  // The quick appearance builder and the birthday, shared by the character and persona editors.
  // The builder writes one appearance paragraph into the description. It keeps that paragraph in
  // step with the picks for as long as it is still there unchanged; once it is edited or polished,
  // the picks only offer to update it.
  import { untrack } from "svelte";
  import { api, errorText } from "$lib/api";
  import * as opt from "$lib/appearanceOptions";
  import type { Appearance, AppearanceSubject, Sex } from "$lib/types";
  import ChipGroup from "./ChipGroup.svelte";

  let {
    subject,
    appearance = $bindable(null),
    description = $bindable(""),
    birthDate = $bindable(null),
  }: {
    subject: AppearanceSubject;
    appearance: Appearance | null;
    description: string;
    birthDate: string | null;
  } = $props();

  const who = $derived(subject === "persona" ? "{{user}}" : "{{char}}");
  let builderOpen = $state(untrack(() => appearance !== null || description.trim() === ""));
  /** The template for the current picks. */
  let rendered = $state<string | null>(null);
  /** The paragraph this builder last put in the description (the template, or its polished form). */
  let managed = $state<string | null>(null);
  let undo = $state<{ description: string; managed: string | null } | null>(null);
  let polishing = $state(false);
  let error = $state("");
  let ageInput = $state<number | null>(null);

  const sex = $derived(appearance?.sex ?? null);
  const inDescription = $derived(managed !== null && managed !== "" && description.includes(managed));
  const stale = $derived(rendered !== null && (managed !== rendered || !inDescription));

  /** Puts `paragraph` where the managed one is, or after the description. */
  function write(paragraph: string) {
    if (managed && description.includes(managed)) description = description.replace(managed, paragraph);
    else description = description.trim() ? `${description.trimEnd()}\n\n${paragraph}` : paragraph;
    managed = paragraph;
  }

  // Set when the picks were started in this editing session (so the first render is inserted).
  let startedHere = false;
  let firstRender = true;
  $effect(() => {
    if (!appearance) return;
    const picks = $state.snapshot(appearance) as Appearance; // reads every field, so any pick re-runs this
    const timer = setTimeout(async () => {
      try {
        const next = await api.renderAppearance(picks, subject);
        untrack(() => {
          const following = firstRender ? startedHere || description.includes(next) : managed === rendered && inDescription;
          if (firstRender && !startedHere && description.includes(next)) managed = next;
          else if (following) write(next);
          firstRender = false;
          rendered = next;
        });
      } catch (e) {
        error = errorText(e);
      }
    }, 120);
    return () => clearTimeout(timer);
  });

  function start(chosen: Sex | null) {
    startedHere = true;
    appearance = chosen ? opt.emptyAppearance(chosen) : opt.randomAppearance(null);
    builderOpen = true;
  }

  function setSex(chosen: Sex) {
    if (appearance) appearance.sex = chosen;
  }

  function randomize() {
    appearance = opt.randomAppearance(appearance);
  }

  function clearPicks() {
    appearance = null;
    rendered = null;
    managed = null;
    startedHere = false;
    firstRender = true;
  }

  function applyPicks() {
    if (rendered === null) return;
    undo = { description, managed };
    write(rendered);
  }

  async function polish() {
    if (!managed || !inDescription) return;
    error = "";
    polishing = true;
    try {
      const target = managed;
      const polished = await api.polishAppearance(target);
      undo = { description, managed };
      write(polished);
    } catch (e) {
      error = errorText(e);
    } finally {
      polishing = false;
    }
  }

  function revert() {
    if (!undo) return;
    description = undo.description;
    managed = undo.managed;
    undo = null;
  }

  function toggleMark(mark: string) {
    if (!appearance) return;
    appearance.marks = appearance.marks.includes(mark) ? appearance.marks.filter((m) => m !== mark) : [...appearance.marks, mark];
  }

  let markDraft = $state("");
  function addCustomMark() {
    const mark = markDraft.trim();
    if (appearance && mark && !appearance.marks.includes(mark)) appearance.marks = [...appearance.marks, mark];
    markDraft = "";
  }

  // A birthday that makes them `age` on the default story start (January 1, 2025); any day of the year.
  const STORY_START = new Date(2025, 0, 1);
  function birthdayFromAge() {
    if (ageInput === null || ageInput < 0) return;
    const latest = new Date(STORY_START.getFullYear() - ageInput, 0, 1);
    const earliest = new Date(latest.getFullYear() - 1, 0, 2);
    const day = new Date(earliest.getTime() + Math.random() * (latest.getTime() - earliest.getTime()));
    const pad = (n: number) => String(n).padStart(2, "0");
    birthDate = `${String(day.getFullYear()).padStart(4, "0")}-${pad(day.getMonth() + 1)}-${pad(day.getDate())}`;
  }

  const number = (v: string) => (v.trim() === "" || Number.isNaN(Number(v)) ? null : Math.max(0, Math.round(Number(v))));
</script>

<details class="builder card" bind:open={builderOpen}>
  <summary>
    <span>Quick appearance builder</span>
    {#if appearance?.sex}<span class="pill">{appearance.sex}</span>{/if}
  </summary>

  {#if !appearance}
    <div class="startrow">
      <span class="muted">Start with:</span>
      <button type="button" onclick={() => start("female")}>♀ Female</button>
      <button type="button" onclick={() => start("male")}>♂ Male</button>
      <button type="button" onclick={() => start(null)} title="Random picks for a random sex">🎲 Surprise me</button>
    </div>
  {:else}
    <div class="toolbar">
      <div class="segmented" role="radiogroup" aria-label="Sex">
        <button type="button" role="radio" aria-checked={sex === "female"} class:on={sex === "female"} onclick={() => setSex("female")}>♀ Female</button>
        <button type="button" role="radio" aria-checked={sex === "male"} class:on={sex === "male"} onclick={() => setSex("male")}>♂ Male</button>
      </div>
      <span class="spacer"></span>
      <button type="button" onclick={randomize} title="Fill every pick at random (keeps the sex and the 18+ section)">🎲 Randomize</button>
      <button type="button" class="ghost" onclick={clearPicks} title="Forget the picks; the text below stays">Clear picks</button>
    </div>

    <div class="sizes">
      <label class="field">
        Height {#if appearance.heightCm}<span class="unit">{opt.cmToFeetInches(appearance.heightCm)}</span>{/if}
        <div class="slider">
          <input
            type="range"
            min="120"
            max="220"
            value={appearance.heightCm ?? opt.AVERAGE_HEIGHT[sex ?? "female"]}
            class:unset={!appearance.heightCm}
            oninput={(e) => (appearance!.heightCm = Number(e.currentTarget.value))}
            aria-label="Height slider"
          />
          <input type="number" min="30" max="400" placeholder="cm" value={appearance.heightCm ?? ""} oninput={(e) => (appearance!.heightCm = number(e.currentTarget.value))} aria-label="Height in cm" />
          <span class="muted">cm</span>
        </div>
      </label>
      <label class="field">
        Weight {#if appearance.weightKg}<span class="unit">{opt.kgToPounds(appearance.weightKg)} lb</span>{/if}
        <div class="slider">
          <input
            type="range"
            min="35"
            max="180"
            value={appearance.weightKg ?? (sex === "male" ? 78 : 60)}
            class:unset={!appearance.weightKg}
            oninput={(e) => (appearance!.weightKg = Number(e.currentTarget.value))}
            aria-label="Weight slider"
          />
          <input type="number" min="1" max="700" placeholder="kg" value={appearance.weightKg ?? ""} oninput={(e) => (appearance!.weightKg = number(e.currentTarget.value))} aria-label="Weight in kg" />
          <span class="muted">kg</span>
        </div>
      </label>
    </div>

    <ChipGroup label="Build" options={opt.BUILDS[sex ?? "female"]} bind:value={appearance.build} />
    <ChipGroup label="Muscle tone" options={opt.MUSCLE_TONES} bind:value={appearance.muscleTone} />
    <ChipGroup label="Skin" options={opt.SKIN_TONES} bind:value={appearance.skinTone} />
    <ChipGroup label="Hair colour" options={opt.HAIR_COLORS} bind:value={appearance.hairColor} />
    <ChipGroup label="Hair length" options={opt.HAIR_LENGTHS} bind:value={appearance.hairLength} />
    <ChipGroup label="Hairstyle" options={opt.HAIR_STYLES} bind:value={appearance.hairStyle} />
    <ChipGroup label="Eyes" options={opt.EYE_COLORS} bind:value={appearance.eyeColor} />

    {#if sex === "female"}
      <h3>Figure</h3>
      <ChipGroup label="Cup size" options={opt.CUP_SIZES} bind:value={appearance.female.cupSize} />
      <ChipGroup label="Bust" options={opt.BUSTS} bind:value={appearance.female.bust} />
      <ChipGroup label="Waist" options={opt.WAISTS} bind:value={appearance.female.waist} />
      <ChipGroup label="Hips" options={opt.HIPS} bind:value={appearance.female.hips} />
      <ChipGroup label="Butt" options={opt.BUTTS} bind:value={appearance.female.butt} />
    {:else if sex === "male"}
      <h3>Physique</h3>
      <ChipGroup label="Shoulders" options={opt.SHOULDERS} bind:value={appearance.male.shoulders} />
      <ChipGroup label="Chest" options={opt.CHESTS} bind:value={appearance.male.chest} />
      <ChipGroup label="Facial hair" options={opt.FACIAL_HAIR} bind:value={appearance.male.facialHair} />
    {/if}

    <ChipGroup label="Body hair" options={opt.BODY_HAIR} bind:value={appearance.bodyHair} />

    <div class="group" role="group" aria-label="Distinguishing features">
      <span class="label">Distinguishing features</span>
      <div class="chips">
        {#each [...opt.MARKS.map((m) => m.value), ...appearance.marks.filter((m) => !opt.MARKS.some((o) => o.value === m))] as mark (mark)}
          <button type="button" class="chip" class:on={appearance.marks.includes(mark)} aria-pressed={appearance.marks.includes(mark)} onclick={() => toggleMark(mark)}>{mark}</button>
        {/each}
        <input
          class="custom"
          placeholder="Add… (Enter)"
          aria-label="Add a distinguishing feature"
          bind:value={markDraft}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addCustomMark();
            }
          }}
          onblur={addCustomMark}
        />
      </div>
    </div>

    <label class="field">
      Other details
      <input value={appearance.notes ?? ""} oninput={(e) => (appearance!.notes = e.currentTarget.value || null)} placeholder="Always wears a red scarf" />
    </label>

    <details class="adult">
      <summary>18+ details <span class="muted">(only added to the text when filled in)</span></summary>
      <ChipGroup label="Nipples" options={opt.NIPPLES} bind:value={appearance.adult.nipples} />
      <ChipGroup label="Pubic hair" options={opt.PUBIC_HAIR} bind:value={appearance.adult.pubicHair} />
      <ChipGroup label="Genitals" options={opt.GENITALS[sex ?? "female"]} bind:value={appearance.adult.genitals} />
      {#if sex === "male"}
        <ChipGroup label="Circumcision" options={opt.CIRCUMCISION} bind:value={appearance.adult.circumcision} />
      {/if}
      <label class="field">
        Notes
        <input value={appearance.adult.notes ?? ""} oninput={(e) => (appearance!.adult.notes = e.currentTarget.value || null)} />
      </label>
    </details>
  {/if}
</details>

<div class="btnrow">
  {#if stale && appearance}
    <button type="button" onclick={applyPicks} title="Write the template for the current picks into the description">
      {inDescription ? "↻ Update description from picks" : "＋ Add appearance to description"}
    </button>
  {/if}
  <button
    type="button"
    onclick={polish}
    disabled={polishing || !inDescription}
    title={inDescription ? "Rewrite the appearance paragraph as natural prose with your last-used model" : "Use the builder first; this rewrites its paragraph in the description"}
  >
    {polishing ? "Polishing…" : "✨ Polish appearance with AI"}
  </button>
  {#if undo}<button type="button" class="ghost" onclick={revert}>Undo</button>{/if}
</div>
<p class="hint">The builder writes one appearance paragraph into the description above ({who} names {subject === "persona" ? "you" : "the character"}). Edit it there freely — after an edit, picks only offer to update it.</p>
{#if error}<p class="notice" role="alert">{error}</p>{/if}

<div class="birthday">
  <label class="field">
    Birthday
    <input type="date" value={birthDate ?? ""} onchange={(e) => (birthDate = e.currentTarget.value || null)} />
  </label>
  <label class="field">
    …or from age
    <span class="agerow">
      <input type="number" min="0" max="999" placeholder="e.g. 24" bind:value={ageInput} aria-label="Age" />
      <button type="button" onclick={birthdayFromAge} disabled={ageInput === null}>Pick a date</button>
    </span>
  </label>
  {#if birthDate}<button type="button" class="ghost" onclick={() => (birthDate = null)}>Clear</button>{/if}
</div>
<p class="hint">Ages and upcoming birthdays are worked out from each roleplay's in-story date. "From age" counts from the default story start, January 1, 2025.</p>

<style>
  .builder {
    padding: 10px 14px;
  }

  .builder[open] {
    padding-bottom: 16px;
  }

  .builder > summary {
    cursor: pointer;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .builder[open] > summary {
    margin-bottom: 12px;
  }

  .builder > :global(* + *) {
    margin-top: 12px;
  }

  .startrow,
  .toolbar {
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

  .sizes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 12px;
  }

  .slider {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .slider input[type="range"] {
    flex: 1;
    accent-color: var(--accent);
  }

  .slider input[type="range"].unset {
    opacity: 0.45;
  }

  .slider input[type="number"] {
    width: 72px;
  }

  .unit {
    color: var(--text);
    margin-left: 4px;
  }

  h3 {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .label {
    font-size: 12px;
    color: var(--muted);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .chip {
    font-size: 12px;
    padding: 3px 11px;
    border-radius: 999px;
    background: var(--panel-2);
    border: 1px solid var(--border);
  }

  .chip:hover {
    background: var(--panel-3);
  }

  .chip.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 22%, var(--panel-2));
  }

  .custom {
    font-size: 12px;
    padding: 3px 11px;
    border-radius: 999px;
    width: 150px;
  }

  .adult {
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }

  .adult > summary {
    cursor: pointer;
    font-size: 13px;
  }

  .adult[open] > summary {
    margin-bottom: 12px;
  }

  .adult > :global(* + *) {
    margin-top: 12px;
  }

  .hint {
    font-size: 12px;
    color: var(--muted);
  }

  .birthday {
    display: flex;
    align-items: flex-end;
    gap: 16px;
    flex-wrap: wrap;
  }

  .agerow {
    display: flex;
    gap: 6px;
  }

  .agerow input {
    width: 80px;
  }
</style>

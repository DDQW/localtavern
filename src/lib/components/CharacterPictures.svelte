<script lang="ts">
  import { api, errorText } from "#lib/api.js";
  import { lightbox } from "#lib/lightbox.svelte.js";
  import { fileUrl } from "#lib/media.js";
  import { pickFolder, pickImages } from "#lib/pick.js";
  import type { Character, CharacterShot, ExpressionImage, Outfit } from "#lib/types.js";
  import { newId } from "#lib/util.js";

  // The character's picture sets: moods for the default clothes, costumes with their own moods,
  // and action pictures - all drawn with the same base look and seed so they look like one person.
  let {
    character = $bindable(),
    discard,
  }: {
    character: Character;
    /** A picture that was replaced or removed; the page deletes it once the edit is saved. */
    discard: (path: string) => void;
  } = $props();

  const SUGGESTED_EMOTIONS = ["neutral", "happy", "sad", "angry", "surprised", "scared", "disgusted", "love"];

  let error = $state("");
  let notice = $state("");
  let writingLook = $state(false);
  /** The batch being drawn: what is being drawn right now, and how far along. */
  let drawing = $state<{ label: string; done: number; total: number } | null>(null);
  let stopRequested = false;

  // ---- the base look ----

  async function writeLook(): Promise<boolean> {
    error = "";
    writingLook = true;
    try {
      const look = await api.describeCharacterLook($state.snapshot(character) as Character);
      character.pictureStyle.baseLook = look.baseLook;
      if (look.defaultOutfit && !character.pictureStyle.defaultOutfit.trim()) character.pictureStyle.defaultOutfit = look.defaultOutfit;
      return true;
    } catch (e) {
      error = errorText(e);
      return false;
    } finally {
      writingLook = false;
    }
  }

  // ---- drawing ----

  interface Job {
    label: string;
    shot: CharacterShot;
    outfitId: string | null;
    /** Puts the finished picture where it belongs (a mood row, an action row). */
    apply: (path: string) => void;
  }

  /** Draws the jobs one after another; Stop ends the batch after (or during) the current one. */
  async function drawAll(jobs: Job[]) {
    if (drawing) return;
    error = "";
    notice = "";
    if (jobs.length === 0) {
      notice = "Nothing missing - every entry already has a picture. Use Redraw to replace one.";
      return;
    }
    if (!character.pictureStyle.baseLook.trim() && !(await writeLook())) return;
    stopRequested = false;
    let finished = 0;
    try {
      for (const [i, job] of jobs.entries()) {
        if (stopRequested) break;
        drawing = { label: job.label, done: i, total: jobs.length };
        const picture = await api.drawCharacterPicture($state.snapshot(character) as Character, job.shot, job.outfitId);
        if (!picture) break; // stopped
        // The first picture fixes the seed, so the rest of the set (and later ones) match it.
        if (character.pictureStyle.seed < 0) character.pictureStyle.seed = picture.seed;
        job.apply(picture.path);
        finished++;
      }
      if (finished > 0) notice = `Drew ${finished} picture${finished === 1 ? "" : "s"}. Save the character to keep them.`;
    } catch (e) {
      error = errorText(e);
    } finally {
      drawing = null;
    }
  }

  function stop() {
    stopRequested = true;
    void api.stopCharacterPicture(character.id);
  }

  function moodJob(row: ExpressionImage, outfit: Outfit | null): Job {
    const where = outfit ? ` (${outfit.name || "outfit"})` : "";
    return {
      label: `${row.emotion}${where}`,
      shot: { kind: "mood", emotion: row.emotion.trim() },
      outfitId: outfit?.id ?? null,
      apply: (path) => {
        if (row.imagePath) discard(row.imagePath);
        row.imagePath = path;
      },
    };
  }

  /** Every mood of a set that has a name but no picture yet; an empty set gets the suggested moods. */
  function missingMoods(rows: ExpressionImage[], outfit: Outfit | null): Job[] {
    if (rows.length === 0) rows.push(...SUGGESTED_EMOTIONS.map((emotion) => ({ emotion, imagePath: "" })));
    return rows.filter((r) => r.emotion.trim() && !r.imagePath).map((r) => moodJob(r, outfit));
  }

  function actionJob(row: { action: string; imagePath: string }): Job {
    return {
      label: row.action,
      shot: { kind: "action", action: row.action.trim() },
      outfitId: null,
      apply: (path) => {
        if (row.imagePath) discard(row.imagePath);
        row.imagePath = path;
      },
    };
  }

  // ---- editing the sets ----

  function addMood(rows: ExpressionImage[]) {
    const used = new Set(rows.map((e) => e.emotion.trim().toLowerCase()));
    rows.push({ emotion: SUGGESTED_EMOTIONS.find((e) => !used.has(e)) ?? "", imagePath: "" });
  }

  function removeRow<T extends { imagePath: string }>(rows: T[], index: number) {
    const [removed] = rows.splice(index, 1);
    if (removed?.imagePath) discard(removed.imagePath);
  }

  async function choosePicture(row: { imagePath: string }) {
    try {
      const [picked] = await pickImages(false);
      if (!picked) return;
      const stored = await api.storeImage(picked);
      if (row.imagePath) discard(row.imagePath);
      row.imagePath = stored;
    } catch (e) {
      error = errorText(e);
    }
  }

  /** SillyTavern sprite folder: one image per emotion, named after the emotion. */
  async function importSprites(rows: ExpressionImage[]) {
    error = "";
    try {
      const folder = await pickFolder();
      if (!folder) return;
      const imported = await api.importExpressionFolder(folder);
      if (imported.length === 0) {
        notice = "No pictures found in that folder.";
        return;
      }
      for (const sprite of imported) {
        const existing = rows.find((e) => e.emotion.trim().toLowerCase() === sprite.emotion);
        if (existing) {
          if (existing.imagePath) discard(existing.imagePath);
          existing.imagePath = sprite.imagePath;
        } else {
          rows.push(sprite);
        }
      }
      notice = `Imported ${imported.length} expression${imported.length === 1 ? "" : "s"}.`;
    } catch (e) {
      error = errorText(e);
    }
  }

  function addOutfit() {
    character.outfits.push({ id: newId(), name: "", clothing: "", expressions: [] });
  }

  function removeOutfit(index: number) {
    const [removed] = character.outfits.splice(index, 1);
    for (const e of removed?.expressions ?? []) if (e.imagePath) discard(e.imagePath);
  }

  /** Gives an outfit the same mood names as the default set (or the suggested ones). */
  function copyMoodNames(outfit: Outfit) {
    const names = character.expressions.map((e) => e.emotion.trim()).filter(Boolean);
    const have = new Set(outfit.expressions.map((e) => e.emotion.trim().toLowerCase()));
    for (const emotion of names.length ? names : SUGGESTED_EMOTIONS) {
      if (!have.has(emotion.toLowerCase())) outfit.expressions.push({ emotion, imagePath: "" });
    }
  }
</script>

{#snippet moodRows(rows: ExpressionImage[], outfit: Outfit | null)}
  {#each rows as row, i (i)}
    <div class="row">
      <input bind:value={row.emotion} list="emotions" placeholder="emotion" aria-label="Emotion name" />
      {#if row.imagePath}
        <button type="button" class="thumbbtn" onclick={() => (lightbox.src = fileUrl(row.imagePath))} aria-label="View {row.emotion}">
          <img src={fileUrl(row.imagePath)} alt="" class="thumb" />
        </button>
      {:else}
        <span class="thumb empty muted">none</span>
      {/if}
      <button type="button" onclick={() => drawAll([moodJob(row, outfit)])} disabled={!!drawing || !row.emotion.trim()}>
        {row.imagePath ? "Redraw" : "Draw"}
      </button>
      <button type="button" class="ghost" onclick={() => choosePicture(row)}>Choose…</button>
      <button type="button" class="ghost danger" onclick={() => removeRow(rows, i)} aria-label="Remove {row.emotion || 'mood'}">✕</button>
    </div>
  {/each}
{/snippet}

<h2>Pictures</h2>
<p class="muted small">
  Drawn with your Stable Diffusion server (Settings → Pictures). Every picture starts with the same base look and seed, so
  the set looks like one person. Moods become the portraits that swap in during chat; each outfit has its own. Positive and
  negative prompts here are added on top of the global ones from Settings.
</p>

<label class="field">
  Base look <span class="muted">(permanent features, no clothing)</span>
  <textarea rows="2" bind:value={character.pictureStyle.baseLook} placeholder="1girl, young woman, long silver hair, blue eyes, freckles"></textarea>
</label>
<label class="field">
  Default outfit
  <input bind:value={character.pictureStyle.defaultOutfit} placeholder="navy flight jacket, white shirt, boots" />
</label>
<label class="field">
  Positive prompt <span class="muted">(extra tags for this character)</span>
  <textarea rows="2" bind:value={character.pictureStyle.promptPrefix} placeholder="score_9, score_8_up, delicate lighting"></textarea>
</label>
<label class="field">
  Negative prompt <span class="muted">(extra for this character)</span>
  <textarea rows="2" bind:value={character.pictureStyle.negativePrompt} placeholder="blonde, pale skin, blue eyes"></textarea>
</label>
<div class="btnrow">
  <button type="button" onclick={writeLook} disabled={writingLook || !!drawing}>
    {writingLook ? "Writing…" : "✨ Write from description"}
  </button>
  <label class="field inline">
    Seed
    <input type="number" min="-1" bind:value={character.pictureStyle.seed} title="-1: picked with the first picture" />
  </label>
  <button type="button" class="ghost" onclick={() => (character.pictureStyle.seed = -1)} title="The next picture picks a new seed (and the rest follow it)">🎲 New seed</button>
</div>

{#if drawing}
  <div class="progress" role="status">
    <span class="spinner" aria-hidden="true"></span>
    <span>Drawing {drawing.label}{drawing.total > 1 ? ` (${drawing.done + 1} of ${drawing.total})` : ""}…</span>
    <button type="button" class="ghost" onclick={stop}>Stop</button>
  </div>
{/if}
{#if error}<p class="notice" role="alert">{error}</p>{/if}
{#if notice}<p class="ok small" role="status">{notice}</p>{/if}

<h3>Moods <span class="muted">— default outfit</span></h3>
<p class="muted small">The model ends each reply with one of these emotion names, so any name works.</p>
{@render moodRows(character.expressions, null)}
<div class="btnrow">
  <button type="button" onclick={() => addMood(character.expressions)}>＋ Add mood</button>
  <button type="button" onclick={() => drawAll(missingMoods(character.expressions, null))} disabled={!!drawing}>Draw missing moods</button>
  <button type="button" class="ghost" onclick={() => importSprites(character.expressions)} title="A folder with one picture per emotion, named like happy.png">
    Import sprite folder…
  </button>
</div>

<h3>Outfits</h3>
<p class="muted small">Costumes with their own moods. Pick one for a chat in its World panel; the model is told what they're wearing.</p>
{#each character.outfits as outfit, o (outfit.id)}
  <div class="outfit card">
    <div class="outfithead">
      <label class="field grow">
        Name
        <input bind:value={outfit.name} placeholder="Ball gown" />
      </label>
      <button type="button" class="ghost danger" onclick={() => removeOutfit(o)} aria-label="Remove outfit {outfit.name}">Remove outfit</button>
    </div>
    <label class="field">
      Clothing
      <input bind:value={outfit.clothing} placeholder="red ball gown, long satin gloves, pearl necklace" />
    </label>
    {@render moodRows(outfit.expressions, outfit)}
    <div class="btnrow">
      <button type="button" onclick={() => addMood(outfit.expressions)}>＋ Add mood</button>
      <button type="button" onclick={() => copyMoodNames(outfit)} title="The same mood names as the default outfit">Copy mood names</button>
      <button type="button" onclick={() => drawAll(missingMoods(outfit.expressions, outfit))} disabled={!!drawing || !(outfit.clothing.trim() || outfit.name.trim())}>
        Draw missing moods
      </button>
    </div>
  </div>
{/each}
<div><button type="button" onclick={addOutfit}>＋ Add outfit</button></div>

<h3>Actions</h3>
<p class="muted small">Full-body pictures of {character.name || "them"} doing something, in the default outfit.</p>
{#each character.actionPictures as row, i (i)}
  <div class="row">
    <input class="wide" bind:value={row.action} placeholder="reading a book by the fireplace" aria-label="Action" />
    {#if row.imagePath}
      <button type="button" class="thumbbtn" onclick={() => (lightbox.src = fileUrl(row.imagePath))} aria-label="View {row.action}">
        <img src={fileUrl(row.imagePath)} alt="" class="thumb" />
      </button>
    {:else}
      <span class="thumb empty muted">none</span>
    {/if}
    <button type="button" onclick={() => drawAll([actionJob(row)])} disabled={!!drawing || !row.action.trim()}>{row.imagePath ? "Redraw" : "Draw"}</button>
    <button type="button" class="ghost" onclick={() => choosePicture(row)}>Choose…</button>
    <button type="button" class="ghost danger" onclick={() => removeRow(character.actionPictures, i)} aria-label="Remove action">✕</button>
  </div>
{/each}
<div class="btnrow">
  <button type="button" onclick={() => character.actionPictures.push({ action: "", imagePath: "" })}>＋ Add action</button>
  {#if character.actionPictures.some((a) => a.action.trim() && !a.imagePath)}
    <button
      type="button"
      onclick={() => drawAll(character.actionPictures.filter((a) => a.action.trim() && !a.imagePath).map(actionJob))}
      disabled={!!drawing}
    >
      Draw missing actions
    </button>
  {/if}
</div>

<datalist id="emotions">
  {#each SUGGESTED_EMOTIONS as emotion (emotion)}<option value={emotion}></option>{/each}
</datalist>

<style>
  h3 {
    font-size: 13px;
    margin: 6px 0 0;
  }

  .small {
    font-size: 12px;
    margin: 0;
  }

  .ok {
    color: #4fc27a;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .row input {
    width: 160px;
  }

  .row input.wide {
    flex: 1;
    width: auto;
  }

  .thumbbtn {
    padding: 0;
    border: none;
    background: none;
    line-height: 0;
  }

  .thumb {
    width: 44px;
    height: 44px;
    object-fit: cover;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
  }

  .thumb.empty {
    display: grid;
    place-items: center;
    font-size: 11px;
  }

  .inline {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }

  .inline input {
    width: 150px;
  }

  .outfit {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
  }

  .outfithead {
    display: flex;
    align-items: flex-end;
    gap: 10px;
  }

  .grow {
    flex: 1;
  }

  .progress {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--muted);
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>

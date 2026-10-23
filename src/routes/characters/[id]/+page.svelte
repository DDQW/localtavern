<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, errorText } from "#lib/api.js";
  import AiDraft from "#lib/components/AiDraft.svelte";
  import AppearanceSection from "#lib/components/AppearanceSection.svelte";
  import ScheduleSection from "#lib/components/ScheduleSection.svelte";
  import CharacterPictures from "#lib/components/CharacterPictures.svelte";
  import Avatar from "#lib/components/Avatar.svelte";
  import { fileUrl } from "#lib/media.js";
  import { pickImages, pickSaveJson } from "#lib/pick.js";
  import type { Character, Lorebook } from "#lib/types.js";
  import { newId, nowIso, splitList } from "#lib/util.js";

  let form = $state<Character | null>(null);
  let tagsText = $state("");
  let lorebooks = $state<Lorebook[]>([]);
  let error = $state("");
  let notice = $state("");
  let saving = $state(false);
  // Pictures the user removed while editing; only deleted from disk once the edit is saved, so
  // cancelling never leaves the saved character pointing at a missing file.
  let removedImages = $state<string[]>([]);

  const isNew = $derived(page.params.id === "new");

  function blank(): Character {
    const now = nowIso();
    return {
      id: newId(),
      name: "",
      avatarPath: null,
      description: "",
      personality: "",
      scenario: "",
      firstMessages: [""],
      exampleDialogue: "",
      tags: [],
      linkedLorebookIds: [],
      expressions: [],
      galleryImagePaths: [],
      appearance: null,
      birthDate: null,
      schedule: null,
      pictureStyle: { baseLook: "", defaultOutfit: "", promptPrefix: "", negativePrompt: "", seed: -1 },
      outfits: [],
      actionPictures: [],
      createdAt: now,
      updatedAt: now,
    };
  }

  $effect(() => {
    const id = page.params.id;
    void (async () => {
      try {
        lorebooks = await api.listLorebooks();
        const loaded = id === "new" ? blank() : (await api.listCharacters()).find((c) => c.id === id);
        if (!loaded) {
          error = "That character no longer exists.";
          return;
        }
        if (loaded.firstMessages.length === 0) loaded.firstMessages = [""];
        form = loaded;
        tagsText = loaded.tags.join(", ");
      } catch (e) {
        error = errorText(e);
      }
    })();
  });

  function snapshot(): Character {
    const copy = $state.snapshot(form!) as Character;
    copy.tags = splitList(tagsText);
    return copy;
  }

  async function save() {
    if (!form) return;
    error = "";
    notice = "";
    saving = true;
    try {
      await api.saveCharacter(snapshot());
      for (const path of removedImages) await api.deleteImage(path).catch(() => false);
      await goto("/characters");
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!form || isNew) return;
    if (!confirm(`Delete ${form.name || "this character"}?`)) return;
    try {
      await api.deleteCharacter(form.id);
      await goto("/characters");
    } catch (e) {
      error = errorText(e);
    }
  }

  async function exportCard() {
    if (!form) return;
    error = "";
    notice = "";
    try {
      const path = await pickSaveJson(form.name.trim() || "character");
      if (!path) return;
      await api.exportCharacterCard(snapshot(), path);
      notice = `Exported to ${path}`;
    } catch (e) {
      error = errorText(e);
    }
  }

  // ---- AI drafts ----

  let scenarioDraft = $state<string | null>(null);
  let scenarioBefore = $state<string | null>(null);
  let greetingDrafts = $state<{ text: string; pick: boolean }[]>([]);
  let greetingCount = $state(2);

  async function draftScenario(idea: string) {
    scenarioDraft = await api.draftScenario(snapshot(), idea);
  }

  function useScenario(mode: "replace" | "append") {
    if (!form || scenarioDraft === null) return;
    scenarioBefore = form.scenario;
    form.scenario = mode === "replace" || !form.scenario.trim() ? scenarioDraft : `${form.scenario.trim()}\n\n${scenarioDraft}`;
    scenarioDraft = null;
  }

  async function draftGreetings(idea: string) {
    greetingDrafts = (await api.draftGreetings(snapshot(), idea, greetingCount)).map((text) => ({ text, pick: true }));
  }

  function addGreetings() {
    if (!form) return;
    const picked = greetingDrafts.filter((g) => g.pick).map((g) => g.text);
    // Fill an empty first box before adding new ones.
    if (form.firstMessages.length === 1 && !form.firstMessages[0].trim() && picked.length) form.firstMessages = picked;
    else form.firstMessages.push(...picked);
    greetingDrafts = [];
  }

  // ---- pictures ----

  async function chooseAvatar() {
    if (!form) return;
    try {
      const [picked] = await pickImages(false);
      if (picked) form.avatarPath = await api.storeImage(picked);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function addGallery() {
    if (!form) return;
    try {
      for (const picked of await pickImages(true)) form.galleryImagePaths.push(await api.storeImage(picked));
    } catch (e) {
      error = errorText(e);
    }
  }

  function removeGallery(path: string) {
    if (!form) return;
    form.galleryImagePaths = form.galleryImagePaths.filter((p) => p !== path);
    removedImages.push(path);
  }

  function toggleLorebook(id: string, on: boolean) {
    if (!form) return;
    form.linkedLorebookIds = on ? [...form.linkedLorebookIds, id] : form.linkedLorebookIds.filter((x) => x !== id);
  }
</script>

<section class="page">
  <header>
    <a href="/characters" class="back" aria-label="Back to characters">←</a>
    <h1>{isNew ? "New character" : (form?.name || "Edit character")}</h1>
  </header>

  {#if error}<p class="notice" role="alert">{error}</p>{/if}
  {#if notice}<p class="ok" role="status">{notice}</p>{/if}

  {#if form}
    <form
      class="form"
      onsubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div class="idrow">
        <button type="button" class="avatarbtn" onclick={chooseAvatar} title="Choose a picture" aria-label="Choose avatar">
          <Avatar name={form.name || "?"} path={form.avatarPath} size={88} />
        </button>
        <label class="field grow">
          Name
          <input bind:value={form.name} placeholder="Nova" required />
        </label>
      </div>

      <label class="field">
        Description
        <textarea rows="6" bind:value={form.description} placeholder="Who they are, what they look like…"></textarea>
      </label>
      <AppearanceSection subject="character" bind:appearance={form.appearance} bind:description={form.description} bind:birthDate={form.birthDate} />
      <label class="field">
        Personality
        <textarea rows="3" bind:value={form.personality}></textarea>
      </label>
      <label class="field">
        Scenario
        <textarea rows="3" bind:value={form.scenario}></textarea>
      </label>
      <AiDraft generate={draftScenario} placeholder={"Optional: e.g. {{user}} is the new apprentice at her bakery"}>
        {#if scenarioDraft !== null}
          <div class="preview">{scenarioDraft}</div>
          <div class="btnrow">
            <button type="button" onclick={() => useScenario("replace")}>Use as scenario</button>
            <button type="button" onclick={() => useScenario("append")}>Append</button>
            <button type="button" class="ghost" onclick={() => (scenarioDraft = null)}>Discard</button>
          </div>
        {/if}
        {#if scenarioBefore !== null}
          <button type="button" class="ghost" onclick={() => { form!.scenario = scenarioBefore!; scenarioBefore = null; }}>Undo scenario change</button>
        {/if}
      </AiDraft>

      <h2>Greetings</h2>
      <p class="muted small">The first one opens a new chat; the others are alternate greetings (SillyTavern's “alternate_greetings”).</p>
      {#each form.firstMessages as _, i (i)}
        <div class="greeting">
          <textarea rows="3" bind:value={form.firstMessages[i]} aria-label="Greeting {i + 1}"></textarea>
          <button type="button" class="ghost danger" onclick={() => form!.firstMessages.splice(i, 1)} aria-label="Remove greeting {i + 1}">✕</button>
        </div>
      {/each}
      <div><button type="button" onclick={() => form!.firstMessages.push("")}>＋ Add greeting</button></div>
      <AiDraft generate={draftGreetings} placeholder="Optional: mood or situation for the opening">
        {#snippet options()}
          <label class="field inline">
            How many
            <select bind:value={greetingCount}>{#each [1, 2, 3, 4, 5] as n (n)}<option value={n}>{n}</option>{/each}</select>
          </label>
        {/snippet}
        {#each greetingDrafts as draft, i (i)}
          <label class="pick">
            <input type="checkbox" bind:checked={draft.pick} />
            <span class="preview">{draft.text}</span>
          </label>
        {/each}
        {#if greetingDrafts.length}
          <div class="btnrow">
            <button type="button" onclick={addGreetings} disabled={!greetingDrafts.some((g) => g.pick)}>Add selected greetings</button>
            <button type="button" class="ghost" onclick={() => (greetingDrafts = [])}>Discard</button>
          </div>
        {/if}
      </AiDraft>

      <h2>Schedule</h2>
      <p class="muted small">What a normal week looks like. In roleplays the model is told what the routine has {form.name || "them"} doing right now — workdays, weekends and public holidays included.</p>
      <ScheduleSection bind:schedule={form.schedule} character={snapshot} />

      <label class="field">
        Example dialogue
        <textarea rows="5" bind:value={form.exampleDialogue} placeholder={"<START>\n{{user}}: Hi\n{{char}}: Hello!"}></textarea>
      </label>
      <label class="field">
        Tags (comma separated)
        <input bind:value={tagsText} placeholder="sci-fi, friendly" />
      </label>

      <h2>Lorebooks</h2>
      {#if lorebooks.length === 0}
        <p class="muted small">No lorebooks yet — create one under Lorebooks, then link it here.</p>
      {/if}
      {#each lorebooks as book (book.id)}
        <label class="check">
          <input
            type="checkbox"
            checked={form.linkedLorebookIds.includes(book.id)}
            onchange={(e) => toggleLorebook(book.id, e.currentTarget.checked)}
          />
          {book.name} <span class="muted small">({book.entries.length} entries)</span>
        </label>
      {/each}

      <CharacterPictures bind:character={form} discard={(path) => removedImages.push(path)} />

      <h2>Gallery</h2>
      <div class="gallery">
        {#each form.galleryImagePaths as path (path)}
          <div class="gitem">
            <img src={fileUrl(path)} alt="" />
            <button type="button" class="ghost danger" onclick={() => removeGallery(path)} aria-label="Remove picture">✕</button>
          </div>
        {/each}
      </div>
      <div><button type="button" onclick={addGallery}>＋ Add pictures</button></div>

      <div class="actionbar">
        <button class="primary" type="submit" disabled={saving || !form.name.trim()}>{saving ? "Saving…" : "Save"}</button>
        <a class="cancel" href="/characters">Cancel</a>
        <span class="spacer"></span>
        <button type="button" onclick={exportCard} title="Save as a SillyTavern character card (v2 JSON)">Export card…</button>
        {#if !isNew}<button type="button" class="danger" onclick={remove}>Delete</button>{/if}
      </div>
    </form>
  {:else if !error}
    <p class="muted">Loading…</p>
  {/if}
</section>

<style>
  .back {
    text-decoration: none;
    font-size: 20px;
    color: var(--muted);
  }

  .ok {
    color: #4fc27a;
  }

  .small {
    font-size: 12px;
    margin: 0;
  }

  .idrow {
    display: flex;
    gap: 16px;
    align-items: flex-end;
  }

  .grow {
    flex: 1;
  }

  .avatarbtn {
    padding: 0;
    border-radius: 50%;
    border: 2px dashed var(--border);
    background: transparent;
    line-height: 0;
  }

  .greeting {
    display: flex;
    gap: 6px;
    align-items: flex-start;
  }





  .gallery {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .gitem {
    position: relative;
    width: 96px;
    height: 96px;
  }

  .gitem img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
  }

  .gitem button {
    position: absolute;
    top: 2px;
    right: 2px;
    padding: 0 6px;
    background: rgba(0, 0, 0, 0.6);
    color: white;
  }

  .cancel {
    color: var(--muted);
    text-decoration: none;
    padding: 6px 10px;
  }

  .preview {
    white-space: pre-wrap;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 8px 10px;
    font-size: 13px;
  }

  .pick {
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }

  .pick .preview {
    flex: 1;
  }

  .inline {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
</style>

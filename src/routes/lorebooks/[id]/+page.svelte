<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, errorText } from "#lib/api.js";
  import AiDraft from "#lib/components/AiDraft.svelte";
  import type { Character, Lorebook, WorldInfoEntry, WorldInfoPosition } from "#lib/types.js";
  import { newId, splitList } from "#lib/util.js";

  /** An entry as the form edits it: the key lists are comma-separated text until saved. */
  interface EntryForm extends Omit<WorldInfoEntry, "keys" | "secondaryKeys"> {
    keysText: string;
    secondaryText: string;
  }

  const POSITIONS: { id: WorldInfoPosition; label: string }[] = [
    { id: "beforeCharacterDescription", label: "Before character description" },
    { id: "afterCharacterDescription", label: "After character description" },
    { id: "atDepth", label: "At depth in chat history" },
  ];

  let id = $state("");
  let name = $state("");
  let scanDepth = $state(4);
  let tokenBudget = $state(1000);
  let entries = $state<EntryForm[]>([]);
  let loaded = $state(false);
  let error = $state("");
  let saving = $state(false);
  const isNew = $derived(page.params.id === "new");

  const toForm = (e: WorldInfoEntry): EntryForm => {
    const { keys, secondaryKeys, ...rest } = e;
    return { ...rest, keysText: keys.join(", "), secondaryText: (secondaryKeys ?? []).join(", ") };
  };

  // ---- AI drafts ----

  let characters = $state<Character[]>([]);
  let draftCount = $state(3);
  let draftCharacterId = $state("");
  let drafts = $state<{ entry: WorldInfoEntry; pick: boolean }[]>([]);

  $effect(() => {
    api.listCharacters().then((list) => (characters = list), () => (characters = []));
  });

  async function draftEntries(topic: string) {
    const existingKeys = entries.flatMap((e) => splitList(e.keysText));
    const result = await api.draftLoreEntries({
      lorebookName: name.trim() || "Untitled",
      topic,
      count: draftCount,
      existingKeys,
      character: characters.find((c) => c.id === draftCharacterId) ?? null,
    });
    drafts = result.map((entry) => ({ entry, pick: true }));
  }

  function addDrafts() {
    for (const { entry } of drafts.filter((d) => d.pick)) {
      entries.push({ ...toForm(entry), id: newId(), insertionOrder: entries.length });
    }
    drafts = [];
  }

  function blankEntry(): EntryForm {
    return {
      id: newId(),
      keysText: "",
      secondaryText: "",
      content: "",
      caseSensitive: false,
      enabled: true,
      insertionOrder: entries.length,
      position: "beforeCharacterDescription",
      depth: 0,
    };
  }

  $effect(() => {
    const wanted = page.params.id;
    void (async () => {
      try {
        if (wanted === "new") {
          id = newId();
          name = "";
          scanDepth = 4;
          tokenBudget = 1000;
          entries = [];
          loaded = true;
          return;
        }
        const book = (await api.listLorebooks()).find((b) => b.id === wanted);
        if (!book) {
          error = "That lorebook no longer exists.";
          return;
        }
        id = book.id;
        name = book.name;
        scanDepth = book.scanDepth;
        tokenBudget = book.tokenBudget;
        entries = book.entries.map(toForm);
        loaded = true;
      } catch (e) {
        error = errorText(e);
      }
    })();
  });

  async function save() {
    error = "";
    saving = true;
    try {
      const book: Lorebook = {
        id,
        name,
        scanDepth: Number(scanDepth) || 0,
        tokenBudget: Number(tokenBudget) || 0,
        entries: entries.map(({ keysText, secondaryText, ...rest }) => ({
          ...$state.snapshot(rest),
          keys: splitList(keysText),
          secondaryKeys: splitList(secondaryText).length > 0 ? splitList(secondaryText) : null,
          insertionOrder: Number(rest.insertionOrder) || 0,
          depth: Number(rest.depth) || 0,
        })),
      };
      await api.saveLorebook(book);
      await goto("/lorebooks");
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }
</script>

<section class="page">
  <header>
    <a href="/lorebooks" class="back" aria-label="Back to lorebooks">←</a>
    <h1>{isNew ? "New lorebook" : (name || "Edit lorebook")}</h1>
  </header>

  {#if error}<p class="notice" role="alert">{error}</p>{/if}

  {#if loaded}
    <form
      class="form"
      onsubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <label class="field">
        Name
        <input bind:value={name} placeholder="Lantern Belt" required />
      </label>
      <div class="two">
        <label class="field">
          Scan depth (recent messages searched for keywords)
          <input type="number" min="0" bind:value={scanDepth} />
        </label>
        <label class="field">
          Token budget (approximate)
          <input type="number" min="0" step="100" bind:value={tokenBudget} />
        </label>
      </div>

      <h2>Entries ({entries.length})</h2>
      {#each entries as entry, i (entry.id)}
        <fieldset class="entry card" class:off={!entry.enabled}>
          <div class="entryhead">
            <label class="check"><input type="checkbox" bind:checked={entry.enabled} /> Enabled</label>
            <label class="check"><input type="checkbox" bind:checked={entry.caseSensitive} /> Case sensitive</label>
            <span class="grow"></span>
            <button type="button" class="ghost danger" onclick={() => entries.splice(i, 1)}>Remove</button>
          </div>
          <label class="field">
            Keywords (comma separated) — any one triggers the entry
            <input bind:value={entry.keysText} placeholder="dragon, wyrm" />
          </label>
          <label class="field">
            Also require one of (optional)
            <input bind:value={entry.secondaryText} placeholder="fire, cave" />
          </label>
          <label class="field">
            Content
            <textarea rows="4" bind:value={entry.content}></textarea>
          </label>
          <div class="three">
            <label class="field">
              Position
              <select bind:value={entry.position}>
                {#each POSITIONS as position (position.id)}<option value={position.id}>{position.label}</option>{/each}
              </select>
            </label>
            {#if entry.position === "atDepth"}
              <label class="field">
                Depth (messages from the end)
                <input type="number" min="0" bind:value={entry.depth} />
              </label>
            {/if}
            <label class="field">
              Priority (lower = first)
              <input type="number" bind:value={entry.insertionOrder} />
            </label>
          </div>
        </fieldset>
      {/each}
      <div><button type="button" onclick={() => entries.push(blankEntry())}>＋ Add entry</button></div>
      <AiDraft label="✨ Draft entries with AI" placeholder="What should the entries be about? e.g. the factions of the harbor city" generate={draftEntries}>
        {#snippet options()}
          <label class="field inline">
            How many
            <select bind:value={draftCount}>{#each [1, 2, 3, 4, 5, 6, 8, 10] as n (n)}<option value={n}>{n}</option>{/each}</select>
          </label>
          <label class="field inline">
            Fit to
            <select bind:value={draftCharacterId}>
              <option value="">no particular character</option>
              {#each characters as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
            </select>
          </label>
        {/snippet}
        {#each drafts as draft, i (i)}
          <label class="pick">
            <input type="checkbox" bind:checked={draft.pick} />
            <span class="draftentry">
              <strong>{draft.entry.keys.join(", ")}</strong>
              <span>{draft.entry.content}</span>
            </span>
          </label>
        {/each}
        {#if drafts.length}
          <div class="btnrow">
            <button type="button" onclick={addDrafts} disabled={!drafts.some((d) => d.pick)}>Add selected entries</button>
            <button type="button" class="ghost" onclick={() => (drafts = [])}>Discard</button>
            <span class="muted small">Added entries are saved with the lorebook.</span>
          </div>
        {/if}
      </AiDraft>

      <div class="actionbar">
        <button class="primary" type="submit" disabled={saving || !name.trim()}>{saving ? "Saving…" : "Save"}</button>
        <a class="cancel" href="/lorebooks">Cancel</a>
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

  .two,
  .three {
    display: grid;
    gap: 12px;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  }

  .entry {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 0;
    min-width: 0;
  }

  .entry.off {
    opacity: 0.6;
  }

  .entryhead {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .grow {
    flex: 1;
  }

  .cancel {
    color: var(--muted);
    text-decoration: none;
    padding: 6px 10px;
  }

  .inline {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }

  .pick {
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }

  .draftentry {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    white-space: pre-wrap;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 8px 10px;
    font-size: 13px;
  }

  .small {
    font-size: 12px;
  }
</style>

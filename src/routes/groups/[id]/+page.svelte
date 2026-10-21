<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, errorText } from "$lib/api";
  import AiDraft from "$lib/components/AiDraft.svelte";
  import Avatar from "$lib/components/Avatar.svelte";
  import type { Character, Country, GroupActivationMode, GroupChat } from "$lib/types";
  import { move, newId } from "$lib/util";

  const MODES: { id: GroupActivationMode; label: string; help: string }[] = [
    { id: "manual", label: "Manual", help: "After each of your messages you choose who replies." },
    { id: "roundRobin", label: "Round robin", help: "Members take turns in the order listed below, skipping whoever spoke last." },
    { id: "triggerWord", label: "Trigger word", help: "Anyone whose name you mention replies (in the order named); if nobody is named, it's round robin." },
  ];

  let form = $state<GroupChat | null>(null);
  let characters = $state<Character[]>([]);
  let countries = $state<Country[]>([]);
  let scenarioDraft = $state<string | null>(null);
  let scenarioBefore = $state<string | null>(null);
  let error = $state("");
  let saving = $state(false);
  const isNew = $derived(page.params.id === "new");

  $effect(() => {
    const wanted = page.params.id;
    void (async () => {
      try {
        characters = await api.listCharacters();
        void api.holidayCountries().then((list) => (countries = list)).catch(() => {});
        const loaded: GroupChat | undefined =
          wanted === "new"
            ? {
                id: newId(),
                name: "",
                characterIds: [],
                activationMode: "manual",
                allowSelfResponses: false,
                jointReplies: false,
                privateTalks: true,
                scenario: "",
                setting: { countryCode: null, regionCode: null, storyStart: null },
              }
            : (await api.listGroupChats()).find((g) => g.id === wanted);
        if (loaded) form = loaded;
        else error = "That group no longer exists.";
      } catch (e) {
        error = errorText(e);
      }
    })();
  });

  const byId = (id: string) => characters.find((c) => c.id === id);
  const help = $derived(MODES.find((m) => m.id === form?.activationMode)?.help ?? "");

  function toggle(id: string, on: boolean) {
    if (!form) return;
    form.characterIds = on ? [...form.characterIds, id] : form.characterIds.filter((x) => x !== id);
  }

  async function draftScenario(idea: string) {
    if (!form) return;
    scenarioDraft = await api.draftGroupScenario($state.snapshot(form.characterIds), form.scenario, idea);
  }

  function useScenario() {
    if (!form || scenarioDraft === null) return;
    scenarioBefore = form.scenario;
    form.scenario = scenarioDraft;
    scenarioDraft = null;
  }

  async function save() {
    if (!form) return;
    error = "";
    saving = true;
    try {
      await api.saveGroupChat($state.snapshot(form) as GroupChat);
      await goto("/groups");
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }
</script>

<section class="page">
  <header>
    <a href="/groups" class="back" aria-label="Back to group chats">←</a>
    <h1>{isNew ? "New group chat" : (form?.name || "Edit group chat")}</h1>
  </header>

  {#if error}<p class="notice" role="alert">{error}</p>{/if}

  {#if form}
    <form
      class="form"
      onsubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <label class="field">
        Name
        <input bind:value={form.name} placeholder="The Crew" required />
      </label>

      <label class="field">
        Who replies
        <select bind:value={form.activationMode}>
          {#each MODES as mode (mode.id)}<option value={mode.id}>{mode.label}</option>{/each}
        </select>
      </label>
      <p class="muted small">{help}</p>

      <label class="check">
        <input type="checkbox" bind:checked={form.allowSelfResponses} />
        Let a character reply twice in a row
      </label>

      <label class="check">
        <input type="checkbox" bind:checked={form.jointReplies} />
        Let several characters answer one message
      </label>
      <p class="muted small">
        After each reply the model decides whether another member would chime in too, and if so they answer right after. Each member answers a
        message at most once this way.
      </p>

      <label class="check">
        <input type="checkbox" bind:checked={form.privateTalks} />
        Let characters take you aside for a private talk
      </label>
      <p class="muted small">
        A member can pull you into a one-on-one chat of their own; you can always start one from the group chat's header. A private talk
        sees the group chat, but the group and the other members never see the talk.
      </p>

      <h2>Members</h2>
      {#if characters.length === 0}
        <p class="muted">No characters yet — create some first.</p>
      {/if}
      <div class="picker">
        {#each characters as character (character.id)}
          <label class="check member">
            <input
              type="checkbox"
              checked={form.characterIds.includes(character.id)}
              onchange={(e) => toggle(character.id, e.currentTarget.checked)}
            />
            <Avatar name={character.name} path={character.avatarPath} size={24} />
            {character.name}
          </label>
        {/each}
      </div>

      {#if form.characterIds.length > 1}
        <h2>Order</h2>
        <p class="muted small">Used for round robin, and as the order replies are considered.</p>
        <ol class="order">
          {#each form.characterIds as id, i (id)}
            <li class="card">
              <span class="muted">{i + 1}</span>
              <span class="grow">{byId(id)?.name ?? "(deleted character)"}</span>
              <button type="button" class="ghost" onclick={() => move(form!.characterIds, i, -1)} disabled={i === 0} aria-label="Move up">↑</button>
              <button
                type="button"
                class="ghost"
                onclick={() => move(form!.characterIds, i, 1)}
                disabled={i === form.characterIds.length - 1}
                aria-label="Move down">↓</button
              >
            </li>
          {/each}
        </ol>
      {/if}

      <h2>Story</h2>
      <label class="field">
        Scenario
        <textarea rows="4" bind:value={form.scenario} placeholder={"Where {{user}} and the members are and what's going on - for everyone in the group"}></textarea>
      </label>
      <p class="muted small">
        Used for the whole group, in place of the members' own scenarios (each card's is written for a chat with that character alone). Leave
        it empty to keep theirs.
      </p>
      <AiDraft generate={draftScenario} placeholder={"Optional: e.g. a storm traps everyone at the lodge"}>
        {#if scenarioDraft !== null}
          <div class="preview">{scenarioDraft}</div>
          <div class="btnrow">
            <button type="button" onclick={useScenario}>Use as scenario</button>
            <button type="button" class="ghost" onclick={() => (scenarioDraft = null)}>Discard</button>
          </div>
        {/if}
        {#if scenarioBefore !== null}
          <button type="button" class="ghost" onclick={() => { form!.scenario = scenarioBefore!; scenarioBefore = null; }}>Undo scenario change</button>
        {/if}
      </AiDraft>

      <div class="setting">
        <label class="field">
          Set in (country)
          {#if countries.length}
            <select value={form.setting.countryCode ?? ""} onchange={(e) => (form!.setting.countryCode = e.currentTarget.value || null)}>
              <option value="">— the members' home —</option>
              {#each countries as c (c.countryCode)}<option value={c.countryCode}>{c.name}</option>{/each}
            </select>
          {:else}
            <input maxlength="2" placeholder="e.g. DE" value={form.setting.countryCode ?? ""} oninput={(e) => (form!.setting.countryCode = e.currentTarget.value.toUpperCase() || null)} />
          {/if}
        </label>
        <label class="field">
          Region (optional)
          <input placeholder="e.g. DE-BY" value={form.setting.regionCode ?? ""} oninput={(e) => (form!.setting.regionCode = e.currentTarget.value.toUpperCase() || null)} />
        </label>
        <label class="field">
          Story starts (date and time)
          <input
            type="datetime-local"
            value={form.setting.storyStart?.slice(0, 16) ?? ""}
            onchange={(e) => (form!.setting.storyStart = e.currentTarget.value ? `${e.currentTarget.value}:00` : null)}
          />
        </label>
      </div>
      <p class="muted small">
        New chats with this group are set here (its public holidays apply) and begin at this date and time. Whatever you leave empty comes from
        the members' own settings. A chat that's already running keeps its clock: change that in its Story panel.
      </p>

      <div class="actionbar">
        <button class="primary" type="submit" disabled={saving || !form.name.trim()}>{saving ? "Saving…" : "Save"}</button>
        <a class="cancel" href="/groups">Cancel</a>
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

  .small {
    font-size: 12px;
    margin: 0;
  }

  .picker {
    display: grid;
    gap: 6px 16px;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  }

  .member {
    gap: 8px;
  }

  .order {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-width: 420px;
  }

  .order li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 8px 4px 12px;
  }

  .grow {
    flex: 1;
  }

  .setting {
    display: flex;
    flex-wrap: wrap;
    gap: 10px 12px;
  }

  .preview {
    white-space: pre-wrap;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    margin-bottom: 6px;
  }

  .cancel {
    color: var(--muted);
    text-decoration: none;
    padding: 6px 10px;
  }
</style>

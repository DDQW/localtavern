<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, errorText } from "$lib/api";
  import Avatar from "$lib/components/Avatar.svelte";
  import type { Character, GroupActivationMode, GroupChat } from "$lib/types";
  import { move, newId } from "$lib/util";

  const MODES: { id: GroupActivationMode; label: string; help: string }[] = [
    { id: "manual", label: "Manual", help: "After each of your messages you choose who replies." },
    { id: "roundRobin", label: "Round robin", help: "Members take turns in the order listed below, skipping whoever spoke last." },
    { id: "triggerWord", label: "Trigger word", help: "Anyone whose name you mention replies (in the order named); if nobody is named, it's round robin." },
  ];

  let form = $state<GroupChat | null>(null);
  let characters = $state<Character[]>([]);
  let error = $state("");
  let saving = $state(false);
  const isNew = $derived(page.params.id === "new");

  $effect(() => {
    const wanted = page.params.id;
    void (async () => {
      try {
        characters = await api.listCharacters();
        const loaded =
          wanted === "new"
            ? { id: newId(), name: "", characterIds: [], activationMode: "manual" as const, allowSelfResponses: false }
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

  .cancel {
    color: var(--muted);
    text-decoration: none;
    padding: 6px 10px;
  }
</style>

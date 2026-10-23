<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, errorText } from "#lib/api.js";
  import AppearanceSection from "#lib/components/AppearanceSection.svelte";
  import Avatar from "#lib/components/Avatar.svelte";
  import { pickImages } from "#lib/pick.js";
  import type { Persona } from "#lib/types.js";
  import { newId } from "#lib/util.js";

  let form = $state<Persona | null>(null);
  let error = $state("");
  let saving = $state(false);
  const isNew = $derived(page.params.id === "new");

  $effect(() => {
    const id = page.params.id;
    void (async () => {
      try {
        const loaded =
          id === "new"
            ? {
                id: newId(),
                name: "",
                avatarPath: null,
                description: "",
                isDefault: false,
                appearance: null,
                birthDate: null,
              }
            : (await api.listPersonas()).find((p) => p.id === id);
        if (loaded) form = loaded;
        else error = "That persona no longer exists.";
      } catch (e) {
        error = errorText(e);
      }
    })();
  });

  async function chooseAvatar() {
    if (!form) return;
    try {
      const [picked] = await pickImages(false);
      if (picked) form.avatarPath = await api.storeImage(picked);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function save() {
    if (!form) return;
    error = "";
    saving = true;
    try {
      await api.savePersona($state.snapshot(form) as Persona);
      await goto("/personas");
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }
</script>

<section class="page">
  <header>
    <a href="/personas" class="back" aria-label="Back to personas">←</a>
    <h1>{isNew ? "New persona" : (form?.name || "Edit persona")}</h1>
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
      <div class="idrow">
        <button type="button" class="avatarbtn" onclick={chooseAvatar} aria-label="Choose avatar" title="Choose a picture">
          <Avatar name={form.name || "?"} path={form.avatarPath} size={72} />
        </button>
        <label class="field grow">
          Name
          <input bind:value={form.name} placeholder="Alex" required />
        </label>
      </div>

      <label class="field">
        Description
        <textarea rows="5" bind:value={form.description} placeholder="A curious traveler…"></textarea>
      </label>
      <AppearanceSection subject="persona" bind:appearance={form.appearance} bind:description={form.description} bind:birthDate={form.birthDate} />

      <label class="check">
        <input type="checkbox" bind:checked={form.isDefault} />
        Use as the default persona (only one persona can be the default)
      </label>

      <div class="actionbar">
        <button class="primary" type="submit" disabled={saving || !form.name.trim()}>{saving ? "Saving…" : "Save"}</button>
        <a class="cancel" href="/personas">Cancel</a>
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

  .cancel {
    color: var(--muted);
    text-decoration: none;
    padding: 6px 10px;
  }
</style>

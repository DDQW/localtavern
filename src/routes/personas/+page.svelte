<script lang="ts">
  import { api, errorText } from "$lib/api";
  import Avatar from "$lib/components/Avatar.svelte";
  import type { Persona } from "$lib/types";

  let personas = $state<Persona[]>([]);
  let error = $state("");
  let loaded = $state(false);

  $effect(() => {
    api
      .listPersonas()
      .then((list) => (personas = list))
      .catch((e) => (error = errorText(e)))
      .finally(() => (loaded = true));
  });

  async function remove(persona: Persona) {
    if (!confirm(`Delete the persona “${persona.name}”?`)) return;
    try {
      await api.deletePersona(persona.id);
      personas = personas.filter((p) => p.id !== persona.id);
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<section class="page">
  <header>
    <h1>Personas</h1>
    <span class="grow"></span>
    <a class="newbtn" href="/personas/new">＋ New persona</a>
  </header>
  <p class="muted intro">A persona is who <em>you</em> are in a chat — its name replaces <code>{"{{user}}"}</code>, and its description is shown to the model.</p>

  {#if error}<p class="notice">{error}</p>{/if}
  {#if loaded && personas.length === 0}<p class="muted">No personas yet.</p>{/if}

  <ul class="list">
    {#each personas as persona (persona.id)}
      <li class="card listrow">
        <Avatar name={persona.name} path={persona.avatarPath} size={44} />
        <div class="grow">
          <div class="title">
            {persona.name}
            {#if persona.isDefault}<span class="pill">default</span>{/if}
          </div>
          <div class="muted desc">{persona.description || "No description"}</div>
        </div>
        <a class="linkbtn" href="/personas/{persona.id}">Edit</a>
        <button class="ghost danger" onclick={() => remove(persona)}>Delete</button>
      </li>
    {/each}
  </ul>
</section>

<style>
  .grow {
    flex: 1;
  }

  .intro {
    margin-top: -8px;
  }

  .newbtn,
  .linkbtn {
    display: inline-block;
    text-decoration: none;
    border: 1px solid var(--border);
    background: var(--panel-2);
    color: var(--text);
    border-radius: var(--radius-sm);
    padding: 6px 12px;
  }

  .newbtn {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }

  .desc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }

  code {
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 12px;
    background: var(--panel-3);
    padding: 1px 5px;
    border-radius: 4px;
  }
</style>

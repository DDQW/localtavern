<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, errorText } from "$lib/api";
  import type { PromptPresetFull } from "$lib/types";
  import { newId } from "$lib/util";

  let presets = $state<PromptPresetFull[]>([]);
  let error = $state("");
  let loaded = $state(false);

  async function load() {
    try {
      presets = await api.listPresets();
    } catch (e) {
      error = errorText(e);
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    void load();
  });

  async function duplicate(preset: PromptPresetFull) {
    try {
      const copy: PromptPresetFull = {
        ...$state.snapshot(preset),
        id: newId(),
        name: `${preset.name} (copy)`,
        blocks: preset.blocks.map((b) => ({ ...$state.snapshot(b), id: newId() })),
      };
      const saved = await api.savePreset(copy);
      await goto(`/presets/${saved.id}`);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function remove(preset: PromptPresetFull) {
    if (!confirm(`Delete the preset “${preset.name}”? Chats that used it fall back to the default.`)) return;
    try {
      await api.deletePreset(preset.id);
      presets = presets.filter((p) => p.id !== preset.id);
    } catch (e) {
      error = errorText(e);
    }
  }

  const sampling = (p: PromptPresetFull) =>
    [
      p.temperature !== null ? `temp ${p.temperature}` : "",
      p.topP !== null ? `top-p ${p.topP}` : "",
      p.maxOutputTokens !== null ? `${p.maxOutputTokens} max tokens` : "",
      p.reasoningEffort ? `${p.reasoningEffort} reasoning` : "",
    ]
      .filter(Boolean)
      .join(" · ") || "provider defaults";
</script>

<section class="page">
  <header>
    <h1>Prompt Presets</h1>
    <span class="grow"></span>
    <a class="newbtn" href="/presets/new">＋ New preset</a>
  </header>
  <p class="muted intro">A preset decides what goes into the prompt, in what order, plus sampling settings. Pick one per chat from the chat header.</p>

  {#if error}<p class="notice">{error}</p>{/if}
  {#if loaded && presets.length === 0}<p class="muted">No presets yet.</p>{/if}

  <ul class="list">
    {#each presets as preset (preset.id)}
      <li class="card listrow">
        <div class="grow">
          <div class="title">{preset.name}</div>
          <div class="muted meta">{preset.blocks.length} blocks · {sampling(preset)}</div>
        </div>
        <a class="linkbtn" href="/presets/{preset.id}">Edit</a>
        <button class="ghost" onclick={() => duplicate(preset)}>Duplicate</button>
        <button class="ghost danger" onclick={() => remove(preset)}>Delete</button>
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
    max-width: 700px;
  }

  .meta {
    font-size: 12px;
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
</style>

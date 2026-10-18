<script lang="ts">
  import { api, errorText } from "$lib/api";
  import { chat } from "$lib/chat.svelte";
  import type { ImagePromptDraft } from "$lib/types";

  let dialog = $state<HTMLDialogElement>();
  let draft = $state<ImagePromptDraft | null>(null);
  let prefix = $state("");
  let prefixStatus = $state("");

  const job = $derived(chat.picture);
  const character = $derived(chat.session?.characters.find((c) => c.id === draft?.characterId));

  // Open while a draft waits for review; close when it's rendered, cancelled or the chat changes.
  $effect(() => {
    if (job?.phase === "review" && job.draft) {
      draft = structuredClone($state.snapshot(job.draft));
      prefixStatus = "";
      if (dialog && !dialog.open) dialog.showModal();
    } else if (dialog?.open) {
      dialog.close();
    }
  });

  $effect(() => {
    const id = character?.id;
    prefix = "";
    if (id) void api.getCharacterImagePrefix(id).then((p) => (prefix = p)).catch(() => {});
  });

  function oncancel(event: Event) {
    event.preventDefault();
    chat.cancelPictureReview();
  }

  function draw() {
    if (draft && draft.prompt.trim()) void chat.renderPicture($state.snapshot(draft));
  }

  async function savePrefix() {
    if (!character) return;
    try {
      await api.setCharacterImagePrefix(character.id, prefix);
      prefixStatus = "Saved - used from the next picture of " + character.name + " on.";
    } catch (e) {
      prefixStatus = errorText(e);
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      draw();
    }
  }
</script>

<dialog bind:this={dialog} {oncancel} aria-labelledby="picture-title">
  {#if draft}
    <h2 id="picture-title">Draw this picture?</h2>
    <p class="muted small">Written by the chat model. Change anything you like; the image server gets exactly this.</p>

    <label class="field">
      Prompt
      <textarea rows="5" bind:value={draft.prompt} {onkeydown}></textarea>
    </label>
    <label class="field">
      Negative prompt
      <textarea rows="2" bind:value={draft.negativePrompt} {onkeydown}></textarea>
    </label>
    <div class="size">
      <label class="field">Width <input type="number" min="64" max="4096" step="64" bind:value={draft.width} /></label>
      <span class="muted">×</span>
      <label class="field">Height <input type="number" min="64" max="4096" step="64" bind:value={draft.height} /></label>
      <button class="ghost" onclick={() => draft && ([draft.width, draft.height] = [draft.height, draft.width])} title="Swap width and height">⇄</button>
    </div>

    {#if character}
      <details class="fold">
        <summary>Always add for {character.name}</summary>
        <p class="muted small">
          Put in front of every picture of {character.name}: a LoRA like <code>&lt;lora:name:0.8&gt;</code> or the look that must
          never change. Not added to this prompt - edit that above.
        </p>
        <div class="row">
          <input bind:value={prefix} placeholder="e.g. <lora:nova:0.8>, silver hair, blue eyes" aria-label="Prefix for {character.name}" />
          <button onclick={savePrefix}>Save</button>
        </div>
        {#if prefixStatus}<p class="muted small" role="status">{prefixStatus}</p>{/if}
      </details>
    {/if}

    <div class="actions">
      <span class="muted small hint">Ctrl+Enter draws</span>
      <button onclick={() => chat.cancelPictureReview()}>Cancel</button>
      <button class="primary" onclick={draw} disabled={!draft.prompt.trim()}>Draw</button>
    </div>
  {/if}
</dialog>

<style>
  dialog {
    max-width: min(620px, calc(100vw - 32px));
    width: 100%;
    padding: 20px 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--panel);
    color: var(--text);
  }

  dialog::backdrop {
    background: rgba(0, 0, 0, 0.55);
  }

  h2 {
    font-size: 17px;
    margin: 0 0 4px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 10px;
    font-size: 12px;
    color: var(--muted);
  }

  .field textarea,
  .field input {
    color: var(--text);
    font-size: 13.5px;
  }

  textarea {
    resize: vertical;
  }

  .size {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }

  .size input {
    width: 100px;
  }

  .size span,
  .size button {
    margin-bottom: 6px;
  }

  .fold {
    margin-top: 12px;
    font-size: 13px;
  }

  .fold summary {
    cursor: pointer;
    color: var(--muted);
  }

  .row {
    display: flex;
    gap: 8px;
  }

  .row input {
    flex: 1;
  }

  .small {
    font-size: 12px;
    margin: 6px 0;
  }

  code {
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 12px;
    background: var(--panel-3);
    padding: 1px 5px;
    border-radius: 4px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 8px;
    margin-top: 16px;
  }

  .hint {
    margin-right: auto;
  }
</style>

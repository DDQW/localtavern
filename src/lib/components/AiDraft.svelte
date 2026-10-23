<script lang="ts">
  // "✨ Draft with AI": an idea box and a Generate button. The caller does the request in
  // `generate` and shows the result itself (so each field can offer replace / add / undo).
  import type { Snippet } from "svelte";
  import { errorText } from "#lib/api.js";

  let {
    label = "✨ Draft with AI",
    placeholder = "Optional: your idea, in a few words",
    generate,
    options,
    children,
  }: {
    label?: string;
    placeholder?: string;
    generate: (idea: string) => Promise<void>;
    /** Extra controls next to the Generate button (count, character, …). */
    options?: Snippet;
    /** The result preview, shown under the controls. */
    children?: Snippet;
  } = $props();

  let open = $state(false);
  let idea = $state("");
  let busy = $state(false);
  let error = $state("");

  async function run() {
    error = "";
    busy = true;
    try {
      await generate(idea);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="draft" class:open>
  <button type="button" class="toggle" aria-expanded={open} onclick={() => (open = !open)}>{label}</button>
  {#if open}
    <div class="body">
      <textarea rows="2" bind:value={idea} {placeholder} aria-label="Idea for the draft"></textarea>
      <div class="btnrow">
        {@render options?.()}
        <button type="button" class="primary" onclick={run} disabled={busy}>{busy ? "Writing…" : "Generate"}</button>
        <span class="muted small">Uses your last-used model.</span>
      </div>
      {#if error}<p class="notice" role="alert">{error}</p>{/if}
      {@render children?.()}
    </div>
  {/if}
</div>

<style>
  .draft {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }

  .draft.open {
    border: 1px dashed var(--border);
    border-radius: var(--radius-sm);
    padding: 8px 10px 10px;
    align-items: stretch;
  }

  .toggle {
    align-self: flex-start;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .small {
    font-size: 12px;
  }
</style>

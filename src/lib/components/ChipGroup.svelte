<script lang="ts">
  // One trait of the appearance builder: preset chips (optionally with a colour swatch) plus a
  // custom text entry. Clicking the selected chip clears it.
  import type { Choice } from "#lib/appearanceOptions.js";

  let {
    label,
    options,
    value = $bindable(null),
  }: { label: string; options: Choice[]; value: string | null } = $props();

  const isCustom = $derived(value !== null && value.trim() !== "" && !options.some((o) => o.value === value));
  let editing = $state(false);
  let draft = $state("");

  $effect(() => {
    if (isCustom) draft = value ?? "";
  });

  function choose(v: string) {
    value = value === v ? null : v;
    editing = false;
  }
</script>

<div class="group" role="group" aria-label={label}>
  <span class="label">{label}</span>
  <div class="chips">
    {#each options as o (o.value)}
      <button type="button" class="chip" class:on={value === o.value} aria-pressed={value === o.value} onclick={() => choose(o.value)}>
        {#if o.swatch}<span class="swatch" style:background={o.swatch}></span>{/if}{o.value}
      </button>
    {/each}
    {#if editing || isCustom}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="custom"
        class:on={isCustom}
        placeholder="Custom…"
        aria-label="Custom {label.toLowerCase()}"
        autofocus={editing && !isCustom}
        bind:value={draft}
        oninput={() => (value = draft.trim() || null)}
        onblur={() => (editing = false)}
      />
    {:else}
      <button type="button" class="chip add" onclick={() => ((draft = ""), (editing = true))}>Custom…</button>
    {/if}
  </div>
</div>

<style>
  .group {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .label {
    font-size: 12px;
    color: var(--muted);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    padding: 3px 11px;
    border-radius: 999px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    color: var(--text);
  }

  .chip:hover {
    background: var(--panel-3);
  }

  .chip.on,
  .custom.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 22%, var(--panel-2));
  }

  .chip.add {
    color: var(--muted);
    border-style: dashed;
  }

  .swatch {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1px solid color-mix(in srgb, var(--text) 25%, transparent);
  }

  .custom {
    font-size: 12px;
    padding: 3px 11px;
    border-radius: 999px;
    width: 150px;
  }
</style>

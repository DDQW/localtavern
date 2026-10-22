<script lang="ts">
  // Every text the app adds to a chat request on its own (roleplay rules, tags, the story clock,
  // photos, group and private-talk notes, Continue, Impersonate...): reworded here when one
  // misfires with a model, plus the switches around them.
  import { api, errorText } from "$lib/api";
  import type { InstructionInfo, Instructions } from "$lib/types";

  let instructions = $state<Instructions | null>(null);
  let defaults = $state<InstructionInfo[]>([]);
  let status = $state("");

  $effect(() => {
    void api
      .getInstructions()
      .then((view) => {
        instructions = view.instructions;
        defaults = view.defaults;
      })
      .catch((e) => (status = errorText(e)));
  });

  const groups = $derived([...new Set(defaults.map((d) => d.group))]);

  const changed = (info: InstructionInfo) => {
    const text = instructions?.texts[info.key];
    return text !== undefined && text.trim() !== "" && text.trim() !== info.default.trim();
  };

  function reset(info: InstructionInfo) {
    if (instructions) delete instructions.texts[info.key];
  }

  async function save() {
    if (!instructions) return;
    try {
      instructions = await api.saveInstructions($state.snapshot(instructions));
      status = "Instructions saved - they apply from the next reply.";
    } catch (e) {
      status = errorText(e);
    }
  }
</script>

<article class="card block" id="instructions">
  <h2>Built-in instructions</h2>
  <p class="muted small">
    Everything the app adds to a chat request besides your preset and your cards is here, word for word. If something misfires with your
    model, reword it.
    Placeholders like <code>{"{char}"}</code> and <code>{"{user}"}</code> are filled in for each reply. Keep the bracketed line a text asks
    for (<code>[photo: …]</code>, <code>[private]</code>…): that's how the app spots what the reply did.
  </p>

  {#if instructions}
    <label class="check">
      <input type="checkbox" bind:checked={instructions.userPhotos} />
      Draw the photos you take - when your message mentions one, the reply describes it
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={instructions.intimatePhotos} />
      Characters can send intimate photos (never a character whose birth date makes them under 18)
    </label>
    <label class="field">
      Photo words - your message mentioning one of these asks for your photo
      <input
        value={instructions.photoWords.join(", ")}
        onchange={(e) => (instructions!.photoWords = e.currentTarget.value.split(",").map((w) => w.trim()).filter(Boolean))}
      />
    </label>

    {#each groups as group (group)}
    <h3>{group}</h3>
    <div class="texts">
      {#each defaults.filter((d) => d.group === group) as info (info.key)}
        <details class="text">
          <summary>
            {info.title}
            {#if changed(info)}<span class="badge">changed</span>{/if}
          </summary>
          <p class="muted small">{info.help} Placeholders: {info.placeholders.join(", ")}.</p>
          <textarea
            rows="7"
            value={instructions.texts[info.key] ?? info.default}
            oninput={(e) => (instructions!.texts[info.key] = e.currentTarget.value)}
            aria-label={info.title}
          ></textarea>
          <div class="btnrow">
            <button type="button" class="ghost" onclick={() => reset(info)} disabled={!changed(info)}>Reset to built-in</button>
          </div>
        </details>
      {/each}
    </div>
    {/each}

    <div class="btnrow">
      <button class="primary" onclick={save}>Save instructions</button>
      {#if status}<span class="muted small" role="status">{status}</span>{/if}
    </div>
  {:else if status}
    <p class="small err">{status}</p>
  {/if}
</article>

<style>
  .block {
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  h2 {
    font-size: 16px;
  }

  .small {
    font-size: 12px;
    margin: 0;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--muted);
  }

  .field input {
    color: var(--text);
    font-size: 14px;
  }

  h3 {
    font-size: 13px;
    margin: 8px 0 0;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .texts {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .text {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 6px 10px;
  }

  .text summary {
    cursor: pointer;
    font-size: 14px;
  }

  .text p {
    margin: 6px 0;
  }

  .text textarea {
    width: 100%;
    font-size: 13px;
    resize: vertical;
  }

  .badge {
    margin-left: 6px;
    font-size: 11px;
    color: var(--accent);
  }

  code {
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 12px;
    background: var(--panel-3);
    padding: 1px 5px;
    border-radius: 4px;
  }

  .err {
    color: var(--danger);
  }
</style>

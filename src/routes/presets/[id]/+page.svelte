<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, errorText } from "$lib/api";
  import type { PromptBlockFull, PromptBlockType, PromptPresetFull, ReasoningEffort } from "$lib/types";
  import { move, newId, numberOrNull } from "$lib/util";

  const LABELS: Record<PromptBlockType, string> = {
    systemPrompt: "System prompt",
    personaDescription: "Persona description",
    characterDescription: "Character description",
    scenario: "Scenario",
    exampleDialogue: "Example dialogue",
    worldInfo: "World info (lorebooks)",
    chatHistory: "Chat history",
    authorsNote: "Author's note",
    custom: "Custom",
    userMemory: "User memory",
    postHistoryInstructions: "Post-history instructions",
    worldState: "World state (date, time, weather)",
    characterNotebook: "Character notebook",
    questLog: "Quest log",
  };

  const HELP: Partial<Record<PromptBlockType, string>> = {
    systemPrompt: "In Assistant chats, text here replaces the chat's own system prompt box.",
    worldInfo: "Turns lorebooks on. Where each entry lands is set on the entry itself.",
    chatHistory: "The conversation. Blocks above it go in the system prompt; blocks below it are injected into the history.",
    worldState: "Filled automatically while a chat has an in-story clock; the text here is used only when it doesn't.",
    authorsNote: "Injected into the history at the depth below.",
    postHistoryInstructions: "Injected into the history at the depth below - depth 0 puts it last.",
    userMemory: "What the assistant has learned about you (Assistant chats only).",
    characterNotebook: "What each character privately knows (roleplay only).",
    questLog: "Tracked objectives (roleplay only).",
  };

  /** Blocks whose own text is used by the prompt assembler; for the rest, custom text would be ignored. */
  const TEXT_TYPES: PromptBlockType[] = ["systemPrompt", "authorsNote", "custom", "postHistoryInstructions", "worldState"];
  /** Blocks that never take a position in the history, so a depth means nothing. */
  const NO_DEPTH: PromptBlockType[] = ["worldInfo", "chatHistory"];

  const EFFORTS: { id: "" | ReasoningEffort; label: string }[] = [
    { id: "", label: "Model default" },
    { id: "off", label: "Off" },
    { id: "low", label: "Low" },
    { id: "medium", label: "Medium" },
    { id: "high", label: "High" },
  ];

  let id = $state("");
  let name = $state("");
  let temperature = $state<number | null>(null);
  let topP = $state<number | null>(null);
  let topK = $state<number | null>(null);
  let maxOutputTokens = $state<number | null>(null);
  let maxContextTokens = $state<number | null>(null);
  let effort = $state<"" | ReasoningEffort>("");
  let blocks = $state<PromptBlockFull[]>([]);
  let loaded = $state(false);
  let error = $state("");
  let saving = $state(false);
  const isNew = $derived(page.params.id === "new");

  const block = (type: PromptBlockType, order: number): PromptBlockFull => ({
    id: newId(),
    type,
    enabled: true,
    order,
    customText: null,
    injectionDepth: 0,
  });

  $effect(() => {
    const wanted = page.params.id;
    void (async () => {
      try {
        let preset: PromptPresetFull | undefined;
        if (wanted === "new") {
          // A preset must at minimum carry the chat history, or no messages would ever reach the model.
          preset = {
            id: newId(),
            name: "",
            blocks: [block("chatHistory", 0)],
            temperature: null,
            topP: null,
            topK: null,
            maxOutputTokens: null,
            reasoningEffort: null,
            maxContextTokens: null,
          };
        } else {
          preset = (await api.listPresets()).find((p) => p.id === wanted);
        }
        if (!preset) {
          error = "That preset no longer exists.";
          return;
        }
        id = preset.id;
        name = preset.name;
        temperature = preset.temperature;
        topP = preset.topP;
        topK = preset.topK;
        maxOutputTokens = preset.maxOutputTokens;
        maxContextTokens = preset.maxContextTokens;
        effort = preset.reasoningEffort ?? "";
        blocks = [...preset.blocks].sort((a, b) => a.order - b.order);
        // Every preset carries this block; older ones simply don't have it yet.
        if (!blocks.some((b) => b.type === "postHistoryInstructions")) {
          blocks.push(block("postHistoryInstructions", blocks.length));
        }
        loaded = true;
      } catch (e) {
        error = errorText(e);
      }
    })();
  });

  const chatHistoryIndex = $derived(blocks.findIndex((b) => b.type === "chatHistory"));

  async function save() {
    error = "";
    saving = true;
    try {
      const preset: PromptPresetFull = {
        id,
        name,
        temperature: numberOrNull(temperature),
        topP: numberOrNull(topP),
        topK: numberOrNull(topK),
        maxOutputTokens: numberOrNull(maxOutputTokens),
        maxContextTokens: numberOrNull(maxContextTokens),
        reasoningEffort: effort === "" ? null : effort,
        blocks: blocks.map((b, order) => ({ ...$state.snapshot(b), order, customText: b.customText?.trim() ? b.customText : null })),
      };
      await api.savePreset(preset);
      await goto("/presets");
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }
</script>

<section class="page">
  <header>
    <a href="/presets" class="back" aria-label="Back to presets">←</a>
    <h1>{isNew ? "New preset" : (name || "Edit preset")}</h1>
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
        <input bind:value={name} placeholder="Untitled preset" />
      </label>

      <h2>Sampling — leave empty for the provider's default</h2>
      <div class="grid">
        <label class="field">Temperature <input type="number" step="0.05" min="0" max="2" bind:value={temperature} /></label>
        <label class="field">Top-p <input type="number" step="0.05" min="0" max="1" bind:value={topP} /></label>
        <label class="field">Top-k <input type="number" step="1" min="0" bind:value={topK} /></label>
        <label class="field">Max output tokens <input type="number" step="64" min="1" bind:value={maxOutputTokens} /></label>
        <label class="field">
          Max history tokens (approx.)
          <input type="number" step="500" min="1" bind:value={maxContextTokens} placeholder="unlimited" />
        </label>
        <label class="field">
          Reasoning effort
          <select bind:value={effort}>
            {#each EFFORTS as option (option.id)}<option value={option.id}>{option.label}</option>{/each}
          </select>
        </label>
      </div>

      <h2>Blocks — top to bottom is the order they're sent</h2>
      {#each blocks as b, i (b.id)}
        <fieldset class="block card" class:off={!b.enabled} class:below={chatHistoryIndex >= 0 && i > chatHistoryIndex}>
          <div class="head">
            <label class="check"><input type="checkbox" bind:checked={b.enabled} /> <strong>{LABELS[b.type]}</strong></label>
            <span class="grow"></span>
            <button type="button" class="ghost" onclick={() => move(blocks, i, -1)} disabled={i === 0} aria-label="Move up">↑</button>
            <button type="button" class="ghost" onclick={() => move(blocks, i, 1)} disabled={i === blocks.length - 1} aria-label="Move down">↓</button>
            <button type="button" class="ghost danger" onclick={() => blocks.splice(i, 1)} disabled={b.type === "chatHistory"} aria-label="Remove block">✕</button>
          </div>
          {#if HELP[b.type]}<p class="muted small">{HELP[b.type]}</p>{/if}
          {#if TEXT_TYPES.includes(b.type)}
            <textarea
              rows="3"
              placeholder="Text for this block…"
              value={b.customText ?? ""}
              oninput={(e) => (b.customText = e.currentTarget.value)}
              aria-label="{LABELS[b.type]} text"
            ></textarea>
          {/if}
          {#if !NO_DEPTH.includes(b.type) && chatHistoryIndex >= 0 && i > chatHistoryIndex}
            <label class="field depth">
              Injection depth (messages from the end)
              <input type="number" min="0" bind:value={b.injectionDepth} />
            </label>
          {/if}
        </fieldset>
      {/each}
      <div><button type="button" onclick={() => blocks.push(block("custom", blocks.length))}>＋ Add custom block</button></div>

      <div class="actionbar">
        <button class="primary" type="submit" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
        <a class="cancel" href="/presets">Cancel</a>
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

  .grid {
    display: grid;
    gap: 12px;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  }

  .block {
    padding: 10px 14px;
    margin: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .block.off {
    opacity: 0.55;
  }

  .block.below {
    border-left: 3px solid var(--accent);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .grow {
    flex: 1;
  }

  .small {
    font-size: 12px;
    margin: 0;
  }

  .depth {
    max-width: 260px;
  }

  .cancel {
    color: var(--muted);
    text-decoration: none;
    padding: 6px 10px;
  }
</style>

<script lang="ts">
  import { api, errorText } from "$lib/api";
  import type { GeminiOptions, OllamaOptions, ProviderInfo, SafetyCategory, SafetyThreshold } from "$lib/types";

  let providers = $state<ProviderInfo[]>([]);
  let dataDir = $state("");
  let keys = $state<Record<string, string>>({});
  let status = $state<Record<string, string>>({});
  let busy = $state<Record<string, boolean>>({});
  let modelCounts = $state<Record<string, number>>({});
  let gemini = $state<GeminiOptions | null>(null);
  let ollama = $state<OllamaOptions | null>(null);
  let error = $state("");

  const categories: { id: SafetyCategory; label: string }[] = [
    { id: "harassment", label: "Harassment" },
    { id: "hateSpeech", label: "Hate speech" },
    { id: "sexuallyExplicit", label: "Sexually explicit" },
    { id: "dangerousContent", label: "Dangerous content" },
  ];

  const thresholds: { id: SafetyThreshold; label: string }[] = [
    { id: "unspecified", label: "Model default" },
    { id: "off", label: "Off (skip the check)" },
    { id: "blockNone", label: "Block none" },
    { id: "blockOnlyHigh", label: "Block only high" },
    { id: "blockMediumAndAbove", label: "Block medium and above" },
    { id: "blockLowAndAbove", label: "Block low and above" },
  ];

  $effect(() => {
    void (async () => {
      try {
        const boot = await api.bootstrap();
        providers = boot.providers;
        dataDir = boot.dataDir;
        for (const p of boot.providers) {
          modelCounts[p.id] = (await api.listModels(p.id)).length;
        }
        gemini = await api.getGeminiOptions();
        ollama = await api.getOllamaOptions();
      } catch (e) {
        error = errorText(e);
      }
    })();
  });

  async function saveKey(provider: ProviderInfo) {
    busy[provider.id] = true;
    try {
      const updated = await api.setApiKey(provider.id, keys[provider.id] ?? "");
      providers = providers.map((p) => (p.id === updated.id ? updated : p));
      keys[provider.id] = "";
      status[provider.id] = updated.hasKey ? "Key saved." : "Key removed.";
    } catch (e) {
      status[provider.id] = errorText(e);
    } finally {
      busy[provider.id] = false;
    }
  }

  async function refresh(provider: ProviderInfo) {
    busy[provider.id] = true;
    status[provider.id] = "Fetching models…";
    try {
      const models = await api.refreshModels(provider.id);
      modelCounts[provider.id] = models.length;
      status[provider.id] = `${models.length} models available.`;
    } catch (e) {
      status[provider.id] = errorText(e);
    } finally {
      busy[provider.id] = false;
    }
  }

  async function saveGemini() {
    if (!gemini) return;
    try {
      await api.saveGeminiOptions($state.snapshot(gemini));
      status.gemini = "Gemini options saved.";
    } catch (e) {
      status.gemini = errorText(e);
    }
  }

  async function saveOllama() {
    if (!ollama) return;
    try {
      await api.saveOllamaOptions($state.snapshot(ollama));
      status.ollama = "Ollama options saved.";
    } catch (e) {
      status.ollama = errorText(e);
    }
  }
</script>

<section class="page">
  <header><h1>Settings</h1></header>
  {#if error}<p class="err">{error}</p>{/if}

  <div class="stack">
    {#each providers as provider (provider.id)}
      <article class="card block">
        <div class="titleline">
          <h2>{provider.name}</h2>
          <span class="pill" class:ok={provider.hasKey}>{provider.auth === "local" ? "Local server" : provider.hasKey ? "Key saved" : "No key"}</span>
          <span class="muted models">{modelCounts[provider.id] ?? 0} models cached</span>
        </div>

        {#if provider.auth === "apiKey"}
          <form
            class="keyrow"
            onsubmit={(e) => {
              e.preventDefault();
              void saveKey(provider);
            }}
          >
            <input
              type="password"
              autocomplete="off"
              placeholder={provider.hasKey ? "Enter a new key to replace the saved one" : "API key"}
              bind:value={keys[provider.id]}
              aria-label="{provider.name} API key"
            />
            <button class="primary" type="submit" disabled={busy[provider.id]}>{keys[provider.id] ? "Save key" : provider.hasKey ? "Remove key" : "Save key"}</button>
          </form>
          <p class="muted small">Stored in the Windows Credential Manager, never in the database. Leave the box empty and press the button to remove a saved key.</p>
        {/if}

        <div class="keyrow">
          <button onclick={() => refresh(provider)} disabled={busy[provider.id] || (provider.auth === "apiKey" && !provider.hasKey)}>
            ↻ Refresh models
          </button>
          {#if status[provider.id]}<span class="muted small" role="status">{status[provider.id]}</span>{/if}
        </div>

        {#if provider.id === "gemini" && gemini}
          <h3>Gemini options</h3>
          <label class="check">
            <input type="checkbox" bind:checked={gemini.enableGoogleSearch} /> Ground answers with Google Search by default
          </label>
          <div class="safety">
            {#each categories as category (category.id)}
              <label class="field">
                {category.label}
                <select bind:value={gemini.safetyThresholds[category.id]}>
                  {#each thresholds as threshold (threshold.id)}
                    <option value={threshold.id}>{threshold.label}</option>
                  {/each}
                </select>
              </label>
            {/each}
          </div>
          <div class="keyrow">
            <button onclick={saveGemini}>Save Gemini options</button>
            {#if status.gemini}<span class="muted small" role="status">{status.gemini}</span>{/if}
          </div>
        {/if}

        {#if provider.id === "ollama" && ollama}
          <h3>Ollama options</h3>
          <div class="ollama">
            <label class="field">
              Server address
              <input type="url" bind:value={ollama.baseUrl} placeholder="http://localhost:11434" />
            </label>
            <label class="field">
              Context size (tokens)
              <input type="number" min="512" step="512" bind:value={ollama.contextSize} />
            </label>
          </div>
          <p class="muted small">Sent as <code>num_ctx</code> on every request, so switching models never silently changes how much history a model sees.</p>
          <div class="keyrow">
            <button onclick={saveOllama}>Save Ollama options</button>
            {#if status.ollama}<span class="muted small" role="status">{status.ollama}</span>{/if}
          </div>
        {/if}
      </article>
    {/each}

    <article class="card block">
      <h2>About</h2>
      <p class="muted small">Data folder: <code>{dataDir}</code></p>
      <p class="muted small">
        ChatGPT and SuperGrok sign-in, PowerShell tools, memory and character import are being ported from the original app.
      </p>
    </article>
  </div>
</section>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 760px;
  }

  .block {
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .titleline {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }

  h2 {
    font-size: 16px;
  }

  h3 {
    font-size: 13px;
    margin-top: 6px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .models {
    margin-left: auto;
    font-size: 12px;
  }

  .pill {
    font-size: 11px;
    padding: 1px 9px;
    border-radius: 999px;
    background: var(--panel-3);
    color: var(--muted);
  }

  .pill.ok {
    background: rgba(80, 200, 120, 0.18);
    color: #4fc27a;
  }

  .keyrow {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .keyrow input {
    flex: 1;
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

  .safety,
  .ollama {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 10px;
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

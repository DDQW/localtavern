<script lang="ts">
  import { api, errorText } from "#lib/api.js";
  import ImageGenSection from "#lib/components/ImageGenSection.svelte";
  import InstructionsSection from "#lib/components/InstructionsSection.svelte";
  import OnDeviceSection from "#lib/components/OnDeviceSection.svelte";
  import type {
    DefaultPersonas,
    GeminiOptions,
    Persona,
    OllamaOptions,
    VastAiOptions,
    PowerShellOptions,
    ProviderInfo,
    SafetyCategory,
    SafetyThreshold,
    SessionStatus,
    UserMemory,
  } from "#lib/types.js";

  let providers = $state<ProviderInfo[]>([]);
  let dataDir = $state("");
  let keys = $state<Record<string, string>>({});
  let status = $state<Record<string, string>>({});
  let busy = $state<Record<string, boolean>>({});
  let modelCounts = $state<Record<string, number>>({});
  let gemini = $state<GeminiOptions | null>(null);
  let ollama = $state<OllamaOptions | null>(null);
  let vastai = $state<VastAiOptions | null>(null);
  let error = $state("");
  let sessions = $state<Record<string, SessionStatus>>({});
  let signingIn = $state<Record<string, boolean>>({});
  let powershell = $state<PowerShellOptions | null>(null);
  let newPattern = $state("");
  let memories = $state<UserMemory[]>([]);
  let personas = $state<Persona[]>([]);
  let defaults = $state<DefaultPersonas>({ roleplayPersonaId: null, assistantPersonaId: null });

  /** What to know before signing in with a subscription (from the original app). */
  const SUBSCRIPTION_NOTES: Record<string, string> = {
    "xai-subscription":
      "Uses your SuperGrok / X Premium subscription instead of an API key: signing in opens xAI in your browser. Some plans don't include API access and get rejected with HTTP 403 - use an xAI API key in that case. Refreshed models show up in every chat's model picker.",
    "chatgpt-subscription":
      "Uses your ChatGPT Plus/Pro/Business plan instead of an API key: signing in opens OpenAI in your browser. Requests count against your plan's Codex usage limits. Temperature/top-p aren't sent (these reasoning models reject them). Refreshed models show up in every chat's model picker.",
  };

  /** Providers that can fetch their API key through a browser sign-in instead of pasting it. */
  const BROWSER_SIGN_IN_NOTES: Record<string, string> = {
    mistral:
      "Signs in through your Mistral AI Studio account and saves the API key it issues, the same sign-in Mistral's own Vibe CLI uses. The key belongs to your AI Studio account, so usage is billed to (or limited by) that account's API plan, not necessarily your Le Chat/Vibe subscription.",
  };

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
        for (const p of boot.providers.filter((p) => p.auth === "subscription")) {
          sessions[p.id] = await api.subscriptionStatus(p.id);
        }
        gemini = await api.getGeminiOptions();
        ollama = await api.getOllamaOptions();
        vastai = await api.getVastAiOptions();
        powershell = await api.getPowerShellOptions();
        memories = await api.listMemories();
        personas = await api.listPersonas();
        const loaded = await api.getDefaultPersonas();
        const ids = new Set(personas.map((p) => p.id));
        if (loaded.roleplayPersonaId && !ids.has(loaded.roleplayPersonaId)) loaded.roleplayPersonaId = null;
        if (loaded.assistantPersonaId && !ids.has(loaded.assistantPersonaId)) loaded.assistantPersonaId = null;
        defaults = loaded;
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

  async function signIn(provider: ProviderInfo) {
    signingIn[provider.id] = true;
    status[provider.id] = "Waiting for you to approve access in the browser…";
    try {
      const result = await api.subscriptionSignIn(provider.id);
      sessions[provider.id] = result;
      providers = providers.map((p) => (p.id === provider.id ? { ...p, hasKey: result.signedIn } : p));
      status[provider.id] = result.signedIn ? "" : "Sign-in cancelled.";
    } catch (e) {
      status[provider.id] = errorText(e);
      signingIn[provider.id] = false;
      return;
    }
    signingIn[provider.id] = false;
    if (sessions[provider.id]?.signedIn) await refresh(provider);
  }

  /** Mistral: sign in through the browser and let it fill in the API key. */
  async function browserSignIn(provider: ProviderInfo) {
    signingIn[provider.id] = true;
    status[provider.id] = "Waiting for you to approve access in the browser…";
    try {
      const updated = await api.browserKeySignIn(provider.id);
      providers = providers.map((p) => (p.id === updated.id ? updated : p));
      status[provider.id] = updated.hasKey ? "" : "Sign-in cancelled.";
      signingIn[provider.id] = false;
      if (updated.hasKey) await refresh(provider);
    } catch (e) {
      status[provider.id] = errorText(e);
      signingIn[provider.id] = false;
    }
  }

  async function signOut(provider: ProviderInfo) {
    try {
      sessions[provider.id] = await api.subscriptionSignOut(provider.id);
      providers = providers.map((p) => (p.id === provider.id ? { ...p, hasKey: false } : p));
      status[provider.id] = "";
    } catch (e) {
      status[provider.id] = errorText(e);
    }
  }

  function pillText(provider: ProviderInfo): string {
    if (provider.auth === "local") return "Local server";
    if (provider.auth === "subscription") {
      const session = sessions[provider.id];
      if (!session?.signedIn) return "Not signed in";
      return session.email ? `Signed in as ${session.email}` : "Signed in";
    }
    return provider.hasKey ? "Key saved" : "No key";
  }

  async function setDefault(which: "roleplay" | "assistant", id: string) {
    const next: DefaultPersonas = {
      roleplayPersonaId: which === "roleplay" ? id || null : defaults.roleplayPersonaId,
      assistantPersonaId: which === "assistant" ? id || null : defaults.assistantPersonaId,
    };
    const previous = { ...defaults };
    defaults = next;
    try {
      await api.saveDefaultPersonas(next);
      status.defaults = "";
    } catch (e) {
      defaults = previous;
      status.defaults = errorText(e);
    }
  }

  async function savePowerShell() {
    if (!powershell) return;
    try {
      await api.savePowerShellOptions($state.snapshot(powershell));
      status.powershell = "";
    } catch (e) {
      status.powershell = errorText(e);
    }
  }

  async function setAutoMode(on: boolean) {
    if (!powershell) return;
    if (on) {
      const sure = confirm(
        "With Auto mode on, the assistant runs PowerShell commands immediately, with no confirmation dialog, as long as they don't request Administrator rights (those always ask separately). Turn this off any time from this same page.\n\nEnable Auto mode?",
      );
      if (!sure) {
        powershell.autoMode = false;
        return;
      }
    }
    powershell.autoMode = on;
    await savePowerShell();
  }

  async function addPattern() {
    const pattern = newPattern.trim();
    if (!powershell || !pattern || powershell.allowlistPatterns.some((p) => p.toLowerCase() === pattern.toLowerCase())) return;
    powershell.allowlistPatterns.push(pattern);
    newPattern = "";
    await savePowerShell();
  }

  async function removePattern(pattern: string) {
    if (!powershell) return;
    powershell.allowlistPatterns = powershell.allowlistPatterns.filter((p) => p !== pattern);
    await savePowerShell();
  }

  async function deleteMemory(memory: UserMemory) {
    try {
      await api.deleteMemory(memory.id);
      memories = memories.filter((m) => m.id !== memory.id);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function clearMemories() {
    if (!confirm("Forget everything the assistant has learned about you?")) return;
    try {
      await api.clearMemories();
      memories = [];
    } catch (e) {
      error = errorText(e);
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

  async function saveVastAi() {
    if (!vastai) return;
    try {
      await api.saveVastAiOptions($state.snapshot(vastai));
      status.vastai = "Vast.ai options saved.";
    } catch (e) {
      status.vastai = errorText(e);
    }
  }
</script>

<section class="page">
  <header><h1>Settings</h1></header>
  {#if error}<p class="err">{error}</p>{/if}

  <div class="stack">
    <article class="card block">
      <h2>Default personas</h2>
      <p class="muted small">
        A persona is who you are in a chat. Roleplay and Assistant can each use a different one, so the model sees a different name and description and can react very differently. Opening that section selects its persona for the chat you land in. You can still switch the current chat from its header. None uses the persona marked default.
      </p>
      <div class="defaults">
        <label class="field">
          Roleplay
          <select value={defaults.roleplayPersonaId ?? ""} onchange={(e) => setDefault("roleplay", e.currentTarget.value)}>
            <option value="">None — the default persona</option>
            {#each personas as persona (persona.id)}
              <option value={persona.id}>{persona.name}</option>
            {/each}
          </select>
        </label>
        <label class="field">
          Assistant
          <select value={defaults.assistantPersonaId ?? ""} onchange={(e) => setDefault("assistant", e.currentTarget.value)}>
            <option value="">None — the default persona</option>
            {#each personas as persona (persona.id)}
              <option value={persona.id}>{persona.name}</option>
            {/each}
          </select>
        </label>
      </div>
      {#if status.defaults}<p class="err small" role="alert">{status.defaults}</p>{/if}
    </article>

    {#each providers as provider (provider.id)}
      <article class="card block">
        <div class="titleline">
          <h2>{provider.name}</h2>
          <span class="pill" class:ok={provider.hasKey}>{pillText(provider)}</span>
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
          {#if provider.browserSignIn}
            <div class="keyrow">
              <button onclick={() => browserSignIn(provider)} disabled={signingIn[provider.id]}>
                {signingIn[provider.id] ? "Waiting for the browser…" : `Sign in with ${provider.name}`}
              </button>
              {#if signingIn[provider.id]}
                <button onclick={() => api.subscriptionCancelSignIn(provider.id)}>Cancel</button>
              {/if}
            </div>
            <p class="muted small">{BROWSER_SIGN_IN_NOTES[provider.id]}</p>
          {/if}
        {/if}

        {#if provider.auth === "subscription"}
          <div class="keyrow">
            <button class="primary" onclick={() => signIn(provider)} disabled={signingIn[provider.id]}>
              {signingIn[provider.id] ? "Waiting for the browser…" : `Sign in with ${provider.name}`}
            </button>
            {#if signingIn[provider.id]}
              <button onclick={() => api.subscriptionCancelSignIn(provider.id)}>Cancel</button>
            {/if}
            <button onclick={() => signOut(provider)} disabled={signingIn[provider.id] || !sessions[provider.id]?.signedIn}>Sign out</button>
          </div>
          <p class="muted small">{SUBSCRIPTION_NOTES[provider.id]}</p>
        {/if}

        <div class="keyrow">
          <button onclick={() => refresh(provider)} disabled={busy[provider.id] || (provider.auth !== "local" && !provider.hasKey)}>
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
        {#if provider.id === "ondevice"}
          <OnDeviceSection onchange={() => refresh(provider)} />
        {/if}
        {#if provider.id === "vastai" && vastai}
          <h3>Vast.ai instance</h3>
          <p class="muted small">
            Rent an instance running Ollama (e.g. Vast's Ollama template), then enter its ID here. It is started when a reply needs it and
            stopped again afterwards, so it only bills for its disk in between. The key above is your Vast.ai API key.
          </p>
          <div class="ollama">
            <label class="field">
              Instance ID
              <input type="number" min="0" bind:value={vastai.instanceId} />
            </label>
            <label class="field">
              Ollama port in the container
              <input type="number" min="1" max="65535" bind:value={vastai.port} />
            </label>
            <label class="field">
              Address override (optional)
              <input type="url" bind:value={vastai.baseUrl} placeholder="https://my-tunnel.example.com" />
            </label>
            <label class="field">
              Context size (tokens)
              <input type="number" min="512" step="512" bind:value={vastai.contextSize} />
            </label>
            <label class="field">
              Stop after idle (seconds, 0 = at once)
              <input type="number" min="0" bind:value={vastai.idleStopSecs} />
            </label>
            <label class="field">
              Start timeout (seconds)
              <input type="number" min="30" bind:value={vastai.readyTimeoutSecs} />
            </label>
          </div>
          <p class="muted small">
            A short idle time keeps the instance up across the follow-up calls the app makes right after a reply, instead of restarting it for each.
            Starting a stopped instance usually takes a minute or more, and can fail if its GPU was rented to someone else meanwhile.
          </p>
          <div class="keyrow">
            <button onclick={saveVastAi}>Save Vast.ai options</button>
            {#if status.vastai}<span class="muted small" role="status">{status.vastai}</span>{/if}
          </div>
        {/if}
      </article>
    {/each}

    <ImageGenSection />

    <InstructionsSection />

    <article class="card block">
      <h2>PowerShell tool</h2>
      <p class="muted small">
        Lets the assistant run real PowerShell 7 commands on this machine, in Assistant chats only (never in roleplays). This runs arbitrary code you approve - only enable it if you understand that risk. Needs <code>pwsh</code> on your PATH.
      </p>
      {#if powershell}
        <label class="check">
          <input
            type="checkbox"
            checked={powershell.enabled}
            onchange={(e) => {
              if (powershell) powershell.enabled = e.currentTarget.checked;
              void savePowerShell();
            }}
          />
          Enable PowerShell tool
        </label>
        <label class="check">
          <input type="checkbox" checked={powershell.autoMode} onchange={(e) => setAutoMode(e.currentTarget.checked)} /> Auto mode
        </label>
        <p class="muted small">
          Runs non-elevated commands immediately, with no confirmation dialog. Commands that request Administrator rights always ask separately, regardless of this setting.
        </p>

        <h3>Always-allowed commands</h3>
        <p class="muted small">
          Commands matching one of these patterns run without a confirmation dialog. Use <code>*</code> as a wildcard, e.g. <code>Get-Process*</code>. Added when you choose "Always allow" in a confirmation dialog, or add one here.
        </p>
        {#each powershell.allowlistPatterns as pattern (pattern)}
          <div class="keyrow">
            <code class="grow">{pattern}</code>
            <button class="ghost" onclick={() => removePattern(pattern)}>Remove</button>
          </div>
        {/each}
        <form
          class="keyrow"
          onsubmit={(e) => {
            e.preventDefault();
            void addPattern();
          }}
        >
          <input bind:value={newPattern} placeholder="e.g. Get-Process*" aria-label="New allowed command pattern" />
          <button type="submit" disabled={!newPattern.trim()}>Add</button>
        </form>
        {#if status.powershell}<p class="err small" role="alert">{status.powershell}</p>{/if}
      {/if}
    </article>

    <article class="card block">
      <h2>What the assistant remembers about you</h2>
      <p class="muted small">
        After each Assistant reply, the app asks the model to note any new lasting facts about you and feeds them back into future Assistant chats. Delete anything you don't want remembered.
      </p>
      {#each memories as memory (memory.id)}
        <div class="keyrow">
          <span class="grow">{memory.content}</span>
          <button class="ghost" onclick={() => deleteMemory(memory)}>Delete</button>
        </div>
      {:else}
        <p class="muted small">Nothing remembered yet.</p>
      {/each}
      {#if memories.length > 0}
        <div><button onclick={clearMemories}>Clear all</button></div>
      {/if}
    </article>

    <article class="card block">
      <h2>About</h2>
      <p class="muted small">Data folder: <code>{dataDir}</code></p>
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

  .grow {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
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
  .ollama,
  .defaults {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 10px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--muted);
  }

  .field select {
    color: var(--text);
    font-size: 14px;
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

<script lang="ts">
  import { tick } from "svelte";
  import { api, errorText } from "#lib/api.js";
  import { pickLauncher } from "#lib/pick.js";
  import ImageModelsOnDevice from "#lib/components/ImageModelsOnDevice.svelte";
  import type { ImageBackendConfig, ImageBackendOptions, ImageGenSettings } from "#lib/types.js";

  let settings = $state<ImageGenSettings | null>(null);
  let options = $state<ImageBackendOptions | null>(null);
  let status = $state("");
  let testing = $state(false);
  let showWorkflow = $state(false);

  const backend = $derived<ImageBackendConfig | null>(
    settings ? { a1111: settings.a1111, comfyUi: settings.comfy, grok: settings.grok, onDevice: settings.onDevice }[settings.backend] : null,
  );
  /** Grok Imagine draws in the cloud: no address, sampler, steps or graphics card to share. */
  const grok = $derived(settings?.backend === "grok");
  /** stable-diffusion.cpp on the phone: no address; the checkpoint comes from the image models folder. */
  const onPhone = $derived(settings?.backend === "onDevice");
  /** Whether this build can draw on the device; the choice is only offered there. */
  let phoneAvailable = $state(false);

  $effect(() => {
    void api
      .ondeviceImageStatus()
      .then((s) => (phoneAvailable = s.available))
      .catch(() => (phoneAvailable = false));
  });

  $effect(() => {
    void api
      .getImageSettings()
      .then(async (loaded) => {
        settings = loaded;
        showWorkflow = loaded.comfyWorkflow.trim().length > 0;
        // The chat's "Picture settings…" link lands here; the page above loads first, so jump now.
        if (location.hash === "#pictures") {
          await tick();
          document.getElementById("pictures")?.scrollIntoView({ block: "start" });
        }
      })
      .catch((e) => (status = errorText(e)));
  });

  /** Asks the server for its checkpoints and samplers - doubles as a connection test. */
  async function connect() {
    if (!settings) return;
    testing = true;
    status = settings.backend === "comfyUi" && settings.comfyLauncher.trim() ? "Connecting (starting ComfyUI if needed)…" : "Connecting…";
    options = null;
    try {
      options = await api.imageBackendOptions($state.snapshot(settings));
      status = grok
        ? `Signed in: ${options.models.length} Grok model${options.models.length === 1 ? "" : "s"}.`
        : onPhone
          ? `${options.models.length} picture model${options.models.length === 1 ? "" : "s"} on this phone.`
          : `Connected: ${options.models.length} checkpoint${options.models.length === 1 ? "" : "s"}, ${options.samplers.length} samplers.`;
    } catch (e) {
      status = errorText(e);
    } finally {
      testing = false;
    }
  }

  async function save() {
    if (!settings) return;
    if (!showWorkflow) settings.comfyWorkflow = "";
    try {
      settings = await api.saveImageSettings($state.snapshot(settings));
      status = "Picture settings saved.";
    } catch (e) {
      status = errorText(e);
    }
  }

  async function browseLauncher() {
    const picked = await pickLauncher();
    if (picked && settings) settings.comfyLauncher = picked;
  }

  /** The saved value stays selectable even before (or without) a connection. */
  function choices(list: string[] | undefined, current: string): string[] {
    const all = list ?? [];
    return current && !all.includes(current) ? [current, ...all] : all;
  }

  // Switching backend invalidates the other server's lists.
  let lastBackend = "";
  $effect(() => {
    if (settings && settings.backend !== lastBackend) {
      if (lastBackend) options = null;
      lastBackend = settings.backend;
      // Grok reads sentences (the backend insists on it when saving, too).
      if (settings.backend === "grok") settings.promptStyle = "natural";
    }
  });
</script>

<article class="card block" id="pictures">
  <h2>Pictures</h2>
  <p class="muted small">
    The Picture button in a roleplay has the chat model describe the character or scene, then sends that to your own
    Stable Diffusion server or to Grok Imagine, or draws it on the phone itself. Pictures show in the chat but are never sent back to the model.
  </p>

  {#if settings && backend}
    <div class="grid">
      <label class="field">
        Server
        <select bind:value={settings.backend}>
          <option value="a1111">AUTOMATIC1111 / Forge / SD.Next</option>
          <option value="comfyUi">ComfyUI</option>
          <option value="grok">Grok Imagine (SuperGrok)</option>
          {#if phoneAvailable || onPhone}
            <option value="onDevice">On this phone (stable-diffusion.cpp)</option>
          {/if}
        </select>
      </label>
      {#if !grok && !onPhone}
        <label class="field">
          Address
          <input type="url" bind:value={backend.url} placeholder={settings.backend === "a1111" ? "http://127.0.0.1:7860" : "http://127.0.0.1:8188"} />
        </label>
      {/if}
    </div>
    {#if grok}
      <p class="muted small">
        Draws on xAI's servers with your SuperGrok / X Premium subscription - sign in under SuperGrok in the models section
        above. Some plans don't include API access and are rejected with HTTP 403. xAI moderates prompts and may refuse
        explicit pictures. Grok picks the steps and seed itself and has no negative prompt; your width and height only set
        the shape.
      </p>
    {:else if onPhone}
      <p class="muted small">
        Draws on the phone itself: nothing leaves the device and nothing is filtered. The model loads for the first picture (that
        takes a little while) and is unloaded again after five minutes without one. A phone is slow at this: about a minute a
        picture with an LCM model. Pictures are drawn at SD 1.5's size (your portrait shape becomes 512×768). Uses as many CPU
        threads as the on-device chat model; Android's power saving mode slows it down further.
      </p>
      <ImageModelsOnDevice onchange={connect} />
    {:else if settings.backend === "a1111"}
      <p class="muted small">Start the WebUI with <code>--api</code> (add it to <code>COMMANDLINE_ARGS</code> in <code>webui-user.bat</code>).</p>
    {:else}
      <p class="muted small">Works with a stock ComfyUI. Uses a plain checkpoint text-to-image workflow unless you paste your own below.</p>
      <label class="field">
        Start ComfyUI with LocalTavern
        <span class="pathrow">
          <input bind:value={settings.comfyLauncher} placeholder="e.g. D:\ComfyUI_windows_portable\run_nvidia_gpu.bat" spellcheck="false" />
          <button type="button" onclick={browseLauncher}>Browse…</button>
        </span>
      </label>
      <p class="muted small">
        The file you start ComfyUI with. LocalTavern runs it in the background when it starts (unless ComfyUI is already
        running) and closes it again when you quit. Its output goes to <code>comfyui.log</code> in LocalTavern's data folder.
        Leave it empty if you start ComfyUI yourself.
      </p>
    {/if}

    <div class="keyrow">
      <button onclick={connect} disabled={testing}>{options ? "↻ Reload lists" : grok ? "Check sign-in" : onPhone ? "Load lists" : "Connect"}</button>
      {#if status}<span class="muted small" role="status">{status}</span>{/if}
    </div>

    <div class="grid">
      <label class="field">
        {grok || onPhone ? "Model" : "Checkpoint"}
        <select bind:value={backend.model}>
          <option value="">{{ a1111: "Whatever is loaded", comfyUi: "The first one found", grok: "Default (grok-imagine-image-2.0)", onDevice: "The first one on this phone" }[settings.backend]}</option>
          {#each choices(options?.models, backend.model) as model (model)}
            <option value={model}>{model}</option>
          {/each}
        </select>
      </label>
      {#if !grok}
      <label class="field">
        Sampler
        <select bind:value={backend.sampler}>
          <option value="">Server default</option>
          {#each choices(options?.samplers, backend.sampler) as sampler (sampler)}
            <option value={sampler}>{sampler}</option>
          {/each}
        </select>
      </label>
      <label class="field">
        Scheduler
        <select bind:value={backend.scheduler}>
          <option value="">Server default</option>
          {#each choices(options?.schedulers, backend.scheduler) as scheduler (scheduler)}
            <option value={scheduler}>{scheduler}</option>
          {/each}
        </select>
      </label>
      {/if}
    </div>

    <div class="grid numbers">
      <label class="field">Width <input type="number" min="64" max="4096" step="64" bind:value={settings.width} /></label>
      <label class="field">Height <input type="number" min="64" max="4096" step="64" bind:value={settings.height} /></label>
      {#if !grok}
        <label class="field">Steps <input type="number" min="1" max="150" bind:value={settings.steps} /></label>
        <label class="field">CFG scale <input type="number" min="1" max="30" step="0.5" bind:value={settings.cfgScale} /></label>
        <label class="field">Seed <input type="number" min="-1" bind:value={settings.seed} title="-1 picks a new one every time" /></label>
      {/if}
    </div>
    <p class="muted small">
      The size is for full-body portraits. Faces are drawn square and scenes landscape, at about the same pixel count.
    </p>

    <label class="field">
      Prompt prefix
      <input bind:value={settings.promptPrefix} placeholder="e.g. masterpiece, best quality, anime style" />
    </label>
    {#if !grok}
      <label class="field">
        Negative prompt
        <textarea rows="2" bind:value={settings.negativePrompt}></textarea>
      </label>

      <div class="grid">
        <label class="field">
          The chat model writes prompts as
          <select bind:value={settings.promptStyle}>
            <option value="tags">Tags (SD 1.5, SDXL, Pony, Illustrious)</option>
            <option value="natural">Sentences (FLUX and similar)</option>
          </select>
        </label>
      </div>
    {/if}
    <label class="check">
      <input type="checkbox" bind:checked={settings.reviewPrompt} /> Let me review and edit the prompt before drawing
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={settings.autoPhotos} /> Draw the photos characters send in the story
    </label>
    <p class="muted small">
      When a character sends you a picture in a roleplay (a selfie over the phone, a snapshot of where they are), the model
      describes it and the photo is drawn into the chat right away, without the review step.
    </p>
    <label class="check">
      <input type="checkbox" bind:checked={settings.autoVideos} /> Draw the videos characters send in the story
    </label>
    <p class="muted small">
      Same idea with <code>[video: …]</code> clips. Needs the ComfyUI backend. If a Wan text-to-video model is installed, the
      clip is drawn with it. Otherwise the shot is drawn as a still with your picture model, so it does not fail. Still photos
      keep using the image workflow.
    </p>
    {#if onPhone}
      <label class="check">
        <input type="checkbox" bind:checked={settings.unloadOllamaFirst} /> Share the phone's memory with the on-device chat model
      </label>
      <p class="muted small">
        Unloads the on-device chat model before each picture, and the picture model right after it. Keep this on unless the
        phone has plenty of memory: both models together rarely fit, and Android then closes the app. The chat model loads
        again with your next message.
      </p>
    {:else if !grok}
      <label class="check">
        <input type="checkbox" bind:checked={settings.unloadOllamaFirst} /> Share the graphics card with Ollama
      </label>
      <p class="muted small">
        Unloads Ollama's model before each picture and has ComfyUI free the card again afterwards. Keep this on when both run
        on one graphics card: two big models rarely fit in its memory together, and then both slow to a crawl. Ollama loads
        its model again with your next message.
      </p>
    {/if}

    {#if settings.backend === "comfyUi"}
      <label class="field">
        Video workflow (optional, API export)
        <textarea
          class="workflow"
          rows="6"
          bind:value={settings.comfyVideoWorkflow}
          spellcheck="false"
          placeholder="Leave empty for the built-in Wan text-to-video workflow. Or paste an API export. Same %prompt% placeholders."
        ></textarea>
      </label>
      <p class="muted small">
        Used for <code>[video: …]</code> clips. Leave it empty unless you already have a graph. A custom one should end in
        Save Video, Save WEBM, or Video Combine so ComfyUI returns a file.
      </p>
      <label class="check">
        <input type="checkbox" bind:checked={showWorkflow} /> Use my own ComfyUI workflow
      </label>
      {#if showWorkflow}
        <p class="muted small">
          In ComfyUI choose Workflow → Export (API) and paste the file here. These placeholders get filled in:
          <code>%prompt%</code> <code>%negative_prompt%</code> <code>%model%</code> <code>%sampler%</code> <code>%scheduler%</code>
          <code>%seed%</code> <code>%steps%</code> <code>%scale%</code> <code>%width%</code> <code>%height%</code>.
          Leave it empty for the built-in workflow.
        </p>
        <textarea class="workflow" rows="8" bind:value={settings.comfyWorkflow} spellcheck="false" placeholder="&#123;&quot;3&quot;: &#123;&quot;class_type&quot;: &quot;KSampler&quot;, …&#125;&#125;"></textarea>
      {/if}
    {/if}

    <div class="keyrow">
      <button class="primary" onclick={save}>Save picture settings</button>
    </div>
  {:else if status}
    <p class="err small">{status}</p>
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

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 10px;
  }

  .grid.numbers {
    grid-template-columns: repeat(auto-fill, minmax(110px, 1fr));
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--muted);
  }

  .field select,
  .field input,
  .field textarea {
    color: var(--text);
    font-size: 14px;
  }

  .pathrow {
    display: flex;
    gap: 8px;
  }

  .pathrow input {
    flex: 1;
    min-width: 0;
  }

  .keyrow {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .small {
    font-size: 12px;
    margin: 0;
  }

  .workflow {
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 12px;
    resize: vertical;
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

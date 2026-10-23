<script lang="ts">
  import { api, errorText } from "#lib/api.js";
  import type { DownloadProgress, ModelFile, OnDeviceStatus } from "#lib/types.js";

  let { onchange }: { onchange?: () => void } = $props();

  let info = $state<OnDeviceStatus | null>(null);
  let url = $state("");
  let progress = $state<DownloadProgress | null>(null);
  let downloading = $state(false);
  let message = $state("");

  const models = $derived(info?.files.filter((f) => !f.projector) ?? []);
  const projectors = $derived(info?.files.filter((f) => f.projector) ?? []);

  async function load() {
    try {
      info = await api.ondeviceStatus();
    } catch (e) {
      message = errorText(e);
    }
  }

  $effect(() => {
    void load();
  });

  function gb(bytes: number) {
    return `${(bytes / 1e9).toFixed(bytes < 1e9 ? 2 : 1)} GB`;
  }

  async function save(note = "Saved. The model reloads with these settings on the next reply.") {
    if (!info) return;
    try {
      await api.saveOndeviceOptions($state.snapshot(info.options));
      info.loaded = null;
      message = note;
    } catch (e) {
      message = errorText(e);
    }
  }

  function setProjector(model: string, fileName: string) {
    if (!info) return;
    if (fileName) info.options.projectors[model] = fileName;
    else delete info.options.projectors[model];
    void save(fileName ? `${model} now sees pictures.` : `${model} no longer gets pictures.`);
  }

  async function remove(file: ModelFile) {
    if (!confirm(`Delete ${file.fileName} (${gb(file.sizeBytes)}) from this device?`)) return;
    try {
      await api.deleteOndeviceModel(file.fileName);
      message = `Deleted ${file.fileName}.`;
      await load();
      onchange?.();
    } catch (e) {
      message = errorText(e);
    }
  }

  async function download() {
    downloading = true;
    message = "";
    progress = null;
    try {
      const saved = await api.downloadOndeviceModel(url.trim(), (p) => (progress = p));
      message = `Downloaded ${saved}.`;
      url = "";
      await load();
      onchange?.();
    } catch (e) {
      message = errorText(e);
    } finally {
      downloading = false;
    }
  }

  const percent = $derived(progress?.total ? Math.floor((progress.received / progress.total) * 100) : null);
</script>

<h3>Models on this device</h3>
<p class="muted small">
  Runs GGUF models on the phone itself with llama.cpp: nothing leaves the device and no key is needed. The model loads when a chat
  using it sends a message (that first load takes a little while) and is unloaded after it has been idle, to give the memory back.
</p>

{#if info}
  {#if models.length === 0}
    <p class="muted small">No models yet. Download one below.</p>
  {/if}
  <ul class="files">
    {#each models as file (file.fileName)}
      <li>
        <div class="grow">
          <div class="name">{file.name}{#if info.loaded === file.name}<span class="pill">loaded</span>{/if}</div>
          <div class="muted small">{gb(file.sizeBytes)}</div>
          <label class="field">
            Pictures
            <select value={info.options.projectors[file.name] ?? ""} onchange={(e) => setProjector(file.name, e.currentTarget.value)}>
              <option value="">Can't see pictures</option>
              {#each projectors as p (p.fileName)}
                <option value={p.fileName}>Through {p.name}</option>
              {/each}
            </select>
          </label>
        </div>
        <button class="ghost" onclick={() => remove(file)} aria-label="Delete {file.name}">Delete</button>
      </li>
    {/each}
    {#each projectors as file (file.fileName)}
      <li>
        <div class="grow">
          <div class="name">{file.name}</div>
          <div class="muted small">{gb(file.sizeBytes)} · vision projector</div>
        </div>
        <button class="ghost" onclick={() => remove(file)} aria-label="Delete {file.name}">Delete</button>
      </li>
    {/each}
  </ul>
  <p class="muted small">
    A model only sees pictures through the vision projector (<code>mmproj</code> file) made for its base model, e.g. Gemma 4 E4B's for a Gemma 4 E4B merge.
  </p>

  <div class="ondevice">
    <label class="field">
      Context size (tokens)
      <input type="number" min="512" step="512" bind:value={info.options.contextSize} />
    </label>
    <label class="field">
      CPU threads
      <input type="number" min="1" max="16" bind:value={info.options.threads} />
    </label>
    <label class="field">
      Unload after idle (seconds, 0 = never)
      <input type="number" min="0" step="60" bind:value={info.options.idleStopSecs} />
    </label>
  </div>
  <p class="muted small">
    More context remembers more of the story but needs more memory and makes the first reply slower to start. Phones are fastest
    with as many threads as they have big cores (usually 4).
  </p>
  <div class="row">
    <button onclick={() => save()}>Save</button>
  </div>

  <h3>Download a model</h3>
  <label class="field">
    Link to a .gguf file
    <input type="url" bind:value={url} placeholder="https://huggingface.co/…/resolve/main/model-Q4_K_M.gguf" disabled={downloading} />
  </label>
  <div class="row">
    {#if downloading}
      <button onclick={() => api.cancelOndeviceDownload()}>Stop</button>
      {#if progress}
        <progress max="100" value={percent ?? undefined}></progress>
        <span class="muted small">
          {gb(progress.received)}{#if progress.total} of {gb(progress.total)} ({percent}%){/if}
        </span>
      {:else}
        <span class="muted small">Connecting…</span>
      {/if}
    {:else}
      <button onclick={download} disabled={!url.trim()}>Download</button>
    {/if}
  </div>
  <p class="muted small">A stopped or broken download continues where it left off when started again with the same link.</p>
{/if}
{#if message}<p class="muted small" role="status">{message}</p>{/if}

<style>
  .files {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .files li {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }

  .grow {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .ondevice {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 10px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    margin-top: 8px;
  }

  progress {
    flex: 1;
    min-width: 120px;
  }

  input[type="url"] {
    width: 100%;
  }
</style>

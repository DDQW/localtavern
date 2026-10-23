<script lang="ts">
  import { api, errorText } from "#lib/api.js";
  import type { DownloadProgress, ImageModelFile, OnDeviceImageStatus } from "#lib/types.js";

  /** Called after a download or delete, so the model picker can reload. */
  let { onchange }: { onchange?: () => void } = $props();

  /** Few-step (LCM) SD 1.5 models made for stable-diffusion.cpp: about a minute a picture on a phone. */
  const SUGGESTED = [
    { label: "Anime: Anything v3 LCM (1.8 GB)", url: "https://huggingface.co/mzwing/Anything-LCM-GGUF/resolve/main/anything_v30-LCM.q8_0.gguf" },
    {
      label: "Realistic: DreamShaper 8 LCM (1.6 GB)",
      url: "https://huggingface.co/stduhpf/dreamshaper-8LCM-im-GGUF-sdcpp/resolve/main/dreamshaper_8LCM-iq4_nl-imv2.gguf",
    },
  ];

  let info = $state<OnDeviceImageStatus | null>(null);
  let url = $state("");
  let progress = $state<DownloadProgress | null>(null);
  let downloading = $state(false);
  let message = $state("");

  async function load() {
    try {
      info = await api.ondeviceImageStatus();
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

  async function remove(file: ImageModelFile) {
    if (!confirm(`Delete ${file.fileName} (${gb(file.sizeBytes)}) from this device?`)) return;
    try {
      await api.deleteOndeviceImageModel(file.fileName);
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
      const saved = await api.downloadOndeviceImageModel(url.trim(), (p) => (progress = p));
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

{#if info}
  {#if info.files.length === 0}
    <p class="muted small">No picture models on this phone yet. Download one below.</p>
  {/if}
  <ul class="files">
    {#each info.files as file (file.fileName)}
      <li>
        <div class="grow">
          <div class="name">{file.fileName}</div>
          <div class="muted small">{gb(file.sizeBytes)}</div>
        </div>
        <button class="ghost" onclick={() => remove(file)} aria-label="Delete {file.fileName}">Delete</button>
      </li>
    {/each}
  </ul>

  <label class="field">
    Link to a Stable Diffusion checkpoint (.gguf or .safetensors)
    <input type="url" bind:value={url} placeholder="https://huggingface.co/…/resolve/main/model.gguf" disabled={downloading} />
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
  {#if !downloading}
    <div class="row">
      {#each SUGGESTED.filter((s) => !info?.files.some((f) => s.url.endsWith(f.fileName))) as suggestion (suggestion.url)}
        <button class="ghost" onclick={() => (url = suggestion.url)}>{suggestion.label}</button>
      {/each}
    </div>
  {/if}
  <p class="muted small">
    LCM and Turbo models (the name says so) draw in 4-5 steps and take about a minute a picture; LocalTavern sets their steps,
    CFG and sampler itself. A plain SD 1.5 model takes over ten minutes. SDXL models need about 7 GB of memory and are slower
    still. The first download also fetches TAESD (10 MB), a fast decoder for SD 1.5 models. A stopped or broken download
    continues where it left off when started again with the same link.
  </p>
{/if}
{#if message}<p class="muted small" role="status">{message}</p>{/if}

<style>
  .files {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .files li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
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
    width: 100%;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  progress {
    flex: 1;
    min-width: 120px;
  }

  .small {
    font-size: 12px;
    margin: 0;
  }
</style>

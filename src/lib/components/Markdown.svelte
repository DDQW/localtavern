<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { inTauri } from "$lib/api";
  import { lightbox } from "$lib/lightbox.svelte";
  import { renderMarkdown } from "$lib/markdown";

  let { text, dialogue = false }: { text: string; dialogue?: boolean } = $props();

  const html = $derived(renderMarkdown(text, dialogue));

  async function onclick(event: MouseEvent) {
    const target = event.target as HTMLElement;

    const copy = target.closest<HTMLElement>("[data-copy-code]");
    if (copy) {
      const code = copy.closest("pre")?.querySelector("code")?.textContent ?? "";
      await navigator.clipboard.writeText(code);
      copy.textContent = "Copied";
      setTimeout(() => (copy.textContent = "Copy"), 1200);
      return;
    }

    const link = target.closest("a");
    if (link) {
      event.preventDefault();
      const href = link.getAttribute("href") ?? "";
      if (/^https?:\/\//i.test(href)) {
        if (inTauri) await openUrl(href);
        else window.open(href, "_blank", "noopener");
      }
      return;
    }

    const image = target.closest("img.md-media");
    if (image) lightbox.src = image.getAttribute("src") ?? "";
  }
</script>

<!-- The content is sanitized HTML from renderMarkdown; the handler only does event delegation. -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="md" role="presentation" {onclick}>{@html html}</div>

<style>
  .md {
    overflow-wrap: anywhere;
  }

  .md :global(> :first-child) {
    margin-top: 0;
  }

  .md :global(> :last-child) {
    margin-bottom: 0;
  }

  .md :global(p) {
    margin: 0.5em 0;
  }

  .md :global(h1),
  .md :global(h2),
  .md :global(h3),
  .md :global(h4) {
    margin: 0.9em 0 0.4em;
    line-height: 1.25;
  }

  .md :global(h1) {
    font-size: 1.4em;
  }
  .md :global(h2) {
    font-size: 1.2em;
  }
  .md :global(h3) {
    font-size: 1.05em;
  }

  .md :global(ul),
  .md :global(ol) {
    margin: 0.5em 0;
    padding-left: 1.6em;
  }

  .md :global(li) {
    margin: 0.15em 0;
  }

  /* Spoken dialogue (see dialogue.ts). Actions in *asterisks* are plain italics. */
  .md :global(.md-quote) {
    color: var(--accent);
  }

  .md :global(blockquote) {
    margin: 0.6em 0;
    padding: 0.1em 0.9em;
    border-left: 3px solid var(--accent);
    color: var(--muted);
  }

  .md :global(hr) {
    border: 0;
    border-top: 1px solid var(--border);
    margin: 1em 0;
  }

  .md :global(:not(pre) > code) {
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 0.9em;
    background: var(--panel-3);
    padding: 0.1em 0.4em;
    border-radius: 4px;
  }

  .md :global(pre.code) {
    margin: 0.7em 0;
    background: var(--code-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .md :global(.code-head) {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 4px 8px 4px 12px;
    background: rgba(255, 255, 255, 0.04);
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    color: #9aa3b2;
  }

  .md :global(.code-copy) {
    font-size: 11px;
    padding: 2px 8px;
    background: transparent;
    color: #9aa3b2;
    border-color: transparent;
  }

  .md :global(.code-copy:hover) {
    background: rgba(255, 255, 255, 0.08);
  }

  .md :global(pre.code code.hljs) {
    display: block;
    overflow-x: auto;
    padding: 12px;
    background: transparent;
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 13px;
    line-height: 1.5;
    white-space: pre;
  }

  .md :global(table) {
    border-collapse: collapse;
    margin: 0.7em 0;
    display: block;
    overflow-x: auto;
    max-width: 100%;
  }

  .md :global(th),
  .md :global(td) {
    border: 1px solid var(--border);
    padding: 5px 10px;
    text-align: left;
  }

  .md :global(th) {
    background: var(--panel-2);
  }

  .md :global(.md-media) {
    display: block;
    max-width: min(100%, 520px);
    max-height: 420px;
    border-radius: var(--radius);
    margin: 0.5em 0;
  }

  .md :global(img.md-media) {
    cursor: zoom-in;
  }

  .md :global(.md-audio) {
    display: block;
    margin: 0.5em 0;
    max-width: 100%;
  }
</style>

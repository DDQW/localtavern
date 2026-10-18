<script lang="ts">
  import { chat } from "$lib/chat.svelte";

  const context = $derived(chat.context);
  const percent = $derived(context?.window ? Math.round((context.used / context.window) * 100) : null);
  const level = $derived(
    percent === null ? "" : percent >= 100 ? "full" : percent >= (context?.autoCompactPercent ?? 80) ? "high" : "",
  );

  function short(tokens: number): string {
    if (tokens < 1000) return String(tokens);
    if (tokens < 10_000) return `${(tokens / 1000).toFixed(1)}k`;
    if (tokens < 1_000_000) return `${Math.round(tokens / 1000)}k`;
    return `${(tokens / 1_000_000).toFixed(tokens < 10_000_000 ? 1 : 0)}M`;
  }

  const sourceText = $derived.by(() => {
    const provider = chat.session?.conversation.providerId;
    if (context?.windowSource === "preset") return "the preset's max history tokens";
    if (context?.windowSource === "provider") return provider === "ollama" ? "the Ollama context size in Settings" : "the model's own limit";
    return "";
  });

  const tooltip = $derived.by(() => {
    if (!context) return "";
    const used = `${context.estimated ? "About " : ""}${context.used.toLocaleString()} tokens`;
    const lines = [
      context.window
        ? `${used} of ${context.window.toLocaleString()} (${percent}%) - window from ${sourceText}.`
        : `${used}. This provider doesn't report the model's window; set "Max history tokens" in the preset to get one.`,
      context.estimated ? "Estimated from the text (about 4 characters per token)." : "As counted by the provider for the last reply.",
    ];
    if (context.window) lines.push(`Older messages are summarized automatically at ${context.autoCompactPercent}%.`);
    if (context.summarizedCount > 0) lines.push(`${context.summarizedCount} earlier messages are summarized.`);
    return lines.join("\n");
  });
</script>

{#if context}
  <div class="meter {level}" title={tooltip}>
    <span class="label">
      {context.estimated ? "≈" : ""}{short(context.used)}{context.window ? ` / ${short(context.window)}` : " tokens"}
    </span>
    {#if percent !== null}
      <span class="bar" role="meter" aria-label="Context used" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.min(percent, 100)}>
        <span class="fill" style:width="{Math.min(percent, 100)}%"></span>
      </span>
    {/if}
    <button
      class="ghost"
      onclick={() => chat.compact()}
      disabled={chat.busy || context.compactableCount === 0}
      title={context.compactableCount === 0
        ? "Nothing to compact yet - the newest messages are always kept in full."
        : `Summarize ${context.compactableCount} older messages so they stop filling the context`}
    >
      {chat.compacting ? "Compacting…" : "Compact"}
    </button>
  </div>
{/if}

<style>
  .meter {
    display: flex;
    align-items: center;
    gap: 8px;
    align-self: center;
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
  }

  .label {
    font-variant-numeric: tabular-nums;
  }

  .bar {
    width: 90px;
    height: 6px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }

  .fill {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.3s;
  }

  .high .fill {
    background: var(--warn);
  }

  .high .label {
    color: var(--warn);
  }

  .full .fill {
    background: var(--danger);
  }

  .full .label {
    color: var(--danger);
  }

  button {
    padding: 2px 8px;
    font-size: 12px;
  }
</style>

<script lang="ts">
  import { chat } from "$lib/chat.svelte";
  import { fileUrl } from "$lib/media";
  import { lightbox } from "$lib/lightbox.svelte";
  import type { UiMessage } from "$lib/types";
  import Avatar from "./Avatar.svelte";
  import Markdown from "./Markdown.svelte";

  let { message, latest }: { message: UiMessage; latest: boolean } = $props();

  const speaker = $derived(chat.speaker(message));
  const isUser = $derived(message.role === "user");
  const isCharacter = $derived(message.role === "character");
  const isTool = $derived(message.role === "tool");
  const isPicture = $derived(message.role === "image");
  const ofCharacter = $derived(isPicture && chat.session?.characters.some((c) => c.id === message.speakerCharacterId));
  let pictureStatus = $state("");

  async function usePicture(as: "avatar" | "gallery") {
    pictureStatus = await chat.usePicture(message, as);
    if (pictureStatus) setTimeout(() => (pictureStatus = ""), 2500);
  }
  const swipeLabel = $derived(`${message.activeAlternateIndex + 1}/${Math.max(message.alternates.length, 1)}`);
  const canSwipePrevious = $derived(message.activeAlternateIndex > 0);
  const waiting = $derived(message.streaming && message.text.length === 0);
  const clock = $derived(message.clock && message.clock.advancedMinutes > 0 ? message.clock : null);
  const clockLabel = $derived.by(() => {
    const m = clock?.advancedMinutes ?? 0;
    const text = m >= 1440 ? `${+(m / 1440).toFixed(1)} d` : m >= 60 ? `${+(m / 60).toFixed(1)} h` : `${m} min`;
    return `+${text}`;
  });
  const clockHint = $derived(
    !clock
      ? ""
      : clock.clampedFrom
        ? `The model asked for ${clock.clampedFrom} minutes; held to the limit`
        : clock.estimated
          ? "The reply gave no time, so the per-message default was used"
          : "Story time this reply covered, as estimated by the model",
  );

  let copied = $state(false);
  async function copy() {
    await navigator.clipboard.writeText(message.text);
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  function onEditKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") chat.cancelEdit(message);
    else if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) void chat.commitEdit(message);
  }
</script>

<article class="msg" class:user={isUser} class:streaming={message.streaming}>
  <Avatar name={speaker.name} path={speaker.avatarPath} />

  {#if isPicture}
    <div class="tools" role="toolbar" aria-label="Picture actions">
      <button class="ghost" onclick={() => chat.drawAgain(message)} disabled={!!chat.picture} title="Draw it again with the same prompt">Again</button>
      <button class="ghost" onclick={() => chat.redrawPicture(message)} disabled={!!chat.picture} title="Edit the prompt, then draw it again">Edit prompt</button>
      {#if ofCharacter}
        <button class="ghost" onclick={() => usePicture("avatar")}>Set as avatar</button>
        <button class="ghost" onclick={() => usePicture("gallery")}>Add to gallery</button>
      {/if}
      <button class="ghost" onclick={copy}>{copied ? "Copied" : "Copy prompt"}</button>
      <button class="ghost danger" onclick={() => chat.remove(message)}>Delete</button>
    </div>
  {:else if !message.editing && !message.streaming}
    <div class="tools" role="toolbar" aria-label="Message actions">
      <button class="ghost" onclick={() => chat.beginEdit(message)}>Edit</button>
      <button class="ghost" onclick={copy}>{copied ? "Copied" : "Copy"}</button>
      <button class="ghost danger" onclick={() => chat.remove(message)}>Delete</button>
    </div>
  {/if}

  <div class="body">
    <div class="meta">
      <strong>{speaker.name}</strong>
      {#if isPicture && message.speakerCharacterId}<span class="muted tokens">picture</span>{/if}
      {#if message.usage?.totalTokens}<span class="muted tokens">{message.usage.totalTokens} tokens</span>{/if}
      {#if clock}<span class="muted tokens" title={clockHint}>⏱ {clockLabel}{clock.clampedFrom ? " (capped)" : clock.estimated ? " (default)" : ""}</span>{/if}
    </div>

    {#if message.reasoningText}
      <details class="fold">
        <summary>Reasoning</summary>
        <div class="plain muted">{message.reasoningText}</div>
      </details>
    {/if}

    {#if isPicture}
      {#each message.attachments as attachment (attachment.imagePath)}
        <button class="picture" onclick={() => (lightbox.src = fileUrl(attachment.imagePath))} aria-label="View picture full size">
          <img src={fileUrl(attachment.imagePath)} alt={message.text} />
        </button>
      {/each}
      {#if pictureStatus}<p class="muted status" role="status">{pictureStatus}</p>{/if}
      <details class="fold">
        <summary>Prompt</summary>
        <div class="plain muted">{message.text}</div>
      </details>
    {:else if message.editing}
      <textarea
        class="edit"
        rows="4"
        bind:value={message.editBuffer}
        onkeydown={onEditKeydown}
        aria-label="Edit message"
      ></textarea>
      <div class="edit-actions">
        <button class="primary" onclick={() => chat.commitEdit(message)}>Save</button>
        <button onclick={() => chat.cancelEdit(message)}>Cancel</button>
        <span class="muted hint">Ctrl+Enter saves · Esc cancels</span>
      </div>
    {:else if waiting}
      <div class="typing" aria-label="Generating"><span></span><span></span><span></span></div>
    {:else if isTool}
      <pre class="toolout" aria-label="PowerShell output">{message.text}</pre>
    {:else}
      <div class="content"><Markdown text={message.text} dialogue={chat.isRoleplay} />{#if message.streaming}<span class="caret"></span>{/if}</div>
    {/if}

    {#if message.attachments.length > 0 && !isPicture}
      <div class="attachments">
        {#each message.attachments as attachment (attachment.imagePath)}
          <button class="thumb" onclick={() => (lightbox.src = fileUrl(attachment.imagePath))} aria-label="View picture">
            <img src={fileUrl(attachment.imagePath)} alt={attachment.description ?? ""} title={attachment.description ?? undefined} loading="lazy" />
          </button>
        {/each}
      </div>
    {/if}

    {#if latest && isCharacter && !message.editing && !message.streaming}
      <div class="swipebar">
        <button class="ghost" onclick={() => chat.swipePrevious(message)} disabled={!canSwipePrevious || chat.busy} aria-label="Previous reply">←</button>
        <span class="muted">{swipeLabel}</span>
        <button class="ghost" onclick={() => chat.swipeNext(message)} disabled={chat.busy} aria-label="Next reply, or generate another">→</button>
      </div>
    {/if}

    {#if message.rawPromptText && !isPicture}
      <details class="fold">
        <summary>Prompt sent to model</summary>
        <pre class="plain mono">{message.rawPromptText}</pre>
      </details>
    {/if}
  </div>
</article>

<style>
  .msg {
    position: relative;
    display: flex;
    gap: 12px;
    padding: 12px 14px;
    border-radius: var(--radius);
    max-width: 860px;
    margin: 0 auto 6px;
    width: 100%;
  }

  .msg.user {
    background: var(--user-bg);
  }

  .body {
    flex: 1;
    min-width: 0;
  }

  .meta {
    display: flex;
    align-items: baseline;
    gap: 10px;
    font-size: 13px;
    margin-bottom: 2px;
  }

  .tokens {
    font-size: 11px;
  }

  .content {
    font-size: 14.5px;
  }

  .toolout {
    margin: 0;
    padding: 8px 10px;
    max-height: 320px;
    overflow: auto;
    background: var(--panel-3);
    border-radius: var(--radius-sm);
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 12.5px;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .caret {
    display: inline-block;
    width: 7px;
    height: 1em;
    margin-left: 2px;
    vertical-align: text-bottom;
    background: var(--accent);
    animation: blink 1s steps(2, start) infinite;
  }

  @keyframes blink {
    to {
      visibility: hidden;
    }
  }

  .typing {
    display: flex;
    gap: 4px;
    padding: 8px 0;
  }

  .typing span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--muted);
    animation: bounce 1.2s infinite ease-in-out;
  }

  .typing span:nth-child(2) {
    animation-delay: 0.15s;
  }
  .typing span:nth-child(3) {
    animation-delay: 0.3s;
  }

  @keyframes bounce {
    0%,
    60%,
    100% {
      transform: translateY(0);
      opacity: 0.4;
    }
    30% {
      transform: translateY(-4px);
      opacity: 1;
    }
  }

  .fold {
    margin: 4px 0;
    font-size: 13px;
  }

  .fold summary {
    cursor: pointer;
    color: var(--muted);
    user-select: none;
  }

  .plain {
    margin: 6px 0 0;
    padding: 8px 10px;
    border-left: 2px solid var(--border);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .mono {
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 12px;
    max-height: 300px;
    overflow: auto;
  }

  .edit {
    width: 100%;
    margin-top: 4px;
  }

  .attachments {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 8px;
  }

  .thumb {
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
    line-height: 0;
    background: var(--panel-2);
  }

  .thumb img {
    max-height: 160px;
    max-width: 220px;
    object-fit: cover;
  }

  .picture {
    display: block;
    margin-top: 4px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
    line-height: 0;
    background: var(--panel-2);
    max-width: 100%;
  }

  .picture img {
    max-width: 100%;
    max-height: 520px;
    object-fit: contain;
  }

  .status {
    margin: 4px 0 0;
    font-size: 12px;
  }

  .tools {
    position: absolute;
    top: 4px;
    right: 8px;
    display: flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel);
    box-shadow: var(--shadow);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.12s;
  }

  .msg:hover .tools,
  .msg:focus-within .tools {
    opacity: 1;
    pointer-events: auto;
  }

  .tools button {
    font-size: 12px;
    padding: 2px 8px;
  }

  .swipebar {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: 4px;
    font-size: 12px;
  }

  .edit-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
  }

  .hint {
    margin-left: 8px;
    font-size: 11px;
  }
</style>

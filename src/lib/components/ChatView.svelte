<script lang="ts">
  import { tick } from "svelte";
  import { chat } from "#lib/chat.svelte.js";
  import ChatHeader from "./ChatHeader.svelte";
  import Composer from "./Composer.svelte";
  import Message from "./Message.svelte";
  import StoryPanel from "./StoryPanel.svelte";
  import PictureDialog from "./PictureDialog.svelte";
  import ToolApprovalDialog from "./ToolApprovalDialog.svelte";
  import { ui } from "#lib/ui.svelte.js";

  const PAGE = 60;

  let scroller = $state<HTMLDivElement>();
  let visible = $state(PAGE);
  // Follow new text only while the reader is already at the bottom; scrolling up pauses it.
  let stick = true;

  const hidden = $derived(Math.max(0, chat.messages.length - visible));
  const shown = $derived(chat.messages.slice(hidden));
  const lastSummarized = $derived(chat.lastSummarizedIndex);
  // Swipes belong to the newest reply, even when pictures were drawn after it.
  const lastTurnId = $derived(chat.lastTurn?.id);

  function onscroll() {
    if (!scroller) return;
    stick = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  }

  async function scrollToBottom() {
    await tick();
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  // A different conversation starts at its newest page, pinned to the bottom.
  $effect(() => {
    chat.conversationId;
    visible = PAGE;
    stick = true;
    ui.storyOpen = false;
    void scrollToBottom();
  });

  // New messages and streamed text keep the view pinned (when it is).
  $effect(() => {
    const last = chat.messages.at(-1);
    chat.messages.length;
    last?.text;
    last?.reasoningText;
    if (stick) void scrollToBottom();
  });

  async function loadEarlier() {
    if (!scroller) return;
    const before = scroller.scrollHeight;
    visible += PAGE;
    await tick();
    // Keep what the reader was looking at in place while older messages appear above it.
    scroller.scrollTop += scroller.scrollHeight - before;
  }
</script>

<div class="chat">
  <div class="column">
    <ChatHeader />

    <div class="scroller" bind:this={scroller} {onscroll}>
      {#if chat.loading && !chat.session}
        <p class="muted center">Loading…</p>
      {:else if chat.session && chat.messages.length === 0}
        <p class="muted center empty">No messages yet — say something below.</p>
      {/if}

      {#if hidden > 0}
        <div class="earlier">
          <button onclick={loadEarlier}>Load {Math.min(hidden, PAGE)} earlier messages ({hidden} hidden)</button>
        </div>
      {/if}

      {#each shown as message, i (message.id)}
        <div class:summarized={hidden + i <= lastSummarized}>
          {#if message.role === "system"}
            <!-- Something the app noted in the story, e.g. where a private talk happened. -->
            <p class="marker">
              <span>{message.text}</span>
              {#if chat.isGroup && message.speakerCharacterId}
                <button class="ghost" onclick={() => chat.openBreakout(message.speakerCharacterId!)}>Open the talk</button>
              {/if}
              <button class="ghost" onclick={() => chat.remove(message)} disabled={chat.busy} title="Remove this note" aria-label="Remove this note">✕</button>
            </p>
          {:else}
            <Message {message} latest={message.id === lastTurnId} />
          {/if}
        </div>
        {#if hidden + i === lastSummarized && chat.context?.summary}
          <details class="summary">
            <summary>
              ▲ {lastSummarized + 1} earlier messages are summarized — the model sees this summary instead of them
            </summary>
            <p>{chat.context.summary.text}</p>
            <button class="ghost" onclick={() => chat.clearSummary()} disabled={chat.busy}>Undo compaction</button>
          </details>
        {/if}
      {/each}
    </div>

    <Composer />
  </div>

  {#if chat.isRoleplay}
    <!-- On a phone the story panel is a drawer, opened from the header. -->
    {#if ui.storyOpen}
      <button class="backdrop" aria-label="Close story panel" onclick={() => (ui.storyOpen = false)}></button>
    {/if}
    <div class="story" class:open={ui.storyOpen}>
      <StoryPanel />
    </div>
  {/if}
</div>

<ToolApprovalDialog />
<PictureDialog />

<style>
  .marker {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: 4px 10px;
    margin: 10px 20px;
    font-size: 13px;
    font-style: italic;
    color: var(--muted);
    text-align: center;
  }

  .marker button {
    font-style: normal;
    font-size: 12px;
  }

  .chat {
    display: flex;
    height: 100%;
    min-width: 0;
  }

  .column {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .story {
    display: contents;
  }

  .backdrop {
    display: none;
  }

  @media (max-width: 760px) {
    .story {
      display: block;
      position: fixed;
      inset: 0 0 0 auto;
      width: min(340px, 88vw);
      z-index: 40;
      transform: translateX(100%);
      transition: transform 0.18s ease-out;
      box-shadow: var(--shadow);
    }

    .story.open {
      transform: none;
    }

    .story :global(.panel) {
      width: 100%;
      height: 100%;
      padding-top: calc(env(safe-area-inset-top) + 12px);
      padding-bottom: calc(env(safe-area-inset-bottom) + 24px);
    }

    .backdrop {
      display: block;
      position: fixed;
      inset: 0;
      z-index: 35;
      padding: 0;
      border: 0;
      border-radius: 0;
      background: rgb(0 0 0 / 0.45);
    }
  }

  .scroller {
    flex: 1;
    overflow-y: auto;
    padding: 16px 12px 8px;
  }

  .center {
    text-align: center;
    margin-top: 40px;
  }

  .earlier {
    text-align: center;
    margin-bottom: 12px;
  }

  .summarized {
    opacity: 0.55;
  }

  .summary {
    margin: 10px 8px 16px;
    padding: 8px 12px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-sm);
    font-size: 13px;
    color: var(--muted);
  }

  .summary summary {
    cursor: pointer;
  }

  .summary p {
    white-space: pre-wrap;
    color: var(--text);
    margin: 8px 0;
  }
</style>

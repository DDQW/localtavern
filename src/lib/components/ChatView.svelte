<script lang="ts">
  import { tick } from "svelte";
  import { chat } from "$lib/chat.svelte";
  import ChatHeader from "./ChatHeader.svelte";
  import Composer from "./Composer.svelte";
  import Message from "./Message.svelte";
  import StoryPanel from "./StoryPanel.svelte";
  import ToolApprovalDialog from "./ToolApprovalDialog.svelte";

  const PAGE = 60;

  let scroller = $state<HTMLDivElement>();
  let visible = $state(PAGE);
  // Follow new text only while the reader is already at the bottom; scrolling up pauses it.
  let stick = true;

  const hidden = $derived(Math.max(0, chat.messages.length - visible));
  const shown = $derived(chat.messages.slice(hidden));

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
        <Message {message} latest={hidden + i === chat.messages.length - 1} />
      {/each}
    </div>

    <Composer />
  </div>

  {#if chat.isRoleplay}
    <StoryPanel />
  {/if}
</div>

<ToolApprovalDialog />

<style>
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
</style>

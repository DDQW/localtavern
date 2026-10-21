<script lang="ts">
  import { goto } from "$app/navigation";
  import { api } from "$lib/api";
  import { chat } from "$lib/chat.svelte";
  import type { Character } from "$lib/types";
  import ContextMeter from "./ContextMeter.svelte";
  import RoomTabs from "./RoomTabs.svelte";
  import { ui } from "$lib/ui.svelte";

  let providers = $state<{ id: string; name: string }[]>([]);
  let allCharacters = $state<Character[]>([]);
  let showPrompt = $state(false);
  // Phone only: the pickers fold away behind a button so the messages get the screen.
  let showPickers = $state(false);
  let refreshing = $state(false);

  $effect(() => {
    void api.bootstrap().then((b) => (providers = b.providers.map((p) => ({ id: p.id, name: p.name }))));
    void api.listCharacters().then((c) => (allCharacters = c));
  });

  const session = $derived(chat.session);
  const conversation = $derived(session?.conversation);
  const isAssistant = $derived(conversation?.mode === "assistant");
  const singleCharacter = $derived(chat.isRoleplay && !chat.isGroup && !chat.isBreakout);
  const title = $derived(
    chat.isRoleplay ? session?.characters.map((c) => c.name).join(", ") || conversation?.title || "" : "Assistant",
  );
  // A conversation might be pinned to a model the cached list no longer offers; keep it selectable.
  const models = $derived.by(() => {
    const list = session?.models ?? [];
    const current = conversation?.model;
    return current && !list.includes(current) ? [current, ...list] : list;
  });

  async function refreshModels() {
    refreshing = true;
    await chat.refreshModels();
    refreshing = false;
  }

  async function newChat() {
    if (await chat.newChat()) await goto(`/chat/${chat.conversationId}`, { replaceState: true });
  }
</script>

{#if session && conversation}
  <header class="head">
    <div class="titlebar">
      <h1>{title}</h1>
      {#if chat.isBreakout}
        <span class="muted subtitle">in private, away from {session.room?.title ?? "the group"}</span>
      {:else if conversation.title && !isAssistant && conversation.title !== title}
        <span class="muted subtitle">{conversation.title}</span>
      {:else if isAssistant && conversation.title !== "Assistant chat"}
        <span class="muted subtitle">{conversation.title}</span>
      {/if}
      <span class="spacer"></span>
      <span class="meter"><ContextMeter /></span>
      {#if isAssistant}
        <button class="ghost" onclick={() => (showPrompt = !showPrompt)} aria-expanded={showPrompt}>System prompt</button>
      {/if}
      <button class="narrow" onclick={() => (showPickers = !showPickers)} aria-expanded={showPickers} title="Persona, preset, model and history">⚙</button>
      {#if chat.isRoleplay}
        <button class="narrow" onclick={() => (ui.storyOpen = true)} title="Story panel">Story</button>
      {/if}
      {#if !chat.isBreakout}
        <button onclick={newChat} disabled={chat.busy} title="Start a new chat" aria-label="New chat">＋<span class="wide"> New chat</span></button>
      {/if}
    </div>

    <RoomTabs />

    <div class="pickers" class:open={showPickers}>
      <label class="field">
        Persona
        <select value={conversation.personaId ?? ""} onchange={(e) => chat.setPersona(e.currentTarget.value || null)}>
          <option value="">No persona</option>
          {#each session.personas as persona (persona.id)}
            <option value={persona.id}>{persona.name}</option>
          {/each}
        </select>
      </label>

      {#if singleCharacter}
        <label class="field">
          Character
          <select value={conversation.characterId ?? ""} onchange={(e) => goto(`/character/${e.currentTarget.value}`)}>
            {#each allCharacters as character (character.id)}
              <option value={character.id}>{character.name}</option>
            {/each}
          </select>
        </label>
      {/if}

      <label class="field">
        Preset
        <select value={session.preset.id} onchange={(e) => chat.setPreset(e.currentTarget.value)}>
          {#each session.presets as preset (preset.id)}
            <option value={preset.id}>{preset.name}</option>
          {/each}
        </select>
      </label>

      <label class="field">
        Provider
        <select value={conversation.providerId} onchange={(e) => chat.setModel(e.currentTarget.value, "")}>
          {#each providers as provider (provider.id)}
            <option value={provider.id}>{provider.name}</option>
          {/each}
        </select>
      </label>

      <label class="field model">
        Model
        <span class="with-button">
          <select value={conversation.model} onchange={(e) => chat.setModel(conversation.providerId, e.currentTarget.value)}>
            {#if models.length === 0}
              <option value="">No models yet</option>
            {/if}
            {#each models as model (model)}
              <option value={model}>{model}</option>
            {/each}
          </select>
          <button onclick={refreshModels} disabled={refreshing} title="Fetch the provider's current model list" aria-label="Refresh models">
            {refreshing ? "…" : "↻"}
          </button>
        </span>
      </label>

      <label class="field history">
        History
        <!-- A private talk is part of its group chat, which is what History picks between. -->
        <select value={conversation.parentConversationId ?? conversation.id} onchange={(e) => goto(`/chat/${e.currentTarget.value}`)}>
          {#each session.history as entry (entry.id)}
            <option value={entry.id}>{entry.label}</option>
          {/each}
        </select>
      </label>
    </div>

    {#if isAssistant && showPrompt}
      <label class="field prompt">
        System prompt for this chat
        <textarea
          rows="3"
          value={conversation.assistantSystemPrompt ?? ""}
          placeholder="e.g. You are a terse coding assistant."
          onchange={(e) => chat.setAssistantPrompt(e.currentTarget.value)}
        ></textarea>
      </label>
    {/if}
  </header>
{/if}

<style>
  .head {
    padding: 14px 20px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .titlebar {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }

  h1 {
    font-size: 20px;
  }

  .subtitle {
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .spacer {
    flex: 1;
  }

  .titlebar button {
    align-self: center;
  }



  .pickers {
    display: flex;
    flex-wrap: wrap;
    gap: 10px 12px;
  }

  .field select {
    max-width: 240px;
  }

  .model .with-button {
    display: flex;
    gap: 4px;
  }

  .model select {
    width: 190px;
  }

  .history select {
    width: 250px;
  }

  .prompt textarea {
    width: 100%;
  }

  .narrow {
    display: none;
  }

  @media (max-width: 760px) {
    .head {
      padding: 8px 12px;
      gap: 8px;
    }

    .titlebar {
      flex-wrap: wrap;
      gap: 6px;
      align-items: center;
    }

    /* The meter gets a line of its own under the title and buttons. */
    .meter {
      order: 1;
      flex-basis: 100%;
    }

    h1 {
      flex: 1;
      font-size: 17px;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .subtitle,
    .spacer,
    .wide {
      display: none;
    }

    .narrow {
      display: inline-block;
    }

    .pickers:not(.open) {
      display: none;
    }

    .pickers {
      display: grid;
      grid-template-columns: 1fr 1fr;
    }

    .field select,
    .model select,
    .history select {
      max-width: none;
      width: 100%;
    }

    .model,
    .history {
      grid-column: 1 / -1;
    }

    .model .with-button select {
      flex: 1;
    }
  }
</style>

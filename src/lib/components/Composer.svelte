<script lang="ts">
  import { tick } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, errorText, inTauri } from "$lib/api";
  import { chat } from "$lib/chat.svelte";
  import { fileUrl } from "$lib/media";
  import type { ImageMode, Uuid } from "$lib/types";

  // A touch keyboard has no Shift+Enter, so there Enter is a newline and the Send button sends.
  const touch = typeof matchMedia !== "undefined" && matchMedia("(pointer: coarse)").matches;

  /** Enter sends; Shift+Enter inserts a newline (the textarea's default). */
  function onkeydown(event: KeyboardEvent) {
    if (touch || event.key !== "Enter" || event.shiftKey || event.isComposing) return;
    event.preventDefault();
    void chat.send();
  }

  async function attach() {
    try {
      let picked: string[] = [];
      if (inTauri) {
        const selection = await open({
          multiple: true,
          filters: [{ name: "Pictures", extensions: ["png", "jpg", "jpeg", "webp", "gif", "bmp"] }],
        });
        picked = selection === null ? [] : Array.isArray(selection) ? selection : [selection];
      } else {
        picked = ["C:\\mock\\picture.png"];
      }
      for (const path of picked) chat.pendingAttachments.push(await api.storeImage(path));
    } catch (e) {
      chat.error = errorText(e);
    }
  }

  // ---- the Picture menu ----
  let pictureMenu = $state(false);
  let menuWrap = $state<HTMLDivElement>();
  let menuEl = $state<HTMLDivElement>();
  // The menu opens leftwards from the button, unless that would run off the screen (on a phone the
  // button sits on the left once the row wraps); then it opens rightwards.
  let menuOpensRight = $state(false);

  async function togglePictureMenu() {
    pictureMenu = !pictureMenu;
    if (!pictureMenu) return;
    menuOpensRight = false;
    await tick();
    menuOpensRight = (menuEl?.getBoundingClientRect().left ?? 0) < 8;
  }

  /** Any click outside the menu closes it. */
  function onWindowClick(event: MouseEvent) {
    if (pictureMenu && !menuWrap?.contains(event.target as Node)) pictureMenu = false;
  }
  const userName = $derived(chat.session?.persona?.name || "Me");
  const pictureBusy = $derived(chat.busy || !!chat.picture);

  function draw(mode: ImageMode, characterId: Uuid | null = null) {
    pictureMenu = false;
    void chat.drawPicture(mode, characterId);
  }

  const search = $derived(chat.session?.conversation.enableGoogleSearch ?? false);
  const showSearch = $derived(chat.session?.conversation.providerId === "gemini" && !chat.isRoleplay);
</script>

<svelte:window onclick={onWindowClick} onkeydown={(e) => e.key === "Escape" && (pictureMenu = false)} />

<div class="composer">
  {#if chat.picture?.phase === "prompt" || chat.picture?.phase === "render"}
    <div class="drawing" role="status">
      <span class="spinner" aria-hidden="true"></span>
      <span>{chat.picture.phase === "prompt" ? "Describing the picture…" : "Drawing the picture…"}</span>
      <button class="ghost" onclick={() => chat.stopPicture()}>Stop</button>
    </div>
  {/if}

  {#if chat.awaitingSpeaker && chat.session}
    <div class="speakers" role="group" aria-label="Who replies?">
      <span class="muted">Who replies?</span>
      {#each chat.session.characters as character (character.id)}
        <button onclick={() => chat.pickSpeaker(character.id)}>{character.name}</button>
      {/each}
    </div>
  {:else if chat.isGroup && !chat.busy && chat.lastTurn && chat.session}
    <!-- Any member can add another reply, as often as you like, even if they already answered. -->
    <div class="speakers" role="group" aria-label="Add a reply from">
      <span class="muted">Add a reply from</span>
      {#each chat.session.characters as character (character.id)}
        <button class="ghost" onclick={() => chat.pickSpeaker(character.id)}>{character.name}</button>
      {/each}
    </div>
  {/if}

  {#if chat.error}
    <div class="error" role="alert">
      <span>{chat.error}</span>
      <button class="ghost" onclick={() => (chat.error = "")} aria-label="Dismiss">✕</button>
    </div>
  {/if}

  {#if chat.pendingAttachments.length > 0}
    <div class="pending">
      {#each chat.pendingAttachments as path (path)}
        <div class="chip">
          <img src={fileUrl(path)} alt="" />
          <button
            class="ghost"
            onclick={() => (chat.pendingAttachments = chat.pendingAttachments.filter((p) => p !== path))}
            aria-label="Remove picture">✕</button
          >
        </div>
      {/each}
    </div>
  {/if}

  <div class="row">
    <textarea
      bind:value={chat.input}
      {onkeydown}
      rows="1"
      placeholder={touch ? "Message…" : "Message…  (Enter to send, Shift+Enter for a new line)"}
      aria-label="Message"
    ></textarea>

    <button onclick={attach} title="Attach a picture to send with your next message" aria-label="Attach picture">📎</button>

    {#if showSearch}
      <label class="check" title="Let Gemini ground its answer with Google Search">
        <input type="checkbox" checked={search} onchange={(e) => chat.setSearch(e.currentTarget.checked)} />
        Search
      </label>
    {/if}

    {#if chat.isRoleplay && chat.session}
      <div class="menuwrap" bind:this={menuWrap}>
        <button
          onclick={togglePictureMenu}
          disabled={pictureBusy}
          aria-haspopup="menu"
          aria-expanded={pictureMenu}
          title="Have the AI describe a picture and draw it with your Stable Diffusion server"
        >
          Picture
        </button>
        {#if pictureMenu}
          <div class="menu" class:opens-right={menuOpensRight} role="menu" bind:this={menuEl}>
            {#each chat.session.characters as character (character.id)}
              <button role="menuitem" onclick={() => draw("character", character.id)}>{character.name}, full body</button>
              <button role="menuitem" onclick={() => draw("face", character.id)}>{character.name}'s face</button>
            {/each}
            <button role="menuitem" onclick={() => draw("persona")}>{userName} (you)</button>
            <button role="menuitem" onclick={() => draw("scene")}>The current scene</button>
            <button role="menuitem" onclick={() => draw("lastMessage")} disabled={!chat.lastTurn}>The last message</button>
            <a role="menuitem" href="/settings#pictures">Picture settings…</a>
          </div>
        {/if}
      </div>
    {/if}

    {#if chat.isRoleplay && !chat.busy}
      <button onclick={() => chat.impersonate()} title="Ask the AI to draft your next message, for you to review and send">
        Impersonate
      </button>
      <button
        onclick={() => chat.continueLast()}
        disabled={!chat.canContinue}
        title="Extend the last reply instead of starting a new one"
      >
        Continue
      </button>
    {/if}

    {#if chat.busy}
      <button onclick={() => chat.stop()} title="Stop generating and keep what has arrived so far">Stop</button>
    {:else}
      <button class="primary" disabled={!chat.canSend} onclick={() => chat.send()}>Send</button>
    {/if}
  </div>
</div>

<style>
  .composer {
    max-width: 860px;
    width: 100%;
    margin: 0 auto;
    padding: 8px 16px 16px;
    container-type: inline-size;
  }


  .row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }

  textarea {
    flex: 1;
    min-height: 40px;
    max-height: 200px;
    resize: none;
    padding: 9px 12px;
    line-height: 1.4;
    /* Grows with its content up to max-height, then scrolls. */
    field-sizing: content;
  }

  .row button {
    height: 40px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 40px;
    color: var(--muted);
    font-size: 13px;
    white-space: nowrap;
  }

  .menuwrap {
    position: relative;
  }

  .menu {
    position: absolute;
    bottom: calc(100% + 6px);
    right: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    min-width: 220px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel);
    box-shadow: var(--shadow);
  }

  .menu.opens-right {
    right: auto;
    left: 0;
  }

  /* Never wider than the screen; long names wrap instead. */
  .menu {
    max-width: calc(100vw - 16px);
  }

  .menu button,
  .menu a {
    height: auto;
    padding: 7px 10px;
    border: none;
    background: none;
    text-align: left;
    font-size: 13px;
    color: var(--text);
    white-space: nowrap;
    border-radius: 4px;
    text-decoration: none;
  }

  .menu button:hover:not(:disabled),
  .menu a:hover {
    background: var(--panel-3);
  }

  .menu a {
    margin-top: 2px;
    border-top: 1px solid var(--border);
    border-radius: 0 0 4px 4px;
    color: var(--muted);
  }

  .drawing {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
    font-size: 13px;
    color: var(--muted);
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .error {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 8px;
    background: var(--danger-bg);
    color: var(--danger);
    border-radius: var(--radius-sm);
    padding: 8px 12px;
    margin-bottom: 8px;
    white-space: pre-wrap;
  }

  .speakers {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 8px;
  }

  .pending {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }

  .chip {
    position: relative;
    width: 72px;
    height: 72px;
    border-radius: var(--radius-sm);
    overflow: hidden;
    border: 1px solid var(--border);
  }

  .chip img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .chip button {
    position: absolute;
    top: 2px;
    right: 2px;
    padding: 0 5px;
    background: rgba(0, 0, 0, 0.6);
    color: white;
    line-height: 1.3;
  }

  /* A narrow chat column (story panel open, small window): the message box gets the full width
     and the buttons wrap onto the line below it. Last, so it overrides the rules above. */
  @container (max-width: 620px) {
    .row {
      flex-wrap: wrap;
      justify-content: flex-end;
    }

    textarea {
      flex-basis: 100%;
    }
  }
</style>

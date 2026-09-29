<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, errorText, inTauri } from "$lib/api";
  import { chat } from "$lib/chat.svelte";
  import { fileUrl } from "$lib/media";

  /** Enter sends; Shift+Enter inserts a newline (the textarea's default). */
  function onkeydown(event: KeyboardEvent) {
    if (event.key !== "Enter" || event.shiftKey || event.isComposing) return;
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

  const search = $derived(chat.session?.conversation.enableGoogleSearch ?? false);
  const showSearch = $derived(chat.session?.conversation.providerId === "gemini" && !chat.isRoleplay);
</script>

<div class="composer">
  {#if chat.awaitingSpeaker && chat.session}
    <div class="speakers" role="group" aria-label="Who replies?">
      <span class="muted">Who replies?</span>
      {#each chat.session.characters as character (character.id)}
        <button onclick={() => chat.pickSpeaker(character.id)}>{character.name}</button>
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
      placeholder="Message…  (Enter to send, Shift+Enter for a new line)"
      aria-label="Message"
    ></textarea>

    <button onclick={attach} title="Attach a picture to send with your next message" aria-label="Attach picture">📎</button>

    {#if showSearch}
      <label class="check" title="Let Gemini ground its answer with Google Search">
        <input type="checkbox" checked={search} onchange={(e) => chat.setSearch(e.currentTarget.checked)} />
        Search
      </label>
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
</style>

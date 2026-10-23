<script lang="ts">
  import { chat } from "#lib/chat.svelte.js";

  let dialog = $state<HTMLDialogElement>();
  let pattern = $state("");
  const approval = $derived(chat.approval);

  // Open while a command waits for an answer; close as soon as it has one (or generation ended).
  $effect(() => {
    if (approval) {
      pattern = approval.command;
      if (dialog && !dialog.open) dialog.showModal();
    } else if (dialog?.open) {
      dialog.close();
    }
  });

  /** Escape counts as "Deny" - never as a silent yes. */
  function oncancel(event: Event) {
    event.preventDefault();
    void chat.answerApproval(false);
  }
</script>

<dialog bind:this={dialog} {oncancel} aria-labelledby="tool-approval-title">
  {#if approval}
    {#if approval.elevate}
      <h2 id="tool-approval-title">Run as Administrator?</h2>
      <p>The assistant wants to run this command as Administrator:</p>
    {:else}
      <h2 id="tool-approval-title">Run PowerShell command?</h2>
      <p>The assistant wants to run this PowerShell command:</p>
    {/if}

    <pre class="command">{approval.command}</pre>

    {#if approval.elevate}
      <p class="muted">Windows will also show its own UAC prompt after you approve. This is asked every time, whatever your other settings.</p>
    {:else}
      <label class="field">
        Pattern to remember (edit to generalize, e.g. with a trailing *)
        <input bind:value={pattern} />
      </label>
    {/if}

    <div class="actions">
      {#if approval.elevate}
        <button class="danger-action" onclick={() => chat.answerApproval(true)}>Run as Administrator</button>
      {:else}
        <button class="primary" onclick={() => chat.answerApproval(true)}>Run once</button>
        <button onclick={() => chat.answerApproval(true, pattern.trim() || approval.command)}>Always allow</button>
      {/if}
      <button onclick={() => chat.answerApproval(false)}>Deny</button>
    </div>
  {/if}
</dialog>

<style>
  dialog {
    max-width: min(560px, calc(100vw - 32px));
    width: 100%;
    padding: 20px 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--panel);
    color: var(--text);
  }

  dialog::backdrop {
    background: rgba(0, 0, 0, 0.55);
  }

  h2 {
    font-size: 17px;
    margin: 0 0 8px;
  }

  p {
    margin: 6px 0;
  }

  .command {
    margin: 8px 0 12px;
    padding: 10px 12px;
    max-height: 200px;
    overflow: auto;
    background: var(--panel-3);
    border-radius: var(--radius-sm);
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 13px;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }

  .danger-action {
    background: var(--danger);
    border-color: var(--danger);
    color: white;
  }
</style>

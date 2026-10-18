// The chat page's state: one open conversation, its messages, and the operations on them. A single
// shared instance, so the state survives navigating between pages.

import { api, errorText } from "./api";
import type {
  ChatEvent,
  ChatMessage,
  ChatSession,
  Character,
  ContextStatus,
  Persona,
  StoryState,
  ToolApproval,
  UiMessage,
  Uuid,
  WorldState,
} from "./types";

function toUi(message: ChatMessage, streaming = false): UiMessage {
  return { ...message, streaming, editing: false, editBuffer: "" };
}

export interface Speaker {
  name: string;
  /** Where the avatar (or the current expression) image lives, if there is one. */
  avatarPath: string | null;
}

class ChatStore {
  session = $state<ChatSession | null>(null);
  messages = $state<UiMessage[]>([]);
  story = $state<StoryState | null>(null);
  loading = $state(false);
  busy = $state(false);
  error = $state("");
  input = $state("");
  pendingAttachments = $state<string[]>([]);
  /** A PowerShell command the assistant is waiting for permission to run. */
  approval = $state<ToolApproval | null>(null);
  /** How full the model's context window is; null until loaded. */
  context = $state<ContextStatus | null>(null);
  /** Older messages are being summarized (by hand, or automatically before a reply). */
  compacting = $state(false);

  get conversationId(): Uuid | null {
    return this.session?.conversation.id ?? null;
  }

  get isRoleplay(): boolean {
    return this.session?.conversation.mode === "roleplay";
  }

  get isGroup(): boolean {
    return !!this.session?.conversation.groupChatId;
  }

  /** In a group whose last message is the user's own: nobody has answered yet, so offer the members. */
  get awaitingSpeaker(): boolean {
    return this.isGroup && !this.busy && this.messages.at(-1)?.role === "user";
  }

  /** Continue extends the last reply, so it needs one (and nothing already running). */
  get canContinue(): boolean {
    return !!this.session && !this.busy && this.messages.at(-1)?.role === "character";
  }

  get canSend(): boolean {
    if (!this.session || this.busy) return false;
    const last = this.messages.at(-1);
    return this.input.trim().length > 0 || this.pendingAttachments.length > 0 || last?.role === "user";
  }

  // ---- loading ----

  /** Makes `session` the open conversation. */
  load(session: ChatSession) {
    this.session = session;
    this.messages = session.messages.map((m) => toUi(m));
    this.error = "";
    this.input = "";
    this.pendingAttachments = [];
    this.busy = false;
    this.approval = null;
    this.story = null;
    this.context = null;
    this.compacting = false;
    void this.refreshStory();
    void this.refreshContext();
  }

  private async open(action: Promise<ChatSession | null>): Promise<boolean> {
    this.loading = true;
    try {
      const session = await action;
      if (!session) return false;
      this.load(session);
      return true;
    } catch (e) {
      this.error = errorText(e);
      return false;
    } finally {
      this.loading = false;
    }
  }

  openAssistant(forceNew = false) {
    return this.open(api.openAssistant(forceNew));
  }

  openCharacter(characterId: Uuid, forceNew = false) {
    return this.open(api.openCharacter(characterId, forceNew));
  }

  openGroup(groupId: Uuid, forceNew = false) {
    return this.open(api.openGroup(groupId, forceNew));
  }

  openLastRoleplay() {
    return this.open(api.openLastRoleplay());
  }

  resume(conversationId: Uuid) {
    return this.open(api.resumeConversation(conversationId));
  }

  newChat() {
    if (!this.conversationId || this.busy) return Promise.resolve(false);
    return this.open(api.newChat(this.conversationId));
  }

  async refreshStory() {
    const id = this.conversationId;
    if (!id || !this.isRoleplay) return;
    try {
      const story = await api.storyState(id);
      if (this.conversationId === id) this.story = story;
    } catch {
      // The side panel is a nicety; never let it break the chat.
    }
  }

  async refreshContext() {
    const id = this.conversationId;
    if (!id) return;
    try {
      const context = await api.contextStatus(id);
      if (this.conversationId === id) this.context = context;
    } catch {
      // Only the meter depends on it.
    }
  }

  /** Summarizes all but the newest messages into the chat's running summary. */
  async compact() {
    const id = this.conversationId;
    if (!id || this.busy) return;
    this.error = "";
    this.busy = this.compacting = true;
    try {
      const context = await api.compactConversation(id);
      if (this.conversationId === id) this.context = context;
    } catch (e) {
      if (this.conversationId === id) this.error = errorText(e);
    } finally {
      if (this.conversationId === id) this.busy = this.compacting = false;
    }
  }

  /** Undoes compaction: the model sees every message in full again. */
  async clearSummary() {
    const id = this.conversationId;
    if (!id || this.busy) return;
    try {
      const context = await api.clearConversationSummary(id);
      if (this.conversationId === id) this.context = context;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  /** Index of the last message the summary covers (the model no longer sees it in full), or -1. */
  get lastSummarizedIndex(): number {
    const summary = this.context?.summary;
    if (!summary) return -1;
    const cut = this.messages.findIndex((m) => m.id === summary.coversThroughMessageId);
    if (cut >= 0) return cut;
    // That message was deleted; its timestamp still marks the spot.
    const cutoff = Date.parse(summary.coversThroughTimestamp);
    return this.messages.findLastIndex((m) => Date.parse(m.timestamp) <= cutoff);
  }

  // ---- lookups ----

  private characterFor(message: ChatMessage): Character | undefined {
    const cast = this.session?.characters ?? [];
    return cast.find((c) => c.id === message.speakerCharacterId) ?? (cast.length === 1 ? cast[0] : undefined);
  }

  speaker(message: ChatMessage): Speaker {
    const persona: Persona | null = this.session?.persona ?? null;
    if (message.role === "user") return { name: persona?.name || "You", avatarPath: persona?.avatarPath ?? null };
    if (message.role === "tool") return { name: "PowerShell", avatarPath: null };
    if (message.role === "system") return { name: "System", avatarPath: null };

    const character = this.characterFor(message);
    if (!character) return { name: "Assistant", avatarPath: null };
    // A character with an expression pack shows the portrait matching the reply's detected emotion.
    const expression = message.detectedEmotion
      ? character.expressions.find((e) => e.emotion === message.detectedEmotion)
      : undefined;
    return { name: character.name, avatarPath: expression?.imagePath ?? character.avatarPath };
  }

  // ---- events from the engine ----

  private find(id: Uuid): UiMessage | undefined {
    return this.messages.find((m) => m.id === id);
  }

  private replace(message: ChatMessage) {
    const i = this.messages.findIndex((m) => m.id === message.id);
    if (i >= 0) this.messages[i] = toUi(message);
    else this.messages.push(toUi(message));
  }

  /** Applies one engine event. Events from a conversation that is no longer open are ignored. */
  private handle(conversationId: Uuid, event: ChatEvent) {
    if (this.conversationId !== conversationId) return;

    switch (event.type) {
      case "messageAdded":
        // An empty message that isn't the user's is about to stream in.
        this.messages.push(toUi(event.message, event.message.role !== "user" && event.message.text === ""));
        break;
      case "regenerating": {
        const m = this.find(event.messageId);
        if (m) {
          m.streaming = true;
          m.text = "";
          m.reasoningText = null;
        }
        break;
      }
      case "continuing": {
        const m = this.find(event.messageId);
        if (m) m.streaming = true; // keeps its text; the deltas that follow are appended
        break;
      }
      case "delta": {
        const m = this.find(event.messageId);
        if (m) {
          m.text += event.text;
          if (event.reasoning) m.reasoningText = (m.reasoningText ?? "") + event.reasoning;
        }
        break;
      }
      case "messageUpdated":
        this.replace(event.message);
        break;
      case "messageRemoved":
        this.messages = this.messages.filter((m) => m.id !== event.messageId);
        break;
      case "worldStateChanged":
        if (this.session) this.session.conversation.worldState = event.worldState as WorldState;
        break;
      case "storyChanged":
        void this.refreshStory();
        break;
      case "compacting":
        this.compacting = true;
        break;
      case "compacted":
        this.compacting = false;
        void this.refreshContext();
        break;
      case "titleChanged":
        if (this.session) this.session.conversation.title = event.title;
        break;
      case "error":
        this.error = event.message;
        break;
      case "toolApproval":
        this.approval = { requestId: event.requestId, command: event.command, elevate: event.elevate };
        break;
      case "finished":
        this.approval = null;
        for (const m of this.messages) m.streaming = false;
        this.busy = false;
        this.compacting = false;
        void this.refreshStory();
        void this.refreshContext();
        break;
    }
  }

  // ---- sending and generating ----

  async send() {
    const id = this.conversationId;
    if (!id || !this.canSend) return;
    const text = this.input.trim();
    const attachments = [...this.pendingAttachments];
    this.input = "";
    this.pendingAttachments = [];
    this.error = "";
    this.busy = true;
    try {
      await api.sendMessage(id, text, attachments, (e) => this.handle(id, e));
    } catch (e) {
      if (this.conversationId === id) {
        // The engine has already reported its own errors as events; this covers a failed call.
        this.error ||= errorText(e);
        this.busy = false;
      }
    }
  }

  async pickSpeaker(characterId: Uuid) {
    const id = this.conversationId;
    if (!id || this.busy) return;
    this.error = "";
    this.busy = true;
    try {
      await api.pickSpeaker(id, characterId, (e) => this.handle(id, e));
    } catch (e) {
      if (this.conversationId === id) {
        this.error ||= errorText(e);
        this.busy = false;
      }
    }
  }

  /** Answers the open tool-approval question (see `approval`). */
  async answerApproval(allow: boolean, alwaysPattern: string | null = null) {
    const open = this.approval;
    if (!open) return;
    this.approval = null;
    try {
      await api.resolveToolApproval(open.requestId, { allow, alwaysPattern });
    } catch (e) {
      this.error = errorText(e);
    }
  }

  /** Extends the last reply in place. */
  async continueLast() {
    const id = this.conversationId;
    if (!id || !this.canContinue) return;
    this.error = "";
    this.busy = true;
    try {
      await api.continueMessage(id, (e) => this.handle(id, e));
    } catch (e) {
      if (this.conversationId === id) {
        this.error ||= errorText(e);
        this.busy = false;
      }
    }
  }

  /** Has the model draft the user's next message into the input box, to be edited and sent by hand. */
  async impersonate() {
    const id = this.conversationId;
    if (!id || this.busy) return;
    this.error = "";
    this.busy = true;
    try {
      const draft = await api.impersonate(id);
      if (draft && this.conversationId === id) this.input = draft;
    } catch (e) {
      if (this.conversationId === id) this.error = errorText(e);
    } finally {
      if (this.conversationId === id) this.busy = false;
    }
  }

  async stop() {
    if (this.conversationId) await api.stopGeneration(this.conversationId);
  }

  async swipeNext(message: UiMessage) {
    const id = this.conversationId;
    if (!id || this.busy) return;
    this.error = "";
    this.busy = true;
    try {
      await api.swipeNext(id, message.id, (e) => this.handle(id, e));
    } catch (e) {
      if (this.conversationId === id) {
        this.error ||= errorText(e);
        this.busy = false;
      }
    }
  }

  async swipePrevious(message: UiMessage) {
    if (this.busy) return;
    try {
      this.replace(await api.swipePrevious(message.id));
    } catch (e) {
      this.error = errorText(e);
    }
  }

  // ---- editing ----

  beginEdit(message: UiMessage) {
    message.editBuffer = message.text;
    message.editing = true;
  }

  cancelEdit(message: UiMessage) {
    message.editing = false;
  }

  async commitEdit(message: UiMessage) {
    try {
      this.replace(await api.editMessage(message.id, message.editBuffer));
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async remove(message: UiMessage) {
    try {
      await api.deleteMessage(message.id);
      this.messages = this.messages.filter((m) => m.id !== message.id);
      void this.refreshContext();
    } catch (e) {
      this.error = errorText(e);
    }
  }

  // ---- notebook ----

  async addNote(characterId: Uuid, text: string): Promise<boolean> {
    const id = this.conversationId;
    if (!id || !text.trim()) return false;
    try {
      const note = await api.addNote(id, characterId, text);
      this.story?.notebook.push(note);
      return true;
    } catch (e) {
      this.error = errorText(e);
      return false;
    }
  }

  /** Saves an edited note; an emptied one is put back the way it was (deleting is its own button). */
  async saveNote(note: { id: Uuid; content: string }, text: string) {
    if (!text.trim() || text.trim() === note.content) return;
    try {
      note.content = (await api.updateNote(note.id, text)).content;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async deleteNote(noteId: Uuid) {
    try {
      await api.deleteNote(noteId);
      if (this.story) this.story.notebook = this.story.notebook.filter((n) => n.id !== noteId);
    } catch (e) {
      this.error = errorText(e);
    }
  }

  // ---- per-conversation settings ----

  private async updateConversation(change: Promise<ChatSession["conversation"]>) {
    try {
      const updated = await change;
      if (this.session && this.session.conversation.id === updated.id) this.session.conversation = updated;
      // A different model or preset can mean a different window.
      void this.refreshContext();
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async setModel(providerId: string, model: string) {
    const id = this.conversationId;
    if (!id) return;
    if (this.session && this.session.conversation.providerId !== providerId) {
      // A different provider has a different model list; load its cached one.
      this.session.models = await api.listModels(providerId).catch(() => []);
      if (!model) model = this.session.models[0] ?? "";
    }
    await this.updateConversation(api.setConversationModel(id, providerId, model));
  }

  async setPreset(presetId: Uuid) {
    const id = this.conversationId;
    if (id) await this.updateConversation(api.setConversationPreset(id, presetId));
  }

  async setPersona(personaId: Uuid | null) {
    const id = this.conversationId;
    if (!id || !this.session) return;
    await this.updateConversation(api.setConversationPersona(id, personaId));
    this.session.persona = this.session.personas.find((p) => p.id === personaId) ?? null;
  }

  async setAssistantPrompt(text: string) {
    const id = this.conversationId;
    if (id) await this.updateConversation(api.setAssistantPrompt(id, text));
  }

  /** Edits the in-story clock/weather. Ignored mid-reply (the reply is about to advance the clock itself). */
  async updateWorld(patch: Partial<WorldState>) {
    const id = this.conversationId;
    const current = this.session?.conversation.worldState;
    if (!id || !current || this.busy) return;
    try {
      const saved = await api.setWorldState(id, { ...current, ...patch });
      if (this.conversationId === id && this.session) this.session.conversation.worldState = saved;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async setSearch(enabled: boolean | null) {
    const id = this.conversationId;
    if (id) await this.updateConversation(api.setSearchOverride(id, enabled));
  }

  async refreshModels() {
    const providerId = this.session?.conversation.providerId;
    if (!providerId || !this.session) return;
    try {
      const models = await api.refreshModels(providerId);
      if (this.session.conversation.providerId === providerId) {
        this.session.models = models;
        // A brand-new chat has no model yet; pick the first one on offer so it can be used at once.
        if (!this.session.conversation.model && models.length > 0) {
          await this.setModel(providerId, models[0]);
        }
      }
    } catch (e) {
      this.error = errorText(e);
    }
  }
}

export const chat = new ChatStore();

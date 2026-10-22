// The chat page's state: one open conversation, its messages, and the operations on them. A single
// shared instance, so the state survives navigating between pages.

import { goto } from "$app/navigation";
import { api, errorText } from "./api";
import type {
  ChatEvent,
  ChatMessage,
  ChatSession,
  Character,
  ContextStatus,
  ImageMode,
  ImagePromptDraft,
  Persona,
  ToolApproval,
  UiMessage,
  Uuid,
  StoryCharacter,
  WorldState,
} from "./types";

function toUi(message: ChatMessage, streaming = false): UiMessage {
  return { ...message, streaming, editing: false, editBuffer: "" };
}

/** Where a picture is in its life: the model is writing its prompt, the prompt waits for review,
 * or the image server is rendering it. */
export interface PictureJob {
  phase: "prompt" | "review" | "render";
  draft: ImagePromptDraft | null;
}

/** The draft a picture message was drawn from (kept as its raw prompt), for drawing it again. */
export function pictureDraft(message: ChatMessage): ImagePromptDraft | null {
  if (message.role !== "image") return null;
  try {
    const draft = JSON.parse(message.rawPromptText ?? "") as ImagePromptDraft;
    return { ...draft, prompt: message.text };
  } catch {
    return null;
  }
}

export interface Speaker {
  name: string;
  /** Where the avatar (or the current expression) image lives, if there is one. */
  avatarPath: string | null;
}

class ChatStore {
  session = $state<ChatSession | null>(null);
  messages = $state<UiMessage[]>([]);
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
  /** Side characters this story has introduced (roleplay only). */
  cast = $state<StoryCharacter[]>([]);
  /** A scan of the recent story for new people is running. */
  scanning = $state(false);
  /** The picture being made for this chat, if any. */
  picture = $state<PictureJob | null>(null);
  /** Photos characters sent while a picture was already under way in their chat; drawn next, in order. */
  private photoQueue = new Map<Uuid, ImagePromptDraft[]>();
  /** Pictures being rendered, by conversation (open or not), so a chat opened again shows its render. */
  private rendering = new Map<Uuid, ImagePromptDraft>();
  /** Conversations with a streaming call still running (open or not). */
  private inFlight = new Set<Uuid>();
  /** A member took the user aside during the running reply: their private talk opens once it ends. */
  private takenAside: { breakoutId: Uuid; characterId: Uuid } | null = null;
  /** What a still-generating conversation looked like when it was left, kept up to date from its
   * events until it is opened again or finishes. */
  private parked = new Map<Uuid, { messages: UiMessage[]; approval: ToolApproval | null; error: string }>();

  get conversationId(): Uuid | null {
    return this.session?.conversation.id ?? null;
  }

  get isRoleplay(): boolean {
    return this.session?.conversation.mode === "roleplay";
  }

  get isGroup(): boolean {
    return !!this.session?.conversation.groupChatId;
  }

  /** A private talk with one member, inside a group conversation. */
  get isBreakout(): boolean {
    return !!this.session?.conversation.parentConversationId;
  }

  /** The newest message of the exchange. Pictures sit in the chat but aren't part of the
   * conversation, and a note like "stepped away to talk in private" doesn't end it either, so
   * neither counts as "the last reply". */
  get lastTurn(): UiMessage | undefined {
    return this.messages.findLast((m) => m.role !== "image" && m.role !== "system");
  }

  /** In a group whose last message is the user's own: nobody has answered yet, so offer the members. */
  get awaitingSpeaker(): boolean {
    return this.isGroup && !this.busy && this.lastTurn?.role === "user";
  }

  /** In a group: the members who haven't answered the user's latest message yet (any of them can
   * still add a reply to it). */
  get notYetAnswered(): Character[] {
    if (!this.isGroup || !this.session) return [];
    const since = this.messages.findLastIndex((m) => m.role === "user");
    const answered = new Set(
      this.messages.slice(since + 1).filter((m) => m.role === "character").map((m) => m.speakerCharacterId),
    );
    return this.session.characters.filter((c) => !answered.has(c.id));
  }

  /** Continue extends the last reply, so it needs one (and nothing already running). */
  get canContinue(): boolean {
    return !!this.session && !this.busy && this.lastTurn?.role === "character";
  }

  get canSend(): boolean {
    if (!this.session || this.busy) return false;
    return this.input.trim().length > 0 || this.pendingAttachments.length > 0 || this.lastTurn?.role === "user";
  }

  // ---- loading ----

  /** Makes `session` the open conversation. */
  load(session: ChatSession) {
    // A reply still streaming in the chat being left keeps going; its events are collected in `parked`.
    const leaving = this.conversationId;
    if (leaving && leaving !== session.conversation.id && this.inFlight.has(leaving)) {
      this.parked.set(leaving, {
        messages: this.messages,
        approval: this.approval,
        error: this.error,
      });
    }
    const resumed = this.parked.get(session.conversation.id);
    this.parked.delete(session.conversation.id);

    this.session = session;
    this.takenAside = null;
    this.messages = resumed ? resumed.messages : session.messages.map((m) => toUi(m));
    this.error = resumed?.error ?? "";
    this.input = "";
    this.pendingAttachments = [];
    this.busy = !!resumed;
    this.approval = resumed?.approval ?? null;
    this.context = null;
    this.compacting = false;
    const rendering = this.rendering.get(session.conversation.id);
    this.picture = rendering ? { phase: "render", draft: rendering } : null;
    this.nextPhoto(session.conversation.id);
    this.cast = [];
    this.scanning = false;
    void this.refreshCast();
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

  /** Goes into a private talk with a member of the open group conversation (or of the group a
   * private talk belongs to). */
  async openBreakout(characterId: Uuid) {
    const conversation = this.session?.conversation;
    const groupConversation = conversation?.parentConversationId ?? (conversation?.groupChatId ? conversation.id : null);
    if (!groupConversation) return false;
    if (!(await this.open(api.openBreakout(groupConversation, characterId)))) return false;
    await goto(`/chat/${this.conversationId}`);
    return true;
  }

  resume(conversationId: Uuid) {
    return this.open(api.resumeConversation(conversationId));
  }

  newChat() {
    if (!this.conversationId || this.busy) return Promise.resolve(false);
    return this.open(api.newChat(this.conversationId));
  }

  async refreshCast() {
    const id = this.conversationId;
    if (!id || !this.isRoleplay) return;
    try {
      const cast = await api.listStoryCast(id);
      if (this.conversationId === id) this.cast = cast;
    } catch {
      /* the sidebar just stays empty */
    }
  }

  /** Reads the latest stretch of the story for people it has introduced. */
  async scanCast() {
    const id = this.conversationId;
    if (!id || this.scanning) return;
    this.scanning = true;
    try {
      const cast = await api.scanStoryCast(id);
      if (this.conversationId === id) this.cast = cast;
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.scanning = false;
    }
  }

  async saveCastMember(character: StoryCharacter) {
    try {
      const cast = await api.saveStoryCharacter($state.snapshot(character));
      if (this.conversationId === character.conversationId) this.cast = cast;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async removeCastMember(id: string) {
    const conversationId = this.conversationId;
    if (!conversationId) return;
    try {
      const cast = await api.deleteStoryCharacter(conversationId, id);
      if (this.conversationId === conversationId) this.cast = cast;
    } catch (e) {
      this.error = errorText(e);
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

  /** The expression pack a character shows in this chat: their outfit's, or the default one. */
  expressionsFor(character: Character) {
    const outfitId = this.session?.conversation.worldState?.outfits?.[character.id];
    const outfit = character.outfits.find((o) => o.id === outfitId);
    return outfit?.expressions.some((e) => e.imagePath) ? outfit.expressions : character.expressions;
  }

  speaker(message: ChatMessage): Speaker {
    const persona: Persona | null = this.session?.persona ?? null;
    if (message.role === "user") return { name: persona?.name || "You", avatarPath: persona?.avatarPath ?? null };
    if (message.role === "tool") return { name: "PowerShell", avatarPath: null };
    if (message.role === "system") return { name: "System", avatarPath: null };
    if (message.role === "image" && !message.speakerCharacterId) return { name: "Picture", avatarPath: null };

    const character = this.characterFor(message);
    if (!character) return { name: "Assistant", avatarPath: null };
    // A character with an expression pack shows the portrait matching the reply's detected emotion,
    // from the outfit this story has them in when that outfit has its own pictures.
    const expression = message.detectedEmotion
      ? this.expressionsFor(character).find((e) => e.emotion === message.detectedEmotion)
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

  /** Applies the events that matter to a conversation that was left mid-reply. */
  private handleParked(conversationId: Uuid, event: ChatEvent) {
    const parked = this.parked.get(conversationId);
    if (!parked) return;
    const find = (id: Uuid) => parked.messages.find((m) => m.id === id);
    switch (event.type) {
      case "messageAdded":
        parked.messages.push(toUi(event.message, event.message.role !== "user" && event.message.text === ""));
        break;
      case "regenerating": {
        const m = find(event.messageId);
        if (m) {
          m.streaming = true;
          m.text = "";
          m.reasoningText = null;
        }
        break;
      }
      case "continuing": {
        const m = find(event.messageId);
        if (m) m.streaming = true;
        break;
      }
      case "delta": {
        const m = find(event.messageId);
        if (m) {
          m.text += event.text;
          if (event.reasoning) m.reasoningText = (m.reasoningText ?? "") + event.reasoning;
        }
        break;
      }
      case "messageUpdated": {
        const i = parked.messages.findIndex((m) => m.id === event.message.id);
        if (i >= 0) parked.messages[i] = toUi(event.message);
        else parked.messages.push(toUi(event.message));
        break;
      }
      case "messageRemoved":
        parked.messages = parked.messages.filter((m) => m.id !== event.messageId);
        break;
      case "toolApproval":
        parked.approval = { requestId: event.requestId, command: event.command, elevate: event.elevate };
        break;
      case "error":
        parked.error = event.message;
        break;
      case "finished":
        // Nothing left to show: opening the chat again loads the finished reply from the database.
        this.parked.delete(conversationId);
        break;
    }
  }

  /** Applies one engine event. A conversation that is no longer open only keeps its parked state
   * and its photos. */
  private handle(conversationId: Uuid, event: ChatEvent) {
    if (this.conversationId !== conversationId) {
      if (event.type === "photoSent") void this.drawSentPhoto(conversationId, event.characterId, event.description);
      else this.handleParked(conversationId, event);
      return;
    }

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
      case "castChanged":
        if (event.conversationId === this.conversationId) this.cast = event.cast;
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
      case "photoSent":
        void this.drawSentPhoto(conversationId, event.characterId, event.description);
        break;
      case "breakoutStarted":
        this.takenAside = { breakoutId: event.breakoutId, characterId: event.characterId };
        break;
      case "error":
        this.error = event.message;
        break;
      case "toolApproval":
        this.approval = { requestId: event.requestId, command: event.command, elevate: event.elevate };
        break;
      case "finished": {
        this.approval = null;
        for (const m of this.messages) m.streaming = false;
        this.busy = false;
        this.compacting = false;
        void this.refreshContext();
        const aside = this.takenAside;
        this.takenAside = null;
        if (aside) void this.followAside(aside.breakoutId, aside.characterId);
        break;
      }
    }
  }

  /** A member took the user aside: opens their private talk, where they speak first. */
  private async followAside(breakoutId: Uuid, characterId: Uuid) {
    if (!(await this.resume(breakoutId))) return;
    await goto(`/chat/${breakoutId}`);
    if (this.conversationId === breakoutId) await this.pickSpeaker(characterId);
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
    await this.stream(id, (onEvent) => api.sendMessage(id, text, attachments, onEvent));
  }

  /** Runs one streaming call for conversation `id`, which stays "generating" even if the user
   * opens another chat meanwhile. */
  private async stream(id: Uuid, start: (onEvent: (e: ChatEvent) => void) => Promise<void>) {
    this.inFlight.add(id);
    try {
      await start((e) => this.handle(id, e));
    } catch (e) {
      // The engine has already reported its own errors as events; this covers a failed call.
      if (this.conversationId === id) {
        this.error ||= errorText(e);
        this.busy = false;
      }
    } finally {
      this.inFlight.delete(id);
      this.parked.delete(id);
    }
  }

  async pickSpeaker(characterId: Uuid) {
    const id = this.conversationId;
    if (!id || this.busy) return;
    this.error = "";
    this.busy = true;
    await this.stream(id, (onEvent) => api.pickSpeaker(id, characterId, onEvent));
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
    await this.stream(id, (onEvent) => api.continueMessage(id, onEvent));
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
    await this.stream(id, (onEvent) => api.swipeNext(id, message.id, onEvent));
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
      // A picture's file goes with it.
      const rewound = await (message.role === "image" ? api.deletePicture(message.id) : api.deleteMessage(message.id));
      this.messages = this.messages.filter((m) => m.id !== message.id);
      if (rewound && this.session) this.session.conversation.worldState = rewound as WorldState;
      void this.refreshContext();
    } catch (e) {
      this.error = errorText(e);
    }
  }

  // ---- pictures ----

  /** Has the chat model describe a picture, then renders it - after review if Settings ask for it. */
  async drawPicture(mode: ImageMode, characterId: Uuid | null = null) {
    const id = this.conversationId;
    if (!id || this.busy || this.picture) return;
    this.error = "";
    // Writing the prompt uses the chat model, so it occupies the chat like a reply would.
    this.busy = true;
    this.picture = { phase: "prompt", draft: null };
    let draft: ImagePromptDraft | null = null;
    let review = true;
    try {
      [draft, review] = await Promise.all([
        api.imagePrompt(id, mode, characterId),
        api.getImageSettings().then((s) => s.reviewPrompt),
      ]);
    } catch (e) {
      if (this.conversationId === id) this.error = errorText(e);
    } finally {
      if (this.conversationId === id) this.busy = false;
    }
    if (this.conversationId !== id) return;
    if (!draft) {
      this.picture = null; // stopped or failed
      this.nextPhoto(id);
      return;
    }
    if (review) this.picture = { phase: "review", draft };
    else await this.renderPicture(draft);
  }

  /** Opens a picture's prompt for editing and drawing again. */
  redrawPicture(message: ChatMessage) {
    const draft = pictureDraft(message);
    if (!draft || this.picture) return;
    this.picture = { phase: "review", draft };
  }

  /** Renders a (reviewed) draft into chat `id`, the open one by default. The chat stays usable
   * meanwhile, and the render carries on when another chat is opened. */
  async renderPicture(draft: ImagePromptDraft, id: Uuid | null = this.conversationId) {
    if (!id) return;
    this.rendering.set(id, draft);
    if (this.conversationId === id) {
      this.error = "";
      this.picture = { phase: "render", draft };
    }
    try {
      const message = await api.generateImage(id, draft);
      if (message) {
        if (this.conversationId === id) this.messages.push(toUi(message));
        else this.parked.get(id)?.messages.push(toUi(message));
      }
    } catch (e) {
      if (this.conversationId === id) this.error = errorText(e);
    } finally {
      this.rendering.delete(id);
      if (this.conversationId === id) this.picture = null;
      this.nextPhoto(id);
    }
  }

  /** A photo a character sent in chat `id` (open or not): drawn straight away with no review, or
   * after the picture already under way there. */
  private async drawSentPhoto(id: Uuid, characterId: Uuid | null, description: string) {
    try {
      const draft = await api.photoDraft(characterId, description);
      this.photoQueue.set(id, [...(this.photoQueue.get(id) ?? []), draft]);
      this.nextPhoto(id);
    } catch (e) {
      if (this.conversationId === id) this.error = errorText(e);
    }
  }

  /** Starts chat `id`'s next queued photo, unless a picture is already under way there. */
  private nextPhoto(id: Uuid) {
    if (this.rendering.has(id) || (this.conversationId === id && this.picture)) return;
    const [next, ...rest] = this.photoQueue.get(id) ?? [];
    if (rest.length) this.photoQueue.set(id, rest);
    else this.photoQueue.delete(id);
    if (next) void this.renderPicture(next, id);
  }

  /** Draws a picture again from exactly the same draft (a new seed when the seed is random). */
  async drawAgain(message: ChatMessage) {
    const draft = pictureDraft(message);
    if (draft && !this.picture) await this.renderPicture(draft);
  }

  /** Copies a picture into the character's avatar or gallery. A copy, so deleting the chat's
   * picture later doesn't take the avatar with it. Returns a status line. */
  async usePicture(message: ChatMessage, as: "avatar" | "gallery"): Promise<string> {
    const character = this.session?.characters.find((c) => c.id === message.speakerCharacterId);
    const source = message.attachments[0]?.imagePath;
    if (!character || !source) return "";
    try {
      const copy = await api.storeImage(source);
      const updated = structuredClone($state.snapshot(character));
      if (as === "avatar") updated.avatarPath = copy;
      else updated.galleryImagePaths = [...updated.galleryImagePaths, copy];
      const saved = await api.saveCharacter(updated);
      if (this.session) this.session.characters = this.session.characters.map((c) => (c.id === saved.id ? saved : c));
      return as === "avatar" ? `Set as ${saved.name}'s avatar.` : `Added to ${saved.name}'s gallery.`;
    } catch (e) {
      this.error = errorText(e);
      return "";
    }
  }

  cancelPictureReview() {
    const id = this.conversationId;
    if (!id || this.picture?.phase !== "review") return;
    this.picture = null;
    this.nextPhoto(id);
  }

  /** Stops the picture under way, and the photos waiting behind it. */
  async stopPicture() {
    const id = this.conversationId;
    if (!id || !this.picture) return;
    this.photoQueue.delete(id);
    if (this.picture.phase === "prompt") await api.stopGeneration(id);
    else if (this.picture.phase === "render") await api.stopImage(id);
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
      const saved = await api.setWorldState(id, { ...$state.snapshot(current), ...patch });
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

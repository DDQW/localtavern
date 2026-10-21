// The typed surface over the Rust commands. Outside Tauri (a plain browser, e.g. `npm run dev` for
// UI work) the same interface is served by an in-memory mock, so every page can be exercised without
// the native shell.

import { Channel, invoke } from "@tauri-apps/api/core";
import { mockApi } from "./mock";
import type {
  Appearance,
  AppearanceSubject,
  Country,
  HolidayOverview,
  LoreDraftRequest,
  Schedule,
  WorldInfoEntry,
  Bootstrap,
  Character,
  ChatEvent,
  ChatMessage,
  ChatSession,
  Conversation,
  ExpressionImage,
  GeminiOptions,
  GroupChat,
  Lorebook,
  OllamaOptions,
  VastAiOptions,
  OnDeviceOptions,
  OnDeviceStatus,
  DownloadProgress,
  Persona,
  PromptPresetFull,
  ProviderInfo,
  SearchHit,
  ContextStatus,
  ImageBackendOptions,
  ImageGenSettings,
  ImageMode,
  ImagePromptDraft,
  CharacterPicture,
  CharacterShot,
  LookDraft,
  DefaultPersonas,
  Uuid,
  StoryCharacter,
  WorldState,
  PowerShellOptions,
  SessionStatus,
  UserMemory,
} from "./types";

export type EventHandler = (event: ChatEvent) => void;

export interface Api {
  bootstrap(): Promise<Bootstrap>;
  listCharacters(): Promise<Character[]>;
  listPersonas(): Promise<Persona[]>;
  listConversations(): Promise<Conversation[]>;

  openAssistant(forceNew: boolean): Promise<ChatSession>;
  openCharacter(characterId: Uuid, forceNew: boolean): Promise<ChatSession>;
  openGroup(groupId: Uuid, forceNew: boolean): Promise<ChatSession>;
  /** The private talk with one member of a group conversation (resumed, or started empty). */
  openBreakout(conversationId: Uuid, characterId: Uuid): Promise<ChatSession>;
  openLastRoleplay(): Promise<ChatSession | null>;
  resumeConversation(conversationId: Uuid): Promise<ChatSession>;
  newChat(conversationId: Uuid): Promise<ChatSession>;
  deleteConversation(conversationId: Uuid): Promise<void>;
  searchMessages(query: string): Promise<SearchHit[]>;

  sendMessage(conversationId: Uuid, text: string, attachments: string[], onEvent: EventHandler): Promise<void>;
  swipeNext(conversationId: Uuid, messageId: Uuid, onEvent: EventHandler): Promise<void>;
  pickSpeaker(conversationId: Uuid, characterId: Uuid, onEvent: EventHandler): Promise<void>;
  continueMessage(conversationId: Uuid, onEvent: EventHandler): Promise<void>;
  /** A drafted user message for the input box; null when stopped. */
  impersonate(conversationId: Uuid): Promise<string | null>;
  contextStatus(conversationId: Uuid): Promise<ContextStatus>;
  /** Summarizes all but the newest messages so they stop filling the context window. */
  compactConversation(conversationId: Uuid): Promise<ContextStatus>;
  /** Undoes compaction: every message is sent in full again. */
  clearConversationSummary(conversationId: Uuid): Promise<ContextStatus>;
  swipePrevious(messageId: Uuid): Promise<ChatMessage>;
  stopGeneration(conversationId: Uuid): Promise<void>;
  editMessage(messageId: Uuid, text: string): Promise<ChatMessage>;
  /** Returns the story clock when deleting the reply put it back. */
  deleteMessage(messageId: Uuid): Promise<WorldState | null>;

  setConversationModel(conversationId: Uuid, providerId: string, model: string): Promise<Conversation>;
  setConversationPreset(conversationId: Uuid, presetId: Uuid): Promise<Conversation>;
  setConversationPersona(conversationId: Uuid, personaId: Uuid | null): Promise<Conversation>;
  setAssistantPrompt(conversationId: Uuid, text: string): Promise<Conversation>;
  setSearchOverride(conversationId: Uuid, enabled: boolean | null): Promise<Conversation>;
  /** Saves the in-story clock; returns it as stored (minutes clamped, blank notes dropped). */
  setWorldState(conversationId: Uuid, worldState: WorldState): Promise<WorldState>;

  listModels(providerId: string): Promise<string[]>;
  refreshModels(providerId: string): Promise<string[]>;
  subscriptionStatus(providerId: string): Promise<SessionStatus>;
  /** Opens the browser and waits for the sign-in to finish (or be cancelled). */
  subscriptionSignIn(providerId: string): Promise<SessionStatus>;
  subscriptionCancelSignIn(providerId: string): Promise<void>;
  subscriptionSignOut(providerId: string): Promise<SessionStatus>;

  getDefaultPersonas(): Promise<DefaultPersonas>;
  saveDefaultPersonas(personas: DefaultPersonas): Promise<void>;

  getPowerShellOptions(): Promise<PowerShellOptions>;
  savePowerShellOptions(options: PowerShellOptions): Promise<void>;
  resolveToolApproval(requestId: Uuid, decision: { allow: boolean; alwaysPattern: string | null }): Promise<boolean>;

  listMemories(): Promise<UserMemory[]>;
  deleteMemory(memoryId: Uuid): Promise<void>;
  clearMemories(): Promise<void>;

  setApiKey(providerId: string, key: string): Promise<ProviderInfo>;
  /** Browser sign-in that stores the API key it returns. Cancel with `subscriptionCancelSignIn`. */
  browserKeySignIn(providerId: string): Promise<ProviderInfo>;
  getGeminiOptions(): Promise<GeminiOptions>;
  saveGeminiOptions(options: GeminiOptions): Promise<void>;
  getOllamaOptions(): Promise<OllamaOptions>;
  saveOllamaOptions(options: OllamaOptions): Promise<void>;
  getVastAiOptions(): Promise<VastAiOptions>;
  saveVastAiOptions(options: VastAiOptions): Promise<void>;
  ondeviceStatus(): Promise<OnDeviceStatus>;
  /** Also unloads the running model, so the new settings apply on the next reply. */
  saveOndeviceOptions(options: OnDeviceOptions): Promise<void>;
  deleteOndeviceModel(fileName: string): Promise<void>;
  /** Resolves with the saved file's name once the download is complete. */
  downloadOndeviceModel(url: string, onProgress: (progress: DownloadProgress) => void): Promise<string>;
  cancelOndeviceDownload(): Promise<void>;

  storeImage(path: string): Promise<string>;
  deleteImage(path: string): Promise<boolean>;

  saveCharacter(character: Character): Promise<Character>;
  deleteCharacter(characterId: Uuid): Promise<void>;
  importCharacter(path: string): Promise<Character>;
  exportCharacterCard(character: Character, path: string): Promise<void>;
  importExpressionFolder(folder: string): Promise<ExpressionImage[]>;

  savePersona(persona: Persona): Promise<Persona>;
  deletePersona(personaId: Uuid): Promise<void>;

  renderAppearance(appearance: Appearance, subject: AppearanceSubject): Promise<string>;
  polishAppearance(text: string): Promise<string>;

  draftScenario(character: Character, idea: string): Promise<string>;
  /** A group's scenario from its members' cards, rewriting `current` when there is one. */
  draftGroupScenario(characterIds: Uuid[], current: string, idea: string): Promise<string>;
  draftGreetings(character: Character, idea: string, count: number): Promise<string[]>;
  draftLoreEntries(request: LoreDraftRequest): Promise<WorldInfoEntry[]>;
  draftSchedule(character: Character, idea: string): Promise<Schedule>;

  listStoryCast(conversationId: Uuid): Promise<StoryCharacter[]>;
  /** Reads the latest stretch of the story for newly introduced people; returns the whole cast. */
  scanStoryCast(conversationId: Uuid): Promise<StoryCharacter[]>;
  saveStoryCharacter(character: StoryCharacter): Promise<StoryCharacter[]>;
  deleteStoryCharacter(conversationId: Uuid, id: Uuid): Promise<StoryCharacter[]>;
  getTrackStoryCast(): Promise<boolean>;
  setTrackStoryCast(on: boolean): Promise<void>;

  holidayCountries(): Promise<Country[]>;
  holidayOverview(conversationId: Uuid): Promise<HolidayOverview>;

  listLorebooks(): Promise<Lorebook[]>;
  saveLorebook(lorebook: Lorebook): Promise<Lorebook>;
  deleteLorebook(lorebookId: Uuid): Promise<void>;

  listPresets(): Promise<PromptPresetFull[]>;
  savePreset(preset: PromptPresetFull): Promise<PromptPresetFull>;
  deletePreset(presetId: Uuid): Promise<void>;

  listGroupChats(): Promise<GroupChat[]>;
  saveGroupChat(group: GroupChat): Promise<GroupChat>;
  deleteGroupChat(groupId: Uuid): Promise<void>;

  getImageSettings(): Promise<ImageGenSettings>;
  saveImageSettings(settings: ImageGenSettings): Promise<ImageGenSettings>;
  /** Checkpoints/samplers/schedulers from the server in `settings` (which needn't be saved yet). */
  imageBackendOptions(settings: ImageGenSettings): Promise<ImageBackendOptions>;
  /** The chat model's base look + usual clothing for a character, from its description. */
  describeCharacterLook(character: Character): Promise<LookDraft>;
  /** Draws one mood/action picture of the character as edited; null when stopped. */
  drawCharacterPicture(character: Character, shot: CharacterShot, outfitId: Uuid | null): Promise<CharacterPicture | null>;
  stopCharacterPicture(characterId: Uuid): Promise<void>;
  /** The chat model's description for a picture; null when stopped (stopGeneration). */
  imagePrompt(conversationId: Uuid, mode: ImageMode, characterId: Uuid | null): Promise<ImagePromptDraft | null>;
  /** The ready-to-draw draft for a photo a character sent in the story. */
  photoDraft(characterId: Uuid | null, description: string): Promise<ImagePromptDraft>;
  /** Renders a draft and adds it to the chat; null when stopped (stopImage). */
  generateImage(conversationId: Uuid, draft: ImagePromptDraft): Promise<ChatMessage | null>;
  stopImage(conversationId: Uuid): Promise<void>;
  /** Deletes a picture message and its stored file. */
  deletePicture(messageId: Uuid): Promise<void>;
}

/** Runs a streaming command, forwarding each event to `onEvent`; resolves when the command returns. */
function streaming(command: string, args: Record<string, unknown>, onEvent: EventHandler): Promise<void> {
  const channel = new Channel<ChatEvent>();
  channel.onmessage = onEvent;
  return invoke<void>(command, { ...args, onEvent: channel });
}

const tauriApi: Api = {
  bootstrap: () => invoke("bootstrap"),
  listCharacters: () => invoke("list_characters"),
  listPersonas: () => invoke("list_personas"),
  listConversations: () => invoke("list_conversations"),

  openAssistant: (forceNew) => invoke("open_assistant", { forceNew }),
  openCharacter: (characterId, forceNew) => invoke("open_character", { characterId, forceNew }),
  openGroup: (groupId, forceNew) => invoke("open_group", { groupId, forceNew }),
  openBreakout: (conversationId, characterId) => invoke("open_breakout", { conversationId, characterId }),
  openLastRoleplay: () => invoke("open_last_roleplay"),
  resumeConversation: (conversationId) => invoke("resume_conversation", { conversationId }),
  newChat: (conversationId) => invoke("new_chat", { conversationId }),
  deleteConversation: (conversationId) => invoke("delete_conversation", { conversationId }),
  searchMessages: (query) => invoke("search_messages", { query }),

  sendMessage: (conversationId, text, attachments, onEvent) =>
    streaming("send_message", { conversationId, text, attachments }, onEvent),
  swipeNext: (conversationId, messageId, onEvent) => streaming("swipe_next", { conversationId, messageId }, onEvent),
  pickSpeaker: (conversationId, characterId, onEvent) =>
    streaming("pick_speaker", { conversationId, characterId }, onEvent),
  continueMessage: (conversationId, onEvent) => streaming("continue_message", { conversationId }, onEvent),
  impersonate: (conversationId) => invoke("impersonate", { conversationId }),
  contextStatus: (conversationId) => invoke("context_status", { conversationId }),
  compactConversation: (conversationId) => invoke("compact_conversation", { conversationId }),
  clearConversationSummary: (conversationId) => invoke("clear_conversation_summary", { conversationId }),
  swipePrevious: (messageId) => invoke("swipe_previous", { messageId }),
  stopGeneration: (conversationId) => invoke("stop_generation", { conversationId }),
  editMessage: (messageId, text) => invoke("edit_message", { messageId, text }),
  deleteMessage: (messageId) => invoke("delete_message", { messageId }),

  setConversationModel: (conversationId, providerId, model) =>
    invoke("set_conversation_model", { conversationId, providerId, model }),
  setConversationPreset: (conversationId, presetId) => invoke("set_conversation_preset", { conversationId, presetId }),
  setConversationPersona: (conversationId, personaId) =>
    invoke("set_conversation_persona", { conversationId, personaId }),
  setAssistantPrompt: (conversationId, text) => invoke("set_assistant_prompt", { conversationId, text }),
  setSearchOverride: (conversationId, enabled) => invoke("set_search_override", { conversationId, enabled }),
  setWorldState: (conversationId, worldState) => invoke("set_world_state", { conversationId, worldState }),

  listModels: (providerId) => invoke("list_models", { providerId }),
  refreshModels: (providerId) => invoke("refresh_models", { providerId }),
  subscriptionStatus: (providerId) => invoke("subscription_status", { providerId }),
  subscriptionSignIn: (providerId) => invoke("subscription_sign_in", { providerId }),
  subscriptionCancelSignIn: (providerId) => invoke("subscription_cancel_sign_in", { providerId }),
  subscriptionSignOut: (providerId) => invoke("subscription_sign_out", { providerId }),

  getDefaultPersonas: () => invoke("get_default_personas"),
  saveDefaultPersonas: (personas) => invoke("save_default_personas", { personas }),

  getPowerShellOptions: () => invoke("get_powershell_options"),
  savePowerShellOptions: (options) => invoke("save_powershell_options", { options }),
  resolveToolApproval: (requestId, decision) => invoke("resolve_tool_approval", { requestId, decision }),

  listMemories: () => invoke("list_memories"),
  deleteMemory: (memoryId) => invoke("delete_memory", { memoryId }),
  clearMemories: () => invoke("clear_memories"),

  setApiKey: (providerId, key) => invoke("set_api_key", { providerId, key }),
  browserKeySignIn: (providerId) => invoke("browser_key_sign_in", { providerId }),
  getGeminiOptions: () => invoke("get_gemini_options"),
  saveGeminiOptions: (options) => invoke("save_gemini_options", { options }),
  getOllamaOptions: () => invoke("get_ollama_options"),
  saveOllamaOptions: (options) => invoke("save_ollama_options", { options }),
  getVastAiOptions: () => invoke("get_vastai_options"),
  saveVastAiOptions: (options) => invoke("save_vastai_options", { options }),
  ondeviceStatus: () => invoke("ondevice_status"),
  saveOndeviceOptions: (options) => invoke("save_ondevice_options", { options }),
  deleteOndeviceModel: (fileName) => invoke("delete_ondevice_model", { fileName }),
  downloadOndeviceModel: (url, onProgress) => {
    const channel = new Channel<DownloadProgress>();
    channel.onmessage = onProgress;
    return invoke("download_ondevice_model", { url, onProgress: channel });
  },
  cancelOndeviceDownload: () => invoke("cancel_ondevice_download"),

  storeImage: (path) => invoke("store_image", { path }),
  deleteImage: (path) => invoke("delete_image", { path }),

  saveCharacter: (character) => invoke("save_character", { character }),
  deleteCharacter: (characterId) => invoke("delete_character", { characterId }),
  importCharacter: (path) => invoke("import_character", { path }),
  exportCharacterCard: (character, path) => invoke("export_character_card", { character, path }),
  importExpressionFolder: (folder) => invoke("import_expression_folder", { folder }),

  savePersona: (persona) => invoke("save_persona", { persona }),
  deletePersona: (personaId) => invoke("delete_persona", { personaId }),

  renderAppearance: (appearance, subject) => invoke("render_appearance", { appearance, subject }),
  polishAppearance: (text) => invoke("polish_appearance", { text }),

  draftScenario: (character, idea) => invoke("draft_scenario", { character, idea }),
  draftGroupScenario: (characterIds, current, idea) => invoke("draft_group_scenario", { characterIds, current, idea }),
  draftGreetings: (character, idea, count) => invoke("draft_greetings", { character, idea, count }),
  draftLoreEntries: (request) => invoke("draft_lore_entries", { request }),
  draftSchedule: (character, idea) => invoke("draft_schedule", { character, idea }),

  listStoryCast: (conversationId) => invoke("list_story_cast", { conversationId }),
  scanStoryCast: (conversationId) => invoke("scan_story_cast", { conversationId }),
  saveStoryCharacter: (character) => invoke("save_story_character", { character }),
  deleteStoryCharacter: (conversationId, id) => invoke("delete_story_character", { conversationId, id }),
  getTrackStoryCast: () => invoke("get_track_story_cast"),
  setTrackStoryCast: (on) => invoke("set_track_story_cast", { on }),

  holidayCountries: () => invoke("holiday_countries"),
  holidayOverview: (conversationId) => invoke("holiday_overview", { conversationId }),

  listLorebooks: () => invoke("list_lorebooks"),
  saveLorebook: (lorebook) => invoke("save_lorebook", { lorebook }),
  deleteLorebook: (lorebookId) => invoke("delete_lorebook", { lorebookId }),

  listPresets: () => invoke("list_presets"),
  savePreset: (preset) => invoke("save_preset", { preset }),
  deletePreset: (presetId) => invoke("delete_preset", { presetId }),

  listGroupChats: () => invoke("list_group_chats"),
  saveGroupChat: (group) => invoke("save_group_chat", { group }),
  deleteGroupChat: (groupId) => invoke("delete_group_chat", { groupId }),

  getImageSettings: () => invoke("get_image_settings"),
  saveImageSettings: (settings) => invoke("save_image_settings", { settings }),
  imageBackendOptions: (settings) => invoke("image_backend_options", { settings }),
  describeCharacterLook: (character) => invoke("describe_character_look", { character }),
  drawCharacterPicture: (character, shot, outfitId) => invoke("draw_character_picture", { character, shot, outfitId }),
  stopCharacterPicture: (characterId) => invoke("stop_character_picture", { characterId }),
  imagePrompt: (conversationId, mode, characterId) => invoke("image_prompt", { conversationId, mode, characterId }),
  photoDraft: (characterId, description) => invoke("photo_draft", { characterId, description }),
  generateImage: (conversationId, draft) => invoke("generate_image", { conversationId, draft }),
  stopImage: (conversationId) => invoke("stop_image", { conversationId }),
  deletePicture: (messageId) => invoke("delete_picture", { messageId }),
};

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api: Api = inTauri ? tauriApi : mockApi;

/** Turns whatever a rejected command threw (a string from Rust, or an Error) into display text. */
export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

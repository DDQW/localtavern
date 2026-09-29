// The typed surface over the Rust commands. Outside Tauri (a plain browser, e.g. `npm run dev` for
// UI work) the same interface is served by an in-memory mock, so every page can be exercised without
// the native shell.

import { Channel, invoke } from "@tauri-apps/api/core";
import { mockApi } from "./mock";
import type {
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
  Persona,
  PromptPresetFull,
  ProviderInfo,
  SearchHit,
  StoryState,
  CharacterNotebookEntry,
  Uuid,
  WorldState,
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
  openLastRoleplay(): Promise<ChatSession | null>;
  resumeConversation(conversationId: Uuid): Promise<ChatSession>;
  newChat(conversationId: Uuid): Promise<ChatSession>;
  deleteConversation(conversationId: Uuid): Promise<void>;
  storyState(conversationId: Uuid): Promise<StoryState>;
  searchMessages(query: string): Promise<SearchHit[]>;
  addNote(conversationId: Uuid, characterId: Uuid, content: string): Promise<CharacterNotebookEntry>;
  updateNote(noteId: Uuid, content: string): Promise<CharacterNotebookEntry>;
  deleteNote(noteId: Uuid): Promise<void>;

  sendMessage(conversationId: Uuid, text: string, attachments: string[], onEvent: EventHandler): Promise<void>;
  swipeNext(conversationId: Uuid, messageId: Uuid, onEvent: EventHandler): Promise<void>;
  pickSpeaker(conversationId: Uuid, characterId: Uuid, onEvent: EventHandler): Promise<void>;
  continueMessage(conversationId: Uuid, onEvent: EventHandler): Promise<void>;
  /** A drafted user message for the input box; null when stopped. */
  impersonate(conversationId: Uuid): Promise<string | null>;
  swipePrevious(messageId: Uuid): Promise<ChatMessage>;
  stopGeneration(conversationId: Uuid): Promise<void>;
  editMessage(messageId: Uuid, text: string): Promise<ChatMessage>;
  deleteMessage(messageId: Uuid): Promise<void>;

  setConversationModel(conversationId: Uuid, providerId: string, model: string): Promise<Conversation>;
  setConversationPreset(conversationId: Uuid, presetId: Uuid): Promise<Conversation>;
  setConversationPersona(conversationId: Uuid, personaId: Uuid | null): Promise<Conversation>;
  setAssistantPrompt(conversationId: Uuid, text: string): Promise<Conversation>;
  setSearchOverride(conversationId: Uuid, enabled: boolean | null): Promise<Conversation>;
  /** Saves the in-story clock; returns it as stored (minutes clamped, blank notes dropped). */
  setWorldState(conversationId: Uuid, worldState: WorldState): Promise<WorldState>;

  listModels(providerId: string): Promise<string[]>;
  refreshModels(providerId: string): Promise<string[]>;
  setApiKey(providerId: string, key: string): Promise<ProviderInfo>;
  getGeminiOptions(): Promise<GeminiOptions>;
  saveGeminiOptions(options: GeminiOptions): Promise<void>;
  getOllamaOptions(): Promise<OllamaOptions>;
  saveOllamaOptions(options: OllamaOptions): Promise<void>;

  storeImage(path: string): Promise<string>;
  deleteImage(path: string): Promise<boolean>;

  saveCharacter(character: Character): Promise<Character>;
  deleteCharacter(characterId: Uuid): Promise<void>;
  importCharacter(path: string): Promise<Character>;
  exportCharacterCard(character: Character, path: string): Promise<void>;
  importExpressionFolder(folder: string): Promise<ExpressionImage[]>;

  savePersona(persona: Persona): Promise<Persona>;
  deletePersona(personaId: Uuid): Promise<void>;

  listLorebooks(): Promise<Lorebook[]>;
  saveLorebook(lorebook: Lorebook): Promise<Lorebook>;
  deleteLorebook(lorebookId: Uuid): Promise<void>;

  listPresets(): Promise<PromptPresetFull[]>;
  savePreset(preset: PromptPresetFull): Promise<PromptPresetFull>;
  deletePreset(presetId: Uuid): Promise<void>;

  listGroupChats(): Promise<GroupChat[]>;
  saveGroupChat(group: GroupChat): Promise<GroupChat>;
  deleteGroupChat(groupId: Uuid): Promise<void>;
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
  openLastRoleplay: () => invoke("open_last_roleplay"),
  resumeConversation: (conversationId) => invoke("resume_conversation", { conversationId }),
  newChat: (conversationId) => invoke("new_chat", { conversationId }),
  deleteConversation: (conversationId) => invoke("delete_conversation", { conversationId }),
  storyState: (conversationId) => invoke("story_state", { conversationId }),
  searchMessages: (query) => invoke("search_messages", { query }),
  addNote: (conversationId, characterId, content) => invoke("add_note", { conversationId, characterId, content }),
  updateNote: (noteId, content) => invoke("update_note", { noteId, content }),
  deleteNote: (noteId) => invoke("delete_note", { noteId }),

  sendMessage: (conversationId, text, attachments, onEvent) =>
    streaming("send_message", { conversationId, text, attachments }, onEvent),
  swipeNext: (conversationId, messageId, onEvent) => streaming("swipe_next", { conversationId, messageId }, onEvent),
  pickSpeaker: (conversationId, characterId, onEvent) =>
    streaming("pick_speaker", { conversationId, characterId }, onEvent),
  continueMessage: (conversationId, onEvent) => streaming("continue_message", { conversationId }, onEvent),
  impersonate: (conversationId) => invoke("impersonate", { conversationId }),
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
  setApiKey: (providerId, key) => invoke("set_api_key", { providerId, key }),
  getGeminiOptions: () => invoke("get_gemini_options"),
  saveGeminiOptions: (options) => invoke("save_gemini_options", { options }),
  getOllamaOptions: () => invoke("get_ollama_options"),
  saveOllamaOptions: (options) => invoke("save_ollama_options", { options }),

  storeImage: (path) => invoke("store_image", { path }),
  deleteImage: (path) => invoke("delete_image", { path }),

  saveCharacter: (character) => invoke("save_character", { character }),
  deleteCharacter: (characterId) => invoke("delete_character", { characterId }),
  importCharacter: (path) => invoke("import_character", { path }),
  exportCharacterCard: (character, path) => invoke("export_character_card", { character, path }),
  importExpressionFolder: (folder) => invoke("import_expression_folder", { folder }),

  savePersona: (persona) => invoke("save_persona", { persona }),
  deletePersona: (personaId) => invoke("delete_persona", { personaId }),

  listLorebooks: () => invoke("list_lorebooks"),
  saveLorebook: (lorebook) => invoke("save_lorebook", { lorebook }),
  deleteLorebook: (lorebookId) => invoke("delete_lorebook", { lorebookId }),

  listPresets: () => invoke("list_presets"),
  savePreset: (preset) => invoke("save_preset", { preset }),
  deletePreset: (presetId) => invoke("delete_preset", { presetId }),

  listGroupChats: () => invoke("list_group_chats"),
  saveGroupChat: (group) => invoke("save_group_chat", { group }),
  deleteGroupChat: (groupId) => invoke("delete_group_chat", { groupId }),
};

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api: Api = inTauri ? tauriApi : mockApi;

/** Turns whatever a rejected command threw (a string from Rust, or an Error) into display text. */
export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

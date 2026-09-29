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
  GeminiOptions,
  OllamaOptions,
  Persona,
  ProviderInfo,
  SearchHit,
  StoryState,
  Uuid,
} from "./types";

export type EventHandler = (event: ChatEvent) => void;

export interface Api {
  bootstrap(): Promise<Bootstrap>;
  listCharacters(): Promise<Character[]>;
  listPersonas(): Promise<Persona[]>;
  listConversations(): Promise<Conversation[]>;

  openAssistant(forceNew: boolean): Promise<ChatSession>;
  openCharacter(characterId: Uuid, forceNew: boolean): Promise<ChatSession>;
  openLastRoleplay(): Promise<ChatSession | null>;
  resumeConversation(conversationId: Uuid): Promise<ChatSession>;
  newChat(conversationId: Uuid): Promise<ChatSession>;
  deleteConversation(conversationId: Uuid): Promise<void>;
  storyState(conversationId: Uuid): Promise<StoryState>;
  searchMessages(query: string): Promise<SearchHit[]>;

  sendMessage(conversationId: Uuid, text: string, attachments: string[], onEvent: EventHandler): Promise<void>;
  swipeNext(conversationId: Uuid, messageId: Uuid, onEvent: EventHandler): Promise<void>;
  swipePrevious(messageId: Uuid): Promise<ChatMessage>;
  stopGeneration(conversationId: Uuid): Promise<void>;
  editMessage(messageId: Uuid, text: string): Promise<ChatMessage>;
  deleteMessage(messageId: Uuid): Promise<void>;

  setConversationModel(conversationId: Uuid, providerId: string, model: string): Promise<Conversation>;
  setConversationPreset(conversationId: Uuid, presetId: Uuid): Promise<Conversation>;
  setConversationPersona(conversationId: Uuid, personaId: Uuid | null): Promise<Conversation>;
  setAssistantPrompt(conversationId: Uuid, text: string): Promise<Conversation>;
  setSearchOverride(conversationId: Uuid, enabled: boolean | null): Promise<Conversation>;

  listModels(providerId: string): Promise<string[]>;
  refreshModels(providerId: string): Promise<string[]>;
  setApiKey(providerId: string, key: string): Promise<ProviderInfo>;
  getGeminiOptions(): Promise<GeminiOptions>;
  saveGeminiOptions(options: GeminiOptions): Promise<void>;
  getOllamaOptions(): Promise<OllamaOptions>;
  saveOllamaOptions(options: OllamaOptions): Promise<void>;

  storeImage(path: string): Promise<string>;
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
  openLastRoleplay: () => invoke("open_last_roleplay"),
  resumeConversation: (conversationId) => invoke("resume_conversation", { conversationId }),
  newChat: (conversationId) => invoke("new_chat", { conversationId }),
  deleteConversation: (conversationId) => invoke("delete_conversation", { conversationId }),
  storyState: (conversationId) => invoke("story_state", { conversationId }),
  searchMessages: (query) => invoke("search_messages", { query }),

  sendMessage: (conversationId, text, attachments, onEvent) =>
    streaming("send_message", { conversationId, text, attachments }, onEvent),
  swipeNext: (conversationId, messageId, onEvent) => streaming("swipe_next", { conversationId, messageId }, onEvent),
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

  listModels: (providerId) => invoke("list_models", { providerId }),
  refreshModels: (providerId) => invoke("refresh_models", { providerId }),
  setApiKey: (providerId, key) => invoke("set_api_key", { providerId, key }),
  getGeminiOptions: () => invoke("get_gemini_options"),
  saveGeminiOptions: (options) => invoke("save_gemini_options", { options }),
  getOllamaOptions: () => invoke("get_ollama_options"),
  saveOllamaOptions: (options) => invoke("save_ollama_options", { options }),

  storeImage: (path) => invoke("store_image", { path }),
};

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api: Api = inTauri ? tauriApi : mockApi;

/** Turns whatever a rejected command threw (a string from Rust, or an Error) into display text. */
export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

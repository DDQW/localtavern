// Mirrors the JSON the Rust backend produces (camelCase; see src-tauri/src/models and chat/).

export type Uuid = string;

export type MessageRole = "user" | "character" | "system" | "tool";
export type ConversationMode = "roleplay" | "assistant";

export interface MessageAlternate {
  text: string;
  reasoningText: string | null;
  rawPrompt: string | null;
}

export interface MessageAttachment {
  imagePath: string;
  mimeType: string;
}

export interface LlmUsage {
  promptTokens: number | null;
  completionTokens: number | null;
  totalTokens: number | null;
}

export interface ChatMessage {
  id: Uuid;
  conversationId: Uuid;
  role: MessageRole;
  speakerCharacterId: Uuid | null;
  text: string;
  reasoningText: string | null;
  rawPromptText: string | null;
  alternates: MessageAlternate[];
  activeAlternateIndex: number;
  attachments: MessageAttachment[];
  detectedEmotion: string | null;
  timestamp: string;
  usage: LlmUsage | null;
}

/** A message as the UI holds it: the stored message plus transient view state. */
export interface UiMessage extends ChatMessage {
  streaming: boolean;
  editing: boolean;
  editBuffer: string;
}

export interface ExpressionImage {
  emotion: string;
  imagePath: string;
}

export interface Character {
  id: Uuid;
  name: string;
  avatarPath: string | null;
  description: string;
  personality: string;
  scenario: string;
  firstMessages: string[];
  exampleDialogue: string;
  tags: string[];
  linkedLorebookIds: Uuid[];
  expressions: ExpressionImage[];
  galleryImagePaths: string[];
  createdAt: string;
  updatedAt: string;
}

export interface Persona {
  id: Uuid;
  name: string;
  avatarPath: string | null;
  description: string;
  isDefault: boolean;
}

export type WeatherCondition =
  | "Clear"
  | "PartlyCloudy"
  | "Overcast"
  | "Rain"
  | "Thunderstorm"
  | "Snow"
  | "Fog"
  | "Windy";

export interface WorldState {
  fictionalDateTime: string;
  weather: WeatherCondition;
  weatherNotes: string | null;
  defaultAdvanceMinutesPerMessage: number;
  autoDriftWeatherEnabled: boolean;
}

export interface Conversation {
  id: Uuid;
  title: string;
  mode: ConversationMode;
  characterId: Uuid | null;
  groupChatId: Uuid | null;
  personaId: Uuid | null;
  promptPresetId: Uuid | null;
  providerId: string;
  model: string;
  assistantSystemPrompt: string | null;
  enableGoogleSearch: boolean | null;
  worldState: WorldState | null;
  createdAt: string;
  updatedAt: string;
}

export interface PromptBlock {
  id: Uuid;
  type: string;
  enabled: boolean;
  order: number;
  customText: string | null;
  injectionDepth: number;
}

export interface PromptPreset {
  id: Uuid;
  name: string;
  blocks: PromptBlock[];
}

export interface PresetSummary {
  id: Uuid;
  name: string;
}

export interface HistoryEntry {
  id: Uuid;
  label: string;
  updatedAt: string;
}

export interface ChatSession {
  conversation: Conversation;
  messages: ChatMessage[];
  characters: Character[];
  persona: Persona | null;
  personas: Persona[];
  preset: PromptPreset;
  presets: PresetSummary[];
  history: HistoryEntry[];
  providers: string[];
  models: string[];
  memoryCount: number;
}

export interface ProviderInfo {
  id: string;
  name: string;
  auth: "apiKey" | "local";
  hasKey: boolean;
}

export interface Bootstrap {
  providers: ProviderInfo[];
  dataDir: string;
}

export interface QuestLogEntry {
  id: Uuid;
  conversationId: Uuid;
  title: string;
  description: string;
  status: "active" | "completed" | "failed";
  createdAt: string;
  updatedAt: string;
}

export interface CharacterNotebookEntry {
  id: Uuid;
  conversationId: Uuid;
  characterId: Uuid;
  content: string;
  createdAt: string;
}

export interface StoryState {
  quests: QuestLogEntry[];
  notebook: CharacterNotebookEntry[];
}

export interface SearchHit {
  message: ChatMessage;
  conversationTitle: string;
}

export type SafetyCategory = "harassment" | "hateSpeech" | "sexuallyExplicit" | "dangerousContent";
export type SafetyThreshold =
  | "unspecified"
  | "off"
  | "blockNone"
  | "blockOnlyHigh"
  | "blockMediumAndAbove"
  | "blockLowAndAbove";

export interface GeminiOptions {
  enableGoogleSearch: boolean;
  safetyThresholds: Partial<Record<SafetyCategory, SafetyThreshold>>;
}

export interface OllamaOptions {
  baseUrl: string;
  contextSize: number;
}

export type ErrorKind = "auth" | "rateLimit" | "invalidRequest" | "network" | "contentBlocked" | "unknown";

/** What the engine reports while a reply is produced (see src-tauri/src/chat/events.rs). */
export type ChatEvent =
  | { type: "messageAdded"; message: ChatMessage }
  | { type: "regenerating"; messageId: Uuid }
  | { type: "delta"; messageId: Uuid; text: string; reasoning: string }
  | { type: "messageUpdated"; message: ChatMessage }
  | { type: "messageRemoved"; messageId: Uuid }
  | { type: "worldStateChanged"; conversationId: Uuid; worldState: WorldState }
  | { type: "titleChanged"; conversationId: Uuid; title: string }
  | { type: "error"; message: string; kind: ErrorKind | null }
  | { type: "finished" };

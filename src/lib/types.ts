// Mirrors the JSON the Rust backend produces (camelCase; see src-tauri/src/models and chat/).

export type Uuid = string;

/** "image" is a generated picture: shown in the chat, never sent to the model. */
export type MessageRole = "user" | "character" | "system" | "tool" | "image";
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

export type Sex = "male" | "female";

/** Quick-builder picks; every trait is free text (a preset value or a custom entry). */
export interface Appearance {
  sex: Sex | null;
  heightCm: number | null;
  weightKg: number | null;
  build: string | null;
  muscleTone: string | null;
  skinTone: string | null;
  hairColor: string | null;
  hairLength: string | null;
  hairStyle: string | null;
  eyeColor: string | null;
  bodyHair: string | null;
  marks: string[];
  female: { cupSize: string | null; bust: string | null; waist: string | null; hips: string | null; butt: string | null };
  male: { shoulders: string | null; chest: string | null; facialHair: string | null };
  adult: {
    nipples: string | null;
    pubicHair: string | null;
    genitals: string | null;
    circumcision: string | null;
    notes: string | null;
  };
  notes: string | null;
}

export type AppearanceSubject = "character" | "persona";

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
  appearance: Appearance | null;
  /** YYYY-MM-DD */
  birthDate: string | null;
  schedule: Schedule | null;
  createdAt: string;
  updatedAt: string;
}

export interface Persona {
  id: Uuid;
  name: string;
  avatarPath: string | null;
  description: string;
  isDefault: boolean;
  appearance: Appearance | null;
  /** YYYY-MM-DD */
  birthDate: string | null;
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
  /** ISO 3166-1 alpha-2, e.g. "DE": where the story is set (picks the public holidays). */
  countryCode: string | null;
  /** e.g. "DE-BY", for regional holidays. */
  regionCode: string | null;
  customHolidays: CustomHoliday[];
}

/** Every year on month/day, or once when `year` is set. */
export interface CustomHoliday {
  name: string;
  month: number;
  day: number;
  year: number | null;
}

export interface Country {
  countryCode: string;
  name: string;
}

export interface DatedHoliday {
  /** YYYY-MM-DD */
  date: string;
  name: string;
  dayOff: boolean;
}

export interface HolidayOverview {
  countryName: string | null;
  regions: string[];
  holidays: DatedHoliday[];
}

export type DayKind = "work" | "school" | "shift" | "off";

export interface ScheduleBlock {
  /** HH:MM; an end before the start runs past midnight. */
  start: string;
  end: string;
  activity: string;
  place: string | null;
}

export interface DayPlan {
  kind: DayKind;
  blocks: ScheduleBlock[];
}

export interface Schedule {
  homeCountry: string | null;
  homeRegion: string | null;
  /** Monday to Sunday. */
  days: DayPlan[];
  holiday: DayPlan;
  observesHolidays: boolean;
  notes: string | null;
}

export interface LoreDraftRequest {
  lorebookName: string;
  topic: string;
  count: number;
  existingKeys: string[];
  character: Character | null;
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
  /** Set for group chats: who is in the group and how speakers are chosen. */
  group: GroupChat | null;
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
  auth: "apiKey" | "subscription" | "local";
  hasKey: boolean;
  /** An API-key provider whose key can also be fetched through a browser sign-in (Mistral). */
  browserSignIn: boolean;
}

export interface SessionStatus {
  signedIn: boolean;
  email: string | null;
}

export interface PowerShellOptions {
  enabled: boolean;
  autoMode: boolean;
  allowlistPatterns: string[];
}

/** Who you are when Roleplay or Assistant opens. Independent: neither choice is used for the other mode. */
export interface DefaultPersonas {
  roleplayPersonaId: Uuid | null;
  assistantPersonaId: Uuid | null;
}

export interface UserMemory {
  id: Uuid;
  content: string;
  createdAt: string;
}

/** A PowerShell command the assistant wants to run, waiting for the user's answer. */
export interface ToolApproval {
  requestId: Uuid;
  command: string;
  elevate: boolean;
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

/** A compacted chat's summary: it replaces, in prompts, every message up to coversThroughMessageId. */
export interface ConversationSummary {
  conversationId: Uuid;
  text: string;
  coversThroughMessageId: Uuid;
  coversThroughTimestamp: string;
  updatedAt: string;
}

/** How full the conversation's context window is. */
export interface ContextStatus {
  /** Tokens the model can hold, if known. */
  window: number | null;
  /** "provider": the model's own limit (Ollama: the configured context size); "preset": the preset's max history tokens. */
  windowSource: "provider" | "preset" | null;
  /** Tokens the next request would use. */
  used: number;
  /** used is estimated from the text, not the provider's own count. */
  estimated: boolean;
  summary: ConversationSummary | null;
  /** Messages the summary stands in for. */
  summarizedCount: number;
  /** Messages a compaction now would condense (0: nothing to compact). */
  compactableCount: number;
  autoCompactPercent: number;
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
  | { type: "continuing"; messageId: Uuid }
  | { type: "delta"; messageId: Uuid; text: string; reasoning: string }
  | { type: "messageUpdated"; message: ChatMessage }
  | { type: "messageRemoved"; messageId: Uuid }
  | { type: "worldStateChanged"; conversationId: Uuid; worldState: WorldState }
  | { type: "titleChanged"; conversationId: Uuid; title: string }
  | { type: "compacting"; conversationId: Uuid }
  | { type: "compacted"; conversationId: Uuid }
  | { type: "storyChanged"; conversationId: Uuid }
  | { type: "toolApproval"; requestId: Uuid; command: string; elevate: boolean }
  | { type: "error"; message: string; kind: ErrorKind | null }
  | { type: "finished" };

// ---- library (editors) ----

export type WorldInfoPosition = "beforeCharacterDescription" | "afterCharacterDescription" | "atDepth";

export interface WorldInfoEntry {
  id: Uuid;
  keys: string[];
  secondaryKeys: string[] | null;
  content: string;
  caseSensitive: boolean;
  enabled: boolean;
  insertionOrder: number;
  position: WorldInfoPosition;
  depth: number;
}

export interface Lorebook {
  id: Uuid;
  name: string;
  entries: WorldInfoEntry[];
  scanDepth: number;
  tokenBudget: number;
}

export type PromptBlockType =
  | "systemPrompt"
  | "personaDescription"
  | "characterDescription"
  | "scenario"
  | "exampleDialogue"
  | "worldInfo"
  | "chatHistory"
  | "authorsNote"
  | "custom"
  | "userMemory"
  | "postHistoryInstructions"
  | "worldState"
  | "characterNotebook"
  | "questLog";

export type ReasoningEffort = "off" | "low" | "medium" | "high";

export interface PromptBlockFull {
  id: Uuid;
  type: PromptBlockType;
  enabled: boolean;
  order: number;
  customText: string | null;
  injectionDepth: number;
}

export interface PromptPresetFull {
  id: Uuid;
  name: string;
  blocks: PromptBlockFull[];
  temperature: number | null;
  topP: number | null;
  topK: number | null;
  maxOutputTokens: number | null;
  reasoningEffort: ReasoningEffort | null;
  maxContextTokens: number | null;
}

export type GroupActivationMode = "manual" | "roundRobin" | "triggerWord";

export interface GroupChat {
  id: Uuid;
  name: string;
  characterIds: Uuid[];
  activationMode: GroupActivationMode;
  allowSelfResponses: boolean;
}

// ---- picture generation ----

export type ImageBackend = "a1111" | "comfyUi";
export type PromptStyle = "tags" | "natural";
/** What to draw: SillyTavern's modes. */
export type ImageMode = "character" | "face" | "persona" | "scene" | "lastMessage";

export interface ImageBackendConfig {
  url: string;
  model: string;
  sampler: string;
  scheduler: string;
}

export interface ImageGenSettings {
  backend: ImageBackend;
  a1111: ImageBackendConfig;
  comfy: ImageBackendConfig;
  /** ComfyUI workflow in API format with %prompt%-style placeholders; empty = built-in. */
  comfyWorkflow: string;
  width: number;
  height: number;
  steps: number;
  cfgScale: number;
  /** -1 = random each time. */
  seed: number;
  promptPrefix: string;
  negativePrompt: string;
  promptStyle: PromptStyle;
  reviewPrompt: boolean;
}

export interface ImageBackendOptions {
  models: string[];
  samplers: string[];
  schedulers: string[];
}

/** A picture prompt ready for review; editable before rendering. */
export interface ImagePromptDraft {
  mode: ImageMode;
  prompt: string;
  negativePrompt: string;
  width: number;
  height: number;
  characterId: Uuid | null;
}

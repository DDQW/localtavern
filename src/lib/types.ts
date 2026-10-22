// Mirrors the JSON the Rust backend produces (camelCase; see src-tauri/src/models and chat/).

export type Uuid = string;

/** "image" is a generated picture: shown in the chat, never sent to the model. */
export type MessageRole = "user" | "character" | "system" | "tool" | "image";
export type ConversationMode = "roleplay" | "assistant";

export interface MessageAlternate {
  text: string;
  reasoningText: string | null;
}

export interface MessageAttachment {
  imagePath: string;
  mimeType: string;
  /** What a picture the user sent shows; later turns send this instead of the picture. */
  description?: string | null;
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
  /** A picture's draft. A reply's prompt is fetched on demand (`api.messagePrompt`) when `hasPrompt`. */
  rawPromptText: string | null;
  hasPrompt: boolean;
  alternates: MessageAlternate[];
  activeAlternateIndex: number;
  attachments: MessageAttachment[];
  detectedEmotion: string | null;
  /** The story clock before this reply and what it did to it (roleplay replies). */
  clock?: ClockSnapshot | null;
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
  pictureStyle: PictureStyle;
  outfits: Outfit[];
  actionPictures: ActionPicture[];
  createdAt: string;
  updatedAt: string;
}

/** What every picture of a character shares, so a set looks like one person. */
export interface PictureStyle {
  /** Permanent physical features as image-prompt text (no clothing). */
  baseLook: string;
  /** What they wear when no other outfit is chosen. */
  defaultOutfit: string;
  /** Extra positive tags for this character (after the global picture prefix). */
  promptPrefix: string;
  /** Extra negative tags for this character (after the global negative prompt). */
  negativePrompt: string;
  /** -1 until the first picture is drawn. */
  seed: number;
}

/** A costume with its own expression pack; a chat can put the character in it. */
export interface Outfit {
  id: Uuid;
  name: string;
  clothing: string;
  expressions: ExpressionImage[];
}

export interface ActionPicture {
  action: string;
  imagePath: string;
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
  /** Character id -> outfit id; characters not listed wear their default. */
  outfits: Record<Uuid, Uuid>;
  /** Largest jump one reply may make on its own, in minutes (0 = no limit). */
  maxSkipMinutes: number;
  /** Track an arousal / climax meter per character. */
  metersEnabled: boolean;
  /** Character id -> meter. */
  meters: Record<Uuid, Meter>;
}

export interface Meter {
  arousal: number;
  climax: number;
  orgasms: number;
}

export interface ClockSnapshot {
  fictionalDateTime: string;
  advancedMinutes: number;
  /** Kept for older messages; new replies no longer move the clock. */
  estimated: boolean;
  /** Kept for older messages; new replies no longer move the clock. */
  clampedFrom: number | null;
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

/** A side character the story introduced (not on a character card). */
export interface StoryCharacter {
  id: Uuid;
  conversationId: Uuid;
  name: string;
  /** Age in years when the story introduced them. */
  age: number | null;
  role: string;
  appearance: string;
  /** Explicit (18+) details; only kept for characters aged 18 or more. */
  adultDetails: string;
  /** The in-story time of the exchange that introduced them. */
  introducedAt: string | null;
  introducedMessageId: Uuid | null;
  createdAt: string;
}

export interface HolidayOverview {
  countryName: string | null;
  regions: string[];
  holidays: DatedHoliday[];
  /** School holidays (Schulferien) in the next two weeks, for the chosen region. */
  schoolBreaks: SchoolBreak[];
}

export interface SchoolBreak {
  start: string;
  end: string;
  name: string;
  nationwide: boolean;
  regions: string[];
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
  /** In-story date and time (local, no zone) that new roleplays with this character begin at. */
  storyStart: string | null;
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
  /** Set for a private talk: the group conversation it belongs to (`characterId` is the member). */
  parentConversationId: Uuid | null;
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
  /** For a group chat and each of its private talks: the place they make up together. */
  room: Room | null;
}

/** A group conversation and its private talks, shown as tabs. */
export interface Room {
  groupConversationId: Uuid;
  /** The group's name. */
  title: string;
  members: { id: Uuid; name: string; avatarPath: string | null }[];
  /** The private talks, in member order. */
  talks: { conversationId: Uuid; characterId: Uuid; updatedAt: string }[];
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

export interface VastAiOptions {
  instanceId: number;
  port: number;
  baseUrl: string;
  contextSize: number;
  idleStopSecs: number;
  readyTimeoutSecs: number;
}

/** Models run on the phone itself by the bundled llama.cpp server. */
export interface OnDeviceOptions {
  contextSize: number;
  threads: number;
  /** Seconds the model stays loaded after the last reply; 0 = until the app closes. */
  idleStopSecs: number;
  /** Vision projector file name per model name: the model sees pictures through it. */
  projectors: Record<string, string>;
}

export interface ModelFile {
  name: string;
  fileName: string;
  sizeBytes: number;
  /** A vision projector (mmproj) rather than a model. */
  projector: boolean;
}

export interface OnDeviceStatus {
  modelsDir: string;
  files: ModelFile[];
  loaded: string | null;
  options: OnDeviceOptions;
}

export interface DownloadProgress {
  fileName: string;
  received: number;
  total: number | null;
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
  | { type: "castChanged"; conversationId: Uuid; cast: StoryCharacter[] }
  | { type: "titleChanged"; conversationId: Uuid; title: string }
  /** The character sent the user a photo in the story; the app draws it. */
  | { type: "photoSent"; conversationId: Uuid; messageId: Uuid; characterId: Uuid | null; description: string }
  /** The character sent the user a video in the story; the app renders it. */
  | { type: "videoSent"; conversationId: Uuid; messageId: Uuid; characterId: Uuid | null; description: string }
  /** A group member took the user aside: open their private talk and let them speak first. */
  | { type: "breakoutStarted"; conversationId: Uuid; breakoutId: Uuid; characterId: Uuid }
  | { type: "compacting"; conversationId: Uuid }
  | { type: "compacted"; conversationId: Uuid }
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
  | "worldState";

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
  /** A reply may hand the user's message on to another member, so several answer it (the model decides). */
  jointReplies: boolean;
  /** A member may take the user aside into a private talk on their own. */
  privateTalks: boolean;
  /** The group's own scenario; when set, it stands in for the members' own. */
  scenario: string;
  /** Where and when the group's new chats are set; unset parts fall back to the members'. */
  setting: GroupSetting;
}

export interface GroupSetting {
  countryCode: string | null;
  regionCode: string | null;
  /** Local date and time, "YYYY-MM-DDTHH:MM:SS". */
  storyStart: string | null;
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

/** The app's own roleplay instructions: the user's wording (a missing text uses the built-in one) and switches. */
export interface Instructions {
  texts: Record<string, string>;
  userPhotos: boolean;
  intimatePhotos: boolean;
  photoWords: string[];
}

/** One built-in instruction, as Settings shows it. */
export interface InstructionInfo {
  key: string;
  /** Where Settings lists it. */
  group: string;
  title: string;
  /** When the app adds it. */
  help: string;
  placeholders: string[];
  default: string;
}

export interface ImageGenSettings {
  backend: ImageBackend;
  a1111: ImageBackendConfig;
  comfy: ImageBackendConfig;
  /** ComfyUI workflow in API format with %prompt%-style placeholders; empty = built-in. */
  comfyWorkflow: string;
  /** What starts ComfyUI (its run_*.bat), run hidden when LocalTavern starts; empty = started by hand. */
  comfyLauncher: string;
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
  /** Unload Ollama's model before rendering (one graphics card shared by both). */
  unloadOllamaFirst: boolean;
  /** Draw the photos characters send in the story automatically. */
  autoPhotos: boolean;
  /** ComfyUI API workflow for story videos (`[video: ...]`). */
  comfyVideoWorkflow: string;
  /** Render the videos characters send in the story automatically. */
  autoVideos: boolean;
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
  /** For a photo sent or taken in the story: what the reply said it showed. */
  photo?: string | null;
  /** For a video sent in the story: what the reply said it showed. */
  video?: string | null;
}

/** One picture of a character's set. */
export type CharacterShot = { kind: "mood"; emotion: string } | { kind: "action"; action: string };

export interface LookDraft {
  baseLook: string;
  defaultOutfit: string;
}

export interface CharacterPicture {
  path: string;
  seed: number;
}

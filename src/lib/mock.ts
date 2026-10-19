// An in-memory stand-in for the Rust backend, used when the UI runs outside Tauri (plain browser).
// It exists so pages can be developed and checked visually without the native shell; it is never
// used inside the real app (see `inTauri` in api.ts).

import type { Api, EventHandler } from "./api";
import type {
  Character,
  ChatMessage,
  ChatSession,
  ContextStatus,
  Conversation,
  ConversationSummary,
  DefaultPersonas,
  GeminiOptions,
  GroupChat,
  HistoryEntry,
  ImageGenSettings,
  ImageMode,
  ImagePromptDraft,
  Lorebook,
  PowerShellOptions,
  PromptPresetFull,
  SessionStatus,
  UserMemory,
  OllamaOptions,
  Persona,
  ProviderInfo,
  Uuid,
} from "./types";

let counter = 0;
const uid = (): Uuid => `00000000-0000-4000-8000-${String(++counter).padStart(12, "0")}`;
const now = () => new Date().toISOString();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const SAMPLE_REPLY = `Sure — here's a quick tour.

## Highlights

- **Streaming** replies with live markdown
- \`Inline code\` and fenced blocks with syntax highlighting
- Tables, quotes and [links](https://example.com)

\`\`\`rust
fn main() {
    let greeting = "Hello, tavern!";
    println!("{greeting}");
}
\`\`\`

| Feature | Status |
|---|---|
| Swipes | done |
| Reasoning | done |

> Local media works too: use \`![clip](C:\\path\\to\\video.mp4)\`.
`;

const characters: Character[] = ["Nova", "Mira", "Dorian"].map((name, i) => ({
  id: `c0000000-0000-4000-8000-00000000000${i + 1}`,
  name,
  avatarPath: null,
  description: `${name} is a wandering star-mapper.`,
  personality: "Curious and warm.",
  scenario: "Aboard a research vessel.",
  firstMessages: [`Hello {{user}}, I'm {{char}}.`],
  exampleDialogue: "",
  tags: i === 0 ? ["sci-fi", "friendly"] : i === 1 ? ["fantasy"] : ["noir", "mystery"],
  linkedLorebookIds: [],
  expressions:
    i === 0
      ? [
          { emotion: "happy", imagePath: "happy.png" },
          { emotion: "sad", imagePath: "sad.png" },
        ]
      : [],
  galleryImagePaths: [],
  appearance: null,
  birthDate: null,
  schedule: null,
  pictureStyle: { baseLook: "", defaultOutfit: "", seed: -1 },
  outfits: [],
  actionPictures: [],
  createdAt: now(),
  updatedAt: now(),
}));

const personas: Persona[] = [{ id: "d0000000-0000-4000-8000-000000000001", name: "Alex", avatarPath: null, description: "A curious traveler.", isDefault: true, appearance: null, birthDate: null }];

const lorebooks: Lorebook[] = [
  {
    id: "b0000000-0000-4000-8000-000000000001",
    name: "Lantern Belt",
    scanDepth: 4,
    tokenBudget: 1000,
    entries: [
      {
        id: "b1000000-0000-4000-8000-000000000001",
        keys: ["lantern", "belt"],
        secondaryKeys: null,
        content: "The Lantern Belt is a ring of glowing debris.",
        caseSensitive: false,
        enabled: true,
        insertionOrder: 1,
        position: "beforeCharacterDescription",
        depth: 0,
      },
    ],
  },
];

const blocks = (types: PromptPresetFull["blocks"][number]["type"][]): PromptPresetFull["blocks"] =>
  types.map((type, order) => ({ id: uid(), type, enabled: true, order, customText: null, injectionDepth: 0 }));

const presets: PromptPresetFull[] = [
  {
    id: "p1",
    name: "Assistant (default)",
    blocks: blocks(["systemPrompt", "personaDescription", "userMemory", "chatHistory"]),
    temperature: null,
    topP: null,
    topK: null,
    maxOutputTokens: null,
    reasoningEffort: null,
    maxContextTokens: null,
  },
  {
    id: "p2",
    name: "Roleplay (default)",
    blocks: blocks(["systemPrompt", "personaDescription", "characterDescription", "scenario", "exampleDialogue", "worldInfo", "chatHistory", "postHistoryInstructions", "worldState"]),
    temperature: 0.9,
    topP: null,
    topK: null,
    maxOutputTokens: 1024,
    reasoningEffort: "low",
    maxContextTokens: null,
  },
];

const groups: GroupChat[] = [
  {
    id: "g0000000-0000-4000-8000-000000000001",
    name: "The Crew",
    characterIds: [characters[0].id, characters[1].id],
    activationMode: "triggerWord",
    allowSelfResponses: false,
  },
];

const providers: ProviderInfo[] = [
  { id: "gemini", name: "Gemini", auth: "apiKey", hasKey: true, browserSignIn: false },
  { id: "deepseek", name: "DeepSeek", auth: "apiKey", hasKey: false, browserSignIn: false },
  { id: "mistral", name: "Mistral", auth: "apiKey", hasKey: false, browserSignIn: true },
  { id: "xai-subscription", name: "SuperGrok", auth: "subscription", hasKey: false, browserSignIn: false },
  { id: "chatgpt-subscription", name: "ChatGPT", auth: "subscription", hasKey: true, browserSignIn: false },
  { id: "ollama", name: "Ollama", auth: "local", hasKey: true, browserSignIn: false },
];

const models: Record<string, string[]> = {
  gemini: ["gemini-2.5-flash", "gemini-2.5-pro"],
  deepseek: ["deepseek-chat", "deepseek-reasoner"],
  ollama: ["llama3:8b", "mistral:latest"],
};

const conversations: Conversation[] = [];
const messages = new Map<Uuid, ChatMessage[]>();

function message(conversationId: Uuid, role: ChatMessage["role"], text: string, extra: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: uid(),
    conversationId,
    role,
    speakerCharacterId: null,
    text,
    reasoningText: null,
    rawPromptText: null,
    alternates: role === "character" ? [{ text, reasoningText: extra.reasoningText ?? null, rawPrompt: null }] : [],
    activeAlternateIndex: 0,
    attachments: [],
    detectedEmotion: null,
    timestamp: now(),
    usage: null,
    ...extra,
  };
}

function conversation(overrides: Partial<Conversation>): Conversation {
  const c: Conversation = {
    id: uid(),
    title: "Assistant chat",
    mode: "assistant",
    characterId: null,
    groupChatId: null,
    personaId: personas[0].id,
    promptPresetId: null,
    providerId: "gemini",
    model: "gemini-2.5-flash",
    assistantSystemPrompt: null,
    enableGoogleSearch: null,
    worldState: null,
    createdAt: now(),
    updatedAt: now(),
    ...overrides,
  };
  conversations.push(c);
  messages.set(c.id, []);
  return c;
}

// Seed: one assistant chat with a rich reply, and one roleplay with a world clock.
{
  const a = conversation({ title: "Markdown tour" });
  messages.get(a.id)!.push(
    message(a.id, "user", "Show me what the chat can render."),
    message(a.id, "character", SAMPLE_REPLY, { reasoningText: "The user wants a feature tour, so I'll use a heading, list, code and a table." }),
  );
  const r = conversation({
    title: "Chat with Nova",
    mode: "roleplay",
    characterId: characters[0].id,
    worldState: {
      fictionalDateTime: "2025-06-14T15:45:00",
      weather: "Rain",
      weatherNotes: "light drizzle",
      defaultAdvanceMinutesPerMessage: 5,
      autoDriftWeatherEnabled: false,
      countryCode: "DE",
      regionCode: "DE-BY",
      customHolidays: [],
      outfits: {},
    },
  });
  messages.get(r.id)!.push(
    message(r.id, "character", "Hello Alex, I'm Nova.", { speakerCharacterId: characters[0].id, detectedEmotion: "happy" }),
    message(r.id, "user", "Where are we headed?"),
    message(r.id, "character", "*She unfolds a chart across the console.* Toward the Lantern Belt — three days out.", {
      speakerCharacterId: characters[0].id,
    }),
  );
}

function history(c: Conversation): HistoryEntry[] {
  return conversations
    .filter((x) =>
      c.characterId
        ? x.characterId === c.characterId && x.mode === c.mode
        : c.groupChatId
          ? x.groupChatId === c.groupChatId
          : x.mode === c.mode && !x.characterId && !x.groupChatId,
    )
    .map((x) => ({ id: x.id, label: `${x.title} · today`, updatedAt: x.updatedAt }));
}

function session(id: Uuid): ChatSession {
  const c = conversations.find((x) => x.id === id)!;
  const group = groups.find((g) => g.id === c.groupChatId);
  const cast = c.characterId
    ? characters.filter((ch) => ch.id === c.characterId)
    : group
      ? group.characterIds.map((id) => characters.find((ch) => ch.id === id)!).filter(Boolean)
      : [];
  return {
    conversation: c,
    messages: structuredClone(messages.get(id) ?? []),
    characters: cast,
    persona: personas.find((p) => p.id === c.personaId) ?? personas.find((p) => p.isDefault) ?? personas[0] ?? null,
    personas,
    group: groups.find((g) => g.id === c.groupChatId) ?? null,
    preset: { id: c.mode === "assistant" ? "p1" : "p2", name: c.mode === "assistant" ? "Assistant (default)" : "Roleplay (default)", blocks: [] },
    presets: [
      { id: "p1", name: "Assistant (default)" },
      { id: "p2", name: "Roleplay (default)" },
    ],
    history: history(c),
    providers: providers.map((p) => p.id),
    models: models[c.providerId] ?? [],
    memoryCount: 0,
  };
}

const running = new Map<Uuid, boolean>();
const sessions = new Map<string, SessionStatus>([
  ["xai-subscription", { signedIn: false, email: null }],
  ["chatgpt-subscription", { signedIn: true, email: "alex@example.com" }],
]);
let powershell: PowerShellOptions = { enabled: false, autoMode: false, allowlistPatterns: ["Get-Date*"] };
let defaultPersonas: DefaultPersonas = { roleplayPersonaId: null, assistantPersonaId: null };

function personaFor(mode: Conversation["mode"]): Uuid | null {
  const preferred = mode === "assistant" ? defaultPersonas.assistantPersonaId : defaultPersonas.roleplayPersonaId;
  if (preferred && personas.some((p) => p.id === preferred)) return preferred;
  return personas.find((p) => p.isDefault)?.id ?? personas[0]?.id ?? null;
}

/** Selects the mode's persona on a chat you land in from the nav. No-op when that mode has no default. */
function applyModePersona(conversation: Conversation) {
  const preferred = conversation.mode === "assistant" ? defaultPersonas.assistantPersonaId : defaultPersonas.roleplayPersonaId;
  if (preferred && personas.some((p) => p.id === preferred)) conversation.personaId = preferred;
}

let memories: UserMemory[] = [
  { id: "m0000000-0000-4000-8000-000000000001", content: "Lives in Berlin.", createdAt: "2026-09-01T10:00:00Z" },
  { id: "m0000000-0000-4000-8000-000000000002", content: "Prefers Rust for side projects.", createdAt: "2026-09-02T10:00:00Z" },
];
const approvals = new Map<string, (decision: { allow: boolean; alwaysPattern: string | null }) => void>();
const summaries = new Map<Uuid, ConversationSummary>();
/** Ollama-sized, so the demo meter fills up quickly. */
const MOCK_WINDOW = 8192;
const KEEP_RECENT = 6;

function uncovered(conversationId: Uuid): ChatMessage[] {
  const list = messages.get(conversationId) ?? [];
  const summary = summaries.get(conversationId);
  if (!summary) return list;
  return list.slice(list.findIndex((m) => m.id === summary.coversThroughMessageId) + 1);
}

function contextStatus(conversationId: Uuid): ContextStatus {
  const summary = summaries.get(conversationId) ?? null;
  const rest = uncovered(conversationId);
  const chars = rest.reduce((n, m) => n + m.text.length, 0) + (summary?.text.length ?? 0) + 2400;
  return {
    window: MOCK_WINDOW,
    windowSource: "provider",
    used: Math.ceil(chars / 4),
    estimated: true,
    summary: summary ? structuredClone(summary) : null,
    summarizedCount: (messages.get(conversationId)?.length ?? 0) - rest.length,
    compactableCount: Math.max(0, rest.length - KEEP_RECENT),
    autoCompactPercent: 80,
  };
}

async function stream(conversationId: Uuid, target: ChatMessage, onEvent: EventHandler, reply: string, replacing: boolean) {
  const reasoning = "Thinking about how best to answer…";
  running.set(conversationId, true);
  if (replacing) onEvent({ type: "regenerating", messageId: target.id });
  let text = "";
  const words = reply.split(/(?<=\s)/);
  onEvent({ type: "delta", messageId: target.id, text: "", reasoning });
  target.reasoningText = reasoning;
  for (const word of words) {
    if (!running.get(conversationId)) break;
    await sleep(25);
    text += word;
    onEvent({ type: "delta", messageId: target.id, text: word, reasoning: "" });
  }
  running.delete(conversationId);
  if (replacing) {
    target.alternates = target.alternates.length ? target.alternates : [{ text: target.text, reasoningText: null, rawPrompt: null }];
    target.alternates.push({ text, reasoningText: reasoning, rawPrompt: null });
    target.activeAlternateIndex = target.alternates.length - 1;
  } else {
    target.alternates = [{ text, reasoningText: reasoning, rawPrompt: null }];
  }
  target.text = text;
  onEvent({ type: "messageUpdated", message: structuredClone(target) });
}

export const mockApi: Api = {
  async bootstrap() {
    return { providers, dataDir: "(mock data — running outside Tauri)" };
  },
  async listCharacters() {
    return structuredClone(characters);
  },
  async listPersonas() {
    return structuredClone(personas);
  },
  async listConversations() {
    return structuredClone(conversations);
  },

  async openAssistant(forceNew) {
    const existing = [...conversations].reverse().find((c) => c.mode === "assistant");
    if (!forceNew && existing) {
      applyModePersona(existing);
      return session(existing.id);
    }
    return session(conversation({ personaId: personaFor("assistant") }).id);
  },
  async openCharacter(characterId, forceNew) {
    const existing = [...conversations].reverse().find((c) => c.mode === "roleplay" && c.characterId === characterId);
    if (!forceNew && existing) return session(existing.id);
    const ch = characters.find((c) => c.id === characterId)!;
    const user = personas.find((p) => p.id === personaFor("roleplay"))?.name || "Alex";
    const c = conversation({ title: `Chat with ${ch.name}`, mode: "roleplay", characterId, personaId: personaFor("roleplay") });
    messages.get(c.id)!.push(message(c.id, "character", ch.firstMessages[0].replace("{{user}}", user).replace("{{char}}", ch.name), { speakerCharacterId: ch.id }));
    return session(c.id);
  },
  async openGroup(groupId, forceNew) {
    const existing = [...conversations].reverse().find((c) => c.groupChatId === groupId);
    if (!forceNew && existing) return session(existing.id);
    const group = groups.find((g) => g.id === groupId)!;
    return session(conversation({ title: group.name, mode: "roleplay", groupChatId: groupId, personaId: personaFor("roleplay") }).id);
  },
  async openLastRoleplay() {
    const last = [...conversations].reverse().find((c) => c.mode === "roleplay");
    if (!last) return null;
    applyModePersona(last);
    return session(last.id);
  },
  async resumeConversation(id) {
    return session(id);
  },
  async newChat(id) {
    const c = conversations.find((x) => x.id === id)!;
    if (c.mode === "assistant") return mockApi.openAssistant(true);
    if (c.groupChatId) return mockApi.openGroup(c.groupChatId, true);
    return mockApi.openCharacter(c.characterId!, true);
  },
  async deleteConversation(id) {
    conversations.splice(conversations.findIndex((c) => c.id === id), 1);
    messages.delete(id);
  },
  async searchMessages(query) {
    const q = query.toLowerCase();
    return [...messages.values()]
      .flat()
      .filter((m) => m.text.toLowerCase().includes(q))
      .map((m) => ({ message: structuredClone(m), conversationTitle: conversations.find((c) => c.id === m.conversationId)?.title ?? "" }));
  },

  async sendMessage(conversationId, text, attachments, onEvent) {
    const list = messages.get(conversationId)!;
    const last = list.at(-1);
    if (!(text.trim() === "" && last?.role === "user")) {
      const user = message(conversationId, "user", text.trim(), {
        attachments: attachments.map((p) => ({ imagePath: p, mimeType: "image/png" })),
      });
      list.push(user);
      onEvent({ type: "messageAdded", message: structuredClone(user) });
    }
    const status = contextStatus(conversationId);
    if (status.compactableCount > 0 && status.used * 100 >= status.window! * status.autoCompactPercent) {
      onEvent({ type: "compacting", conversationId });
      await mockApi.compactConversation(conversationId);
      onEvent({ type: "compacted", conversationId });
    }
    const c = conversations.find((x) => x.id === conversationId)!;
    const reply = message(conversationId, "character", "", { speakerCharacterId: c.characterId });
    list.push(reply);
    onEvent({ type: "messageAdded", message: structuredClone(reply) });
    const command = /^\/ps(!?) (.+)/s.exec(text.trim());
    if (command && c.mode === "assistant") {
      // A stand-in for the assistant's PowerShell tool: "/ps Get-Date" (or "/ps! ..." for elevation).
      await stream(conversationId, reply, onEvent, "Let me check that for you.", false);
      const decision = await new Promise<{ allow: boolean; alwaysPattern: string | null }>((resolve) => {
        const requestId = uid();
        approvals.set(requestId, resolve);
        onEvent({ type: "toolApproval", requestId, command: command[2], elevate: command[1] === "!" });
      });
      const tool = message(conversationId, "tool", "");
      list.push(tool);
      onEvent({ type: "messageAdded", message: structuredClone(tool) });
      if (decision.allow) {
        onEvent({ type: "delta", messageId: tool.id, text: `$ ${command[2]}\n`, reasoning: "" });
        await sleep(500);
        onEvent({ type: "delta", messageId: tool.id, text: "Monday, 29 September 2026\n", reasoning: "" });
        tool.text = `[PowerShell tool result]\nCommand: ${command[2]}\nElevated: no\nExit code: 0\nOutput:\nMonday, 29 September 2026`;
      } else {
        tool.text = `[PowerShell tool result]\nCommand: ${command[2]}\nThe user declined to run this command.`;
      }
      tool.alternates = [{ text: tool.text, reasoningText: null, rawPrompt: null }];
      onEvent({ type: "messageUpdated", message: structuredClone(tool) });
      const answer = message(conversationId, "character", "");
      list.push(answer);
      onEvent({ type: "messageAdded", message: structuredClone(answer) });
      await stream(conversationId, answer, onEvent, decision.allow ? "It's Monday." : "Okay, I won't run it.", false);
      onEvent({ type: "finished" });
      return;
    }
    await stream(conversationId, reply, onEvent, SAMPLE_REPLY, false);
    onEvent({ type: "finished" });
  },
  async swipeNext(conversationId, messageId, onEvent) {
    const target = messages.get(conversationId)!.find((m) => m.id === messageId)!;
    if (target.activeAlternateIndex + 1 < target.alternates.length) {
      target.activeAlternateIndex += 1;
      const alt = target.alternates[target.activeAlternateIndex];
      target.text = alt.text;
      target.reasoningText = alt.reasoningText;
      onEvent({ type: "messageUpdated", message: structuredClone(target) });
    } else {
      await stream(conversationId, target, onEvent, "A *different* take on the same question, this time a little shorter.", true);
    }
    onEvent({ type: "finished" });
  },
  async pickSpeaker(conversationId, characterId, onEvent) {
    const reply = message(conversationId, "character", "", { speakerCharacterId: characterId });
    messages.get(conversationId)!.push(reply);
    onEvent({ type: "messageAdded", message: structuredClone(reply) });
    await stream(conversationId, reply, onEvent, "*leans in* Happy to weigh in — here's my take.", false);
    onEvent({ type: "finished" });
  },
  async continueMessage(conversationId, onEvent) {
    const target = messages.get(conversationId)!.at(-1);
    if (target?.role !== "character") throw "There is no reply to continue.";
    onEvent({ type: "continuing", messageId: target.id });
    running.set(conversationId, true);
    let added = "";
    for (const word of " The silence stretches on, and then, quietly, she adds a few more words.".split(/(?<=\s)/)) {
      if (!running.get(conversationId)) break;
      await sleep(25);
      added += word;
      onEvent({ type: "delta", messageId: target.id, text: word, reasoning: "" });
    }
    running.delete(conversationId);
    target.text += added;
    if (target.alternates.length) target.alternates[target.activeAlternateIndex].text = target.text;
    onEvent({ type: "messageUpdated", message: structuredClone(target) });
    onEvent({ type: "finished" });
  },
  async contextStatus(conversationId) {
    return contextStatus(conversationId);
  },
  async compactConversation(conversationId) {
    const rest = uncovered(conversationId);
    const condense = rest.slice(0, Math.max(0, rest.length - KEEP_RECENT));
    if (condense.length === 0) throw new Error("Nothing to compact yet - the newest messages are always kept in full.");
    await sleep(900);
    const previous = summaries.get(conversationId)?.text;
    const last = condense.at(-1)!;
    summaries.set(conversationId, {
      conversationId,
      text: `${previous ? previous + " " : ""}Alex and the crew talked through ${condense.length} messages' worth of plans for the voyage.`,
      coversThroughMessageId: last.id,
      coversThroughTimestamp: last.timestamp,
      updatedAt: now(),
    });
    return contextStatus(conversationId);
  },
  async clearConversationSummary(conversationId) {
    summaries.delete(conversationId);
    return contextStatus(conversationId);
  },
  async impersonate(conversationId) {
    running.set(conversationId, true);
    await sleep(600);
    const stopped = !running.get(conversationId);
    running.delete(conversationId);
    return stopped ? null : "I lean in closer and lower my voice. \"Tell me more about that.\"";
  },
  async subscriptionStatus(providerId) {
    return structuredClone(sessions.get(providerId)!);
  },
  async subscriptionSignIn(providerId) {
    await sleep(1200);
    sessions.set(providerId, { signedIn: true, email: "alex@example.com" });
    providers.find((p) => p.id === providerId)!.hasKey = true;
    return structuredClone(sessions.get(providerId)!);
  },
  async subscriptionCancelSignIn() {},
  async subscriptionSignOut(providerId) {
    sessions.set(providerId, { signedIn: false, email: null });
    providers.find((p) => p.id === providerId)!.hasKey = false;
    return structuredClone(sessions.get(providerId)!);
  },
  async getDefaultPersonas() {
    return structuredClone(defaultPersonas);
  },
  async saveDefaultPersonas(next) {
    for (const id of [next.roleplayPersonaId, next.assistantPersonaId]) {
      if (id && !personas.some((p) => p.id === id)) throw new Error("Persona not found.");
    }
    defaultPersonas = structuredClone(next);
  },
  async getPowerShellOptions() {
    return structuredClone(powershell);
  },
  async savePowerShellOptions(options) {
    powershell = structuredClone(options);
  },
  async resolveToolApproval(requestId, decision) {
    const waiting = approvals.get(requestId);
    approvals.delete(requestId);
    waiting?.(decision);
    return !!waiting;
  },
  async listMemories() {
    return structuredClone(memories);
  },
  async deleteMemory(id) {
    memories = memories.filter((m) => m.id !== id);
  },
  async clearMemories() {
    memories = [];
  },
  async swipePrevious(messageId) {
    const target = [...messages.values()].flat().find((m) => m.id === messageId)!;
    if (target.activeAlternateIndex > 0) {
      target.activeAlternateIndex -= 1;
      const alt = target.alternates[target.activeAlternateIndex];
      target.text = alt.text;
      target.reasoningText = alt.reasoningText;
    }
    return structuredClone(target);
  },
  async stopGeneration(conversationId) {
    running.set(conversationId, false);
  },
  async editMessage(messageId, text) {
    const target = [...messages.values()].flat().find((m) => m.id === messageId)!;
    target.text = text;
    if (target.alternates.length) target.alternates[target.activeAlternateIndex].text = text;
    return structuredClone(target);
  },
  async deleteMessage(messageId) {
    for (const list of messages.values()) {
      const i = list.findIndex((m) => m.id === messageId);
      if (i >= 0) list.splice(i, 1);
    }
  },

  async setConversationModel(id, providerId, model) {
    const c = conversations.find((x) => x.id === id)!;
    Object.assign(c, { providerId, model });
    return structuredClone(c);
  },
  async setConversationPreset(id, presetId) {
    const c = conversations.find((x) => x.id === id)!;
    c.promptPresetId = presetId;
    return structuredClone(c);
  },
  async setConversationPersona(id, personaId) {
    const c = conversations.find((x) => x.id === id)!;
    c.personaId = personaId;
    return structuredClone(c);
  },
  async setAssistantPrompt(id, text) {
    const c = conversations.find((x) => x.id === id)!;
    c.assistantSystemPrompt = text;
    return structuredClone(c);
  },
  async setWorldState(id, worldState) {
    const c = conversations.find((x) => x.id === id)!;
    if (c.mode !== "roleplay") throw "Only roleplays have an in-story clock.";
    const saved = {
      ...worldState,
      defaultAdvanceMinutesPerMessage: Math.max(1, worldState.defaultAdvanceMinutesPerMessage),
      weatherNotes: worldState.weatherNotes?.trim() ? worldState.weatherNotes : null,
    };
    c.worldState = saved;
    return structuredClone(saved);
  },
  async setSearchOverride(id, enabled) {
    const c = conversations.find((x) => x.id === id)!;
    c.enableGoogleSearch = enabled;
    return structuredClone(c);
  },

  async listModels(providerId) {
    return models[providerId] ?? [];
  },
  async refreshModels(providerId) {
    await sleep(400);
    return models[providerId] ?? [];
  },
  async setApiKey(providerId, key) {
    const p = providers.find((x) => x.id === providerId)!;
    p.hasKey = key.trim().length > 0;
    return structuredClone(p);
  },
  async browserKeySignIn(providerId) {
    await sleep(1200);
    const p = providers.find((x) => x.id === providerId)!;
    p.hasKey = true;
    return structuredClone(p);
  },
  async getGeminiOptions(): Promise<GeminiOptions> {
    return {
      enableGoogleSearch: false,
      safetyThresholds: { harassment: "off", hateSpeech: "off", sexuallyExplicit: "off", dangerousContent: "off" },
    };
  },
  async saveGeminiOptions() {},
  async getOllamaOptions(): Promise<OllamaOptions> {
    return { baseUrl: "http://localhost:11434", contextSize: 8192 };
  },
  async saveOllamaOptions() {},

  async storeImage(path) {
    return path;
  },
  async deleteImage() {
    return true;
  },

  async saveCharacter(character) {
    const i = characters.findIndex((c) => c.id === character.id);
    const saved = { ...structuredClone(character), name: character.name.trim(), updatedAt: now() };
    if (i >= 0) characters[i] = saved;
    else characters.push(saved);
    return structuredClone(saved);
  },
  async deleteCharacter(id) {
    characters.splice(characters.findIndex((c) => c.id === id), 1);
  },
  async importCharacter() {
    const imported: Character = { ...structuredClone(characters[0]), id: uid(), name: "Imported Card", tags: ["imported"] };
    characters.push(imported);
    return structuredClone(imported);
  },
  async exportCharacterCard() {},
  async importExpressionFolder() {
    return [
      { emotion: "happy", imagePath: "C:\\mock\\sprites\\happy.png" },
      { emotion: "angry", imagePath: "C:\\mock\\sprites\\angry.png" },
    ];
  },

  async savePersona(persona) {
    if (persona.isDefault) personas.forEach((p) => (p.isDefault = false));
    const i = personas.findIndex((p) => p.id === persona.id);
    if (i >= 0) personas[i] = structuredClone(persona);
    else personas.push(structuredClone(persona));
    return structuredClone(persona);
  },
  async deletePersona(id) {
    personas.splice(personas.findIndex((p) => p.id === id), 1);
  },

  // A rough stand-in for the Rust template (prompting/appearance.rs).
  async renderAppearance(a, subject) {
    const who = subject === "persona" ? "{{user}}" : "{{char}}";
    const noun = a.sex === "female" ? "woman" : a.sex === "male" ? "man" : "person";
    const picks = [a.heightCm && `${a.heightCm} cm`, a.build, a.hairLength, a.hairColor && `${a.hairColor} hair`, a.hairStyle, a.eyeColor && `${a.eyeColor} eyes`, a.skinTone && `${a.skinTone} skin`];
    const sexed = a.sex === "female" ? [a.female.cupSize && `${a.female.cupSize}-cup`, a.female.hips && `${a.female.hips} hips`] : a.sex === "male" ? [a.male.shoulders && `${a.male.shoulders} shoulders`, a.male.facialHair] : [];
    const rest = [...picks, ...sexed, ...a.marks].filter(Boolean);
    return `${who} is a ${noun}${rest.length ? `: ${rest.join(", ")}` : ""}.`;
  },
  async polishAppearance(text) {
    await sleep(600);
    return `${text.trim()} (polished)`;
  },

  async draftScenario(character, idea) {
    await sleep(500);
    return `{{char}} and {{user}} meet ${idea.trim() || "aboard the research vessel"}. ${character.name} is wary but curious.`;
  },
  async draftGreetings(_character, _idea, count) {
    await sleep(500);
    return Array.from({ length: count }, (_, i) => `*Greeting ${i + 1}: {{char}} looks up as {{user}} enters.* "Oh — you're early."`);
  },
  async draftLoreEntries(request) {
    await sleep(500);
    return Array.from({ length: request.count }, (_, i) => ({
      id: uid(),
      keys: [`${request.topic.trim() || "topic"} ${i + 1}`],
      secondaryKeys: null,
      content: `Reference notes about ${request.topic.trim()} (${i + 1}).`,
      caseSensitive: false,
      enabled: true,
      insertionOrder: 0,
      position: "beforeCharacterDescription" as const,
      depth: 0,
    }));
  },
  async draftSchedule(character) {
    await sleep(500);
    const work = { kind: "work" as const, blocks: [{ start: "08:00", end: "16:00", activity: "work", place: "the bakery" }, { start: "18:00", end: "19:30", activity: "gym", place: null }] };
    const off = { kind: "off" as const, blocks: [{ start: "10:00", end: "12:00", activity: "brunch", place: null }] };
    return {
      homeCountry: character.schedule?.homeCountry ?? null,
      homeRegion: character.schedule?.homeRegion ?? null,
      days: [work, work, work, work, work, off, off].map((d) => structuredClone(d)),
      holiday: structuredClone(off),
      observesHolidays: true,
      notes: null,
    };
  },

  async holidayCountries() {
    return [
      { countryCode: "AT", name: "Austria" },
      { countryCode: "DE", name: "Germany" },
      { countryCode: "JP", name: "Japan" },
      { countryCode: "US", name: "United States" },
    ];
  },
  async holidayOverview(id) {
    const ws = conversations.find((c) => c.id === id)?.worldState;
    const base = ws ? ws.fictionalDateTime.slice(0, 10) : "2025-06-14";
    const plus = (days: number) => {
      const d = new Date(`${base}T12:00:00`);
      d.setDate(d.getDate() + days);
      return d.toISOString().slice(0, 10);
    };
    return {
      countryName: ws?.countryCode === "DE" ? "Germany" : (ws?.countryCode ?? null),
      regions: ws?.countryCode === "DE" ? ["DE-BB", "DE-BE", "DE-BW", "DE-BY"] : [],
      holidays: [
        ...(ws?.countryCode ? [{ date: plus(5), name: "Corpus Christi", dayOff: true }] : []),
        ...Array.from({ length: 15 }, (_, i) => plus(i)).flatMap((date) =>
          (ws?.customHolidays ?? [])
            .filter((h) => Number(date.slice(5, 7)) === h.month && Number(date.slice(8, 10)) === h.day && (h.year === null || Number(date.slice(0, 4)) === h.year))
            .map((h) => ({ date, name: h.name, dayOff: true })),
        ),
      ].sort((a, b) => a.date.localeCompare(b.date)),
    };
  },

  async listLorebooks() {
    return structuredClone(lorebooks);
  },
  async saveLorebook(book) {
    const i = lorebooks.findIndex((b) => b.id === book.id);
    if (i >= 0) lorebooks[i] = structuredClone(book);
    else lorebooks.push(structuredClone(book));
    return structuredClone(book);
  },
  async deleteLorebook(id) {
    lorebooks.splice(lorebooks.findIndex((b) => b.id === id), 1);
  },

  async listPresets() {
    return structuredClone(presets);
  },
  async savePreset(preset) {
    const saved = { ...structuredClone(preset), name: preset.name.trim() || "Untitled preset" };
    saved.blocks.forEach((b, order) => (b.order = order));
    const i = presets.findIndex((p) => p.id === preset.id);
    if (i >= 0) presets[i] = saved;
    else presets.push(saved);
    return structuredClone(saved);
  },
  async deletePreset(id) {
    presets.splice(presets.findIndex((p) => p.id === id), 1);
  },

  async listGroupChats() {
    return structuredClone(groups);
  },
  async saveGroupChat(group) {
    const i = groups.findIndex((g) => g.id === group.id);
    if (i >= 0) groups[i] = structuredClone(group);
    else groups.push(structuredClone(group));
    return structuredClone(group);
  },
  async deleteGroupChat(id) {
    groups.splice(groups.findIndex((g) => g.id === id), 1);
  },

  async getImageSettings() {
    return structuredClone(imageSettings);
  },
  async saveImageSettings(next) {
    imageSettings = structuredClone(next);
    return structuredClone(imageSettings);
  },
  async imageBackendOptions(settings) {
    await sleep(400);
    const url = settings.backend === "a1111" ? settings.a1111.url : settings.comfy.url;
    if (!url.includes("127.0.0.1") && !url.includes("localhost")) throw `Couldn't reach the server at ${url} - is it running?`;
    return settings.backend === "a1111"
      ? { models: ["sdxl_base_1.0.safetensors [31e35c80fc]", "ponyDiffusionV6XL.safetensors [67ab2fd8ec]"], samplers: ["Euler a", "Euler", "DPM++ 2M", "DPM++ SDE"], schedulers: ["Automatic", "Karras", "Exponential"] }
      : { models: ["sdxl_base_1.0.safetensors", "illustriousXL.safetensors"], samplers: ["euler", "euler_ancestral", "dpmpp_2m"], schedulers: ["normal", "karras", "sgm_uniform"] };
  },
  async describeCharacterLook(character) {
    await sleep(700);
    if (!character.description.trim()) throw "Write a description first - the look is taken from it.";
    return { baseLook: "1girl, young woman, slender, pale skin, long silver hair, blue eyes, freckles", defaultOutfit: "navy flight jacket, white shirt, dark trousers, boots" };
  },
  async drawCharacterPicture(character, shot, outfitId) {
    const style = character.pictureStyle;
    if (!style.baseLook.trim()) throw "Fill in the base look first (or let the AI write it from the description).";
    const outfit = character.outfits.find((o) => o.id === outfitId);
    const clothing = outfit ? outfit.clothing || outfit.name : style.defaultOutfit;
    const what = shot.kind === "mood" ? `${shot.emotion}` : shot.action;
    imageRunning.set(character.id, true);
    for (let i = 0; i < 8; i++) {
      await sleep(100);
      if (!imageRunning.get(character.id)) {
        imageRunning.delete(character.id);
        return null;
      }
    }
    imageRunning.delete(character.id);
    const square = shot.kind === "mood";
    const draft: ImagePromptDraft = {
      mode: square ? "face" : "character",
      prompt: `${what}, ${clothing}`,
      negativePrompt: "",
      width: square ? 512 : 416,
      height: square ? 512 : 608,
      characterId: character.id,
    };
    return { path: mockPicture(draft), seed: style.seed >= 0 ? style.seed : Math.floor(Math.random() * 2 ** 32) };
  },
  async stopCharacterPicture(characterId) {
    imageRunning.set(characterId, false);
  },
  async imagePrompt(conversationId, mode, characterId) {
    running.set(conversationId, true);
    await sleep(700);
    const stopped = !running.get(conversationId);
    running.delete(conversationId);
    if (stopped) return null;
    const c = conversations.find((x) => x.id === conversationId)!;
    const cast = c.characterId ? characters.filter((x) => x.id === c.characterId) : [];
    const subject = mode === "character" || mode === "face" ? (cast.find((x) => x.id === characterId) ?? cast[0]) : undefined;
    if ((mode === "character" || mode === "face") && !subject) throw "This chat has no character to draw.";
    const square = mode === "face";
    const landscape = mode === "scene" || mode === "lastMessage";
    const descriptions: Record<ImageMode, string> = {
      character: "full body portrait, young woman, silver hair, blue eyes, navy flight jacket, star charts, standing on a ship bridge",
      face: "close-up portrait, young woman, silver hair, freckles, blue eyes, curious smile",
      persona: "full body portrait, man, short dark hair, grey coat, scarf",
      scene: "scenery, research vessel bridge, night, starfield through windows, two people at a chart table",
      lastMessage: "scenery, observation deck, nebula glow, woman pointing at a star map",
    };
    const prefix = subject?.pictureStyle.baseLook ?? "";
    const draft: ImagePromptDraft = {
      mode,
      prompt: [imageSettings.promptPrefix, prefix, descriptions[mode]].filter((p) => p && p.trim()).join(", "),
      negativePrompt: imageSettings.negativePrompt,
      width: square ? 1024 : landscape ? Math.max(imageSettings.width, imageSettings.height) : imageSettings.width,
      height: square ? 1024 : landscape ? Math.min(imageSettings.width, imageSettings.height) : imageSettings.height,
      characterId: subject?.id ?? null,
    };
    return draft;
  },
  async generateImage(conversationId, draft) {
    imageRunning.set(conversationId, true);
    for (let i = 0; i < 15; i++) {
      await sleep(100);
      if (!imageRunning.get(conversationId)) {
        imageRunning.delete(conversationId);
        return null;
      }
    }
    imageRunning.delete(conversationId);
    const picture = message(conversationId, "image", draft.prompt.trim(), {
      speakerCharacterId: draft.characterId,
      rawPromptText: JSON.stringify(draft),
      attachments: [{ imagePath: mockPicture(draft), mimeType: "image/svg+xml" }],
    });
    messages.get(conversationId)!.push(picture);
    return structuredClone(picture);
  },
  async stopImage(conversationId) {
    imageRunning.set(conversationId, false);
  },
  async deletePicture(messageId) {
    for (const list of messages.values()) {
      const i = list.findIndex((m) => m.id === messageId && m.role === "image");
      if (i >= 0) list.splice(i, 1);
    }
  },
};

let imageSettings: ImageGenSettings = {
  backend: "a1111",
  a1111: { url: "http://127.0.0.1:7860", model: "", sampler: "Euler a", scheduler: "" },
  comfy: { url: "http://127.0.0.1:8188", model: "", sampler: "euler", scheduler: "normal" },
  comfyWorkflow: "",
  width: 832,
  height: 1216,
  steps: 25,
  cfgScale: 7,
  seed: -1,
  promptPrefix: "best quality, highly detailed",
  negativePrompt: "lowres, bad anatomy, bad hands, extra fingers, blurry, watermark, text, signature",
  promptStyle: "tags",
  reviewPrompt: true,
};
const imageRunning = new Map<Uuid, boolean>();

/** A stand-in "render": a gradient at the draft's aspect ratio with its first tags written on it. */
function mockPicture(draft: ImagePromptDraft): string {
  const hue = Math.floor(Math.random() * 360);
  const label = draft.prompt.split(",").slice(0, 3).join(",").replace(/[<>&"]/g, "");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${draft.width}" height="${draft.height}" viewBox="0 0 ${draft.width} ${draft.height}">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue},55%,35%)"/><stop offset="1" stop-color="hsl(${(hue + 80) % 360},60%,18%)"/></linearGradient></defs>
<rect width="100%" height="100%" fill="url(#g)"/>
<text x="50%" y="50%" fill="white" font-family="sans-serif" font-size="${Math.round(draft.width / 24)}" text-anchor="middle">${label}</text></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

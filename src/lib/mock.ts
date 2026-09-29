// An in-memory stand-in for the Rust backend, used when the UI runs outside Tauri (plain browser).
// It exists so pages can be developed and checked visually without the native shell; it is never
// used inside the real app (see `inTauri` in api.ts).

import type { Api, EventHandler } from "./api";
import type {
  Character,
  ChatMessage,
  ChatSession,
  Conversation,
  GeminiOptions,
  HistoryEntry,
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
  createdAt: now(),
  updatedAt: now(),
}));

const personas: Persona[] = [{ id: "d0000000-0000-4000-8000-000000000001", name: "Alex", avatarPath: null, description: "A curious traveler.", isDefault: true }];

const providers: ProviderInfo[] = [
  { id: "gemini", name: "Gemini", auth: "apiKey", hasKey: true },
  { id: "deepseek", name: "DeepSeek", auth: "apiKey", hasKey: false },
  { id: "ollama", name: "Ollama", auth: "local", hasKey: true },
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
    .filter((x) => (c.characterId ? x.characterId === c.characterId : x.mode === c.mode && !x.characterId))
    .map((x) => ({ id: x.id, label: `${x.title} · today`, updatedAt: x.updatedAt }));
}

function session(id: Uuid): ChatSession {
  const c = conversations.find((x) => x.id === id)!;
  const cast = c.characterId ? characters.filter((ch) => ch.id === c.characterId) : [];
  return {
    conversation: c,
    messages: structuredClone(messages.get(id) ?? []),
    characters: cast,
    persona: personas[0],
    personas,
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
    return session(!forceNew && existing ? existing.id : conversation({}).id);
  },
  async openCharacter(characterId, forceNew) {
    const existing = [...conversations].reverse().find((c) => c.characterId === characterId);
    if (!forceNew && existing) return session(existing.id);
    const ch = characters.find((c) => c.id === characterId)!;
    const c = conversation({ title: `Chat with ${ch.name}`, mode: "roleplay", characterId });
    messages.get(c.id)!.push(message(c.id, "character", ch.firstMessages[0].replace("{{user}}", "Alex").replace("{{char}}", ch.name), { speakerCharacterId: ch.id }));
    return session(c.id);
  },
  async openLastRoleplay() {
    const last = [...conversations].reverse().find((c) => c.mode === "roleplay");
    return last ? session(last.id) : null;
  },
  async resumeConversation(id) {
    return session(id);
  },
  async newChat(id) {
    const c = conversations.find((x) => x.id === id)!;
    return c.characterId ? mockApi.openCharacter(c.characterId, true) : mockApi.openAssistant(true);
  },
  async deleteConversation(id) {
    conversations.splice(conversations.findIndex((c) => c.id === id), 1);
    messages.delete(id);
  },
  async storyState(id) {
    return {
      quests: [
        { id: uid(), conversationId: id, title: "Reach the Lantern Belt", description: "Three days out on the current heading.", status: "active", createdAt: now(), updatedAt: now() },
        { id: uid(), conversationId: id, title: "Repair the coolant loop", description: "", status: "completed", createdAt: now(), updatedAt: now() },
      ],
      notebook: [{ id: uid(), conversationId: id, characterId: characters[0].id, content: "Alex prefers the window seat.", createdAt: now() }],
    };
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
    const c = conversations.find((x) => x.id === conversationId)!;
    const reply = message(conversationId, "character", "", { speakerCharacterId: c.characterId });
    list.push(reply);
    onEvent({ type: "messageAdded", message: structuredClone(reply) });
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
};

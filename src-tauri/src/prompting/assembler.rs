//! Merges a conversation's enabled/ordered prompt blocks into one [`LlmRequest`].
//!
//! Blocks before the ChatHistory block collapse into the provider's single system-instruction slot
//! (in the user's configured order); a block placed after ChatHistory is injected as a synthetic
//! message at its configured depth from the end, mirroring SillyTavern's author's-note-at-depth
//! behavior. Provider-agnostic: only touches `LlmRequest`/`LlmMessage`. Such injected messages are
//! marked as system notes, since every other user-role message is the user's character speaking.

use std::collections::BTreeMap;

use crate::llm::{LlmAttachment, LlmGenerationConfig, LlmMessage, LlmRequest, LlmRole};
use crate::models::{
    Character, ConversationMode, MessageRole, Persona, PromptBlock, PromptBlockType, UserMemory, WorldInfoPosition,
};

use super::context::PromptAssemblyContext;
use super::instructions::{self, Instructions};
use super::roles::{join_names, picture_note, user_name};
use super::world_info::{WorldInfoMatch, WorldInfoScanner};

pub fn assemble(ctx: &PromptAssemblyContext, scanner: &dyn WorldInfoScanner) -> LlmRequest {
    let macros = Macros::new(ctx.persona, ctx.characters, ctx.speaker);
    let mut blocks: Vec<&PromptBlock> = ctx.preset.blocks.iter().filter(|b| b.enabled).collect();
    blocks.sort_by_key(|b| b.order); // stable, like LINQ OrderBy

    // World info flows only when the preset contains an enabled WorldInfo block. Each match's
    // position/depth then decides where it lands, so the block itself emits no text - toggling it
    // off suppresses all world info, mirroring a preset without the block.
    let world_info_active = blocks.iter().any(|b| b.block_type == PromptBlockType::WorldInfo);
    let matches = if world_info_active && !ctx.lorebooks.is_empty() {
        scanner.scan(ctx.lorebooks, ctx.history)
    } else {
        Vec::new()
    };

    let mut system_parts: Vec<String> = Vec::new();
    let mut history: Vec<LlmMessage> = Vec::new();
    let mut post_history_blocks: Vec<&PromptBlock> = Vec::new();
    let mut history_seen = false;
    let mut character_description_emitted = false;

    for block in blocks {
        match block.block_type {
            PromptBlockType::ChatHistory => {
                history_seen = true;
                history = build_history(ctx, &macros);
                inject_world_info_at_depth(ctx, &mut history, &matches, &macros);
                continue;
            }
            PromptBlockType::WorldInfo => continue,
            _ => {}
        }

        if history_seen {
            post_history_blocks.push(block);
            continue;
        }

        let is_character_description = block.block_type == PromptBlockType::CharacterDescription;
        if is_character_description {
            character_description_emitted = true;
            add_world_info_parts(&mut system_parts, &matches, WorldInfoPosition::BeforeCharacterDescription, &macros);
        }

        let text = macros.fill(&resolve_block_text(block, ctx, &macros));
        if !text.trim().is_empty() {
            system_parts.push(text);
        }

        if is_character_description {
            add_world_info_parts(&mut system_parts, &matches, WorldInfoPosition::AfterCharacterDescription, &macros);
        }
    }

    // Presets without a CharacterDescription block still surface Before/After world info, appended
    // at the end of the pre-history system prompt in that order.
    if !character_description_emitted {
        add_world_info_parts(&mut system_parts, &matches, WorldInfoPosition::BeforeCharacterDescription, &macros);
        add_world_info_parts(&mut system_parts, &matches, WorldInfoPosition::AfterCharacterDescription, &macros);
    }

    for block in post_history_blocks {
        let text = macros.fill(&resolve_block_text(block, ctx, &macros));
        if text.trim().is_empty() {
            continue;
        }
        let index = history.len().saturating_sub(block.injection_depth.max(0) as usize);
        history.insert(index, LlmMessage::new(LlmRole::User, ctx.instructions.note(&text)));
    }

    let preset = ctx.preset;
    LlmRequest {
        model: ctx.conversation.model.clone(),
        system_instruction: if system_parts.is_empty() { None } else { Some(system_parts.join("\n\n")) },
        history,
        generation_config: LlmGenerationConfig {
            temperature: preset.temperature,
            top_p: preset.top_p,
            top_k: preset.top_k,
            max_output_tokens: preset.max_output_tokens,
            stop_sequences: None,
            reasoning_effort: preset.reasoning_effort,
        },
        enable_web_search: ctx.conversation.enable_google_search,
    }
}

/// Joins matched entries with one position into the system prompt as a single part, after macro substitution.
fn add_world_info_parts(
    system_parts: &mut Vec<String>,
    matches: &[WorldInfoMatch],
    position: WorldInfoPosition,
    macros: &Macros,
) {
    let text = matches
        .iter()
        .filter(|m| m.position == position)
        .map(|m| macros.fill(&m.content))
        .filter(|t| !t.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.trim().is_empty() {
        system_parts.push(text);
    }
}

/// Injects AtDepth matches into the message history as synthetic user messages (system notes).
/// Entries sharing a depth are merged into one message so they keep scanner priority order; deeper
/// groups are injected first so shallower groups stay closer to the end.
fn inject_world_info_at_depth(ctx: &PromptAssemblyContext, history: &mut Vec<LlmMessage>, matches: &[WorldInfoMatch], macros: &Macros) {
    let mut groups: BTreeMap<i32, Vec<&WorldInfoMatch>> = BTreeMap::new();
    for m in matches.iter().filter(|m| m.position == WorldInfoPosition::AtDepth) {
        groups.entry(m.depth).or_default().push(m);
    }

    for (depth, group) in groups.into_iter().rev() {
        let text = group
            .iter()
            .map(|m| macros.fill(&m.content))
            .filter(|t| !t.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() {
            let index = history.len().saturating_sub(depth.max(0) as usize);
            history.insert(index, LlmMessage::new(LlmRole::User, ctx.instructions.note(&text)));
        }
    }
}

fn build_history(ctx: &PromptAssemblyContext, macros: &Macros) -> Vec<LlmMessage> {
    let prefix_speaker = ctx.characters.len() > 1;
    let latest_user = ctx.history.iter().rposition(|m| m.role == MessageRole::User);

    let messages: Vec<LlmMessage> = ctx
        .history
        .iter()
        .enumerate()
        .map(|(index, message)| {
            // Providers only understand a two-party user/model turn sequence - a Tool message (the
            // PowerShell result fed back after a <powershell-run> call) has to be sent as if the user
            // reported it, exactly like everything else that isn't the model's own Character turn.
            // A System message is something the app noted in the story (a private talk in a group),
            // sent as a system note like the app's other user-role insertions.
            let role = match message.role {
                MessageRole::User | MessageRole::Tool | MessageRole::System => LlmRole::User,
                _ => LlmRole::Model,
            };
            let mut text = macros.fill(message.effective_text());
            if message.role == MessageRole::System {
                text = ctx.instructions.note(&text);
            }

            if prefix_speaker && message.role == MessageRole::Character {
                if let Some(speaker) = message
                    .speaker_character_id
                    .and_then(|id| ctx.characters.iter().find(|c| c.id == id))
                {
                    text = format!("{}: {text}", speaker.name);
                }
            }

            // A picture the user sent goes out for real while its message is the latest one. After
            // that its description stands in, once it has one, so it isn't decoded on every reply.
            let descriptions: Option<Vec<&str>> =
                message.attachments.iter().map(|a| a.description.as_deref().filter(|d| !d.trim().is_empty())).collect();
            let described = message.role == MessageRole::User && latest_user.is_some_and(|latest| index < latest);
            let attachments = if message.attachments.is_empty() {
                None
            } else if let Some(descriptions) = descriptions.filter(|_| described) {
                text = format!("{}\n\n{}", text.trim_end(), picture_note(ctx.instructions, &descriptions)).trim_start().to_string();
                None
            } else {
                Some(
                    message
                        .attachments
                        .iter()
                        .map(|a| LlmAttachment { image_path: a.image_path.clone(), mime_type: a.mime_type.clone() })
                        .collect(),
                )
            };

            LlmMessage { role, text, attachments }
        })
        .collect();

    trim_to_context_budget(messages, ctx.preset.max_context_tokens)
}

/// Roughly 4 characters per token for Latin-script text - close enough to keep a long conversation's
/// per-request cost bounded without a round trip to the provider's real tokenizer. Drops whole
/// messages from the oldest end, never truncates inside one, and always keeps at least the most
/// recent message even if it alone exceeds the budget.
fn trim_to_context_budget(messages: Vec<LlmMessage>, max_context_tokens: Option<i32>) -> Vec<LlmMessage> {
    let Some(budget) = max_context_tokens.filter(|b| *b > 0) else {
        return messages;
    };
    const APPROX_CHARS_PER_TOKEN: i64 = 4;
    let char_budget = budget as i64 * APPROX_CHARS_PER_TOKEN;

    let mut kept_rev: Vec<LlmMessage> = Vec::new();
    let mut used = 0i64;
    for message in messages.into_iter().rev() {
        let length = message.text.chars().count() as i64;
        if used + length > char_budget && !kept_rev.is_empty() {
            break;
        }
        used += length;
        kept_rev.push(message);
    }
    kept_rev.reverse();
    kept_rev
}

fn resolve_block_text(block: &PromptBlock, ctx: &PromptAssemblyContext, macros: &Macros) -> String {
    let custom = block.custom_text.as_deref();
    match block.block_type {
        // Assistant mode's SystemPrompt block normally reflects the chat page's inline,
        // per-conversation text box - but a preset that defines its own custom text for this block
        // takes priority, so switching presets actually switches prompts.
        PromptBlockType::SystemPrompt => {
            if ctx.conversation.mode == ConversationMode::Assistant {
                match custom.filter(|t| !t.trim().is_empty()) {
                    Some(t) => t.to_string(),
                    None => ctx.conversation.assistant_system_prompt.clone().unwrap_or_default(),
                }
            } else {
                custom.unwrap_or_default().to_string()
            }
        }
        PromptBlockType::PersonaDescription => resolve_persona_description(ctx.instructions, ctx.persona),
        PromptBlockType::CharacterDescription => {
            join_per_character(ctx.instructions, ctx.characters, macros, |c| description_with_personality(ctx.instructions, c), "description")
        }
        PromptBlockType::Scenario => match ctx.scenario {
            Some(scenario) => scenario.trim().to_string(),
            None => join_per_character(ctx.instructions, ctx.characters, macros, |c| c.scenario.clone(), "scenario"),
        },
        PromptBlockType::ExampleDialogue => {
            join_per_character(ctx.instructions, ctx.characters, macros, |c| c.example_dialogue.clone(), "example dialogue")
        }
        PromptBlockType::AuthorsNote => ctx.authors_note_text.or(custom).unwrap_or_default().to_string(),
        PromptBlockType::Custom | PromptBlockType::PostHistoryInstructions => custom.unwrap_or_default().to_string(),
        PromptBlockType::UserMemory => format_user_memories(ctx.instructions, ctx.user_memories),
        PromptBlockType::WorldState => ctx.world_state_text.or(custom).unwrap_or_default().to_string(),
        // World info placement is per-entry (handled outside block resolution); history is not text.
        PromptBlockType::WorldInfo | PromptBlockType::ChatHistory => String::new(),
    }
}

/// SillyTavern character cards are written with literal `{{user}}`/`{{char}}` placeholders
/// throughout, expected to be substituted wherever they appear - sent as-is they read as a broken
/// template. Public and taking just persona/characters so callers outside prompt assembly (e.g. the
/// greeting shown the moment a roleplay starts) can substitute once at creation time.
pub fn apply_macros(text: &str, persona: Option<&Persona>, characters: &[Character]) -> String {
    Macros::new(persona, characters, None).fill(text)
}

/// What `{{user}}` and `{{char}}` stand for while assembling one request.
struct Macros {
    user: String,
    /// `None` leaves `{{char}}` as written: there is no character to name.
    char: Option<String>,
}

impl Macros {
    /// `{{char}}` is whoever this reply is for, else the only character, else the whole cast
    /// ("Alice and Bob") - a literal `{{char}}` reaching the model would leave it guessing who's meant.
    fn new(persona: Option<&Persona>, characters: &[Character], speaker: Option<&Character>) -> Self {
        let char = match (speaker, characters) {
            (Some(me), _) => Some(me.name.clone()),
            (None, []) => None,
            (None, cast) => Some(join_names(&cast.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), "and")),
        };
        Self { user: user_name(persona).to_string(), char }
    }

    /// The same, but with `{{char}}` meaning `character` - inside a card's own text it always means
    /// that card, even in a group.
    fn for_character(&self, character: &Character) -> Self {
        Self { user: self.user.clone(), char: Some(character.name.clone()) }
    }

    fn fill(&self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        let result = replace_ignore_ascii_case(text, "{{user}}", &self.user);
        match &self.char {
            Some(name) => replace_ignore_ascii_case(&result, "{{char}}", name),
            None => result,
        }
    }
}

/// Case-insensitive replace for an ASCII `needle`. ASCII lowercasing never changes byte lengths, so
/// offsets found in the lowered copy are valid in the original.
fn replace_ignore_ascii_case(haystack: &str, needle: &str, replacement: &str) -> String {
    let lowered = haystack.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut last = 0;
    for (start, _) in lowered.match_indices(&needle) {
        out.push_str(&haystack[last..start]);
        out.push_str(replacement);
        last = start + needle.len();
    }
    out.push_str(&haystack[last..]);
    out
}

fn format_user_memories(ins: &Instructions, memories: &[UserMemory]) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = memories.iter().map(|m| format!("- {}", m.content)).collect();
    ins.render(instructions::MEMORIES_HEADER, &[("memories", &lines.join("\n"))])
}

fn resolve_persona_description(ins: &Instructions, persona: Option<&Persona>) -> String {
    match persona {
        Some(p) if !p.description.trim().is_empty() => {
            ins.render(instructions::PERSONA_LABEL, &[("user", user_name(persona)), ("description", p.description.trim())])
        }
        _ => String::new(),
    }
}

/// The card's description followed by its personality, SillyTavern-style ("{{char}}'s personality:
/// ..."). Presets have no block of their own for the personality, so it rides with the description.
fn description_with_personality(ins: &Instructions, c: &Character) -> String {
    // `{{char}}` stays a macro here: it's filled with this card's name like the rest of the card.
    let label = |personality: &str| ins.render(instructions::PERSONALITY_LABEL, &[("char", "{{char}}"), ("personality", personality)]);
    match (c.description.trim(), c.personality.trim()) {
        (description, "") => description.to_string(),
        ("", personality) => label(personality),
        (description, personality) => format!("{description}\n{}", label(personality)),
    }
}

/// One field from every character's card, each with its own `{{char}}` filled in.
fn join_per_character(
    ins: &Instructions,
    characters: &[Character],
    macros: &Macros,
    select: impl Fn(&Character) -> String,
    label: &str,
) -> String {
    match characters {
        [] => String::new(),
        [only] => macros.for_character(only).fill(select(only).trim()),
        many => many
            .iter()
            .map(|c| (c, select(c)))
            .filter(|(_, text)| !text.trim().is_empty())
            .map(|(c, text)| {
                let text = macros.for_character(c).fill(&text);
                ins.render(instructions::CAST_LABEL, &[("char", &c.name), ("field", label), ("text", &text)])
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChatMessage, Conversation, Lorebook, MessageAttachment, PromptPreset};
    use crate::prompting::defaults::{create_assistant_preset, create_roleplay_preset};

    struct NullScanner;
    impl WorldInfoScanner for NullScanner {
        fn scan(&self, _: &[Lorebook], _: &[ChatMessage]) -> Vec<WorldInfoMatch> {
            Vec::new()
        }
    }

    struct StubScanner(Vec<WorldInfoMatch>);
    impl WorldInfoScanner for StubScanner {
        fn scan(&self, _: &[Lorebook], _: &[ChatMessage]) -> Vec<WorldInfoMatch> {
            self.0.clone()
        }
    }

    fn wi(content: &str, position: WorldInfoPosition, depth: i32) -> WorldInfoMatch {
        WorldInfoMatch { content: content.into(), position, depth }
    }

    fn roleplay() -> Conversation {
        Conversation { mode: ConversationMode::Roleplay, model: "m".into(), ..Default::default() }
    }

    fn assistant() -> Conversation {
        Conversation { mode: ConversationMode::Assistant, model: "gemini-2.5-flash".into(), ..Default::default() }
    }

    fn msg(role: MessageRole, text: &str) -> ChatMessage {
        ChatMessage::new(uuid::Uuid::new_v4(), role, text)
    }

    fn block(t: PromptBlockType, order: i32) -> PromptBlock {
        PromptBlock::new(t, order)
    }

    fn texts(request: &LlmRequest) -> Vec<&str> {
        request.history.iter().map(|m| m.text.as_str()).collect()
    }

    fn preset_of(blocks: Vec<PromptBlock>) -> PromptPreset {
        PromptPreset { blocks, ..Default::default() }
    }

    #[test]
    fn assistant_mode_uses_conversation_system_prompt_and_plain_history() {
        let mut conv = assistant();
        conv.assistant_system_prompt = Some("Be terse.".into());
        let preset = create_assistant_preset();
        let history = [msg(MessageRole::User, "Hi"), msg(MessageRole::Character, "Hello")];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);

        assert_eq!(request.system_instruction.as_deref(), Some("Be terse."));
        assert_eq!(request.history.len(), 2);
        assert_eq!(request.history[0].role, LlmRole::User);
        assert_eq!(request.history[0].text, "Hi");
        assert_eq!(request.history[1].role, LlmRole::Model);
        assert_eq!(request.model, "gemini-2.5-flash");
    }

    #[test]
    fn preset_custom_text_beats_the_inline_assistant_prompt() {
        let mut conv = assistant();
        conv.assistant_system_prompt = Some("inline".into());
        let mut system = block(PromptBlockType::SystemPrompt, 0);
        system.custom_text = Some("from preset".into());
        let preset = preset_of(vec![system]);

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]), &NullScanner);
        assert_eq!(request.system_instruction.as_deref(), Some("from preset"));
    }

    #[test]
    fn tool_messages_are_sent_to_the_provider_as_user_turns() {
        let conv = assistant();
        let preset = create_assistant_preset();
        let history = [
            msg(MessageRole::User, "What's my current directory?"),
            msg(MessageRole::Character, "<powershell-run>Get-Location</powershell-run>"),
            msg(MessageRole::Tool, "[PowerShell tool result]\nCommand: Get-Location\nOutput:\nC:\\Users\\me"),
        ];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);

        assert_eq!(request.history.len(), 3);
        assert_eq!(request.history[2].role, LlmRole::User);
        assert!(request.history[2].text.contains("C:\\Users\\me"));
    }

    #[test]
    fn assistant_mode_includes_user_memories_in_the_system_instruction() {
        let mut conv = assistant();
        conv.assistant_system_prompt = Some("Be terse.".into());
        let preset = create_assistant_preset();
        let memories = [UserMemory::new("User's name is Dominik."), UserMemory::new("User prefers concise answers.")];

        let request =
            assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_user_memories(&memories), &NullScanner);

        let system = request.system_instruction.unwrap();
        assert!(system.contains("User's name is Dominik."));
        assert!(system.contains("User prefers concise answers."));
        assert!(system.contains("Be terse."));
    }

    #[test]
    fn roleplay_mode_ignores_user_memories_since_it_has_no_memory_block() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let memories = [UserMemory::new("Should not appear in roleplay.")];

        let request =
            assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_user_memories(&memories), &NullScanner);

        assert!(!request.system_instruction.unwrap_or_default().contains("Should not appear in roleplay."));
    }

    #[test]
    fn roleplay_combines_persona_and_character_blocks_in_order() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let persona = Persona { name: "Alex".into(), description: "A curious traveler.".into(), ..Default::default() };
        let characters = [Character {
            name: "Nova".into(),
            description: "A wandering star-mapper.".into(),
            scenario: "Aboard a research vessel.".into(),
            example_dialogue: "Nova: Greetings.".into(),
            ..Default::default()
        }];

        let request = assemble(
            &PromptAssemblyContext::new(&conv, &preset, &[]).with_persona(&persona).with_characters(&characters),
            &NullScanner,
        );

        let system = request.system_instruction.unwrap();
        let lines: Vec<&str> = system.split("\n\n").collect();
        assert_eq!(
            lines[0],
            "Write Nova's next reply in this fictional roleplay with Alex. Stay in character and be descriptive."
        );
        for expected in [
            "Alex's persona: A curious traveler.",
            "A wandering star-mapper.",
            "Aboard a research vessel.",
            "Nova: Greetings.",
        ] {
            assert!(lines.contains(&expected), "missing {expected}");
        }
        // Blocks come out in configured order.
        let pos = |s: &str| lines.iter().position(|l| *l == s).unwrap();
        assert!(pos("Alex's persona: A curious traveler.") < pos("A wandering star-mapper."));
        assert!(pos("A wandering star-mapper.") < pos("Aboard a research vessel."));
        assert!(pos("Aboard a research vessel.") < pos("Nova: Greetings."));
    }

    #[test]
    fn the_personality_is_sent_with_the_description() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let solo = [Character {
            name: "Nova".into(),
            description: "A star-mapper.".into(),
            personality: "Curious, calm.".into(),
            ..Default::default()
        }];
        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_characters(&solo), &NullScanner);
        let system = request.system_instruction.unwrap();
        assert!(system.contains("A star-mapper.\nNova's personality: Curious, calm."), "{system}");
        assert_eq!(system.matches("Curious, calm.").count(), 1, "{system}");

        let group = [
            Character { name: "Alice".into(), personality: "Bold.".into(), ..Default::default() },
            Character { name: "Bob".into(), description: "Quiet.".into(), ..Default::default() },
        ];
        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_characters(&group), &NullScanner);
        let system = request.system_instruction.unwrap();
        assert!(system.contains("Alice's description: Alice's personality: Bold.\nBob's description: Quiet."), "{system}");
    }

    #[test]
    fn substitutes_user_and_char_macros_everywhere_in_the_system_instruction() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let persona = Persona { name: "Alex".into(), ..Default::default() };
        let characters = [Character {
            name: "Nova".into(),
            description: "{{char}} is a wandering star-mapper who calls {{user}} by name.".into(),
            ..Default::default()
        }];

        let request = assemble(
            &PromptAssemblyContext::new(&conv, &preset, &[]).with_persona(&persona).with_characters(&characters),
            &NullScanner,
        );

        let system = request.system_instruction.unwrap();
        assert!(!system.contains("{{char}}"));
        assert!(!system.contains("{{user}}"));
        assert!(system.contains("Nova is a wandering star-mapper who calls Alex by name."));
    }

    #[test]
    fn macros_are_case_insensitive() {
        let characters = [Character { name: "Nova".into(), ..Default::default() }];
        assert_eq!(apply_macros("{{CHAR}} & {{User}}", None, &characters), "Nova & User");
    }

    #[test]
    fn in_a_group_char_means_the_speaker_but_each_card_means_itself() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let characters = [
            Character { name: "Alice".into(), description: "{{char}} greets {{user}}.".into(), ..Default::default() },
            Character { name: "Bob".into(), scenario: "{{char}} keeps watch.".into(), ..Default::default() },
        ];
        let history = [msg(MessageRole::User, "Over to you, {{char}}.")];

        let request = assemble(
            &PromptAssemblyContext::new(&conv, &preset, &history).with_characters(&characters).with_speaker(&characters[1]),
            &NullScanner,
        );

        let system = request.system_instruction.clone().unwrap();
        assert!(system.starts_with("Write Bob's next reply in this fictional roleplay with User."), "{system}");
        assert!(system.contains("Alice's description: Alice greets User."), "{system}");
        assert!(system.contains("Bob's scenario: Bob keeps watch."), "{system}");
        assert_eq!(texts(&request), ["Over to you, Bob."]);
    }

    #[test]
    fn in_a_group_without_a_speaker_char_names_the_whole_cast() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let characters = [Character { name: "Alice".into(), ..Default::default() }, Character { name: "Bob".into(), ..Default::default() }];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_characters(&characters), &NullScanner);

        assert!(request.system_instruction.unwrap().starts_with("Write Alice and Bob's next reply"));
    }

    #[test]
    fn multi_character_blocks_are_labelled_per_character() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let characters = [
            Character { name: "Alice".into(), description: "tall".into(), ..Default::default() },
            Character { name: "Bob".into(), description: "  ".into(), ..Default::default() },
            Character { name: "Cy".into(), description: "short".into(), ..Default::default() },
        ];
        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_characters(&characters), &NullScanner);
        let system = request.system_instruction.unwrap();
        assert!(system.contains("Alice's description: tall\nCy's description: short"));
        assert!(!system.contains("Bob's description"));
    }

    #[test]
    fn defaults_user_macro_to_user_when_no_persona_is_set() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let characters = [Character { name: "Nova".into(), ..Default::default() }];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_characters(&characters), &NullScanner);

        assert!(request
            .system_instruction
            .unwrap()
            .contains("Write Nova's next reply in this fictional roleplay with User."));
    }

    #[test]
    fn disabled_blocks_are_excluded() {
        let conv = roleplay();
        let mut preset = create_roleplay_preset();
        preset.blocks.iter_mut().find(|b| b.block_type == PromptBlockType::Scenario).unwrap().enabled = false;
        let characters = [Character { name: "Nova".into(), scenario: "Should not appear.".into(), ..Default::default() }];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_characters(&characters), &NullScanner);

        assert!(!request.system_instruction.unwrap_or_default().contains("Should not appear."));
    }

    #[test]
    fn blocks_are_ordered_by_order_not_list_position() {
        let conv = roleplay();
        let mut a = block(PromptBlockType::Custom, 2);
        a.custom_text = Some("second".into());
        let mut b = block(PromptBlockType::Custom, 1);
        b.custom_text = Some("first".into());
        let preset = preset_of(vec![a, b]);

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]), &NullScanner);
        assert_eq!(request.system_instruction.as_deref(), Some("first\n\nsecond"));
    }

    #[test]
    fn group_chat_prefixes_character_messages_with_the_speaker_name() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let alice = Character { name: "Alice".into(), ..Default::default() };
        let bob = Character { name: "Bob".into(), ..Default::default() };
        let mut m2 = msg(MessageRole::Character, "Hello!");
        m2.speaker_character_id = Some(alice.id);
        let mut m3 = msg(MessageRole::Character, "Hey there.");
        m3.speaker_character_id = Some(bob.id);
        let history = [msg(MessageRole::User, "Hi both"), m2, m3];
        let characters = [alice, bob];

        let request =
            assemble(&PromptAssemblyContext::new(&conv, &preset, &history).with_characters(&characters), &NullScanner);

        assert_eq!(texts(&request), ["Hi both", "Alice: Hello!", "Bob: Hey there."]);
    }

    #[test]
    fn single_character_history_is_not_prefixed() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let nova = Character { name: "Nova".into(), ..Default::default() };
        let mut m = msg(MessageRole::Character, "Hello!");
        m.speaker_character_id = Some(nova.id);
        let history = [m];
        let characters = [nova];

        let request =
            assemble(&PromptAssemblyContext::new(&conv, &preset, &history).with_characters(&characters), &NullScanner);

        assert_eq!(request.history[0].text, "Hello!");
    }

    #[test]
    fn block_after_chat_history_is_injected_at_the_configured_depth() {
        let conv = roleplay();
        let mut note = block(PromptBlockType::AuthorsNote, 1);
        note.injection_depth = 1;
        note.custom_text = Some("Remember the plot twist.".into());
        let preset = preset_of(vec![block(PromptBlockType::ChatHistory, 0), note]);
        let history = [msg(MessageRole::User, "First"), msg(MessageRole::Character, "Second"), msg(MessageRole::User, "Third")];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);

        assert_eq!(request.system_instruction, None);
        assert_eq!(request.history.len(), 4);
        // Depth 1 means inserted one message before the end - marked so it can't pass for the user's own words.
        assert_eq!(request.history[2].role, LlmRole::User);
        assert_eq!(request.history[2].text, "[System note: Remember the plot twist.]");
        assert_eq!(request.history[3].text, "Third");
    }

    #[test]
    fn injected_blocks_get_macros_filled_before_being_marked() {
        let conv = roleplay();
        let mut rule = block(PromptBlockType::PostHistoryInstructions, 1);
        rule.custom_text = Some("don't act or speak for {{user}}\n".into());
        let preset = preset_of(vec![block(PromptBlockType::ChatHistory, 0), rule]);
        let persona = Persona { name: "Alex".into(), ..Default::default() };
        let history = [msg(MessageRole::User, "*I draw my sword.*")];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history).with_persona(&persona), &NullScanner);

        assert_eq!(texts(&request), ["*I draw my sword.*", "[System note: don't act or speak for Alex]"]);
    }

    #[test]
    fn authors_note_text_from_the_context_beats_the_block_custom_text() {
        let conv = roleplay();
        let mut note = block(PromptBlockType::AuthorsNote, 1);
        note.custom_text = Some("from block".into());
        let preset = preset_of(vec![block(PromptBlockType::ChatHistory, 0), note]);

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_authors_note("from chat"), &NullScanner);
        assert_eq!(texts(&request), ["[System note: from chat]"]);
    }

    #[test]
    fn empty_persona_and_character_blocks_are_omitted_rather_than_leaving_blank_lines() {
        let conv = roleplay();
        let preset = create_roleplay_preset();

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]), &NullScanner);

        // No characters: {{char}} has nobody to name and stays literal.
        assert_eq!(
            request.system_instruction.as_deref(),
            Some("Write {{char}}'s next reply in this fictional roleplay with User. Stay in character and be descriptive.")
        );
    }

    #[test]
    fn world_info_before_and_after_wrap_the_character_description() {
        let scanner = StubScanner(vec![
            wi("Before lore.", WorldInfoPosition::BeforeCharacterDescription, 0),
            wi("After lore.", WorldInfoPosition::AfterCharacterDescription, 0),
        ]);
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let characters = [Character { name: "Nova".into(), description: "A wandering star-mapper.".into(), ..Default::default() }];
        let lorebooks = [Lorebook::default()];
        let history = [msg(MessageRole::User, "Hi")];

        let request = assemble(
            &PromptAssemblyContext::new(&conv, &preset, &history).with_characters(&characters).with_lorebooks(&lorebooks),
            &scanner,
        );

        let system = request.system_instruction.unwrap();
        let lines: Vec<&str> = system.split("\n\n").collect();
        assert_eq!(lines[1], "Before lore."); // right before the character description
        assert_eq!(lines[2], "A wandering star-mapper.");
        assert_eq!(lines[3], "After lore."); // right after it
    }

    #[test]
    fn world_info_still_surfaces_without_a_character_description_block() {
        let scanner = StubScanner(vec![
            wi("After lore.", WorldInfoPosition::AfterCharacterDescription, 0),
            wi("Before lore.", WorldInfoPosition::BeforeCharacterDescription, 0),
        ]);
        let conv = roleplay();
        let mut custom = block(PromptBlockType::Custom, 0);
        custom.custom_text = Some("Intro".into());
        let preset = preset_of(vec![custom, block(PromptBlockType::WorldInfo, 1), block(PromptBlockType::ChatHistory, 2)]);
        let lorebooks = [Lorebook::default()];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]).with_lorebooks(&lorebooks), &scanner);

        // "Before" always precedes "After", regardless of scanner order.
        assert_eq!(request.system_instruction.as_deref(), Some("Intro\n\nBefore lore.\n\nAfter lore."));
    }

    #[test]
    fn world_info_at_depth_is_injected_with_deeper_groups_further_back() {
        let scanner = StubScanner(vec![
            wi("Shallow lore.", WorldInfoPosition::AtDepth, 1),
            wi("Deep lore.", WorldInfoPosition::AtDepth, 3),
        ]);
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let lorebooks = [Lorebook::default()];
        let history = [msg(MessageRole::User, "First"), msg(MessageRole::Character, "Second"), msg(MessageRole::User, "Third")];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history).with_lorebooks(&lorebooks), &scanner);

        assert_eq!(
            texts(&request),
            ["[System note: Deep lore.]", "First", "Second", "[System note: Shallow lore.]", "Third"]
        );
    }

    #[test]
    fn world_info_entries_sharing_a_depth_merge_in_priority_order() {
        let scanner = StubScanner(vec![
            wi("First of the pair.", WorldInfoPosition::AtDepth, 2),
            wi("Second of the pair.", WorldInfoPosition::AtDepth, 2),
        ]);
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let lorebooks = [Lorebook::default()];
        let history = [msg(MessageRole::User, "First"), msg(MessageRole::Character, "Second"), msg(MessageRole::User, "Third")];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history).with_lorebooks(&lorebooks), &scanner);

        assert_eq!(
            texts(&request),
            ["First", "[System note: First of the pair.\nSecond of the pair.]", "Second", "Third"]
        );
    }

    #[test]
    fn world_info_is_omitted_when_the_preset_has_no_world_info_block() {
        let scanner = StubScanner(vec![wi("Should not appear.", WorldInfoPosition::BeforeCharacterDescription, 0)]);
        let conv = roleplay();
        let preset = preset_of(vec![block(PromptBlockType::ChatHistory, 0)]);
        let lorebooks = [Lorebook::default()];
        let history = [msg(MessageRole::User, "Hi")];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history).with_lorebooks(&lorebooks), &scanner);

        assert!(!request.system_instruction.as_deref().unwrap_or_default().contains("Should not appear."));
        assert_eq!(texts(&request), ["Hi"]);
    }

    #[test]
    fn disabling_the_world_info_block_suppresses_world_info() {
        let scanner = StubScanner(vec![wi("Should not appear.", WorldInfoPosition::BeforeCharacterDescription, 0)]);
        let conv = roleplay();
        let mut preset = create_roleplay_preset();
        preset.blocks.iter_mut().find(|b| b.block_type == PromptBlockType::WorldInfo).unwrap().enabled = false;
        let lorebooks = [Lorebook::default()];
        let history = [msg(MessageRole::User, "Hi")];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history).with_lorebooks(&lorebooks), &scanner);

        assert!(!request.system_instruction.unwrap_or_default().contains("Should not appear."));
    }

    #[test]
    fn world_info_is_not_scanned_without_lorebooks() {
        struct PanicScanner;
        impl WorldInfoScanner for PanicScanner {
            fn scan(&self, _: &[Lorebook], _: &[ChatMessage]) -> Vec<WorldInfoMatch> {
                panic!("should not scan");
            }
        }
        let conv = roleplay();
        let preset = create_roleplay_preset();
        assemble(&PromptAssemblyContext::new(&conv, &preset, &[]), &PanicScanner);
    }

    #[test]
    fn world_info_content_gets_macro_substitution() {
        let scanner = StubScanner(vec![wi("{{char}} rules over {{user}}.", WorldInfoPosition::BeforeCharacterDescription, 0)]);
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let characters = [Character { name: "Nova".into(), ..Default::default() }];
        let persona = Persona { name: "Alex".into(), ..Default::default() };
        let lorebooks = [Lorebook::default()];
        let history = [msg(MessageRole::User, "Hi")];

        let request = assemble(
            &PromptAssemblyContext::new(&conv, &preset, &history)
                .with_persona(&persona)
                .with_characters(&characters)
                .with_lorebooks(&lorebooks),
            &scanner,
        );

        assert!(request.system_instruction.unwrap().contains("Nova rules over Alex."));
    }

    #[test]
    fn preset_sampling_parameters_flow_into_generation_config() {
        let conv = roleplay();
        let mut preset = create_roleplay_preset();
        preset.temperature = Some(0.7);
        preset.top_p = Some(0.9);
        preset.top_k = Some(40);
        preset.max_output_tokens = Some(512);
        preset.reasoning_effort = Some(crate::models::ReasoningEffort::Low);

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]), &NullScanner);

        let cfg = request.generation_config;
        assert_eq!(cfg.temperature, Some(0.7));
        assert_eq!(cfg.top_p, Some(0.9));
        assert_eq!(cfg.top_k, Some(40));
        assert_eq!(cfg.max_output_tokens, Some(512));
        assert_eq!(cfg.reasoning_effort, Some(crate::models::ReasoningEffort::Low));
    }

    #[test]
    fn preset_without_sampling_parameters_leaves_generation_config_empty() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]), &NullScanner);
        assert_eq!(request.generation_config, LlmGenerationConfig::default());
    }

    #[test]
    fn conversation_search_override_flows_into_the_request() {
        let mut conv = roleplay();
        conv.enable_google_search = Some(true);
        let preset = create_roleplay_preset();
        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &[]), &NullScanner);
        assert_eq!(request.enable_web_search, Some(true));
    }

    fn history_preset(tail: PromptBlockType) -> PromptPreset {
        let mut t = block(tail, 1);
        t.injection_depth = 0;
        preset_of(vec![block(PromptBlockType::ChatHistory, 0), t])
    }

    #[test]
    fn world_state_block_is_injected_after_history_when_text_is_provided() {
        let conv = roleplay();
        let preset = history_preset(PromptBlockType::WorldState);
        let history = [msg(MessageRole::User, "Hi")];

        let request = assemble(
            &PromptAssemblyContext::new(&conv, &preset, &history).with_world_state_text("It is a rainy morning."),
            &NullScanner,
        );

        assert_eq!(texts(&request), ["Hi", "[System note: It is a rainy morning.]"]);
    }

    #[test]
    fn world_state_block_contributes_nothing_without_text() {
        let conv = roleplay();
        let preset = history_preset(PromptBlockType::WorldState);
        let history = [msg(MessageRole::User, "Hi")];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);

        assert_eq!(texts(&request), ["Hi"]);
    }

    #[test]
    fn attachments_are_carried_on_the_message_that_had_them() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let mut with_image = msg(MessageRole::User, "look");
        with_image.attachments = vec![MessageAttachment { image_path: "a.png".into(), mime_type: "image/png".into(), description: None }];
        let history = [with_image, msg(MessageRole::Character, "nice")];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);

        let sent = request.history[0].attachments.as_ref().unwrap();
        assert_eq!(sent, &[LlmAttachment { image_path: "a.png".into(), mime_type: "image/png".into() }]);
        assert!(request.history[1].attachments.is_none());
    }

    #[test]
    fn an_earlier_picture_is_sent_as_its_description_once_it_has_one() {
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let picture = |path: &str, description: Option<&str>| MessageAttachment {
            image_path: path.into(),
            mime_type: "image/png".into(),
            description: description.map(Into::into),
        };
        let mut described = msg(MessageRole::User, "look");
        described.attachments = vec![picture("a.png", Some("A grey cat."))];
        let mut undescribed = msg(MessageRole::User, "");
        undescribed.attachments = vec![picture("b.png", None)];
        let mut latest = msg(MessageRole::User, "and this");
        latest.attachments = vec![picture("c.png", Some("A dog."))];
        let history = [
            described,
            msg(MessageRole::Character, "cute"),
            undescribed,
            msg(MessageRole::Character, "hm"),
            latest,
        ];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);

        assert!(request.history[0].attachments.is_none());
        assert_eq!(
            request.history[0].text,
            "look\n\n[System note: The user sent a picture with this message. It showed: A grey cat.]"
        );
        assert!(request.history[2].attachments.is_some(), "no description yet, so the picture itself goes");
        assert!(request.history[4].attachments.is_some(), "the latest message's picture is always the real one");
        assert_eq!(request.history[4].text, "and this");
    }

    #[test]
    fn swiped_messages_send_the_active_alternate() {
        use crate::models::MessageAlternate;
        let conv = roleplay();
        let preset = create_roleplay_preset();
        let mut m = msg(MessageRole::Character, "stale");
        m.alternates = vec![
            MessageAlternate { text: "first".into(), reasoning_text: None, raw_prompt: None },
            MessageAlternate { text: "second".into(), reasoning_text: None, raw_prompt: None },
        ];
        m.active_alternate_index = 1;
        let history = [m];

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);
        assert_eq!(request.history[0].text, "second");
    }

    // ---- context budget ----

    fn budget_history(lengths: &[usize]) -> Vec<ChatMessage> {
        lengths.iter().map(|n| msg(MessageRole::User, &"x".repeat(*n))).collect()
    }

    #[test]
    fn context_budget_drops_oldest_whole_messages() {
        let conv = roleplay();
        let mut preset = create_roleplay_preset();
        preset.max_context_tokens = Some(10); // 40 chars
        let history = budget_history(&[30, 20, 15]);

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);

        // 15 + 20 = 35 fits; adding 30 would make 65 > 40.
        let lens: Vec<usize> = request.history.iter().map(|m| m.text.len()).collect();
        assert_eq!(lens, [20, 15]);
    }

    #[test]
    fn context_budget_always_keeps_the_most_recent_message() {
        let conv = roleplay();
        let mut preset = create_roleplay_preset();
        preset.max_context_tokens = Some(1); // 4 chars
        let history = budget_history(&[5, 100]);

        let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);

        assert_eq!(request.history.len(), 1);
        assert_eq!(request.history[0].text.len(), 100);
    }

    #[test]
    fn no_context_budget_means_unlimited() {
        let conv = roleplay();
        let mut preset = create_roleplay_preset();
        let history = budget_history(&[1000, 1000, 1000]);
        for budget in [None, Some(0), Some(-5)] {
            preset.max_context_tokens = budget;
            let request = assemble(&PromptAssemblyContext::new(&conv, &preset, &history), &NullScanner);
            assert_eq!(request.history.len(), 3, "{budget:?}");
        }
    }
}

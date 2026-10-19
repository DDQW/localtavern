use crate::models::{Character, ChatMessage, Conversation, Lorebook, Persona, PromptPreset, UserMemory};

/// Everything the assembler needs to build one LLM request for a conversation turn. Borrowed, so
/// callers can build it from whatever they've loaded without cloning.
#[derive(Clone, Copy)]
pub struct PromptAssemblyContext<'a> {
    pub conversation: &'a Conversation,
    pub preset: &'a PromptPreset,
    pub history: &'a [ChatMessage],
    pub persona: Option<&'a Persona>,
    pub characters: &'a [Character],
    /// Whose reply this is - what `{{char}}` means in a group chat. A one-character chat doesn't need it.
    pub speaker: Option<&'a Character>,
    pub lorebooks: &'a [Lorebook],
    pub authors_note_text: Option<&'a str>,
    pub user_memories: &'a [UserMemory],
    pub world_state_text: Option<&'a str>,
}

impl<'a> PromptAssemblyContext<'a> {
    pub fn new(conversation: &'a Conversation, preset: &'a PromptPreset, history: &'a [ChatMessage]) -> Self {
        Self {
            conversation,
            preset,
            history,
            persona: None,
            characters: &[],
            speaker: None,
            lorebooks: &[],
            authors_note_text: None,
            user_memories: &[],
            world_state_text: None,
        }
    }

    pub fn with_persona(mut self, persona: &'a Persona) -> Self {
        self.persona = Some(persona);
        self
    }

    pub fn with_characters(mut self, characters: &'a [Character]) -> Self {
        self.characters = characters;
        self
    }

    pub fn with_speaker(mut self, speaker: &'a Character) -> Self {
        self.speaker = Some(speaker);
        self
    }

    pub fn with_lorebooks(mut self, lorebooks: &'a [Lorebook]) -> Self {
        self.lorebooks = lorebooks;
        self
    }

    pub fn with_authors_note(mut self, text: &'a str) -> Self {
        self.authors_note_text = Some(text);
        self
    }

    pub fn with_user_memories(mut self, memories: &'a [UserMemory]) -> Self {
        self.user_memories = memories;
        self
    }

    pub fn with_world_state_text(mut self, text: &'a str) -> Self {
        self.world_state_text = Some(text);
        self
    }
}

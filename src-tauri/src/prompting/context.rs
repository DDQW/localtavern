use crate::models::{
    Character, CharacterNotebookEntry, ChatMessage, Conversation, Lorebook, Persona, PromptPreset, QuestLogEntry,
    UserMemory,
};

/// Everything the assembler needs to build one LLM request for a conversation turn. Borrowed, so
/// callers can build it from whatever they've loaded without cloning.
#[derive(Clone, Copy)]
pub struct PromptAssemblyContext<'a> {
    pub conversation: &'a Conversation,
    pub preset: &'a PromptPreset,
    pub history: &'a [ChatMessage],
    pub persona: Option<&'a Persona>,
    pub characters: &'a [Character],
    pub lorebooks: &'a [Lorebook],
    pub authors_note_text: Option<&'a str>,
    pub user_memories: &'a [UserMemory],
    pub world_state_text: Option<&'a str>,
    pub character_notebook_entries: &'a [CharacterNotebookEntry],
    pub quest_log_entries: &'a [QuestLogEntry],
}

impl<'a> PromptAssemblyContext<'a> {
    pub fn new(conversation: &'a Conversation, preset: &'a PromptPreset, history: &'a [ChatMessage]) -> Self {
        Self {
            conversation,
            preset,
            history,
            persona: None,
            characters: &[],
            lorebooks: &[],
            authors_note_text: None,
            user_memories: &[],
            world_state_text: None,
            character_notebook_entries: &[],
            quest_log_entries: &[],
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

    pub fn with_notebook_entries(mut self, entries: &'a [CharacterNotebookEntry]) -> Self {
        self.character_notebook_entries = entries;
        self
    }

    pub fn with_quest_log(mut self, entries: &'a [QuestLogEntry]) -> Self {
        self.quest_log_entries = entries;
        self
    }
}

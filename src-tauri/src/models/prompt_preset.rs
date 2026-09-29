use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Persisted as the raw int ordinal - append only, never renumber, or every saved preset's block
/// types would be corrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptBlockType {
    SystemPrompt,
    PersonaDescription,
    CharacterDescription,
    Scenario,
    ExampleDialogue,
    WorldInfo,
    ChatHistory,
    AuthorsNote,
    Custom,
    UserMemory,
    PostHistoryInstructions,
    WorldState,
    CharacterNotebook,
    QuestLog,
}
int_enum!(PromptBlockType {
    SystemPrompt = 0,
    PersonaDescription = 1,
    CharacterDescription = 2,
    Scenario = 3,
    ExampleDialogue = 4,
    WorldInfo = 5,
    ChatHistory = 6,
    AuthorsNote = 7,
    Custom = 8,
    UserMemory = 9,
    PostHistoryInstructions = 10,
    WorldState = 11,
    CharacterNotebook = 12,
    QuestLog = 13,
});

/// How hard a thinking-capable model should reason. `None` on a preset means "send nothing" so each
/// model keeps its own default. Persisted as its raw int - append only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReasoningEffort {
    Off,
    Low,
    Medium,
    High,
}
int_enum!(ReasoningEffort { Off = 0, Low = 1, Medium = 2, High = 3 });

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBlock {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub block_type: PromptBlockType,
    pub enabled: bool,
    pub order: i32,
    pub custom_text: Option<String>,
    /// For AuthorsNote/Custom blocks placed after ChatHistory: how many messages from the end to inject at.
    pub injection_depth: i32,
}

impl PromptBlock {
    pub fn new(block_type: PromptBlockType, order: i32) -> Self {
        Self { id: Uuid::new_v4(), block_type, enabled: true, order, custom_text: None, injection_depth: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptPreset {
    pub id: Uuid,
    pub name: String,
    pub blocks: Vec<PromptBlock>,
    // Sampling parameters - None leaves the choice to the provider's defaults.
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i32>,
    pub max_output_tokens: Option<i32>,
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Approximate token budget for chat history per request; None means unlimited.
    pub max_context_tokens: Option<i32>,
}

impl Default for PromptPreset {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            blocks: Vec::new(),
            temperature: None,
            top_p: None,
            top_k: None,
            max_output_tokens: None,
            reasoning_effort: None,
            max_context_tokens: None,
        }
    }
}

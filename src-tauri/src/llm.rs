//! Provider-agnostic request types. The prompt assembler builds an [`LlmRequest`]; each provider
//! maps it onto its own wire format.

use crate::models::ReasoningEffort;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmRole {
    User,
    Model,
}

/// An image sent alongside a message, referenced by its stored local file path - the provider
/// mapper reads and encodes it at request-build time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmAttachment {
    pub image_path: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub text: String,
    pub attachments: Option<Vec<LlmAttachment>>,
}

impl LlmMessage {
    pub fn new(role: LlmRole, text: impl Into<String>) -> Self {
        Self { role, text: text.into(), attachments: None }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LlmGenerationConfig {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i32>,
    pub max_output_tokens: Option<i32>,
    pub stop_sequences: Option<Vec<String>>,
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmRequest {
    pub model: String,
    pub system_instruction: Option<String>,
    pub history: Vec<LlmMessage>,
    pub generation_config: LlmGenerationConfig,
    /// Per-conversation override for web search grounding; `None` defers to the provider's default.
    pub enable_web_search: Option<bool>,
}

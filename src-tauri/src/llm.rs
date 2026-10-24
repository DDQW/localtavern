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

impl LlmAttachment {
    /// The file's bytes, base64-encoded; `None` when it has gone missing or can't be read.
    pub fn base64(&self) -> Option<String> {
        use base64::Engine;
        std::fs::read(&self.image_path).ok().map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    /// `data:<mime>;base64,...`, the form chat-completions style APIs take for an inline image.
    pub fn data_url(&self) -> Option<String> {
        self.base64().map(|b| format!("data:{};base64,{b}", self.mime_type))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub text: String,
    pub attachments: Option<Vec<LlmAttachment>>,
    /// What a model turn reasoned before replying, for providers that want it replayed.
    pub reasoning: Option<String>,
}

impl LlmMessage {
    pub fn new(role: LlmRole, text: impl Into<String>) -> Self {
        Self { role, text: text.into(), attachments: None, reasoning: None }
    }

    /// The `content` of an OpenAI-style chat-completions message: the bare text, or - when a user
    /// turn carries pictures - a list of text and `image_url` parts. Only user turns can carry
    /// images on these APIs, so a model turn's pictures are left out.
    pub fn chat_content(&self) -> serde_json::Value {
        let images: Vec<String> = match self.role {
            LlmRole::User => self.attachments.iter().flatten().filter_map(LlmAttachment::data_url).collect(),
            LlmRole::Model => Vec::new(),
        };
        if images.is_empty() {
            return serde_json::Value::String(self.text.clone());
        }
        let mut parts = vec![serde_json::json!({"type": "text", "text": self.text})];
        parts.extend(images.into_iter().map(|url| serde_json::json!({"type": "image_url", "image_url": {"url": url}})));
        serde_json::Value::Array(parts)
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

impl LlmGenerationConfig {
    /// For background chores with a one-line answer (meters, memories, titles, summaries). Left to
    /// its default, a thinking model reasons first: Gemma 4 on Ollama spent ~1000 tokens (25 s) to
    /// answer a 14-token meter line, holding up the user's next message on a local server.
    pub fn without_reasoning() -> Self {
        Self { reasoning_effort: Some(ReasoningEffort::Off), ..Default::default() }
    }
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

#[cfg(test)]
mod attachment_tests {
    use super::*;

    fn with_picture(role: LlmRole) -> (tempfile::TempDir, LlmMessage) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::write(&path, [1u8, 2, 3]).unwrap();
        let mut m = LlmMessage::new(role, "look");
        m.attachments = Some(vec![LlmAttachment { image_path: path.to_string_lossy().into_owned(), mime_type: "image/png".into() }]);
        (dir, m)
    }

    #[test]
    fn a_user_picture_becomes_an_image_url_part() {
        let (_dir, m) = with_picture(LlmRole::User);
        let content = m.chat_content();
        assert_eq!(content[0]["text"], "look");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AQID");
    }

    #[test]
    fn plain_text_and_model_turns_stay_a_bare_string() {
        assert_eq!(LlmMessage::new(LlmRole::User, "hi").chat_content(), "hi");
        let (_dir, m) = with_picture(LlmRole::Model);
        assert_eq!(m.chat_content(), "look");
    }
}

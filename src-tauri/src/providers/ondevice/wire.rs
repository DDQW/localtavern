//! llama-server's OpenAI-compatible chat-completions request. Pictures go in as `image_url` parts
//! (see [`LlmMessage::chat_content`]); the server only accepts them when it was started with a
//! vision projector.

use serde::Serialize;

use crate::llm::{LlmRequest, LlmRole};
use crate::models::ReasoningEffort;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Message {
    pub role: &'static str,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

/// Passed to the model's chat template; Gemma's and Qwen's read `enable_thinking`, others ignore it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TemplateKwargs {
    pub enable_thinking: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub stream: bool,
    pub stream_options: StreamOptions,
    /// Reuse the evaluated prompt from the previous request as far as it matches - on a phone
    /// reading a long roleplay prompt again is what takes the time.
    pub cache_prompt: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    pub chat_template_kwargs: Option<TemplateKwargs>,
}

pub fn to_request(request: &LlmRequest) -> ChatRequest {
    let mut messages = Vec::new();
    if let Some(system) = request.system_instruction.as_deref().filter(|s| !s.is_empty()) {
        messages.push(Message { role: "system", content: serde_json::Value::String(system.into()) });
    }
    messages.extend(request.history.iter().map(|m| Message {
        role: if m.role == LlmRole::User { "user" } else { "assistant" },
        content: m.chat_content(),
    }));

    let cfg = &request.generation_config;
    ChatRequest {
        messages,
        stream: true,
        stream_options: StreamOptions { include_usage: true },
        cache_prompt: true,
        temperature: cfg.temperature,
        top_p: cfg.top_p,
        top_k: cfg.top_k,
        max_tokens: cfg.max_output_tokens,
        stop: cfg.stop_sequences.clone(),
        // Thinking only when a level is asked for: left to the model's default, Gemma thinks first, and
        // on a phone that's minutes of hidden text before every reply.
        chat_template_kwargs: Some(TemplateKwargs {
            enable_thinking: cfg.reasoning_effort.is_some_and(|effort| effort != ReasoningEffort::Off),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmAttachment, LlmGenerationConfig, LlmMessage};

    fn request(history: Vec<LlmMessage>, config: LlmGenerationConfig) -> LlmRequest {
        LlmRequest {
            model: "Luchador".into(),
            system_instruction: Some("You are Nova.".into()),
            history,
            generation_config: config,
            enable_web_search: None,
        }
    }

    #[test]
    fn maps_roles_and_leads_with_the_system_message() {
        let history = vec![LlmMessage::new(LlmRole::User, "Hi"), LlmMessage::new(LlmRole::Model, "Hello!")];
        let mapped = to_request(&request(history, Default::default()));
        let roles: Vec<_> = mapped.messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, ["system", "user", "assistant"]);
        assert_eq!(mapped.messages[2].content, "Hello!");
    }

    #[test]
    fn sends_pictures_as_image_parts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cat.png");
        std::fs::write(&path, b"png").unwrap();
        let mut message = LlmMessage::new(LlmRole::User, "What's this?");
        message.attachments =
            Some(vec![LlmAttachment { image_path: path.to_string_lossy().into_owned(), mime_type: "image/png".into() }]);

        let mapped = to_request(&request(vec![message], Default::default()));
        let parts = mapped.messages[1].content.as_array().unwrap();
        assert_eq!(parts[0]["text"], "What's this?");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,cG5n");
    }

    #[test]
    fn thinks_only_when_a_level_is_asked_for_and_forwards_sampling() {
        let config = LlmGenerationConfig {
            temperature: Some(0.9),
            top_k: Some(64),
            reasoning_effort: Some(ReasoningEffort::Off),
            ..Default::default()
        };
        let json = serde_json::to_value(to_request(&request(vec![LlmMessage::new(LlmRole::User, "Hi")], config))).unwrap();
        assert_eq!(json["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(json["temperature"], 0.9);
        assert_eq!(json["top_k"], 64);
        assert_eq!(json["cache_prompt"], true);
        assert!(json.get("top_p").is_none());

        let plain = serde_json::to_value(to_request(&request(vec![], Default::default()))).unwrap();
        assert_eq!(plain["chat_template_kwargs"]["enable_thinking"], false);

        let asked = LlmGenerationConfig { reasoning_effort: Some(ReasoningEffort::Low), ..Default::default() };
        let thinking = serde_json::to_value(to_request(&request(vec![], asked))).unwrap();
        assert_eq!(thinking["chat_template_kwargs"]["enable_thinking"], true);
    }
}

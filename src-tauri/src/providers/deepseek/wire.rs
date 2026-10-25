//! DeepSeek's OpenAI-compatible chat-completions wire types and the request mapper.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::{LlmRequest, LlmRole};
use crate::models::ReasoningEffort;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeepSeekMessage {
    pub role: String,
    /// A string, or — when a user turn carries pictures — a list of text and `image_url` parts.
    pub content: Value,
}

/// Requests usage accounting on a trailing SSE chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeepSeekStreamOptions {
    pub include_usage: bool,
}

/// `type` is "enabled" or "disabled".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeepSeekThinking {
    #[serde(rename = "type")]
    pub kind: String,
}

/// `reasoning_effort` is "low"/"medium"/"high" - DeepSeek aliases the levels itself, so the app's
/// level names are sent as-is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeepSeekChatCompletionRequest {
    pub model: String,
    pub messages: Vec<DeepSeekMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<DeepSeekStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<DeepSeekThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeepSeekErrorDetail {
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct DeepSeekErrorResponse {
    pub error: Option<DeepSeekErrorDetail>,
}

#[derive(Debug, Deserialize)]
pub struct DeepSeekModelInfo {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct DeepSeekListModelsResponse {
    pub data: Option<Vec<DeepSeekModelInfo>>,
}

/// Flash (including the retired `deepseek-v4-flash` and vision-exp names, which DeepSeek still
/// serves as Flash) accepts images. Pro does not, and must not be matched by a looser check.
pub fn sees_pictures(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    !id.contains("pro") && (id.contains("flash") || id.contains("vision"))
}

pub fn to_deepseek_request(request: &LlmRequest) -> DeepSeekChatCompletionRequest {
    let mut messages = Vec::new();
    if let Some(system) = request.system_instruction.as_deref().filter(|s| !s.is_empty()) {
        messages.push(DeepSeekMessage { role: "system".into(), content: Value::from(system) });
    }
    // User pictures become `image_url` parts; a model turn's pictures are left out. DeepSeek
    // rejects images on system or assistant messages, and only Flash can see the user ones.
    messages.extend(request.history.iter().map(|m| DeepSeekMessage {
        role: if m.role == LlmRole::User { "user" } else { "assistant" }.into(),
        // Flash accepts JPEG, PNG, GIF, and WebP. A BMP (or anything else) is re-encoded as PNG.
        content: m.chat_content_as(&["image/jpeg", "image/png", "image/gif", "image/webp"]),
    }));

    let cfg = &request.generation_config;
    let thinking = cfg.reasoning_effort.map(|effort| DeepSeekThinking {
        kind: if effort == ReasoningEffort::Off { "disabled" } else { "enabled" }.into(),
    });
    let reasoning_effort = match cfg.reasoning_effort {
        Some(ReasoningEffort::Low) => Some("low".to_string()),
        Some(ReasoningEffort::Medium) => Some("medium".to_string()),
        Some(ReasoningEffort::High) => Some("high".to_string()),
        _ => None,
    };

    DeepSeekChatCompletionRequest {
        model: request.model.clone(),
        messages,
        stream: true,
        stream_options: Some(DeepSeekStreamOptions { include_usage: true }),
        temperature: cfg.temperature,
        top_p: cfg.top_p,
        max_tokens: cfg.max_output_tokens,
        stop: cfg.stop_sequences.clone(),
        thinking,
        reasoning_effort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage};

    fn request(system: Option<&str>, history: Vec<LlmMessage>, config: LlmGenerationConfig) -> LlmRequest {
        LlmRequest {
            model: "deepseek-chat".into(),
            system_instruction: system.map(String::from),
            history,
            generation_config: config,
            enable_web_search: None,
        }
    }

    fn hi() -> Vec<LlmMessage> {
        vec![LlmMessage::new(LlmRole::User, "Hi")]
    }

    #[test]
    fn system_instruction_becomes_a_leading_system_message() {
        let mapped = to_deepseek_request(&request(Some("Be helpful."), hi(), Default::default()));
        assert_eq!(mapped.messages.len(), 2);
        assert_eq!(mapped.messages[0].role, "system");
        assert_eq!(mapped.messages[0].content, "Be helpful.");
        assert_eq!(mapped.messages[1].role, "user");
    }

    #[test]
    fn omits_the_system_message_when_none_or_empty() {
        for system in [None, Some("")] {
            let mapped = to_deepseek_request(&request(system, hi(), Default::default()));
            assert_eq!(mapped.messages.len(), 1);
            assert_eq!(mapped.messages[0].role, "user");
        }
    }

    #[test]
    fn maps_the_model_role_to_assistant() {
        let history = vec![LlmMessage::new(LlmRole::User, "Hi"), LlmMessage::new(LlmRole::Model, "Hello!")];
        let mapped = to_deepseek_request(&request(None, history, Default::default()));
        assert_eq!(mapped.messages[1].role, "assistant");
        assert_eq!(mapped.messages[1].content, "Hello!");
    }

    #[test]
    fn always_requests_streaming_with_usage() {
        let mapped = to_deepseek_request(&request(None, hi(), Default::default()));
        assert!(mapped.stream);
        assert_eq!(mapped.stream_options, Some(DeepSeekStreamOptions { include_usage: true }));
    }

    #[test]
    fn forwards_generation_config_fields() {
        let config = LlmGenerationConfig {
            temperature: Some(0.5),
            top_p: Some(0.9),
            max_output_tokens: Some(256),
            stop_sequences: Some(vec!["STOP".into()]),
            ..Default::default()
        };
        let mapped = to_deepseek_request(&request(None, hi(), config));
        assert_eq!(mapped.temperature, Some(0.5));
        assert_eq!(mapped.top_p, Some(0.9));
        assert_eq!(mapped.max_tokens, Some(256));
        assert_eq!(mapped.stop, Some(vec!["STOP".to_string()]));
    }

    #[test]
    fn maps_reasoning_effort_to_thinking_and_effort() {
        for (effort, thinking, level) in [
            (None, None, None),
            (Some(ReasoningEffort::Off), Some("disabled"), None),
            (Some(ReasoningEffort::Low), Some("enabled"), Some("low")),
            (Some(ReasoningEffort::Medium), Some("enabled"), Some("medium")),
            (Some(ReasoningEffort::High), Some("enabled"), Some("high")),
        ] {
            let config = LlmGenerationConfig { reasoning_effort: effort, ..Default::default() };
            let mapped = to_deepseek_request(&request(None, hi(), config));
            assert_eq!(mapped.thinking.as_ref().map(|t| t.kind.as_str()), thinking, "{effort:?}");
            assert_eq!(mapped.reasoning_effort.as_deref(), level, "{effort:?}");
        }
    }

    #[test]
    fn a_user_picture_becomes_an_image_url_part_and_a_model_picture_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cat.png");
        std::fs::write(&path, b"png").unwrap();
        let attachment = crate::llm::LlmAttachment { image_path: path.to_string_lossy().into_owned(), mime_type: "image/png".into() };

        let mut user = LlmMessage::new(LlmRole::User, "What's this?");
        user.attachments = Some(vec![attachment.clone()]);
        let mut model = LlmMessage::new(LlmRole::Model, "A cat.");
        model.attachments = Some(vec![attachment]);

        let mapped = to_deepseek_request(&request(None, vec![user, model], Default::default()));
        let parts = mapped.messages[0].content.as_array().unwrap();
        assert_eq!(parts[0]["text"], "What's this?");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,cG5n");
        assert_eq!(mapped.messages[1].content, "A cat.");
    }

    #[test]
    fn only_flash_and_vision_names_see_pictures() {
        for model in ["deepseek-flash", "deepseek-v4-flash", "DeepSeek-V4-Flash-Vision-Exp"] {
            assert!(sees_pictures(model), "{model}");
        }
        for model in ["deepseek-v4-pro", "deepseek-pro", "deepseek-chat", "deepseek-reasoner"] {
            assert!(!sees_pictures(model), "{model}");
        }
    }

    #[test]
    fn serializes_only_the_fields_that_are_set() {
        let json = serde_json::to_string(&to_deepseek_request(&request(None, hi(), Default::default()))).unwrap();
        assert_eq!(
            json,
            r#"{"model":"deepseek-chat","messages":[{"role":"user","content":"Hi"}],"stream":true,"stream_options":{"include_usage":true}}"#
        );
    }
}

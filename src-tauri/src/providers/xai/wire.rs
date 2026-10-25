//! xAI's OpenAI-compatible chat-completions wire types and the request mapper. (The stream itself is
//! read by the shared [`openai_compat`](crate::providers::openai_compat) reader.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::{LlmRequest, LlmRole};
use crate::models::ReasoningEffort;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XaiMessage {
    pub role: String,
    pub content: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XaiStreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XaiChatCompletionRequest {
    pub model: String,
    pub messages: Vec<XaiMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<XaiStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

/// xAI errors arrive either as `{"error": {"message": ...}}` or `{"error": "text", "code": ...}` -
/// hence the raw value.
#[derive(Debug, Deserialize)]
pub struct XaiErrorResponse {
    pub error: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct XaiModelInfo {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct XaiListModelsResponse {
    pub data: Option<Vec<XaiModelInfo>>,
}

/// grok-build-* is a coding-agent model that takes no `reasoning_effort` at all, and xAI rejects
/// "none" on grok-4.5+, so "Off" means: omit the field. An unset level defaults to "high" (what
/// Zed does too) - left to xAI's own default the Grok 4.x models reason noticeably less.
pub fn to_xai_request(request: &LlmRequest) -> XaiChatCompletionRequest {
    let mut messages = Vec::new();
    if let Some(system) = request.system_instruction.as_deref().filter(|s| !s.is_empty()) {
        messages.push(XaiMessage { role: "system".into(), content: Value::from(system) });
    }
    messages.extend(request.history.iter().map(|m| XaiMessage {
        role: if m.role == LlmRole::User { "user" } else { "assistant" }.into(),
        // xAI only accepts JPEG and PNG. WebP, GIF, and BMP are re-encoded as PNG.
        content: m.chat_content_as(&["image/jpeg", "image/png"]),
    }));

    let cfg = &request.generation_config;
    let reasoning_effort = if supports_reasoning_effort(&request.model) {
        match cfg.reasoning_effort {
            Some(ReasoningEffort::Low) => Some("low".to_string()),
            Some(ReasoningEffort::Medium) => Some("medium".to_string()),
            Some(ReasoningEffort::High) | None => Some("high".to_string()),
            Some(ReasoningEffort::Off) => None,
        }
    } else {
        None
    };

    XaiChatCompletionRequest {
        model: request.model.clone(),
        messages,
        stream: true,
        stream_options: Some(XaiStreamOptions { include_usage: true }),
        temperature: cfg.temperature,
        top_p: cfg.top_p,
        max_completion_tokens: cfg.max_output_tokens,
        stop: cfg.stop_sequences.clone(),
        reasoning_effort,
    }
}

fn supports_reasoning_effort(model: &str) -> bool {
    !model.to_ascii_lowercase().starts_with("grok-build")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage};

    fn request(model: &str, effort: Option<ReasoningEffort>) -> LlmRequest {
        LlmRequest {
            model: model.into(),
            system_instruction: Some("sys".into()),
            history: vec![LlmMessage::new(LlmRole::User, "hi"), LlmMessage::new(LlmRole::Model, "yo")],
            generation_config: LlmGenerationConfig {
                temperature: Some(0.5),
                max_output_tokens: Some(100),
                reasoning_effort: effort,
                ..Default::default()
            },
            enable_web_search: None,
        }
    }

    #[test]
    fn a_bmp_is_sent_as_a_png_data_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bmp");
        image::RgbImage::from_pixel(1, 1, image::Rgb([1, 2, 3])).save_with_format(&path, image::ImageFormat::Bmp).unwrap();
        let mut user = LlmMessage::new(LlmRole::User, "What's this?");
        user.attachments = Some(vec![crate::llm::LlmAttachment {
            image_path: path.to_string_lossy().into_owned(),
            mime_type: "image/bmp".into(),
        }]);
        let mut request = request("grok-4.6", None);
        request.history = vec![user];

        let mapped = to_xai_request(&request);
        let url = mapped.messages[1].content[1]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        assert!(!url.contains("image/bmp"));
    }

    #[test]
    fn builds_a_chat_request_with_system_and_roles() {
        let mapped = to_xai_request(&request("grok-4.6", None));
        let roles: Vec<_> = mapped.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["system", "user", "assistant"]);
        assert!(mapped.stream);
        assert_eq!(mapped.stream_options, Some(XaiStreamOptions { include_usage: true }));
        assert_eq!(mapped.max_completion_tokens, Some(100));
        assert_eq!(mapped.temperature, Some(0.5));
    }

    #[test]
    fn maps_reasoning_effort() {
        for (effort, expected) in [
            (Some(ReasoningEffort::Low), Some("low")),
            (Some(ReasoningEffort::Medium), Some("medium")),
            (Some(ReasoningEffort::High), Some("high")),
            (Some(ReasoningEffort::Off), None), // xAI rejects "none" on 4.5+, so omit
            (None, Some("high")), // unset defaults to high rather than xAI's lower default
        ] {
            let mapped = to_xai_request(&request("grok-4.6", effort));
            assert_eq!(mapped.reasoning_effort.as_deref(), expected, "{effort:?}");
        }
    }

    #[test]
    fn omits_reasoning_effort_for_grok_build_models() {
        for model in ["grok-build-0.1", "GROK-BUILD-2"] {
            for effort in [Some(ReasoningEffort::High), None] {
                assert!(to_xai_request(&request(model, effort)).reasoning_effort.is_none(), "{model} {effort:?}");
            }
        }
    }
}

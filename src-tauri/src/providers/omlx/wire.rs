//! oMLX's OpenAI-compatible chat-completions request.
//!
//! The server is Apple Silicon only (MLX, plus GGUF through its Splash engine). It listens on
//! `127.0.0.1:8000` and serves `GET /v1/models` and `POST /v1/chat/completions`. Thinking is a
//! separate `reasoning_content` string on each delta. `enable_thinking` and `reasoning_effort` are
//! forwarded to the model's chat template; a template that doesn't know them answers 400.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::{LlmRequest, LlmRole};
use crate::models::ReasoningEffort;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8000";

/// Where oMLX is. The stored value is the server root (`http://127.0.0.1:8000`), not the `/v1`
/// prefix: a pasted `…/v1` is stripped so the models and chat paths aren't doubled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmlxRequestOptions {
    pub base_url: String,
}

impl Default for OmlxRequestOptions {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
        }
    }
}

impl OmlxRequestOptions {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }
}

pub trait OmlxOptionsSource: Send + Sync {
    fn current(&self) -> OmlxRequestOptions;
}

impl OmlxOptionsSource for OmlxRequestOptions {
    fn current(&self) -> OmlxRequestOptions {
        self.clone()
    }
}

/// `http://127.0.0.1:8000` or `http://127.0.0.1:8000/v1` both become the root the `/v1` routes hang off.
pub fn server_root(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    let trimmed = trimmed
        .strip_suffix("/v1")
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_BASE_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OmlxMessage {
    pub role: String,
    pub content: Value,
    /// An earlier model turn's trace. oMLX feeds this back into the chat template, so the model
    /// still has what it worked out on the turns that were kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OmlxChatCompletionRequest {
    pub model: String,
    pub messages: Vec<OmlxMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OmlxModelInfo {
    pub id: String,
    /// `"llm"`, `"vlm"`, `"embedding"`, `"reranker"`, … Absent on older servers.
    pub model_type: Option<String>,
    pub max_model_len: Option<i64>,
}

impl OmlxModelInfo {
    /// Embeddings and rerankers are in the same list and cannot take a chat.
    pub fn can_chat(&self) -> bool {
        !matches!(self.model_type.as_deref(), Some("embedding" | "reranker"))
    }
}

#[derive(Debug, Deserialize)]
pub struct OmlxListModelsResponse {
    pub data: Option<Vec<OmlxModelInfo>>,
}

/// Whether this body asked the model to think. A rejection of those fields is worth one retry
/// without them; a request that already asked not to think is not.
pub fn asks_to_think(body: &OmlxChatCompletionRequest) -> bool {
    body.enable_thinking == Some(true) || body.reasoning_effort.is_some()
}

pub fn to_omlx_request(request: &LlmRequest) -> OmlxChatCompletionRequest {
    let (enable_thinking, reasoning_effort) =
        thinking_controls(request.generation_config.reasoning_effort);
    to_request(request, enable_thinking, reasoning_effort)
}

/// The same chat with the thinking fields removed, for a model whose template rejects them.
pub fn without_thinking(request: &LlmRequest) -> OmlxChatCompletionRequest {
    to_request(request, None, None)
}

fn thinking_controls(effort: Option<ReasoningEffort>) -> (Option<bool>, Option<String>) {
    match effort {
        Some(ReasoningEffort::Off) => (Some(false), None),
        Some(ReasoningEffort::Low) => (Some(true), Some("low".into())),
        Some(ReasoningEffort::Medium) => (Some(true), Some("medium".into())),
        // Unset follows the same rule as the other local backends: think, at the top of the
        // ordinary ladder. A template whose top is `xhigh` says so in its 400, and the caller retries.
        Some(ReasoningEffort::High) | None => (Some(true), Some("high".into())),
    }
}

fn to_request(
    request: &LlmRequest,
    enable_thinking: Option<bool>,
    reasoning_effort: Option<String>,
) -> OmlxChatCompletionRequest {
    let mut messages = Vec::new();
    if let Some(system) = request
        .system_instruction
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        messages.push(OmlxMessage {
            role: "system".into(),
            content: Value::from(system),
            reasoning_content: None,
        });
    }
    messages.extend(request.history.iter().map(|m| {
        let reasoning = (m.role == LlmRole::Model)
            .then(|| m.reasoning.clone())
            .flatten()
            .filter(|r| !r.is_empty());
        OmlxMessage {
            role: if m.role == LlmRole::User {
                "user"
            } else {
                "assistant"
            }
            .into(),
            content: m.chat_content_as(&["image/jpeg", "image/png", "image/gif", "image/webp"]),
            reasoning_content: reasoning,
        }
    }));

    let cfg = &request.generation_config;
    OmlxChatCompletionRequest {
        model: request.model.clone(),
        messages,
        stream: true,
        temperature: cfg.temperature,
        top_p: cfg.top_p,
        max_tokens: cfg.max_output_tokens,
        stop: cfg.stop_sequences.clone(),
        enable_thinking,
        reasoning_effort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage};

    fn request(effort: Option<ReasoningEffort>, history: Vec<LlmMessage>) -> LlmRequest {
        LlmRequest {
            model: "qwen3".into(),
            system_instruction: Some("Be brief.".into()),
            history,
            generation_config: LlmGenerationConfig {
                reasoning_effort: effort,
                ..Default::default()
            },
            enable_web_search: None,
        }
    }

    #[test]
    fn a_chat_asks_to_think_at_high_and_replays_an_earlier_trace() {
        let history = vec![
            LlmMessage::new(LlmRole::User, "Hi"),
            LlmMessage {
                role: LlmRole::Model,
                text: "Hello.".into(),
                attachments: None,
                reasoning: Some("Greet them.".into()),
            },
        ];
        let body = to_omlx_request(&request(None, history));
        assert_eq!(body.enable_thinking, Some(true));
        assert_eq!(body.reasoning_effort.as_deref(), Some("high"));
        assert!(body.stream);
        assert_eq!(body.messages[0].role, "system");
        assert_eq!(
            body.messages[2].reasoning_content.as_deref(),
            Some("Greet them.")
        );
        assert!(body.messages[1].reasoning_content.is_none());
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"enable_thinking\":true"), "{json}");
        assert!(!json.contains("\"reasoning_content\":null"), "{json}");
    }

    #[test]
    fn an_off_request_disables_thinking_and_sends_no_effort() {
        let body = to_omlx_request(&request(
            Some(ReasoningEffort::Off),
            vec![LlmMessage::new(LlmRole::User, "Hi")],
        ));
        assert_eq!(body.enable_thinking, Some(false));
        assert!(body.reasoning_effort.is_none());
        assert!(!asks_to_think(&body));
    }

    #[test]
    fn a_pasted_v1_suffix_is_not_doubled() {
        assert_eq!(
            server_root("http://127.0.0.1:8000/v1/"),
            "http://127.0.0.1:8000"
        );
        assert_eq!(server_root("  "), DEFAULT_BASE_URL);
    }

    #[test]
    fn embeddings_are_not_offered_as_chat_models() {
        let embed = OmlxModelInfo {
            id: "bge".into(),
            model_type: Some("embedding".into()),
            max_model_len: None,
        };
        let chat = OmlxModelInfo {
            id: "qwen".into(),
            model_type: Some("llm".into()),
            max_model_len: Some(32768),
        };
        let untyped = OmlxModelInfo {
            id: "local".into(),
            model_type: None,
            max_model_len: None,
        };
        assert!(!embed.can_chat());
        assert!(chat.can_chat() && untyped.can_chat());
    }
}

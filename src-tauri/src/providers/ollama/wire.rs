//! Ollama's native `/api/chat` wire types, the request options, and the request mapper.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::{LlmRequest, LlmRole};
use crate::models::ReasoningEffort;

// ---- options ----

pub const DEFAULT_BASE_URL: &str = "http://localhost:11434";
pub const DEFAULT_CONTEXT_SIZE: i32 = 8192;

/// Global, user-configurable options for the Ollama provider. Unlike the cloud providers there is
/// no API key - the two things worth configuring are where the local server lives and how much
/// context window to request.
///
/// `context_size` is sent as every request's `num_ctx`. Without it, Ollama silently falls back to
/// its own (often small, e.g. 2048-4096 token) default regardless of how much history this app
/// actually sends - older messages then get dropped server-side with no error, which reads as the
/// model "forgetting" things, especially right after switching to a different local model with a
/// different built-in default. Explicit and on every request so switching models never silently
/// changes how much history the model can actually see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaRequestOptions {
    pub base_url: String,
    pub context_size: i32,
}

impl OllamaRequestOptions {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), context_size: DEFAULT_CONTEXT_SIZE }
    }
}

impl Default for OllamaRequestOptions {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

/// Where the provider reads the current options from (persisted, user-editable settings in the app).
pub trait OllamaOptionsSource: Send + Sync {
    fn current(&self) -> OllamaRequestOptions;
}

/// Fixed options - handy for tests and callers that don't need persisted settings.
impl OllamaOptionsSource for OllamaRequestOptions {
    fn current(&self) -> OllamaRequestOptions {
        self.clone()
    }
}

// ---- wire types ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OllamaMessage {
    pub role: String,
    pub content: String,
    /// Base64 pictures; Ollama takes them beside the text rather than as content parts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

/// The subset of Ollama's generation options that map onto the generation config, plus `num_ctx`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<i32>,
}

/// `think` is either `false` (thinking off) or "low"/"medium"/"high" - Ollama's field takes a bool
/// or a level string, hence a JSON value. Levels rather than `true`, since gpt-oss ignores booleans
/// while other thinking models accept both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OllamaOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub think: Option<Value>,
}

/// The base chat API keeps the model's thinking trace in `thinking`, separate from `content`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OllamaResponseMessage {
    pub content: Option<String>,
    pub thinking: Option<String>,
}

/// One newline-delimited JSON object from `/api/chat`. The final object has `done` true and carries
/// the eval counts instead of a delta.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OllamaChatStreamEvent {
    pub message: Option<OllamaResponseMessage>,
    #[serde(default)]
    pub done: bool,
    pub done_reason: Option<String>,
    pub prompt_eval_count: Option<i32>,
    pub eval_count: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaErrorResponse {
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaModelInfo {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct OllamaTagsResponse {
    pub models: Option<Vec<OllamaModelInfo>>,
}

// ---- mapper ----

/// `context_size` is the configured `num_ctx`, or `None` to omit it. The provider always passes the
/// current one so a request's context window is never silently left to Ollama's own default.
pub fn to_ollama_request(request: &LlmRequest, context_size: Option<i32>) -> OllamaChatRequest {
    let mut messages = Vec::new();
    if let Some(system) = request.system_instruction.as_deref().filter(|s| !s.is_empty()) {
        messages.push(OllamaMessage { role: "system".into(), content: system.into(), images: Vec::new() });
    }
    messages.extend(request.history.iter().map(|m| OllamaMessage {
        role: if m.role == LlmRole::User { "user" } else { "assistant" }.into(),
        content: m.text.clone(),
        images: if m.role == LlmRole::User { m.attachments.iter().flatten().filter_map(|a| a.base64()).collect() } else { Vec::new() },
    }));

    let cfg = &request.generation_config;
    let has_options = cfg.temperature.is_some()
        || cfg.top_p.is_some()
        || cfg.top_k.is_some()
        || cfg.max_output_tokens.is_some()
        || cfg.stop_sequences.is_some()
        || context_size.is_some();
    let options = has_options.then(|| OllamaOptions {
        temperature: cfg.temperature,
        top_p: cfg.top_p,
        top_k: cfg.top_k,
        num_predict: cfg.max_output_tokens,
        stop: cfg.stop_sequences.clone(),
        num_ctx: context_size,
    });

    OllamaChatRequest {
        model: request.model.clone(),
        messages,
        stream: true,
        options,
        think: to_think(cfg.reasoning_effort),
    }
}

fn to_think(effort: Option<ReasoningEffort>) -> Option<Value> {
    Some(match effort? {
        ReasoningEffort::Off => Value::Bool(false),
        ReasoningEffort::Low => Value::from("low"),
        ReasoningEffort::Medium => Value::from("medium"),
        ReasoningEffort::High => Value::from("high"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage};

    fn request(system: Option<&str>, history: Vec<LlmMessage>, config: LlmGenerationConfig) -> LlmRequest {
        LlmRequest {
            model: "llama3".into(),
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
        let mapped = to_ollama_request(&request(Some("Be helpful."), hi(), Default::default()), None);
        assert_eq!(mapped.messages.len(), 2);
        assert_eq!((mapped.messages[0].role.as_str(), mapped.messages[0].content.as_str()), ("system", "Be helpful."));
        assert_eq!(mapped.messages[1].role, "user");
    }

    #[test]
    fn omits_the_system_message_when_none() {
        let mapped = to_ollama_request(&request(None, hi(), Default::default()), None);
        assert_eq!(mapped.messages.len(), 1);
    }

    #[test]
    fn maps_the_model_role_to_assistant() {
        let history = vec![LlmMessage::new(LlmRole::User, "Hi"), LlmMessage::new(LlmRole::Model, "Hello!")];
        let mapped = to_ollama_request(&request(None, history, Default::default()), None);
        assert_eq!(mapped.messages[1].role, "assistant");
    }

    #[test]
    fn always_requests_streaming() {
        assert!(to_ollama_request(&request(None, hi(), Default::default()), None).stream);
    }

    #[test]
    fn omits_options_when_the_generation_config_is_empty() {
        assert!(to_ollama_request(&request(None, hi(), Default::default()), None).options.is_none());
    }

    #[test]
    fn forwards_generation_config_fields_into_options() {
        let config = LlmGenerationConfig {
            temperature: Some(0.5),
            top_p: Some(0.9),
            top_k: Some(40),
            max_output_tokens: Some(256),
            stop_sequences: Some(vec!["STOP".into()]),
            ..Default::default()
        };
        let options = to_ollama_request(&request(None, hi(), config), None).options.unwrap();
        assert_eq!(options.temperature, Some(0.5));
        assert_eq!(options.top_p, Some(0.9));
        assert_eq!(options.top_k, Some(40));
        assert_eq!(options.num_predict, Some(256));
        assert_eq!(options.stop, Some(vec!["STOP".to_string()]));
        assert_eq!(options.num_ctx, None);
    }

    #[test]
    fn includes_num_ctx_when_a_context_size_is_provided() {
        let options = to_ollama_request(&request(None, hi(), Default::default()), Some(8192)).options.unwrap();
        assert_eq!(options.num_ctx, Some(8192));
    }

    #[test]
    fn includes_num_ctx_even_when_the_generation_config_is_otherwise_empty() {
        let options = to_ollama_request(&request(None, hi(), Default::default()), Some(4096)).options.unwrap();
        assert_eq!(options.temperature, None);
        assert_eq!(options.num_ctx, Some(4096));
    }

    #[test]
    fn maps_reasoning_effort_to_think() {
        for (effort, expected) in [
            (None, None),
            (Some(ReasoningEffort::Off), Some("false")),
            (Some(ReasoningEffort::Low), Some("\"low\"")),
            (Some(ReasoningEffort::Medium), Some("\"medium\"")),
            (Some(ReasoningEffort::High), Some("\"high\"")),
        ] {
            let config = LlmGenerationConfig { reasoning_effort: effort, ..Default::default() };
            let think = to_ollama_request(&request(None, hi(), config), None).think;
            assert_eq!(think.map(|v| v.to_string()).as_deref(), expected, "{effort:?}");
        }
    }

    #[test]
    fn serializes_think_and_options_with_ollamas_field_names() {
        let config = LlmGenerationConfig { max_output_tokens: Some(9), reasoning_effort: Some(ReasoningEffort::Off), ..Default::default() };
        let json = serde_json::to_string(&to_ollama_request(&request(None, hi(), config), Some(100))).unwrap();
        assert!(json.contains("\"options\":{\"num_predict\":9,\"num_ctx\":100}"), "{json}");
        assert!(json.contains("\"think\":false"), "{json}");
    }
}

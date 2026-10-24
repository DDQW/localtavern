//! Mistral's chat-completions wire types and the request mapper.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::{LlmRequest, LlmRole};
use crate::models::ReasoningEffort;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MistralMessage {
    pub role: String,
    pub content: Value,
}

/// No `stream_options`: Mistral already reports usage on the final chunk, and doesn't document the
/// option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MistralChatCompletionRequest {
    pub model: String,
    pub messages: Vec<MistralMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct MistralCapabilities {
    pub completion_chat: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct MistralModelInfo {
    pub id: String,
    pub capabilities: Option<MistralCapabilities>,
    pub max_context_length: Option<i32>,
}

impl MistralModelInfo {
    /// The model list also carries embedding, moderation and OCR models that can't chat.
    pub fn can_chat(&self) -> bool {
        self.capabilities.as_ref().and_then(|c| c.completion_chat).unwrap_or(true)
    }
}

#[derive(Debug, Deserialize)]
pub struct MistralListModelsResponse {
    pub data: Option<Vec<MistralModelInfo>>,
}

/// Matches the app's `[System note: ...]` convention, which roleplay prompts tell the model to follow silently.
const NEXT_REPLY_NOTE: &str = "[System note: Write the next reply.]";

pub fn to_mistral_request(request: &LlmRequest) -> MistralChatCompletionRequest {
    let mut messages = Vec::new();
    if let Some(system) = request.system_instruction.as_deref().filter(|s| !s.is_empty()) {
        messages.push(MistralMessage { role: "system".into(), content: Value::from(system) });
    }
    messages.extend(request.history.iter().map(|m| MistralMessage {
        role: if m.role == LlmRole::User { "user" } else { "assistant" }.into(),
        content: m.chat_content(),
    }));
    // Mistral rejects a conversation that ends on an assistant turn (the next group member's reply
    // follows straight after the previous one's). Its `prefix` flag would accept it, but then the
    // model carries on writing that previous message instead of starting a new one.
    if messages.last().is_some_and(|m| m.role == "assistant") {
        messages.push(MistralMessage { role: "user".into(), content: Value::from(NEXT_REPLY_NOTE) });
    }

    let cfg = &request.generation_config;
    MistralChatCompletionRequest {
        model: request.model.clone(),
        messages,
        stream: true,
        reasoning_effort: to_reasoning_effort(&request.model, cfg.reasoning_effort).map(String::from),
        temperature: cfg.temperature,
        top_p: cfg.top_p,
        max_tokens: cfg.max_output_tokens,
        stop: cfg.stop_sequences.clone(),
    }
}

/// The hybrid models only think when asked (`reasoning_effort`), and take just "high" or "none".
/// Unset asks for "high", so a reasoning-capable model shows its thinking without a preset change.
/// Other models (Ministral, Codestral, older dated ids) get nothing, since they don't take the field.
fn to_reasoning_effort(model: &str, effort: Option<ReasoningEffort>) -> Option<&'static str> {
    let model = model.to_ascii_lowercase();
    // Z.ai GLM takes low/high/max but not "none".
    if model.starts_with("zai-glm") {
        return match effort {
            Some(ReasoningEffort::Off) => None,
            Some(ReasoningEffort::Low) => Some("low"),
            Some(ReasoningEffort::Medium) | None => Some("high"),
            Some(ReasoningEffort::High) => Some("max"),
        };
    }
    const HYBRID: [&str; 8] = [
        "mistral-small-latest",
        "mistral-small-2603",
        "mistral-medium-latest",
        "mistral-medium-3-5",
        "mistral-medium-2604",
        "mistral-large-latest",
        "mistral-large-4",
        "mistral-large-2610",
    ];
    if !HYBRID.iter().any(|id| model.starts_with(id)) {
        return None;
    }
    Some(if effort == Some(ReasoningEffort::Off) { "none" } else { "high" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage};

    fn request(system: Option<&str>, history: Vec<LlmMessage>, config: LlmGenerationConfig) -> LlmRequest {
        LlmRequest {
            model: "mistral-large-latest".into(),
            system_instruction: system.map(String::from),
            history,
            generation_config: config,
            enable_web_search: None,
        }
    }

    #[test]
    fn maps_system_history_and_sampling() {
        let config = LlmGenerationConfig {
            temperature: Some(0.7),
            top_p: Some(0.9),
            max_output_tokens: Some(256),
            stop_sequences: Some(vec!["END".into()]),
            ..Default::default()
        };
        let mapped = to_mistral_request(&request(
            Some("be brief"),
            vec![LlmMessage::new(LlmRole::User, "hi"), LlmMessage::new(LlmRole::Model, "yo"), LlmMessage::new(LlmRole::User, "again")],
            config,
        ));

        let roles: Vec<_> = mapped.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"]);
        assert!(mapped.stream);
        assert_eq!(mapped.temperature, Some(0.7));
        assert_eq!(mapped.top_p, Some(0.9));
        assert_eq!(mapped.max_tokens, Some(256));
        assert_eq!(mapped.stop, Some(vec!["END".to_string()]));
    }

    #[test]
    fn a_trailing_assistant_turn_gets_a_user_note_asking_for_the_next_reply() {
        let mapped = to_mistral_request(&request(
            None,
            vec![LlmMessage::new(LlmRole::User, "hi"), LlmMessage::new(LlmRole::Model, "Alice: Hello!")],
            LlmGenerationConfig::default(),
        ));

        let roles: Vec<_> = mapped.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        assert_eq!(mapped.messages[2].content, NEXT_REPLY_NOTE);
    }

    #[test]
    fn unset_options_are_not_sent() {
        let mut r = request(None, vec![LlmMessage::new(LlmRole::User, "hi")], LlmGenerationConfig::default());
        r.model = "codestral-latest".into();
        let json = serde_json::to_string(&to_mistral_request(&r)).unwrap();

        for absent in ["reasoning", "temperature", "top_p", "max_tokens", "stop", "stream_options", "prefix"] {
            assert!(!json.contains(absent), "{absent} in {json}");
        }
    }

    #[test]
    fn maps_reasoning_effort_per_model() {
        use ReasoningEffort::*;
        for (model, effort, expected) in [
            ("mistral-small-latest", None, Some("high")),
            ("mistral-medium-3-5", Some(Low), Some("high")),
            ("mistral-large-2610", Some(High), Some("high")),
            ("mistral-small-latest", Some(Off), Some("none")),
            ("zai-glm-5-3", Some(Off), None), // GLM rejects "none"
            ("zai-glm-5-3", Some(Low), Some("low")),
            ("zai-glm-5-3", Some(High), Some("max")),
            ("ministral-8b-latest", Some(High), None),
            ("codestral-latest", None, None),
            ("mistral-small-2506", Some(High), None),
        ] {
            let mut r = request(None, vec![LlmMessage::new(LlmRole::User, "hi")], LlmGenerationConfig { reasoning_effort: effort, ..Default::default() });
            r.model = model.into();
            assert_eq!(to_mistral_request(&r).reasoning_effort.as_deref(), expected, "{model} {effort:?}");
        }
    }

    #[test]
    fn only_chat_capable_models_are_listed_as_chat() {
        let parsed: MistralListModelsResponse = serde_json::from_str(
            r#"{"data":[
                {"id":"mistral-large-latest","capabilities":{"completion_chat":true}},
                {"id":"mistral-embed","capabilities":{"completion_chat":false}},
                {"id":"no-capabilities-listed"}
            ]}"#,
        )
        .unwrap();

        let chat: Vec<_> = parsed.data.unwrap().into_iter().filter(|m| m.can_chat()).map(|m| m.id).collect();
        assert_eq!(chat, ["mistral-large-latest", "no-capabilities-listed"]);
    }
}

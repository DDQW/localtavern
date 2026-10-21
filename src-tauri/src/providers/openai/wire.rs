//! Wire types for the ChatGPT Codex backend, which speaks OpenAI's Responses API, plus the mapper.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::{LlmRequest, LlmRole};
use crate::models::ReasoningEffort;

/// `type` is "input_text" for user turns and "output_text" for assistant turns - the Responses API
/// rejects the wrong one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponsesContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `input_image` parts carry a data URL here instead of text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponsesInputItem {
    pub role: String,
    pub content: Vec<ResponsesContentPart>,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponsesReasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// `instructions` is the system prompt; the Codex backend wants the field present even when empty
/// and rejects system-role input items. `store` is always false - the subscription backend doesn't
/// persist responses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponsesRequest {
    pub model: String,
    pub instructions: String,
    pub input: Vec<ResponsesInputItem>,
    pub stream: bool,
    pub store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponsesReasoning>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ResponsesUsage {
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ResponsesIncompleteDetails {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ResponsesError {
    pub message: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ResponsesSummary {
    pub incomplete_details: Option<ResponsesIncompleteDetails>,
    pub error: Option<ResponsesError>,
    pub usage: Option<ResponsesUsage>,
}

/// One SSE `data:` event, kept deliberately loose: the Responses stream has dozens of event types
/// with differing shapes and only a handful matter here. Unknown events deserialize fine and are
/// ignored. `delta` is a raw value because some event types carry non-string deltas.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ResponsesEvent {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub delta: Option<Value>,
    pub response: Option<ResponsesSummary>,
    pub error: Option<ResponsesError>,
    pub message: Option<String>,
    pub code: Option<String>,
}

/// Subset of a catalog entry from `/codex/models`.
#[derive(Debug, Deserialize)]
pub struct CodexModelInfo {
    pub slug: String,
    pub visibility: Option<String>,
    pub priority: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct CodexModelsResponse {
    pub models: Option<Vec<CodexModelInfo>>,
}

/// Builds a Responses-API request for the ChatGPT Codex backend. Sampling parameters (temperature,
/// top_p, max output tokens) are intentionally not sent: this backend serves reasoning models that
/// reject them, and an unsupported parameter fails the whole request. "Off"/unset reasoning effort
/// omits the block entirely so the model keeps its own default.
pub fn to_responses_request(request: &LlmRequest) -> ResponsesRequest {
    let input = request
        .history
        .iter()
        .map(|m| {
            let (role, part_kind) = if m.role == LlmRole::User { ("user", "input_text") } else { ("assistant", "output_text") };
            let mut content = vec![ResponsesContentPart { kind: part_kind.into(), text: Some(m.text.clone()), image_url: None }];
            if m.role == LlmRole::User {
                content.extend(m.attachments.iter().flatten().filter_map(|a| a.data_url()).map(|url| ResponsesContentPart {
                    kind: "input_image".into(),
                    text: None,
                    image_url: Some(url),
                }));
            }
            ResponsesInputItem {
                role: role.into(),
                content,
                kind: "message".into(),
            }
        })
        .collect();

    let effort = match request.generation_config.reasoning_effort {
        Some(ReasoningEffort::Low) => Some("low"),
        Some(ReasoningEffort::Medium) => Some("medium"),
        Some(ReasoningEffort::High) => Some("high"),
        _ => None,
    };

    ResponsesRequest {
        model: request.model.clone(),
        instructions: request.system_instruction.clone().unwrap_or_default(),
        input,
        stream: true,
        store: false,
        reasoning: effort.map(|e| ResponsesReasoning { effort: Some(e.into()), summary: Some("auto".into()) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage};

    fn request(effort: Option<ReasoningEffort>) -> LlmRequest {
        LlmRequest {
            model: "gpt-5-codex".into(),
            system_instruction: Some("be brief".into()),
            history: vec![
                LlmMessage::new(LlmRole::User, "hi"),
                LlmMessage::new(LlmRole::Model, "yo"),
                LlmMessage::new(LlmRole::User, "again"),
            ],
            generation_config: LlmGenerationConfig {
                temperature: Some(0.7),
                top_p: Some(0.9),
                max_output_tokens: Some(50),
                reasoning_effort: effort,
                ..Default::default()
            },
            enable_web_search: None,
        }
    }

    #[test]
    fn uses_instructions_and_typed_content_parts_and_omits_sampling_params() {
        let json = serde_json::to_value(to_responses_request(&request(None))).unwrap();

        assert_eq!(json["instructions"], "be brief");
        assert_eq!(json["store"], false);
        assert_eq!(json["stream"], true);
        for absent in ["temperature", "top_p", "max_output_tokens", "reasoning"] {
            assert!(json.get(absent).is_none(), "{absent} should not be sent");
        }

        let input = json["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[1]["content"][0]["text"], "yo");
    }

    #[test]
    fn sends_empty_instructions_when_there_is_no_system_prompt() {
        let mut r = request(None);
        r.system_instruction = None;
        assert_eq!(to_responses_request(&r).instructions, "");
        // The field must be present in the JSON even when empty.
        assert_eq!(serde_json::to_value(to_responses_request(&r)).unwrap()["instructions"], "");
    }

    #[test]
    fn maps_reasoning_effort_and_requests_summaries() {
        for (effort, expected) in [
            (Some(ReasoningEffort::Low), Some("low")),
            (Some(ReasoningEffort::Medium), Some("medium")),
            (Some(ReasoningEffort::High), Some("high")),
            (Some(ReasoningEffort::Off), None),
            (None, None),
        ] {
            let reasoning = to_responses_request(&request(effort)).reasoning;
            assert_eq!(reasoning.as_ref().and_then(|r| r.effort.as_deref()), expected, "{effort:?}");
            assert_eq!(
                reasoning.as_ref().and_then(|r| r.summary.as_deref()),
                expected.map(|_| "auto"),
                "{effort:?}"
            );
        }
    }
}

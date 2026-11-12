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
        self.capabilities
            .as_ref()
            .and_then(|c| c.completion_chat)
            .unwrap_or(true)
    }
}

#[derive(Debug, Deserialize)]
pub struct MistralListModelsResponse {
    pub data: Option<Vec<MistralModelInfo>>,
}

/// Matches the app's `[System note: ...]` convention, which roleplay prompts tell the model to follow silently.
const NEXT_REPLY_NOTE: &str = "[System note: Write the next reply.]";

/// How much of Mistral's reasoning protocol a call uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestShape {
    /// `reasoning_effort` when this model takes it, and earlier thinking replayed only then.
    Configured,
    /// Magistral's native protocol: replay thinking chunks, and never send `reasoning_effort`.
    /// Those models reason on their own; the effort field is for the newer hybrid models.
    Native,
    /// No `reasoning_effort` and no thinking chunks.
    Plain,
}

pub fn to_mistral_request(request: &LlmRequest) -> MistralChatCompletionRequest {
    to_mistral_request_shaped(request, RequestShape::Configured)
}

/// The same request for a model that turned out not to take `reasoning_effort`.
pub fn to_mistral_request_without_reasoning(request: &LlmRequest) -> MistralChatCompletionRequest {
    to_mistral_request_shaped(request, RequestShape::Plain)
}

pub fn to_mistral_request_shaped(
    request: &LlmRequest,
    shape: RequestShape,
) -> MistralChatCompletionRequest {
    build_request(request, shape)
}

pub fn is_magistral(model: &str) -> bool {
    model.to_ascii_lowercase().starts_with("magistral")
}

pub fn shape_label(shape: RequestShape) -> &'static str {
    match shape {
        RequestShape::Configured => "with reasoning_effort",
        RequestShape::Native => "native (thinking replay, no reasoning_effort)",
        RequestShape::Plain => "plain text (no reasoning_effort, no thinking replay)",
    }
}

/// Whether an error is Mistral refusing the reasoning setting (an unsupported level, or the field on
/// a model that has none).
pub fn is_reasoning_rejection(err: &crate::providers::error::ProviderError) -> bool {
    err.kind == crate::providers::error::ProviderErrorKind::InvalidRequest
        && err.message.to_ascii_lowercase().contains("reasoning")
}

/// The refusal names levels the model does accept (`supported values: low, medium, high`). That is a
/// bad value for this call, not a model without reasoning — remembering it as such would drop
/// thinking on the next chat. "Unsupported reasoning level: high" does not qualify: it names no
/// alternative.
pub fn rejection_names_accepted_levels(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("supported value")
        || msg.contains("allowed value")
        || msg.contains("must be one of")
        || msg.contains("valid value")
}

fn build_request(request: &LlmRequest, shape: RequestShape) -> MistralChatCompletionRequest {
    let mut messages = Vec::new();
    if let Some(system) = request
        .system_instruction
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        // A thinking chunk in the system message is what Mistral's old Magistral docs show, but the
        // live endpoint answers "service not available" to it. The app prompt stays a plain string.
        messages.push(MistralMessage {
            role: "system".into(),
            content: Value::from(system),
        });
    }
    let cfg = &request.generation_config;
    let reasoning_effort = match shape {
        RequestShape::Configured => to_reasoning_effort(&request.model, cfg.reasoning_effort),
        RequestShape::Native | RequestShape::Plain => None,
    };
    // Native Magistral always wants an earlier trace replayed. The other shapes replay only while
    // `reasoning_effort` is on, which is how a model that refused the field stays a plain string.
    let replay_reasoning = match shape {
        RequestShape::Native => true,
        RequestShape::Plain => false,
        RequestShape::Configured => reasoning_effort.is_some_and(|e| e != "none"),
    };
    messages.extend(request.history.iter().map(|m| {
        MistralMessage {
            role: if m.role == LlmRole::User {
                "user"
            } else {
                "assistant"
            }
            .into(),
            content: match m
                .reasoning
                .as_deref()
                .filter(|_| replay_reasoning && m.role == LlmRole::Model)
            {
                Some(thought) => replayed_content(thought, &m.text),
                // The object form of `image_url` is what Mistral actually accepts, even though the docs
                // show a bare string.
                None => m.chat_content_as(&[
                    "image/jpeg",
                    "image/png",
                    "image/gif",
                    "image/webp",
                    "image/bmp",
                ]),
            },
        }
    }));
    // Mistral rejects a conversation that ends on an assistant turn (the next group member's reply
    // follows straight after the previous one's). Its `prefix` flag would accept it, but then the
    // model carries on writing that previous message instead of starting a new one.
    if messages.last().is_some_and(|m| m.role == "assistant") {
        messages.push(MistralMessage {
            role: "user".into(),
            content: Value::from(NEXT_REPLY_NOTE),
        });
    }

    MistralChatCompletionRequest {
        model: request.model.clone(),
        messages,
        stream: true,
        reasoning_effort: reasoning_effort.map(String::from),
        temperature: cfg.temperature,
        top_p: cfg.top_p,
        max_tokens: cfg.max_output_tokens,
        stop: cfg.stop_sequences.clone(),
    }
}

/// Mistral wants an earlier turn's reasoning replayed as a `thinking` chunk ahead of its reply, and
/// says the model loses coherence when it is stripped.
fn replayed_content(thought: &str, reply: &str) -> Value {
    // `closed` marks a finished trace. An open thinking chunk is a prefix the model continues,
    // which is not what a previous turn is.
    serde_json::json!([
        {"type": "thinking", "thinking": [{"type": "text", "text": thought}], "closed": true},
        {"type": "text", "text": reply},
    ])
}

/// Every reasoning model is asked for the strongest level it accepts. A preset cannot lower it.
/// GLM's top is "max". Large 4, Small, Medium, and Magistral top out at "high" and reject anything above it.
/// Other models get nothing, since they don't take the field: Ministral, Codestral, Devstral,
/// Large 3 and older dated ids, and DeepSeek V4.1 Flash (`deepseek-v4-1-flash-1017`) as hosted here.
fn to_reasoning_effort(model: &str, _effort: Option<ReasoningEffort>) -> Option<&'static str> {
    let model = model.to_ascii_lowercase();
    if model.starts_with("zai-glm") {
        return Some("max");
    }
    // `mistral-large-4` also matches `mistral-large-4-0`. `mistral-large-2610` is the dated id of
    // the same 26.10 release. If an alias still points at Large 3, which rejects the field, the
    // provider retries that call without it (see `is_reasoning_rejection`).
    const LARGE_4: [&str; 3] = [
        "mistral-large-latest",
        "mistral-large-4",
        "mistral-large-2610",
    ];
    const HYBRID: [&str; 5] = [
        "mistral-small-latest",
        "mistral-small-2603",
        "mistral-medium-latest",
        "mistral-medium-3-5",
        "mistral-medium-2604",
    ];
    if LARGE_4.iter().any(|id| model.starts_with(id))
        || model.starts_with("magistral")
        || HYBRID.iter().any(|id| model.starts_with(id))
    {
        return Some("high");
    }
    None
}

/// What was sent, without the prompt text. A refusal of "service not available" says nothing about
/// which field the server objected to, so the chat error quotes this next to the raw response.
pub fn describe_request(body: &MistralChatCompletionRequest) -> String {
    let messages: Vec<String> = body.messages.iter().map(describe_message).collect();
    format!(
        "model={} reasoning_effort={} temperature={} top_p={} max_tokens={} stop={} stream={} messages=[{}]",
        body.model,
        body.reasoning_effort.as_deref().unwrap_or("omitted"),
        describe_option(body.temperature),
        describe_option(body.top_p),
        describe_option(body.max_tokens),
        body.stop.as_ref().map(|s| s.join("|")).unwrap_or_else(|| "omitted".into()),
        body.stream,
        messages.join(", "),
    )
}

fn describe_option<T: std::fmt::Display>(value: Option<T>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "omitted".into())
}

fn describe_message(message: &MistralMessage) -> String {
    format!("{}:{}", message.role, describe_content(&message.content))
}

fn describe_content(content: &Value) -> String {
    match content {
        Value::String(text) => format!("text({} chars)", text.chars().count()),
        Value::Array(parts) => {
            let kinds: Vec<String> = parts.iter().map(describe_part).collect();
            format!("[{}]", kinds.join(", "))
        }
        other => format!("other({} chars)", other.to_string().chars().count()),
    }
}

fn describe_part(part: &Value) -> String {
    let kind = part
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("unknown");
    let chars = part
        .get("text")
        .and_then(|t| t.as_str())
        .map(|t| t.chars().count())
        .or_else(|| {
            part.pointer("/thinking/0/text")
                .and_then(|t| t.as_str())
                .map(|t| t.chars().count())
        });
    match chars {
        Some(n) => format!("{kind}({n} chars)"),
        None => kind.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage};

    fn request(
        system: Option<&str>,
        history: Vec<LlmMessage>,
        config: LlmGenerationConfig,
    ) -> LlmRequest {
        LlmRequest {
            model: "mistral-small-latest".into(),
            system_instruction: system.map(String::from),
            history,
            generation_config: config,
            enable_web_search: None,
        }
    }

    #[test]
    fn a_user_picture_is_an_image_url_object() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cat.png");
        std::fs::write(&path, b"png").unwrap();
        let mut user = LlmMessage::new(LlmRole::User, "What's this?");
        user.attachments = Some(vec![crate::llm::LlmAttachment {
            image_path: path.to_string_lossy().into_owned(),
            mime_type: "image/png".into(),
        }]);
        let mut model = LlmMessage::new(LlmRole::Model, "A cat.");
        model.attachments = user.attachments.clone();

        let mapped = to_mistral_request(&request(None, vec![user, model], Default::default()));
        let parts = mapped.messages[0].content.as_array().unwrap();
        assert_eq!(parts[0]["text"], "What's this?");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,cG5n");
        assert_eq!(mapped.messages[1].content, "A cat.");
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
            vec![
                LlmMessage::new(LlmRole::User, "hi"),
                LlmMessage::new(LlmRole::Model, "yo"),
                LlmMessage::new(LlmRole::User, "again"),
            ],
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
            vec![
                LlmMessage::new(LlmRole::User, "hi"),
                LlmMessage::new(LlmRole::Model, "Alice: Hello!"),
            ],
            LlmGenerationConfig::default(),
        ));

        let roles: Vec<_> = mapped.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        assert_eq!(mapped.messages[2].content, NEXT_REPLY_NOTE);
    }

    #[test]
    fn unset_options_are_not_sent() {
        let mut r = request(
            None,
            vec![LlmMessage::new(LlmRole::User, "hi")],
            LlmGenerationConfig::default(),
        );
        r.model = "codestral-latest".into();
        let json = serde_json::to_string(&to_mistral_request(&r)).unwrap();

        for absent in [
            "reasoning",
            "temperature",
            "top_p",
            "max_tokens",
            "stop",
            "stream_options",
            "prefix",
        ] {
            assert!(!json.contains(absent), "{absent} in {json}");
        }
    }

    #[test]
    fn maps_reasoning_effort_per_model() {
        use ReasoningEffort::*;
        for (model, effort, expected) in [
            ("mistral-small-latest", None, Some("high")),
            ("mistral-medium-3-5", Some(Low), Some("high")),
            ("mistral-medium-3-5", Some(Medium), Some("high")),
            ("mistral-large-2610", Some(High), Some("high")),
            ("mistral-large-4-0", None, Some("high")),
            ("mistral-large-4", Some(Low), Some("high")),
            ("mistral-large-4", Some(Medium), Some("high")),
            ("mistral-large-latest", Some(Off), Some("high")),
            ("mistral-large-2512", None, None), // Large 3
            ("mistral-small-latest", Some(Off), Some("high")),
            ("magistral-small-latest", None, Some("high")),
            ("magistral-medium-2509", Some(Low), Some("high")),
            ("magistral-medium-latest", Some(Medium), Some("high")),
            ("magistral-small-2506", Some(Off), Some("high")),
            ("zai-glm-5-3", Some(Off), Some("max")),
            ("zai-glm-5-3-flash", Some(Medium), Some("max")),
            ("zai-glm-5-2", Some(High), Some("max")),
            ("deepseek-v4-1-flash-1017", Some(High), None),
            ("devstral-2512", Some(High), None),
            ("ministral-14b-2512", Some(High), None),
            ("zai-glm-5-3", Some(Low), Some("max")),
            ("zai-glm-5-3", None, Some("max")),
            ("ministral-8b-latest", Some(High), None),
            ("codestral-latest", None, None),
            ("mistral-small-2506", Some(High), None),
        ] {
            let mut r = request(
                None,
                vec![LlmMessage::new(LlmRole::User, "hi")],
                LlmGenerationConfig {
                    reasoning_effort: effort,
                    ..Default::default()
                },
            );
            r.model = model.into();
            assert_eq!(
                to_mistral_request(&r).reasoning_effort.as_deref(),
                expected,
                "{model} {effort:?}"
            );
        }
    }

    #[test]
    fn magistral_system_prompt_stays_a_plain_string() {
        let mut r = request(
            Some("Stay in character."),
            vec![LlmMessage::new(LlmRole::User, "hi")],
            LlmGenerationConfig::default(),
        );
        r.model = "magistral-medium-latest".into();
        let mapped = to_mistral_request(&r);
        assert_eq!(mapped.messages[0].content, "Stay in character.");
        assert_eq!(mapped.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn native_magistral_replays_thinking_and_omits_reasoning_effort() {
        let mut turn = LlmMessage::new(LlmRole::Model, "Alice: Hello!");
        turn.reasoning = Some("She greets him.".into());
        let mut r = request(
            Some("Stay in character."),
            vec![LlmMessage::new(LlmRole::User, "hi"), turn],
            LlmGenerationConfig::default(),
        );
        r.model = "magistral-medium-latest".into();

        let mapped = to_mistral_request_shaped(&r, RequestShape::Native);

        assert_eq!(mapped.reasoning_effort, None);
        assert_eq!(mapped.messages[0].content, "Stay in character.");
        assert_eq!(mapped.messages[2].content[0]["type"], "thinking");
        let described = describe_request(&mapped);
        assert!(
            described.contains("reasoning_effort=omitted"),
            "{described}"
        );
        assert!(described.contains("thinking(15 chars)"), "{described}");
        assert!(!described.contains("She greets him"), "{described}");
        assert!(!described.contains("Stay in character"), "{described}");
    }

    #[test]
    fn earlier_reasoning_is_replayed_only_while_the_model_is_thinking() {
        let mut turn = LlmMessage::new(LlmRole::Model, "Alice: Hello!");
        turn.reasoning = Some("She greets him.".into());
        let history = vec![
            LlmMessage::new(LlmRole::User, "hi"),
            turn,
            LlmMessage::new(LlmRole::User, "again"),
        ];

        let mapped = to_mistral_request(&request(
            None,
            history.clone(),
            LlmGenerationConfig::default(),
        ));
        assert_eq!(
            mapped.messages[1].content,
            serde_json::json!([
                {"type": "thinking", "thinking": [{"type": "text", "text": "She greets him."}], "closed": true},
                {"type": "text", "text": "Alice: Hello!"},
            ])
        );

        // Off is still the top level, so the earlier trace stays on the turn.
        let off = LlmGenerationConfig {
            reasoning_effort: Some(ReasoningEffort::Off),
            ..Default::default()
        };
        assert_eq!(
            to_mistral_request(&request(None, history.clone(), off.clone())).messages[1].content[0]
                ["type"],
            "thinking"
        );

        let mut magistral = request(None, history.clone(), LlmGenerationConfig::default());
        magistral.model = "magistral-medium-latest".into();
        assert_eq!(
            to_mistral_request(&magistral).messages[1].content[0]["type"],
            "thinking"
        );
        let mut magistral_off = request(None, history.clone(), off.clone());
        magistral_off.model = "magistral-small-2506".into();
        assert_eq!(
            to_mistral_request(&magistral_off).messages[1].content[0]["type"],
            "thinking"
        );

        let mut large = request(None, history.clone(), off);
        large.model = "mistral-large-latest".into();
        let replayed = to_mistral_request(&large);
        assert_eq!(replayed.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(replayed.messages[1].content[0]["type"], "thinking");

        let mut codestral = request(None, history, LlmGenerationConfig::default());
        codestral.model = "codestral-latest".into();
        assert_eq!(
            to_mistral_request(&codestral).messages[1].content,
            "Alice: Hello!"
        );
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

        let chat: Vec<_> = parsed
            .data
            .unwrap()
            .into_iter()
            .filter(|m| m.can_chat())
            .map(|m| m.id)
            .collect();
        assert_eq!(chat, ["mistral-large-latest", "no-capabilities-listed"]);
    }
}

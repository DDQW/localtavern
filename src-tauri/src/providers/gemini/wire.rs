//! Gemini `generateContent` wire types, the request options, and the mapper from [`LlmRequest`].

use std::collections::BTreeMap;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::llm::{LlmMessage, LlmRequest, LlmRole};
use crate::models::ReasoningEffort;

// ---- options ----

/// The four configurable categories. HARM_CATEGORY_CIVIC_INTEGRITY is deprecated by Google and
/// deliberately not offered; HARM_CATEGORY_JAILBREAK is off by default and only exists on a single
/// preview model, so it's left alone rather than exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GeminiSafetyCategory {
    Harassment,
    HateSpeech,
    SexuallyExplicit,
    DangerousContent,
}

impl GeminiSafetyCategory {
    pub const ALL: [Self; 4] = [Self::Harassment, Self::HateSpeech, Self::SexuallyExplicit, Self::DangerousContent];

    pub fn api_category(self) -> &'static str {
        match self {
            Self::Harassment => "HARM_CATEGORY_HARASSMENT",
            Self::HateSpeech => "HARM_CATEGORY_HATE_SPEECH",
            Self::SexuallyExplicit => "HARM_CATEGORY_SEXUALLY_EXPLICIT",
            Self::DangerousContent => "HARM_CATEGORY_DANGEROUS_CONTENT",
        }
    }
}

/// Mirrors Gemini's HarmBlockThreshold enum. `Unspecified` means "don't send an override for this
/// category" - Gemini applies its own built-in default. `Off` is distinct from `BlockNone`:
/// BlockNone still runs the category's classifier and returns scores but never blocks on them, while
/// Off skips evaluating that category entirely. Neither touches Gemini's separate, non-configurable
/// PROHIBITED_CONTENT filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GeminiSafetyThreshold {
    Unspecified,
    Off,
    BlockNone,
    BlockOnlyHigh,
    BlockMediumAndAbove,
    BlockLowAndAbove,
}

impl GeminiSafetyThreshold {
    /// `None` for `Unspecified` (nothing is sent).
    pub fn api_threshold(self) -> Option<&'static str> {
        match self {
            Self::Unspecified => None,
            Self::Off => Some("OFF"),
            Self::BlockNone => Some("BLOCK_NONE"),
            Self::BlockOnlyHigh => Some("BLOCK_ONLY_HIGH"),
            Self::BlockMediumAndAbove => Some("BLOCK_MEDIUM_AND_ABOVE"),
            Self::BlockLowAndAbove => Some("BLOCK_LOW_AND_ABOVE"),
        }
    }
}

/// Global, user-configurable options applied to every Gemini request. Safety filtering and built-in
/// tools aren't part of the provider-agnostic request, so they're threaded through separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiRequestOptions {
    pub enable_google_search: bool,
    pub safety_thresholds: BTreeMap<GeminiSafetyCategory, GeminiSafetyThreshold>,
}

impl Default for GeminiRequestOptions {
    /// Explicit Off rather than "no override": Google documents Off as the unset default only for the
    /// 2.5 and 3.x families, and the same provider also serves Gemma and image models.
    fn default() -> Self {
        Self {
            enable_google_search: false,
            safety_thresholds: GeminiSafetyCategory::ALL.iter().map(|c| (*c, GeminiSafetyThreshold::Off)).collect(),
        }
    }
}

/// Where the provider reads the current options from (persisted, user-editable settings in the app).
pub trait GeminiOptionsSource: Send + Sync {
    fn current(&self) -> GeminiRequestOptions;
}

/// Fixed options - handy for tests and callers that don't need persisted settings.
impl GeminiOptionsSource for GeminiRequestOptions {
    fn current(&self) -> GeminiRequestOptions {
        self.clone()
    }
}

// ---- wire types ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiInlineData {
    pub mime_type: String,
    pub data: String,
}

/// `thought` is response-only: true when this part is the model's reasoning trace rather than its
/// reply. `inline_data` and `text` are never both set.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_data: Option<GeminiInlineData>,
}

impl GeminiPart {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self { text: Some(text.into()), ..Default::default() }
    }

    pub fn from_inline_data(mime_type: impl Into<String>, base64_data: impl Into<String>) -> Self {
        Self {
            inline_data: Some(GeminiInlineData { mime_type: mime_type.into(), data: base64_data.into() }),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<GeminiPart>>,
}

/// Requests thought summaries back on models that support thinking - harmless on models that don't.
/// `thinking_level` is "low"/"medium"/"high", or None to let the model apply its own default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiThinkingConfig {
    pub include_thoughts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_config: Option<GeminiThinkingConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeminiSafetySetting {
    pub category: String,
    pub threshold: String,
}

/// Marker for the built-in Google Search grounding tool - an empty object activates it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GeminiGoogleSearchTool {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeminiTool {
    pub google_search: GeminiGoogleSearchTool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeminiGenerateContentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<GeminiContent>,
    pub contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GeminiGenerationConfig>,
    #[serde(rename = "safetySettings", skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<GeminiSafetySetting>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GeminiTool>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiUsageMetadata {
    pub prompt_token_count: Option<i32>,
    pub candidates_token_count: Option<i32>,
    pub total_token_count: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiCandidate {
    pub content: Option<GeminiContent>,
    pub finish_reason: Option<String>,
}

/// Present when Gemini rejects the request before generating any candidate at all - e.g. the prompt
/// itself tripped a safety/policy layer that isn't covered by the per-category safety settings.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiPromptFeedback {
    pub block_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiGenerateContentResponse {
    pub candidates: Option<Vec<GeminiCandidate>>,
    pub usage_metadata: Option<GeminiUsageMetadata>,
    pub prompt_feedback: Option<GeminiPromptFeedback>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiErrorDetail {
    pub message: String,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiErrorResponse {
    pub error: Option<GeminiErrorDetail>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiModelInfo {
    pub name: String,
    pub supported_generation_methods: Option<Vec<String>>,
    pub input_token_limit: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiListModelsResponse {
    pub models: Option<Vec<GeminiModelInfo>>,
}

// ---- mapper ----

pub fn to_gemini_request(request: &LlmRequest, options: &GeminiRequestOptions) -> GeminiGenerateContentRequest {
    let contents = request
        .history
        .iter()
        .map(|m| GeminiContent {
            role: Some(if m.role == LlmRole::User { "user" } else { "model" }.to_string()),
            parts: Some(build_parts(m)),
        })
        .collect();

    let system_instruction = request
        .system_instruction
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| GeminiContent { role: None, parts: Some(vec![GeminiPart::from_text(s)]) });

    let cfg = &request.generation_config;
    let generation_config = GeminiGenerationConfig {
        temperature: cfg.temperature,
        top_p: cfg.top_p,
        top_k: cfg.top_k,
        max_output_tokens: cfg.max_output_tokens,
        stop_sequences: cfg.stop_sequences.clone(),
        thinking_config: Some(GeminiThinkingConfig {
            include_thoughts: true,
            thinking_level: to_thinking_level(cfg.reasoning_effort).map(String::from),
        }),
    };

    let safety: Vec<GeminiSafetySetting> = options
        .safety_thresholds
        .iter()
        .filter_map(|(category, threshold)| {
            threshold.api_threshold().map(|t| GeminiSafetySetting {
                category: category.api_category().to_string(),
                threshold: t.to_string(),
            })
        })
        .collect();

    let enable_search = request.enable_web_search.unwrap_or(options.enable_google_search);

    GeminiGenerateContentRequest {
        system_instruction,
        contents,
        generation_config: Some(generation_config),
        safety_settings: if safety.is_empty() { None } else { Some(safety) },
        tools: enable_search.then(|| vec![GeminiTool { google_search: GeminiGoogleSearchTool {} }]),
    }
}

/// Gemini can't switch thinking off on every model, so Off falls back to its lowest level rather
/// than a request some models would reject.
fn to_thinking_level(effort: Option<ReasoningEffort>) -> Option<&'static str> {
    match effort? {
        ReasoningEffort::Off | ReasoningEffort::Low => Some("low"),
        ReasoningEffort::Medium => Some("medium"),
        ReasoningEffort::High => Some("high"),
    }
}

/// One text part for the message (even if empty, so a picture-only message still has a content part)
/// plus one inline-data part per attachment, read off disk and base64-encoded here. Attachments
/// whose file has gone missing are skipped.
fn build_parts(message: &LlmMessage) -> Vec<GeminiPart> {
    let mut parts = vec![GeminiPart::from_text(&message.text)];
    for attachment in message.attachments.iter().flatten() {
        // Gemini documents JPEG, PNG, and WebP. GIF and BMP are re-encoded as PNG.
        if let Some((mime, encoded)) = attachment.encoded_as(&["image/jpeg", "image/png", "image/webp"]) {
            parts.push(GeminiPart::from_inline_data(mime, encoded));
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmAttachment, LlmGenerationConfig};

    fn request(history: Vec<LlmMessage>, config: LlmGenerationConfig) -> LlmRequest {
        LlmRequest {
            model: "gemini-2.5-flash".into(),
            system_instruction: None,
            history,
            generation_config: config,
            enable_web_search: None,
        }
    }

    fn empty() -> LlmRequest {
        request(vec![], LlmGenerationConfig::default())
    }

    #[test]
    fn maps_user_and_model_roles_to_gemini_role_strings() {
        let r = request(
            vec![LlmMessage::new(LlmRole::User, "Hello"), LlmMessage::new(LlmRole::Model, "Hi there")],
            Default::default(),
        );
        let mapped = to_gemini_request(&r, &Default::default());

        assert_eq!(mapped.contents.len(), 2);
        assert_eq!(mapped.contents[0].role.as_deref(), Some("user"));
        assert_eq!(mapped.contents[0].parts.as_ref().unwrap()[0].text.as_deref(), Some("Hello"));
        assert_eq!(mapped.contents[1].role.as_deref(), Some("model"));
        assert_eq!(mapped.contents[1].parts.as_ref().unwrap()[0].text.as_deref(), Some("Hi there"));
    }

    #[test]
    fn omits_system_instruction_when_none_or_empty() {
        let mut r = empty();
        assert!(to_gemini_request(&r, &Default::default()).system_instruction.is_none());
        r.system_instruction = Some(String::new());
        assert!(to_gemini_request(&r, &Default::default()).system_instruction.is_none());
    }

    #[test]
    fn maps_system_instruction_without_a_role() {
        let mut r = empty();
        r.system_instruction = Some("Be concise.".into());
        let si = to_gemini_request(&r, &Default::default()).system_instruction.unwrap();
        assert!(si.role.is_none());
        assert_eq!(si.parts.unwrap()[0].text.as_deref(), Some("Be concise."));
    }

    #[test]
    fn passes_generation_config_fields_through() {
        let r = request(
            vec![],
            LlmGenerationConfig {
                temperature: Some(0.7),
                top_p: Some(0.9),
                top_k: Some(40),
                max_output_tokens: Some(2048),
                stop_sequences: Some(vec!["STOP".into()]),
                reasoning_effort: None,
            },
        );
        let cfg = to_gemini_request(&r, &Default::default()).generation_config.unwrap();
        assert_eq!(cfg.temperature, Some(0.7));
        assert_eq!(cfg.top_p, Some(0.9));
        assert_eq!(cfg.top_k, Some(40));
        assert_eq!(cfg.max_output_tokens, Some(2048));
        assert_eq!(cfg.stop_sequences, Some(vec!["STOP".to_string()]));
    }

    #[test]
    fn always_requests_thought_summaries() {
        let cfg = to_gemini_request(&empty(), &Default::default()).generation_config.unwrap();
        assert!(cfg.thinking_config.unwrap().include_thoughts);
    }

    #[test]
    fn default_options_send_every_category_explicitly_off_and_no_tools() {
        let mapped = to_gemini_request(&empty(), &GeminiRequestOptions::default());
        let settings = mapped.safety_settings.unwrap();
        let categories: Vec<_> = settings.iter().map(|s| s.category.as_str()).collect();
        assert_eq!(
            categories,
            [
                "HARM_CATEGORY_HARASSMENT",
                "HARM_CATEGORY_HATE_SPEECH",
                "HARM_CATEGORY_SEXUALLY_EXPLICIT",
                "HARM_CATEGORY_DANGEROUS_CONTENT"
            ]
        );
        assert!(settings.iter().all(|s| s.threshold == "OFF"));
        assert!(mapped.tools.is_none());
    }

    #[test]
    fn omits_safety_settings_when_every_category_is_left_to_the_model_default() {
        let options = GeminiRequestOptions {
            enable_google_search: false,
            safety_thresholds: GeminiSafetyCategory::ALL.iter().map(|c| (*c, GeminiSafetyThreshold::Unspecified)).collect(),
        };
        assert!(to_gemini_request(&empty(), &options).safety_settings.is_none());
    }

    #[test]
    fn maps_only_the_configured_thresholds() {
        let options = GeminiRequestOptions {
            enable_google_search: false,
            safety_thresholds: BTreeMap::from([
                (GeminiSafetyCategory::DangerousContent, GeminiSafetyThreshold::BlockNone),
                (GeminiSafetyCategory::HateSpeech, GeminiSafetyThreshold::Unspecified),
            ]),
        };
        let settings = to_gemini_request(&empty(), &options).safety_settings.unwrap();
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0].category, "HARM_CATEGORY_DANGEROUS_CONTENT");
        assert_eq!(settings[0].threshold, "BLOCK_NONE");
    }

    #[test]
    fn attachments_become_inline_data_parts_alongside_the_text_part() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pic.png");
        std::fs::write(&file, [1u8, 2, 3, 4]).unwrap();

        let message = LlmMessage {
            role: LlmRole::User,
            text: "Look at this".into(),
            attachments: Some(vec![LlmAttachment {
                image_path: file.to_string_lossy().into_owned(),
                mime_type: "image/png".into(),
            }]),
            reasoning: None,
        };
        let mapped = to_gemini_request(&request(vec![message], Default::default()), &Default::default());

        let parts = mapped.contents[0].parts.as_ref().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].text.as_deref(), Some("Look at this"));
        assert!(parts[1].text.is_none());
        let inline = parts[1].inline_data.as_ref().unwrap();
        assert_eq!(inline.mime_type, "image/png");
        assert_eq!(inline.data, base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3, 4]));
    }

    #[test]
    fn skips_attachments_whose_file_no_longer_exists() {
        let message = LlmMessage {
            role: LlmRole::User,
            text: "Look at this".into(),
            attachments: Some(vec![LlmAttachment {
                image_path: "C:/does/not/exist.png".into(),
                mime_type: "image/png".into(),
            }]),
            reasoning: None,
        };
        let mapped = to_gemini_request(&request(vec![message], Default::default()), &Default::default());
        assert_eq!(mapped.contents[0].parts.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn adds_the_google_search_tool_when_enabled_in_options() {
        let options = GeminiRequestOptions { enable_google_search: true, safety_thresholds: BTreeMap::new() };
        assert_eq!(to_gemini_request(&empty(), &options).tools.unwrap().len(), 1);
    }

    #[test]
    fn a_per_request_search_override_beats_the_global_option() {
        let mut r = empty();
        r.enable_web_search = Some(true);
        assert!(to_gemini_request(&r, &GeminiRequestOptions::default()).tools.is_some());

        let on = GeminiRequestOptions { enable_google_search: true, ..Default::default() };
        r.enable_web_search = Some(false);
        assert!(to_gemini_request(&r, &on).tools.is_none());
    }

    #[test]
    fn maps_reasoning_effort_to_thinking_level() {
        for (effort, expected) in [
            (None, None),
            (Some(ReasoningEffort::Off), Some("low")),
            (Some(ReasoningEffort::Low), Some("low")),
            (Some(ReasoningEffort::Medium), Some("medium")),
            (Some(ReasoningEffort::High), Some("high")),
        ] {
            let r = request(
                vec![LlmMessage::new(LlmRole::User, "Hi")],
                LlmGenerationConfig { reasoning_effort: effort, ..Default::default() },
            );
            let thinking = to_gemini_request(&r, &Default::default()).generation_config.unwrap().thinking_config.unwrap();
            assert_eq!(thinking.thinking_level.as_deref(), expected, "{effort:?}");
            assert!(thinking.include_thoughts);
        }
    }

    #[test]
    fn serializes_with_the_field_names_and_omissions_gemini_expects() {
        let mut r = request(vec![LlmMessage::new(LlmRole::User, "Hi")], Default::default());
        r.system_instruction = Some("sys".into());
        let json = serde_json::to_string(&to_gemini_request(&r, &Default::default())).unwrap();
        assert!(json.contains("\"system_instruction\":{\"parts\":[{\"text\":\"sys\"}]}"), "{json}");
        assert!(json.contains("\"contents\":[{\"role\":\"user\",\"parts\":[{\"text\":\"Hi\"}]}]"), "{json}");
        assert!(json.contains("\"generationConfig\":{\"thinkingConfig\":{\"includeThoughts\":true}}"), "{json}");
        assert!(!json.contains("null"), "{json}");
        assert!(!json.contains("\"tools\""), "{json}");
    }
}

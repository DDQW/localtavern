//! Pure helpers around one generated reply: how it's shown in the prompt inspector, how emotion and
//! time-skip tags are peeled off it, how the world clock advances, and how blocked replies and
//! provider errors are described to the user.

use chrono::Duration;

use crate::llm::{LlmRequest, LlmRole};
use crate::models::{Character, WeatherCondition, WorldState};
use crate::prompting::{emotion_tag, time_skip_tag};
use crate::providers::{ProviderError, ProviderErrorKind};

/// Renders an assembled request as a plain-text transcript for the "prompt sent to model" panel -
/// not JSON, since the point is for a human to read exactly what the model saw.
pub fn format_prompt(request: &LlmRequest) -> String {
    let mut out = format!("Model: {}\n\n", request.model);

    if let Some(system) = request.system_instruction.as_deref().filter(|s| !s.is_empty()) {
        out.push_str("=== System ===\n");
        out.push_str(system);
        out.push_str("\n\n");
    }

    for message in &request.history {
        let role = if message.role == LlmRole::User { "User" } else { "Model" };
        out.push_str(&format!("=== {role} ===\n{}\n\n", message.text));
    }

    out.trim_end().to_string()
}

/// Appends an instruction to the system prompt (or makes it the whole system prompt).
pub fn append_instruction(request: &mut LlmRequest, instruction: &str) {
    request.system_instruction = Some(match request.system_instruction.as_deref() {
        Some(existing) if !existing.is_empty() => format!("{existing}\n\n{instruction}"),
        _ => instruction.to_string(),
    });
}

/// Strips a trailing `[emotion: x]` tag and validates it against the speaker's configured expression
/// keys. Returns the text unchanged (and no emotion) when the speaker has no expression pack; the
/// tag is still stripped when it doesn't match one of the keys, but no emotion is recorded.
pub fn extract_matched_emotion(speaker: Option<&Character>, raw_text: &str) -> (String, Option<String>) {
    let Some(character) = speaker.filter(|c| !c.expressions.is_empty()) else {
        return (raw_text.to_string(), None);
    };
    let (clean, emotion) = emotion_tag::extract_and_strip(raw_text);
    let matched = emotion.filter(|e| character.expressions.iter().any(|x| &x.emotion == e));
    (clean, matched)
}

/// Strips a trailing `[time-skip: ...]` tag and advances the world clock by the parsed duration, or
/// by the configured per-message default when the tag is absent or unparseable. `roll` supplies
/// uniform randomness in [0, 1) for weather drift, so tests can control it.
pub fn advance_world_state(state: &WorldState, raw_text: &str, roll: &mut dyn FnMut() -> f64) -> (String, WorldState) {
    let (clean, skip) = time_skip_tag::extract_and_strip(raw_text);
    let elapsed = skip.unwrap_or_else(|| Duration::minutes(state.default_advance_minutes_per_message as i64));

    let mut next = state.clone();
    if let Some(moved) = next.fictional_date_time.checked_add_signed(elapsed) {
        next.fictional_date_time = moved;
    }
    (clean, maybe_drift_weather(next, elapsed, roll))
}

/// Occasionally shifts the weather when auto-drift is enabled - roughly one roll per 3 in-story hours
/// elapsed, 10% chance per roll, so a flat per-message tick and a large narrated time-skip both get
/// proportionally more chances to shift it.
fn maybe_drift_weather(mut state: WorldState, elapsed: Duration, roll: &mut dyn FnMut() -> f64) -> WorldState {
    if !state.auto_drift_weather_enabled {
        return state;
    }
    let rolls = ((elapsed.num_seconds() as f64 / 3600.0 / 3.0) as i64).max(1);
    for _ in 0..rolls {
        if roll() < 0.10 {
            let all = WeatherCondition::ALL;
            let index = ((roll() * all.len() as f64) as usize).min(all.len() - 1);
            state.weather = all[index];
            state.weather_notes = None;
            break;
        }
    }
    state
}

/// A cheap uniform [0, 1) source: the random bits of a v4 UUID are plenty for a weather roll.
pub fn random_unit() -> f64 {
    (uuid::Uuid::new_v4().as_u128() >> 75) as f64 / (1u64 << 53) as f64
}

pub fn provider_display_name(provider_id: &str) -> &str {
    match provider_id {
        "gemini" => "Gemini",
        "deepseek" => "DeepSeek",
        "mistral" => "Mistral",
        "xai-subscription" => "SuperGrok",
        "chatgpt-subscription" => "ChatGPT",
        "ollama" => "Ollama",
        other => other,
    }
}

/// A blocked response otherwise arrives as an empty reply with no error - either a blocked prompt
/// (`PROMPT_BLOCKED:x` from the Gemini reader) or a candidate cut short by one of the blocking
/// finish reasons. Each reason gets its own explanation because only SAFETY is governed by the
/// per-category settings - telling the user otherwise sends them tuning settings that can't affect
/// the block they actually hit.
pub fn check_blocked(provider_id: &str, finish_reason: Option<&str>) -> Option<ProviderError> {
    let finish_reason = finish_reason?;
    let (is_prompt_block, reason) = match finish_reason.strip_prefix("PROMPT_BLOCKED:") {
        Some(reason) => (true, reason),
        None => (false, finish_reason),
    };
    const BLOCKING: [&str; 8] = [
        "SAFETY",
        "PROHIBITED_CONTENT",
        "RECITATION",
        "BLOCKLIST",
        "SPII",
        "IMAGE_SAFETY",
        "IMAGE_PROHIBITED_CONTENT",
        "content_filter",
    ];
    if !is_prompt_block && !BLOCKING.contains(&reason) {
        return None;
    }

    let scope = if is_prompt_block { "your message" } else { "the reply" };
    let name = provider_display_name(provider_id);
    let message = format!("{name} blocked {scope} (reason: {reason}). {}", explain_block_reason(reason));
    Some(ProviderError::new(message.trim_end().to_string(), ProviderErrorKind::ContentBlocked))
}

fn explain_block_reason(reason: &str) -> &'static str {
    match reason {
        "SAFETY" | "IMAGE_SAFETY" => {
            "This is the configurable safety filter - check the Gemini safety settings; with every category Off it shouldn't fire."
        }
        "PROHIBITED_CONTENT" | "IMAGE_PROHIBITED_CONTENT" => {
            "This is Google's non-configurable prohibited-content filter (e.g. child safety) - no safety setting can change it."
        }
        "SPII" => {
            "Google's non-configurable filter flagged something as sensitive personal information (addresses, ID numbers, etc.) - no safety setting can change it."
        }
        "BLOCKLIST" => "A term matched Google's non-configurable terminology blocklist.",
        "RECITATION" => {
            "The reply matched existing text too closely (Google's recitation check) - not a safety filter; regenerating usually works."
        }
        "OTHER" => "Google gave no specific reason.",
        _ => "",
    }
}

/// The one-line message shown in the chat UI for a provider failure.
pub fn user_message(error: &ProviderError) -> String {
    if error.message_is_user_facing {
        return error.message.clone();
    }
    match error.kind {
        ProviderErrorKind::Auth => "Authentication failed - check your API key in Settings.".to_string(),
        ProviderErrorKind::RateLimit => "Rate limited - please wait a moment and try again.".to_string(),
        ProviderErrorKind::Network => format!("Network error: {}", error.message),
        ProviderErrorKind::ContentBlocked => error.message.clone(),
        _ => format!("Error: {}", error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage};
    use crate::models::ExpressionImage;

    fn request(system: Option<&str>) -> LlmRequest {
        LlmRequest {
            model: "m".into(),
            system_instruction: system.map(String::from),
            history: vec![LlmMessage::new(LlmRole::User, "hi"), LlmMessage::new(LlmRole::Model, "yo")],
            generation_config: LlmGenerationConfig::default(),
            enable_web_search: None,
        }
    }

    #[test]
    fn format_prompt_reads_like_a_transcript() {
        assert_eq!(
            format_prompt(&request(Some("Be terse."))),
            "Model: m\n\n=== System ===\nBe terse.\n\n=== User ===\nhi\n\n=== Model ===\nyo"
        );
        assert!(!format_prompt(&request(None)).contains("System"));
    }

    #[test]
    fn append_instruction_extends_or_creates_the_system_prompt() {
        let mut r = request(Some("base"));
        append_instruction(&mut r, "extra");
        assert_eq!(r.system_instruction.as_deref(), Some("base\n\nextra"));
        let mut r = request(None);
        append_instruction(&mut r, "extra");
        assert_eq!(r.system_instruction.as_deref(), Some("extra"));
        let mut r = request(Some(""));
        append_instruction(&mut r, "extra");
        assert_eq!(r.system_instruction.as_deref(), Some("extra"));
    }

    fn speaker_with(emotions: &[&str]) -> Character {
        Character {
            name: "Nova".into(),
            expressions: emotions
                .iter()
                .map(|e| ExpressionImage { emotion: e.to_string(), image_path: format!("{e}.png") })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn emotion_is_matched_only_against_the_speakers_own_keys() {
        let nova = speaker_with(&["happy", "sad"]);
        assert_eq!(extract_matched_emotion(Some(&nova), "Hi! [emotion: Happy]"), ("Hi!".into(), Some("happy".into())));
        // A tag for an emotion the character doesn't have is stripped but not recorded.
        assert_eq!(extract_matched_emotion(Some(&nova), "Hi! [emotion: furious]"), ("Hi!".into(), None));
    }

    #[test]
    fn no_expression_pack_means_the_text_is_untouched() {
        let plain = speaker_with(&[]);
        assert_eq!(extract_matched_emotion(Some(&plain), "Hi [emotion: happy]"), ("Hi [emotion: happy]".into(), None));
        assert_eq!(extract_matched_emotion(None, "Hi [emotion: happy]"), ("Hi [emotion: happy]".into(), None));
    }

    fn state() -> WorldState {
        WorldState::create_default() // 2025-01-01 09:00, 5 min default
    }

    #[test]
    fn the_clock_advances_by_the_tag_when_present() {
        let (clean, next) = advance_world_state(&state(), "She left. [time-skip: 3 hours]", &mut || 0.99);
        assert_eq!(clean, "She left.");
        assert_eq!(next.fictional_date_time, state().fictional_date_time + Duration::hours(3));
    }

    #[test]
    fn the_clock_falls_back_to_the_default_tick_without_a_tag() {
        let (clean, next) = advance_world_state(&state(), "No tag here.", &mut || 0.99);
        assert_eq!(clean, "No tag here.");
        assert_eq!(next.fictional_date_time, state().fictional_date_time + Duration::minutes(5));
    }

    #[test]
    fn weather_never_drifts_unless_enabled() {
        let (_, next) = advance_world_state(&state(), "x", &mut || 0.0);
        assert_eq!(next.weather, WeatherCondition::Clear);
    }

    #[test]
    fn weather_drifts_on_a_low_roll_and_clears_the_notes() {
        let mut s = state();
        s.auto_drift_weather_enabled = true;
        s.weather_notes = Some("light drizzle".into());
        let mut rolls = [0.05, 0.5].into_iter(); // first roll triggers drift; second picks index 4
        let (_, next) = advance_world_state(&s, "x", &mut || rolls.next().unwrap());
        assert_eq!(next.weather, WeatherCondition::ALL[4]);
        assert_eq!(next.weather_notes, None);
    }

    #[test]
    fn weather_stays_put_on_a_high_roll_and_long_skips_get_more_rolls() {
        let mut s = state();
        s.auto_drift_weather_enabled = true;
        let mut calls = 0;
        advance_world_state(&s, "x [time-skip: 9 hours]", &mut || {
            calls += 1;
            0.9
        });
        assert_eq!(calls, 3, "9 in-story hours is three 3-hour rolls");
        assert_eq!(advance_world_state(&s, "x", &mut || 0.9).1.weather, WeatherCondition::Clear);
    }

    #[test]
    fn random_unit_stays_in_range() {
        for _ in 0..1000 {
            let r = random_unit();
            assert!((0.0..1.0).contains(&r), "{r}");
        }
    }

    #[test]
    fn blocking_finish_reasons_become_content_blocked_errors_with_specific_advice() {
        let safety = check_blocked("gemini", Some("SAFETY")).unwrap();
        assert_eq!(safety.kind, ProviderErrorKind::ContentBlocked);
        assert!(safety.message.starts_with("Gemini blocked the reply (reason: SAFETY)."));
        assert!(safety.message.contains("configurable safety filter"));

        let prompt = check_blocked("gemini", Some("PROMPT_BLOCKED:PROHIBITED_CONTENT")).unwrap();
        assert!(prompt.message.contains("blocked your message"));
        assert!(prompt.message.contains("non-configurable"));

        let deepseek = check_blocked("deepseek", Some("content_filter")).unwrap();
        assert_eq!(deepseek.message, "DeepSeek blocked the reply (reason: content_filter).");
    }

    #[test]
    fn ordinary_finish_reasons_are_not_blocks() {
        for reason in [None, Some("STOP"), Some("stop"), Some("length"), Some("MAX_TOKENS")] {
            assert!(check_blocked("gemini", reason).is_none(), "{reason:?}");
        }
    }

    #[test]
    fn user_messages_prefer_the_providers_own_words_only_when_they_are_user_facing() {
        let auth = ProviderError::auth("invalid key 1234");
        assert_eq!(user_message(&auth), "Authentication failed - check your API key in Settings.");
        assert_eq!(user_message(&auth.clone().user_facing()), "invalid key 1234");
        assert_eq!(user_message(&ProviderError::network("boom")), "Network error: boom");
        assert_eq!(user_message(&ProviderError::unknown("weird")), "Error: weird");
        assert_eq!(
            user_message(&ProviderError::new("x", ProviderErrorKind::RateLimit)),
            "Rate limited - please wait a moment and try again."
        );
    }
}

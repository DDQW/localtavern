//! The trailing `[time-skip: 3 hours]` tag every roleplay reply is asked to append, estimating how
//! much in-story time that reply covered. The flat per-message tick
//! (`WorldState::default_advance_minutes_per_message`) is only a fallback for when the tag is
//! missing or unparseable.

use std::sync::LazyLock;

use chrono::Duration;
use regex::Regex;

static TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*\[\s*time-skip\s*:\s*(\d+(?:\.\d+)?)\s*(minutes?|mins?|hours?|hrs?|days?)\s*\]\s*$")
        .unwrap()
});

const MAX_SKIP_SECONDS: f64 = 30.0 * 24.0 * 3600.0;

/// One line to append to the system instruction, asking for a structured tag.
pub fn build_instruction() -> &'static str {
    "At the very end of every reply, append a tag estimating how much in-story time that reply \
covered, realistically - a short line of dialogue or a quick reaction might be under a \
minute; a described action, a wait, or a stretch of small talk might be several minutes; an \
explicit time jump (an overnight, a training montage, \"the rest of the evening\") should \
match what was actually narrated. Format: [time-skip: <number> <minutes|hours|days>], e.g. \
[time-skip: 2 minutes] or [time-skip: 3 hours]. Always include it, never explain it."
}

/// Strips a trailing `[time-skip: N unit]` tag, returning the parsed elapsed time clamped to (0, 30 days].
pub fn extract_and_strip(text: &str) -> (String, Option<Duration>) {
    let Some(caps) = TAG.captures(text) else {
        return (text.to_string(), None);
    };
    let clean = text[..caps.get(0).unwrap().start()].trim_end().to_string();

    let Ok(amount) = caps[1].parse::<f64>() else {
        return (clean, None);
    };
    if amount <= 0.0 {
        return (clean, None);
    }

    let unit_seconds = match caps[2].to_lowercase().chars().next() {
        Some('m') => 60.0,
        Some('h') => 3600.0,
        Some('d') => 86_400.0,
        _ => return (clean, None),
    };
    let seconds = (amount * unit_seconds).min(MAX_SKIP_SECONDS);
    (clean, Some(Duration::milliseconds((seconds * 1000.0).round() as i64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_unit_alias() {
        for (text, minutes) in [
            ("She left. [time-skip: 3 hours]", 180),
            ("She left. [time-skip: 0.5 hours]", 30),
            ("She left. [time-skip: 45 minutes]", 45),
            ("She left. [time-skip: 45 mins]", 45),
            ("She left. [time-skip: 2 days]", 2 * 24 * 60),
        ] {
            let (clean, skip) = extract_and_strip(text);
            assert_eq!(clean, "She left.", "{text}");
            assert_eq!(skip, Some(Duration::minutes(minutes)), "{text}");
        }
    }

    #[test]
    fn leaves_text_unchanged_when_no_tag() {
        assert_eq!(extract_and_strip("Just a normal reply."), ("Just a normal reply.".into(), None));
    }

    #[test]
    fn handles_empty_text() {
        assert_eq!(extract_and_strip(""), (String::new(), None));
    }

    #[test]
    fn only_matches_a_trailing_tag_not_one_mid_text() {
        let text = "It was [time-skip: 3 hours] ago, she said.";
        assert_eq!(extract_and_strip(text), (text.into(), None));
    }

    #[test]
    fn clamps_absurd_values_to_thirty_days() {
        let (_, skip) = extract_and_strip("[time-skip: 9999 days]");
        assert_eq!(skip, Some(Duration::days(30)));
    }

    #[test]
    fn rejects_zero_or_negative_amounts() {
        assert_eq!(extract_and_strip("[time-skip: 0 hours]").1, None);
        assert_eq!(extract_and_strip("[time-skip: -1 hours]").1, None);
    }

    #[test]
    fn a_zero_tag_is_still_stripped() {
        assert_eq!(extract_and_strip("Fine. [time-skip: 0 hours]"), ("Fine.".into(), None));
    }

    #[test]
    fn instruction_mentions_the_tag_format() {
        let instruction = build_instruction();
        assert!(instruction.contains("[time-skip:"));
        assert!(instruction.contains("Always include it"));
    }
}

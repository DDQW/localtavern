//! The trailing `[time-skip: 3 hours]` tag every roleplay reply is asked to append, estimating how
//! much in-story time that reply covered. The flat per-message tick
//! (`WorldState::default_advance_minutes_per_message`) is only a fallback for when the tag is
//! missing or unparseable.

use std::sync::LazyLock;

use chrono::Duration;
use regex::Regex;

use super::instructions::{self, Instructions};

/// The tag in the shapes models actually produce: `[time-skip: 3 hours]`, `(Time skip: 1h 30m)`,
/// `[time_skip: 2-3 minutes]`.
static TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[\[(]\s*time[-_ ]?(?:skip|jump|passed|elapsed)\s*[:=]\s*([^\])]{1,60}?)\s*[\])]").unwrap()
});

/// One `number unit` part of a duration; the number may be a range ("2-3").
static PART: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(\d+(?:[.,]\d+)?)(?:\s*(?:-|–|to)\s*(\d+(?:[.,]\d+)?))?\s*([a-z]+)",
    )
    .unwrap()
});

const MAX_SKIP_SECONDS: f64 = 30.0 * 24.0 * 3600.0;

/// One line to append to the system instruction, asking for a structured tag.
pub fn build_instruction(ins: &Instructions) -> String {
    ins.render(instructions::TIME_SKIP_TAG, &[])
}

/// The elapsed time named by a tag in `tail` (the trailing tags peeled off a reply), clamped to (0, 30 days].
pub fn parse_tail(tail: &str) -> Option<Duration> {
    parse_duration(&TAG.captures(tail)?[1])
}

/// "3 hours", "1h 30m", "1,5 hours", "2-3 minutes" (the middle), "an hour", "half an hour".
fn parse_duration(body: &str) -> Option<Duration> {
    let body = body.trim().to_lowercase();
    if body.starts_with('-') {
        return None;
    }
    let body = body.replace("half an ", "0.5 ").replace("half a ", "0.5 ").replace("an ", "1 ").replace("a ", "1 ");

    let number = |s: &str| s.replace(',', ".").parse::<f64>().ok();
    let mut seconds = 0.0;
    for part in PART.captures_iter(&body) {
        let low = number(&part[1])?;
        let amount = match part.get(2) {
            Some(high) => (low + number(high.as_str())?) / 2.0,
            None => low,
        };
        let unit = match &part[3] {
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            "d" | "day" | "days" => 86_400.0,
            "w" | "week" | "weeks" => 7.0 * 86_400.0,
            _ => continue,
        };
        seconds += amount * unit;
    }
    if seconds <= 0.0 {
        return None;
    }
    Some(Duration::milliseconds((seconds.min(MAX_SKIP_SECONDS) * 1000.0).round() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip(text: &str) -> Option<Duration> {
        parse_tail(text)
    }

    #[test]
    fn parses_each_unit_alias() {
        for (text, minutes) in [
            ("[time-skip: 3 hours]", 180),
            ("[time-skip: 0.5 hours]", 30),
            ("[time-skip: 45 minutes]", 45),
            ("[time-skip: 45 mins]", 45),
            ("[time-skip: 2 days]", 2 * 24 * 60),
        ] {
            assert_eq!(skip(text), Some(Duration::minutes(minutes)), "{text}");
        }
    }

    #[test]
    fn accepts_the_shapes_models_actually_write() {
        for (text, seconds) in [
            ("[Time-Skip: 90 min]", 90 * 60),
            ("(time skip: 2 hours)", 2 * 3600),
            ("[time_skip: 1h 30m]", 90 * 60),
            ("[time-skip: 1 hour 15 minutes]", 75 * 60),
            ("[time-skip: 2-3 minutes]", 150),
            ("[time-skip: an hour]", 3600),
            ("[time-skip: half an hour]", 30 * 60),
            ("[time-skip: 1,5 hours]", 90 * 60),
            ("[time-skip: 1 day]", 86_400),
        ] {
            assert_eq!(skip(text), Some(Duration::seconds(seconds)), "{text}");
        }
    }

    #[test]
    fn unreadable_zero_and_negative_amounts_are_none() {
        for text in ["[time-skip: a while]", "[time-skip: 0 hours]", "[time-skip: -1 hours]", "no tag", ""] {
            assert_eq!(skip(text), None, "{text}");
        }
    }

    #[test]
    fn clamps_absurd_values_to_thirty_days() {
        assert_eq!(skip("[time-skip: 9999 days]"), Some(Duration::days(30)));
    }

    #[test]
    fn instruction_mentions_the_tag_format() {
        let instruction = build_instruction(instructions::defaults());
        assert!(instruction.contains("[time-skip:"));
        assert!(instruction.contains("never omit it"));
    }
}

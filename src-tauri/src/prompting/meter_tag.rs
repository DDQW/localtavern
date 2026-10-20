//! The trailing `[meters: arousal 40, climax 10]` tag a roleplay reply is asked to end with while
//! the story tracks arousal. The app keeps the real numbers (and clamps how fast they move); the
//! model only reports where it thinks they are now.

use std::sync::LazyLock;

use regex::Regex;

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[\[(]\s*meters?\s*[:=]([^\])]*)[\])]").unwrap());
static AROUSAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)arousal\s*[:=]?\s*(\d{1,3})").unwrap());
static CLIMAX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:climax|orgasm)\s*[:=]?\s*(\d{1,3})").unwrap());

/// What the model reported; a number it left out stays as it was.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MeterReading {
    pub arousal: Option<u8>,
    pub climax: Option<u8>,
}

/// The tag in `tail` (the trailing tags peeled off a reply), if it has at least one readable number.
pub fn parse_tail(tail: &str) -> Option<MeterReading> {
    let body = TAG.captures(tail)?.get(1)?.as_str().to_string();
    let number = |re: &Regex| re.captures(&body).and_then(|c| c[1].parse::<u32>().ok()).map(|n| n.min(100) as u8);
    let reading = MeterReading { arousal: number(&AROUSAL), climax: number(&CLIMAX) };
    (reading != MeterReading::default()).then_some(reading)
}

/// The system-instruction line asking `name`'s replies to carry the tag.
pub fn build_instruction(name: &str) -> String {
    format!(
        "The story tracks {name}'s arousal. After the time-skip tag, also append [meters: arousal <0-100>, climax <0-100>] \
for {name} as of the end of the reply. Arousal is how aroused {name} is right now (0 = not at all, 100 = at the peak); \
climax is how close {name} is to orgasm (0 = not close, 100 = orgasm happens in this very reply, so write it). Both move \
gradually, by at most about 25 per reply, rise only with stimulation that was actually narrated, and fall back toward 0 when \
the scene calms down or turns non-sexual. Don't reach 100 unless the scene has genuinely built up to it - after an orgasm the \
app resets the meters itself. Use the current values given in the world state as your starting point. Never explain the tag."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_numbers_in_any_shape() {
        for tail in [
            "[meters: arousal 40, climax 10]",
            "[Meters: arousal=40 climax=10]",
            "(meter: Arousal: 40; Climax: 10)",
            "[time-skip: 5 minutes] [meters: arousal 40, orgasm 10]",
        ] {
            assert_eq!(parse_tail(tail), Some(MeterReading { arousal: Some(40), climax: Some(10) }), "{tail}");
        }
    }

    #[test]
    fn a_missing_number_is_left_alone_and_big_ones_are_capped() {
        assert_eq!(parse_tail("[meters: arousal 250]"), Some(MeterReading { arousal: Some(100), climax: None }));
        assert_eq!(parse_tail("[meters: climax 7]"), Some(MeterReading { arousal: None, climax: Some(7) }));
    }

    #[test]
    fn no_tag_or_no_numbers_is_none() {
        assert_eq!(parse_tail("[time-skip: 5 minutes]"), None);
        assert_eq!(parse_tail("[meters: unknown]"), None);
    }

    #[test]
    fn the_instruction_names_the_character_and_the_tag() {
        let text = build_instruction("Nova");
        assert!(text.contains("Nova") && text.contains("[meters: arousal"));
    }
}

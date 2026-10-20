//! The tags a roleplay reply ends with - `[time-skip: ...]`, `[emotion: ...]`, `[meters: ...]` -
//! peeled off together, in whatever order the model wrote them, so one missing or misplaced tag
//! can't leave another in the visible text or hide it from its parser.

use std::sync::LazyLock;

use chrono::Duration;
use regex::Regex;

use super::meter_tag::{self, MeterReading};
use super::{emotion_tag, time_skip_tag};

/// One or more tags at the very end of a reply, each optionally wrapped in `*`, `_` or backticks.
static TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:[\s*_`]*[\[(]\s*(?:time[-_ ]?(?:skip|jump|passed|elapsed)|emotion|meters?)\s*[:=][^\])]*[\])])+[\s*_`]*$",
    )
    .unwrap()
});

#[derive(Debug, Default, PartialEq)]
pub struct ReplyTags {
    pub skip: Option<Duration>,
    /// Lowercased, not yet checked against any character's expression keys.
    pub emotion: Option<String>,
    pub meters: Option<MeterReading>,
}

/// Splits the trailing tags off `text`, returning the text without them and what they said.
pub fn peel(text: &str) -> (String, ReplyTags) {
    let Some(found) = TAIL.find(text) else {
        return (text.to_string(), ReplyTags::default());
    };
    let tail = found.as_str();
    let tags = ReplyTags {
        skip: time_skip_tag::parse_tail(tail),
        emotion: emotion_tag::parse_tail(tail),
        meters: meter_tag::parse_tail(tail),
    };
    (text[..found.start()].trim_end().to_string(), tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peels_all_three_in_any_order() {
        for text in [
            "She smiled.\n\n[time-skip: 5 minutes]\n[emotion: Happy]\n[meters: arousal 30, climax 5]",
            "She smiled. [meters: arousal 30, climax 5] [emotion: happy] [time-skip: 5 minutes]",
            "She smiled.\n**[emotion: happy]** (time skip: 5 min) [meters: arousal=30 climax=5]",
        ] {
            let (clean, tags) = peel(text);
            assert_eq!(clean, "She smiled.", "{text}");
            assert_eq!(tags.skip, Some(Duration::minutes(5)), "{text}");
            assert_eq!(tags.emotion.as_deref(), Some("happy"), "{text}");
            assert_eq!(tags.meters, Some(MeterReading { arousal: Some(30), climax: Some(5) }), "{text}");
        }
    }

    #[test]
    fn text_without_tags_is_untouched_and_mid_text_tags_are_left_alone() {
        assert_eq!(peel("Plain."), ("Plain.".into(), ReplyTags::default()));
        let text = "He said [emotion: happy] and left.";
        assert_eq!(peel(text), (text.into(), ReplyTags::default()));
    }

    #[test]
    fn an_unreadable_tag_is_still_removed() {
        let (clean, tags) = peel("Fine. [time-skip: a while]");
        assert_eq!(clean, "Fine.");
        assert_eq!(tags.skip, None);
    }
}

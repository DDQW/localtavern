//! The tags a roleplay reply ends with - `[time-skip: ...]`, `[emotion: ...]`, `[meters: ...]`,
//! `[photo: ...]`, `[next: ...]`, `[private]` -
//! peeled off together, in whatever order the model wrote them, so one missing or misplaced tag
//! can't leave another in the visible text or hide it from its parser.

use std::sync::LazyLock;

use chrono::Duration;
use regex::Regex;

use super::meter_tag::{self, MeterReading};
use super::{breakout, emotion_tag, next_speaker_tag, photo_tag, time_skip_tag};

/// One or more tags at the very end of a reply, each optionally wrapped in `*`, `_` or backticks.
/// A `[...]` tag may hold parentheses (photo descriptions do, SD weights like `(red hair:1.2)` too);
/// a `(...)` one may not. Either may close with the other bracket.
static TAIL: LazyLock<Regex> = LazyLock::new(|| {
    const KEYS: &str = r"\s*(?:(?:time[-_ ]?(?:skip|jump|passed|elapsed)|emotion|meters?|photo|picture|image|selfie|user[-_ ]?(?:photo|picture|pic)|next(?:[-_ ]?(?:speaker|up))?)\s*[:=]|private\b)";
    Regex::new(&format!(r"(?i)(?:[\s*_`]*(?:\[{KEYS}[^\[\]]*|\({KEYS}[^\])]*)[\])])+[\s*_`]*$")).unwrap()
});

#[derive(Debug, Default, PartialEq)]
pub struct ReplyTags {
    pub skip: Option<Duration>,
    /// Lowercased, not yet checked against any character's expression keys.
    pub emotion: Option<String>,
    pub meters: Option<MeterReading>,
    /// What a photo the character sent in the story shows, for the app to draw.
    pub photo: Option<String>,
    /// What a photo the user's character took in their message shows, for the app to draw.
    pub user_photo: Option<String>,
    /// The group member this reply hands the user's message on to, as the model wrote the name.
    pub next: Option<String>,
    /// The speaker takes the user aside into a private talk (group chats).
    pub private: bool,
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
        photo: photo_tag::parse_tail(tail),
        user_photo: photo_tag::parse_user_tail(tail),
        next: next_speaker_tag::parse_tail(tail),
        private: breakout::parse_tail(tail),
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
            assert_eq!(tags.meters, Some(MeterReading { arousal: Some(30), climax: Some(5), orgasm: false }), "{text}");
        }
    }

    #[test]
    fn peels_a_photo_tag_alongside_the_others() {
        let (clean, tags) = peel("She holds up her phone and snaps one.\n\n[photo: selfie, silver hair, winking, cafe at night]\n[emotion: happy]");
        assert_eq!(clean, "She holds up her phone and snaps one.");
        assert_eq!(tags.photo.as_deref(), Some("selfie, silver hair, winking, cafe at night"));
        assert_eq!(tags.emotion.as_deref(), Some("happy"));
        assert_eq!(peel("No photo here. [emotion: sad]").1.photo, None);
    }

    #[test]
    fn peels_a_long_photo_description_the_way_a_local_model_wrote_it() {
        // Gemma 4 26B's actual answer to "Send me a pic :)" with the photo instruction.
        let reply = "She snaps a quick selfie, tilting her head.

*texts* Just lounging at home enjoying the sun. Here you go! ;)

[photo: A close-up selfie of Nova looking directly into the lens with a teasing smile. Her long silver hair falls across her shoulder, catching the sunlight. She is wearing a simple white ribbed tank top. The background shows her sunlit bedroom wall.]";
        let (clean, tags) = peel(reply);
        assert!(clean.ends_with("Here you go! ;)"), "{clean}");
        let photo = tags.photo.unwrap();
        assert!(photo.starts_with("A close-up selfie of Nova") && photo.ends_with("sunlit bedroom wall."), "{photo}");
    }

    #[test]
    fn parentheses_inside_a_bracketed_photo_tag_dont_end_it() {
        let (clean, tags) = peel("Here!\n\n[photo: Nova (smiling) at the cafe, (silver hair:1.2)]\n[emotion: happy]");
        assert_eq!(clean, "Here!");
        assert_eq!(tags.photo.as_deref(), Some("Nova (smiling) at the cafe, (silver hair:1.2)"));
        assert_eq!(tags.emotion.as_deref(), Some("happy"));
        // A mismatched closing bracket is still tolerated, and can't swallow the story before a later tag.
        assert_eq!(peel("Fine. [emotion: happy)").0, "Fine.");
        assert_eq!(peel("Fine. [photo: x) She left. [emotion: sad]").0, "Fine. [photo: x) She left.");
    }

    #[test]
    fn peels_a_next_speaker_tag_alongside_the_others() {
        let (clean, tags) = peel("Alice laughs. \"Ask Bob, he was there.\"\n\n[emotion: happy]\n[next: Bob]");
        assert_eq!(clean, "Alice laughs. \"Ask Bob, he was there.\"");
        assert_eq!(tags.next.as_deref(), Some("Bob"));
        assert_eq!(tags.emotion.as_deref(), Some("happy"));
        assert_eq!(peel("Just me. [emotion: sad]").1.next, None);
    }

    #[test]
    fn peels_the_users_photo_apart_from_the_characters() {
        let (clean, tags) = peel("Nova grins at your camera.\n\n[user-photo: Nova grinning on the pier]\n[photo: selfie of Nova]");
        assert_eq!(clean, "Nova grins at your camera.");
        assert_eq!(tags.user_photo.as_deref(), Some("Nova grinning on the pier"));
        assert_eq!(tags.photo.as_deref(), Some("selfie of Nova"));
    }

    #[test]
    fn peels_a_private_tag_with_or_without_a_reason() {
        let (clean, tags) = peel("Mira tugs your sleeve.\n\n[emotion: shy]\n[private]");
        assert_eq!(clean, "Mira tugs your sleeve.");
        assert!(tags.private);
        assert!(peel("Come. [private: about the letter]").1.private);
        assert!(!peel("Hi. [emotion: happy]").1.private);
        assert_eq!(peel("It was a private matter.").0, "It was a private matter.");
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

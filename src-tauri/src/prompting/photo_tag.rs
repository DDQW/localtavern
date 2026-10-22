//! The trailing `[photo: ...]` tag a roleplay reply ends with when the character sends the user a
//! picture in the story (a selfie over the phone, a snapshot of where they are), and the
//! `[user-photo: ...]` one describing a photo the user's character took in their own message. The
//! app draws either from the description, so the photo actually shows up in the chat.

use std::sync::LazyLock;

use regex::Regex;

use super::instructions::{self, Instructions};

/// One line for the system instruction. Photos are offered as ordinary behaviour - whenever a real
/// person with a phone would send or take one - not as a rare exception: framed as "most replies
/// send no photo, only when the story calls for it", models never sent one. Explicit that the
/// brackets are required: told only "describe it", a model writes the description into the story
/// text where the app can't find it.
///
/// `intimate` (for a character not known to be under 18) adds that the photos can be as intimate as
/// the moment: suggestive or nude ones of the character themselves when the mood is flirty or sexual.
pub fn build_instruction(ins: &Instructions, character_name: &str, user_name: &str, intimate: bool) -> String {
    let vars = [("char", character_name), ("user", user_name)];
    let photo = ins.render(instructions::PHOTO, &vars);
    if intimate {
        format!("{photo} {}", ins.render(instructions::PHOTO_INTIMATE, &vars))
    } else {
        photo
    }
}

/// One more line for the system instruction when the user's message may have the user's character
/// taking a photo: the reply describes that photo in its own tag, so the app can show it too.
pub fn user_photo_instruction(ins: &Instructions, user_name: &str) -> String {
    ins.render(instructions::USER_PHOTO, &[("user", user_name)])
}

/// `[user-photo: ...]` (or `user picture`), up to its `]`.
static USER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)\[\s*{USER_KEY}([^\[\]]*)\]|[\[(]\s*{USER_KEY}([^\[\])]*)[\])]")).unwrap()
});

/// The description in a `[user-photo: ...]` tag anywhere in `tail`.
pub fn parse_user_tail(tail: &str) -> Option<String> {
    let captures = USER.captures(tail)?;
    clean_description(captures.get(1).or_else(|| captures.get(2))?.as_str())
}

/// The first of `names` (id, name) a photo description mentions as a whole word - whose look goes
/// into the picture.
pub fn first_named<T: Copy>(description: &str, names: &[(T, &str)]) -> Option<T> {
    let lower = description.to_lowercase();
    names
        .iter()
        .filter(|(_, name)| !name.trim().is_empty())
        .filter_map(|(id, name)| {
            let name = name.trim().to_lowercase();
            lower.match_indices(&name).find_map(|(at, _)| {
                let before = lower[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
                let after = lower[at + name.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
                (before && after).then_some((at, *id))
            })
        })
        .min_by_key(|(at, _)| *at)
        .map(|(_, id)| id)
}

/// What opens the character's photo tag: `photo:`, `picture =`, `selfie:`, and the looser spellings
/// models drift into - `Photo attached:`, `Sends a photo:`, `Photo - ...`, `Photo of Nova smiling`.
pub const KEY: &str = r"(?:(?:attached|sent|sends?|sending)\s+(?:a\s+)?)?(?:photo(?:graph)?|picture|pic|image|selfie|snapshot)(?:\s+(?:attached|sent))?\s*(?:[:=\-–—]|of\b)";

/// What opens the `[user-photo: ...]` tag.
pub const USER_KEY: &str = r"user[-_ ]?(?:photo|picture|pic)\s*[:=]";

/// `[photo: ...]` up to its `]`, so parentheses in the description survive; otherwise a `(...)` tag or
/// a `[...)` one with mismatched brackets.
static ANY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)\[\s*{KEY}([^\[\]]*)\]|[\[(]\s*{KEY}([^\[\])]*)[\])]")).unwrap());

/// The description in a photo tag anywhere in `tail` (the trailing tags peeled off a reply).
pub fn parse_tail(tail: &str) -> Option<String> {
    let captures = ANY.captures(tail)?;
    clean_description(captures.get(1).or_else(|| captures.get(2))?.as_str())
}

/// A whole bracketed photo tag in the middle of a reply, with any `*`/`_` around it.
static INLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)[*_]*\[\s*{KEY}([^\[\]]*)\][*_]*")).unwrap());
static USER_INLINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)[*_]*\[\s*{USER_KEY}([^\[\]]*)\][*_]*")).unwrap());

/// Takes a photo tag out of the middle of `text` - a model sometimes writes the line where the photo
/// is sent and carries on with the story, so it isn't among the trailing tags. Returns the text
/// without it and the description.
pub fn take_inline(text: &str) -> Option<(String, String)> {
    take(&INLINE, text)
}

/// [`take_inline`] for the `[user-photo: ...]` tag.
pub fn take_user_inline(text: &str) -> Option<(String, String)> {
    take(&USER_INLINE, text)
}

fn take(tag: &Regex, text: &str) -> Option<(String, String)> {
    let captures = tag.captures(text)?;
    let description = clean_description(captures.get(1)?.as_str())?;
    let whole = captures.get(0)?;
    let (before, after) = (&text[..whole.start()], &text[whole.end()..]);
    let on_own_line = before.trim_end_matches([' ', '\t']).ends_with('\n') || after.trim_start_matches([' ', '\t']).starts_with('\n');
    let (before, after) = (before.trim_end(), after.trim_start());
    let rest = match (before.is_empty(), after.is_empty()) {
        (true, _) => after.to_string(),
        (_, true) => before.to_string(),
        _ if on_own_line => format!("{before}\n\n{after}"),
        _ => format!("{before} {after}"),
    };
    Some((rest, description))
}

/// What the model is told in place of a story photo's picture (sent as a system note).
pub fn context_note(description: &str) -> String {
    format!("A photo was shared in the story here. It showed: {}", description.trim())
}

fn clean_description(description: &str) -> Option<String> {
    Some(description.trim().trim_matches(['"', '*', '_']).trim().to_string()).filter(|d| !d.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ins() -> Instructions {
        Instructions::default()
    }

    #[test]
    fn instruction_names_both_sides_and_the_format() {
        let line = build_instruction(&ins(), "Nova", "Alex", false);
        assert!(line.starts_with("Nova sends Alex photos whenever a real person would: a selfie"), "{line}");
        assert!(!line.contains("nude"), "{line}");
        assert!(line.contains("while they are apart and texting, and whenever Alex asks for one"), "{line}");
        assert!(line.contains("When they are together, Nova can take a picture"), "{line}");
        assert!(line.contains("square brackets included: [photo: <"), "{line}");
        assert!(line.ends_with("Leave the line out when no photo is sent or taken."), "{line}");
        // Not framed as a rare exception any more: that wording never got a photo sent.
        assert!(!line.contains("Most replies") && !line.contains("Only when"), "{line}");
    }

    #[test]
    fn an_adult_character_can_send_intimate_photos_when_the_moment_fits() {
        let line = build_instruction(&ins(), "Nova", "Alex", true);
        assert!(line.contains("Leave the line out when no photo is sent or taken. The photos can be as intimate as the moment"), "{line}");
        assert!(line.ends_with("when the mood is flirty or sexual and Nova wants to, Nova can send Alex suggestive or nude photos of Nova, described plainly."), "{line}");
        assert!(!line.contains("adult"), "the model isn't asked to judge ages: {line}");
    }

    #[test]
    fn a_photo_word_in_the_users_message_asks_for_the_users_photo() {
        let ins = ins();
        assert!(ins.mentions_a_photo("*Soso decides to make a picture of the harbour*"));
        assert!(ins.mentions_a_photo("I snap a quick selfie with you."));
        assert!(ins.mentions_a_photo("*raises the Camera*"));
        assert!(!ins.mentions_a_photo("We walk down to the beach together."));
        let line = user_photo_instruction(&ins, "Soso");
        assert!(line.starts_with("If Soso takes a photo or picture in Soso's latest message, also describe that photo"), "{line}");
        assert!(line.contains("square brackets included: [user-photo: <what Soso's photo shows"), "{line}");
        assert!(line.ends_with("Leave this line out when Soso takes no photo."), "{line}");
    }

    #[test]
    fn the_users_photo_has_its_own_tag() {
        assert_eq!(parse_user_tail("[user-photo: the harbour at dusk, gulls]").as_deref(), Some("the harbour at dusk, gulls"));
        assert_eq!(parse_user_tail("[emotion: happy]\n**[User Photo = Nova (laughing) on the pier]**").as_deref(), Some("Nova (laughing) on the pier"));
        assert_eq!(parse_user_tail("[photo: a selfie]"), None, "the character's own photo is another tag");
        assert_eq!(parse_tail("[user-photo: the harbour]"), None, "...and the other way round");
        assert_eq!(parse_user_tail("[user-photo: ]"), None);
    }

    #[test]
    fn the_first_character_a_description_names_is_found_as_a_whole_word() {
        let cast = [(1, "Nova"), (2, "Al")];
        assert_eq!(first_named("Al and Nova on the pier", &cast), Some(2));
        assert_eq!(first_named("nova laughing, always windy", &cast), Some(1), "not 'Al' in 'always'");
        assert_eq!(first_named("an empty harbour", &cast), None);
    }

    #[test]
    fn reads_the_description_from_any_of_the_usual_spellings() {
        assert_eq!(
            parse_tail("[photo: selfie of a young woman, silver hair, winking, cafe at night]").as_deref(),
            Some("selfie of a young woman, silver hair, winking, cafe at night")
        );
        assert_eq!(parse_tail("(Picture = \"her desk, laptop, coffee\")").as_deref(), Some("her desk, laptop, coffee"));
        assert_eq!(parse_tail("[selfie: smiling at the beach]").as_deref(), Some("smiling at the beach"));
        assert_eq!(parse_tail("[photo: her (silver) hair, (smile:1.2)]").as_deref(), Some("her (silver) hair, (smile:1.2)"));
        assert_eq!(parse_tail("[photo: the harbour)").as_deref(), Some("the harbour"));
        assert_eq!(parse_tail("[photo: ]"), None);
        assert_eq!(parse_tail("[emotion: happy]"), None);
    }
}

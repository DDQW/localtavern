//! The trailing `[photo: ...]` tag a roleplay reply ends with when the character sends the user a
//! picture in the story (a selfie over the phone, a snapshot of where they are), and the
//! `[user-photo: ...]` one describing a photo the user's character took in their own message. The
//! app draws either from the description, so the photo actually shows up in the chat.

use std::sync::LazyLock;

use regex::Regex;

/// One line for the system instruction. Photos are offered as ordinary behaviour - whenever a real
/// person with a phone would send or take one - not as a rare exception: framed as "most replies
/// send no photo, only when the story calls for it", models never sent one. Explicit that the
/// brackets are required: told only "describe it", a model writes the description into the story
/// text where the app can't find it.
///
/// `intimate` (for a character not known to be under 18) adds that the photos can be as intimate as
/// the moment: suggestive or nude ones of the character themselves when the mood is flirty or sexual.
pub fn build_instruction(character_name: &str, user_name: &str, intimate: bool) -> String {
    let (c, u) = (character_name, user_name);
    let intimate = if intimate {
        format!(
            " The photos can be as intimate as the moment between them: when the mood is flirty or sexual and {c} wants to, \
{c} can send {u} suggestive or nude photos of {c}, described plainly."
        )
    } else {
        String::new()
    };
    format!(
        "{c} sends {u} photos whenever a real person would: a selfie to show how {c} looks or what {c} is wearing, a snapshot of \
where {c} is, what {c} is doing or something {c} wants {u} to see - above all while they are apart and texting, and whenever {u} \
asks for one. When they are together, {c} can take a picture of the moment or of the two of them.{intimate} When {c} sends or takes a photo in \
this reply, show it happening in the story and end the reply with one last line in exactly this form, square brackets included: \
[photo: <what the photo shows, as a camera would see it: who is in it, their pose, expression and what they are wearing (if \
anything), the place and the lighting>]. Write the description only inside the brackets, not in the story text. Leave the line out \
when no photo is sent or taken."
    )
}

/// Whether the user's message may have their character taking a photo - then the reply is asked to
/// describe it. Just the words; whether a photo is really taken there is the model's call.
pub fn mentions_a_photo(text: &str) -> bool {
    static WORDS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(?:photo|photograph|picture|pic|pics|selfie|snapshot|snap|snaps|snapped|camera|shot of)\b").unwrap()
    });
    WORDS.is_match(text)
}

/// One more line for the system instruction when the user's message may have the user's character
/// taking a photo: the reply describes that photo in its own tag, so the app can show it too.
pub fn user_photo_instruction(user_name: &str) -> String {
    let u = user_name;
    format!(
        "If {u} takes a photo or picture in {u}'s latest message, also describe that photo, as one more last line in exactly this \
form, square brackets included: [user-photo: <what {u}'s photo shows, as the camera sees it: who or what is in it (anyone by name), \
their pose, expression and what they are wearing (if anything), the place and the lighting>]. Leave this line out when {u} takes no photo."
    )
}

/// `[user-photo: ...]` (or `user picture`), up to its `]`.
static USER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\[\s*user[-_ ]?(?:photo|picture|pic)\s*[:=]([^\[\]]*)\]|[\[(]\s*user[-_ ]?(?:photo|picture|pic)\s*[:=]([^\[\])]*)[\])]",
    )
    .unwrap()
});

/// The description in a `[user-photo: ...]` tag anywhere in `tail`.
pub fn parse_user_tail(tail: &str) -> Option<String> {
    let captures = USER.captures(tail)?;
    let description = captures.get(1).or_else(|| captures.get(2))?.as_str();
    Some(description.trim().trim_matches(['"', '*', '_']).trim().to_string()).filter(|d| !d.is_empty())
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

/// `[photo: ...]` up to its `]`, so parentheses in the description survive; otherwise a `(...)` tag or
/// a `[...)` one with mismatched brackets.
static ANY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\[\s*(?:photo|picture|image|selfie)\s*[:=]([^\[\]]*)\]|[\[(]\s*(?:photo|picture|image|selfie)\s*[:=]([^\[\])]*)[\])]",
    )
    .unwrap()
});

/// The description in a photo tag anywhere in `tail` (the trailing tags peeled off a reply).
pub fn parse_tail(tail: &str) -> Option<String> {
    let captures = ANY.captures(tail)?;
    let description = captures.get(1).or_else(|| captures.get(2))?.as_str();
    Some(description.trim().trim_matches(['"', '*', '_']).trim().to_string()).filter(|d| !d.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_names_both_sides_and_the_format() {
        let line = build_instruction("Nova", "Alex", false);
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
        let line = build_instruction("Nova", "Alex", true);
        assert!(line.contains("When they are together, Nova can take a picture of the moment or of the two of them. The photos can be as intimate"), "{line}");
        assert!(line.contains("when the mood is flirty or sexual and Nova wants to, Nova can send Alex suggestive or nude photos of Nova"), "{line}");
        assert!(!line.contains("adult"), "the model isn't asked to judge ages: {line}");
        assert!(line.ends_with("Leave the line out when no photo is sent or taken."), "{line}");
    }

    #[test]
    fn a_photo_word_in_the_users_message_asks_for_the_users_photo() {
        assert!(mentions_a_photo("*Soso decides to make a picture of the harbour*"));
        assert!(mentions_a_photo("I snap a quick selfie with you."));
        assert!(mentions_a_photo("*raises the Camera*"));
        assert!(!mentions_a_photo("We walk down to the beach together."));
        let line = user_photo_instruction("Soso");
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

//! The trailing `[photo: ...]` tag a roleplay reply ends with when the character sends the user a
//! picture in the story (a selfie over the phone, a snapshot of where they are). The app draws it
//! from the description, so the photo actually shows up in the chat.

use std::sync::LazyLock;

use regex::Regex;

/// One line for the system instruction. Strict about *when* (a photo only when the story sends one,
/// never as decoration) and explicit that the brackets are required: told only "describe it", a
/// model writes the description into the story text where the app can't find it.
pub fn build_instruction(character_name: &str, user_name: &str) -> String {
    format!(
        "Most replies send no photo. Only when {character_name} actually sends {user_name} a photo in this reply (for example a selfie \
or a picture over the phone, because the story calls for it), end the reply with one last line in exactly this form, square brackets \
included: [photo: <what the photo shows, as a camera would see it: who is in it, their pose, expression and clothing, the place and \
the lighting>]. Write the description only inside the brackets, not in the story text. Never add the line when no photo is sent."
    )
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
        let line = build_instruction("Nova", "Alex");
        assert!(line.starts_with("Most replies send no photo. Only when Nova actually sends Alex a photo"));
        assert!(line.contains("square brackets included: [photo: <"));
        assert!(line.ends_with("Never add the line when no photo is sent."));
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

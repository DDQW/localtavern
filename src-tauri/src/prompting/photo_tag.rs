//! The trailing `[photo: ...]` tag a roleplay reply ends with when the character sends the user a
//! picture in the story (a selfie over the phone, a snapshot of where they are). The app draws it
//! from the description, so the photo actually shows up in the chat.

use std::sync::LazyLock;

use regex::Regex;

/// One line for the system instruction. Photos are offered as ordinary behaviour - whenever a real
/// person with a phone would send or take one - not as a rare exception: framed as "most replies
/// send no photo, only when the story calls for it", models never sent one. Explicit that the
/// brackets are required: told only "describe it", a model writes the description into the story
/// text where the app can't find it.
pub fn build_instruction(character_name: &str, user_name: &str) -> String {
    let (c, u) = (character_name, user_name);
    format!(
        "{c} sends {u} photos whenever a real person would: a selfie to show how {c} looks or what {c} is wearing, a snapshot of \
where {c} is, what {c} is doing or something {c} wants {u} to see - above all while they are apart and texting, and whenever {u} \
asks for one. When they are together, {c} can take a picture of the moment or of the two of them. When {c} sends or takes a photo in \
this reply, show it happening in the story and end the reply with one last line in exactly this form, square brackets included: \
[photo: <what the photo shows, as a camera would see it: who is in it, their pose, expression and what they are wearing (if \
anything), the place and the lighting>]. Write the description only inside the brackets, not in the story text. Leave the line out \
when no photo is sent or taken."
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
        assert!(line.starts_with("Nova sends Alex photos whenever a real person would: a selfie"), "{line}");
        assert!(line.contains("while they are apart and texting, and whenever Alex asks for one"), "{line}");
        assert!(line.contains("When they are together, Nova can take a picture"), "{line}");
        assert!(line.contains("square brackets included: [photo: <"), "{line}");
        assert!(line.ends_with("Leave the line out when no photo is sent or taken."), "{line}");
        // Not framed as a rare exception any more: that wording never got a photo sent.
        assert!(!line.contains("Most replies") && !line.contains("Only when"), "{line}");
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

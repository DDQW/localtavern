//! The trailing `[emotion: happy]` tag a character with an expression pack is asked to end its
//! reply with, so the chat UI can swap that character's shown portrait to match.

use std::sync::LazyLock;

use regex::Regex;

static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*\[\s*emotion\s*:\s*([a-zA-Z0-9_\- ]+?)\s*\]\s*$").unwrap());

/// One line to append to the system instruction, naming the character's configured emotion keys so
/// the model picks from that exact set.
pub fn build_instruction(character_name: &str, emotion_keys: &[String]) -> String {
    let keys = emotion_keys.join(", ");
    format!(
        "At the very end of every reply, on its own, append a tag naming the single emotion {character_name} feels while saying it, chosen from exactly this list: {keys}. \
Format: [emotion: <one of the list above>]. Always include it, never explain it."
    )
}

static ANY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[\s*emotion\s*:\s*([a-zA-Z0-9_\- ]+?)\s*\]").unwrap());

/// The emotion named by a tag anywhere in `tail` (the trailing tags peeled off a reply), lowercased.
pub fn parse_tail(tail: &str) -> Option<String> {
    ANY.captures(tail).map(|c| c[1].trim().to_lowercase()).filter(|e| !e.is_empty())
}

/// Strips a trailing `[emotion: xyz]` tag, returning the cleaned text and the lowercased emotion key.
pub fn extract_and_strip(text: &str) -> (String, Option<String>) {
    let Some(caps) = TAG.captures(text) else {
        return (text.to_string(), None);
    };
    let whole = caps.get(0).unwrap();
    let emotion = caps[1].trim().to_lowercase();
    let clean = text[..whole.start()].trim_end().to_string();
    (clean, if emotion.is_empty() { None } else { Some(emotion) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_trailing_tag_and_returns_lowercased_emotion() {
        let (clean, emotion) = extract_and_strip("Hello there!\n\n[emotion: Happy]");
        assert_eq!(clean, "Hello there!");
        assert_eq!(emotion.as_deref(), Some("happy"));
    }

    #[test]
    fn leaves_text_unchanged_when_no_tag() {
        let (clean, emotion) = extract_and_strip("Just a normal reply.");
        assert_eq!(clean, "Just a normal reply.");
        assert_eq!(emotion, None);
    }

    #[test]
    fn handles_empty_text() {
        assert_eq!(extract_and_strip(""), (String::new(), None));
    }

    #[test]
    fn only_matches_a_trailing_tag_not_one_mid_text() {
        let text = "She said [emotion: happy] and walked away.";
        let (clean, emotion) = extract_and_strip(text);
        assert_eq!(clean, text);
        assert_eq!(emotion, None);
    }

    #[test]
    fn instruction_names_the_character_and_lists_keys() {
        let instruction = build_instruction("Trinity", &["happy".into(), "sad".into()]);
        assert!(instruction.contains("Trinity"));
        assert!(instruction.contains("happy, sad"));
        assert!(instruction.contains("[emotion:"));
    }

    #[test]
    fn supports_multi_word_emotion_keys() {
        let (clean, emotion) = extract_and_strip("Hm. [emotion: very sad]");
        assert_eq!(clean, "Hm.");
        assert_eq!(emotion.as_deref(), Some("very sad"));
    }
}

//! The trailing `[sound: ...]` and `[voice: ...]` tags a roleplay reply may end with: a sound worth
//! hearing, or words the character says aloud. The app turns each into a play button; the audio is
//! only made when it is pressed. Square brackets only: "(voice: low)" is just narration.

use std::sync::LazyLock;

use regex::Regex;

use super::instructions::{self, Instructions};

/// What opens a sound tag, for the tail pattern in [`super::reply_tags`].
pub const SOUND_KEY: &str = r"sound(?:\s*effect)?\s*[:=]";
/// What opens a voice tag.
pub const VOICE_KEY: &str = r"voice(?:\s*(?:line|message))?\s*[:=]";

pub fn sound_instruction(ins: &Instructions, character: &str, user: &str) -> String {
    ins.render(instructions::SOUND, &[("char", character), ("user", user)])
}

pub fn voice_instruction(ins: &Instructions, character: &str, user: &str) -> String {
    ins.render(instructions::VOICE, &[("char", character), ("user", user)])
}

static SOUND: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)\[\s*{SOUND_KEY}([^\[\]]*)\]")).unwrap());
static VOICE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)\[\s*{VOICE_KEY}([^\[\]]*)\]")).unwrap());

/// Every sound tag in `tail` (the trailing tags peeled off a reply), in order.
pub fn parse_sounds(tail: &str) -> Vec<String> {
    all(&SOUND, tail)
}

/// Every voice tag in `tail`, in order.
pub fn parse_voices(tail: &str) -> Vec<String> {
    all(&VOICE, tail)
}

fn all(pattern: &Regex, tail: &str) -> Vec<String> {
    pattern
        .captures_iter(tail)
        .filter_map(|c| {
            let text = c.get(1)?.as_str().trim().trim_matches(['"', '*', '_', '“', '”']).trim();
            (!text.is_empty()).then(|| text.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sound_and_voice_tags_in_any_order() {
        let tail = "[emotion: shy] [voice: \"Don't go in there!\"] [sound: glass breaking, far away] [Sound effect: thunder]";
        assert_eq!(parse_sounds(tail), ["glass breaking, far away", "thunder"]);
        assert_eq!(parse_voices(tail), ["Don't go in there!"]);
        assert!(parse_sounds("[voice: hi]").is_empty() && parse_voices("[sound: a door]").is_empty());
        assert!(parse_voices("[voice: ]").is_empty(), "an empty tag asks for nothing");
        assert!(parse_voices("(voice: low and husky)").is_empty(), "only square brackets");
    }

    #[test]
    fn the_instructions_name_their_tags() {
        let sound = sound_instruction(instructions::defaults(), "Nova", "Alex");
        assert!(sound.contains("[sound: <what it sounds like"), "{sound}");
        let voice = voice_instruction(instructions::defaults(), "Nova", "Alex");
        assert!(voice.contains("Nova's voice matters") && voice.contains("[voice: <the exact words Nova says aloud"), "{voice}");
    }
}

//! The trailing `[video: ...]` / `[clip: ...]` tag a roleplay reply ends with when the character
//! sends the user a short video in the story. Same idea as [`super::photo_tag`]: the app renders it
//! so the clip actually shows up in the chat.

use std::sync::LazyLock;

use regex::Regex;

use super::instructions::{self, Instructions};

/// One line for the system instruction while story videos are on.
pub fn build_instruction(ins: &Instructions, character_name: &str, user_name: &str) -> String {
    ins.render(instructions::VIDEO, &[("char", character_name), ("user", user_name)])
}

/// What opens the character's video tag: `video:`, `clip =`, `sends a video:`, etc.
pub const KEY: &str = r"(?:(?:attached|sent|sends?|sending)\s+(?:a\s+)?)?(?:video|clip|movie|footage)(?:\s+(?:attached|sent))?\s*(?:[:=\-–—]|of\b)";

static ANY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)\[\s*{KEY}([^\[\]]*)\]|[\[(]\s*{KEY}([^\[\])]*)[\])]")).unwrap());

static INLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)[*_]*\[\s*{KEY}([^\[\]]*)\][*_]*")).unwrap());

pub fn parse_tail(tail: &str) -> Option<String> {
    let captures = ANY.captures(tail)?;
    clean(captures.get(1).or_else(|| captures.get(2))?.as_str())
}

pub fn take_inline(text: &str) -> Option<(String, String)> {
    let captures = INLINE.captures(text)?;
    let description = clean(captures.get(1)?.as_str())?;
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

pub fn context_note(description: &str) -> String {
    format!("A video was shared in the story here. It showed: {}", description.trim())
}

fn clean(description: &str) -> Option<String> {
    Some(description.trim().trim_matches(['"', '*', '_']).trim().to_string()).filter(|d| !d.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_tags() {
        assert_eq!(parse_tail("[video: walking through rain, neon street]").as_deref(), Some("walking through rain, neon street"));
        assert_eq!(parse_tail("[clip: a short selfie video, waving]").as_deref(), Some("a short selfie video, waving"));
        assert_eq!(parse_tail("[photo: a still]"), None);
    }

    #[test]
    fn instruction_names_the_tag() {
        let line = build_instruction(instructions::defaults(), "Nova", "Alex");
        assert!(line.contains("Nova") && line.contains("[video:"), "{line}");
    }
}

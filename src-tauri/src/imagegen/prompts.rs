//! What to ask the chat model for in each picture mode, and how its answer becomes the prompt the
//! image server gets. The modes follow SillyTavern's: the character full-body, their face, the
//! user's persona, the whole scene, or just the last message.

use serde::{Deserialize, Serialize};

use super::PromptStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageMode {
    /// The character, full body, as they are right now.
    Character,
    /// A close-up of the character's face.
    Face,
    /// The user's persona.
    Persona,
    /// The current scene: setting, who is there and what they're doing.
    Scene,
    /// Only what happens in the last message.
    LastMessage,
}

/// The `[System note: ...]` turn appended to the chat asking for the description. `subject` is the
/// character's (or for [`ImageMode::Persona`], the user's) name; unused for scene modes.
pub fn instruction(mode: ImageMode, style: PromptStyle, subject: &str) -> String {
    let (what, opener) = match mode {
        ImageMode::Character => (
            format!(
                "{subject} as they look right now, full body: species or race, gender, apparent age, build, skin, hair, \
eyes, every piece of clothing and accessory, then their pose, expression and the surroundings in the current scene"
            ),
            "full body portrait",
        ),
        ImageMode::Face => (
            format!(
                "{subject}'s face and head for a close-up: species or race, gender, apparent age, skin, face shape, hair, \
eyes, makeup or markings, headwear or jewelry, and their expression right now"
            ),
            "close-up portrait",
        ),
        ImageMode::Persona => (
            format!(
                "{subject} (the user's character) as they look right now, full body: species or race, gender, apparent age, \
build, skin, hair, eyes, clothing and accessories, then pose, expression and surroundings"
            ),
            "full body portrait",
        ),
        ImageMode::Scene => (
            "the current scene as a single picture: the location, time of day, lighting and mood, then each character present - \
what they look like, what they wear and what they are doing"
                .to_string(),
            "scenery",
        ),
        ImageMode::LastMessage => (
            "only the moment in the last message as a single picture: the location, lighting, each character in it - what they \
look like, wear and do - ignoring earlier events that are no longer visible"
                .to_string(),
            "scenery",
        ),
    };
    let format = match style {
        PromptStyle::Tags => format!(
            "Reply with only a comma-separated list of short visual keywords, starting with \"{opener}\". No sentences, no names \
on their own, no quotes."
        ),
        PromptStyle::Natural => {
            "Reply with only two to four plain sentences of visual description, as if captioning a photo. No names on their own, \
no quotes."
                .to_string()
        }
    };
    format!(
        "[System note: Pause the roleplay. Describe {what}, for an image generator. Use only what a camera could see: no \
personality, thoughts, sounds, smells or backstory. {format}]"
    )
}

/// The picture's size for a mode: portraits use the configured size, the face a square of about
/// the same pixel count, and scenes the landscape version of it.
pub fn size_for(mode: ImageMode, width: u32, height: u32) -> (u32, u32) {
    let round = |v: f64| ((v / 64.0).round() as u32).max(1) * 64;
    match mode {
        ImageMode::Character | ImageMode::Persona => (width, height),
        ImageMode::Face => {
            let side = round(((width as f64) * (height as f64)).sqrt());
            (side, side)
        }
        ImageMode::Scene | ImageMode::LastMessage => (width.max(height), width.min(height)),
    }
}

/// Tidies the model's answer: drops a "Sure, here's..." line, wrapping quotes, a trailing period
/// on a tag list, reasoning tags and line breaks.
pub fn clean_model_prompt(raw: &str) -> String {
    let mut text = raw.trim();
    // Some local models leak their reasoning into the reply.
    if let Some(end) = text.rfind("</think>") {
        text = text[end + "</think>".len()..].trim();
    }
    let mut lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() > 1 {
        let first = lines[0].to_ascii_lowercase();
        if first.ends_with(':') || first.starts_with("sure") || first.starts_with("here") || first.starts_with("okay") {
            lines.remove(0);
        }
    }
    let joined = lines.join(" ");
    let unquoted = joined.trim_matches(|c| matches!(c, '"' | '“' | '”' | '`' | '\'')).trim();
    unquoted.strip_prefix("Prompt:").or_else(|| unquoted.strip_prefix("prompt:")).unwrap_or(unquoted).trim().to_string()
}

/// The prompt the server gets: global prefix, then the character's own prefix, then the
/// description, with empty parts skipped.
pub fn combine(prefix: &str, character_prefix: &str, description: &str) -> String {
    [prefix, character_prefix, description]
        .iter()
        .map(|p| p.trim().trim_end_matches(',').trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_name_the_subject_and_ask_for_the_chosen_format() {
        let tags = instruction(ImageMode::Character, PromptStyle::Tags, "Alice");
        assert!(tags.starts_with("[System note:") && tags.ends_with(']'));
        assert!(tags.contains("Alice as they look right now"));
        assert!(tags.contains("starting with \"full body portrait\""));

        let face = instruction(ImageMode::Face, PromptStyle::Natural, "Bob");
        assert!(face.contains("Bob's face"));
        assert!(face.contains("two to four plain sentences"));
        assert!(!face.contains("comma-separated"));

        assert!(instruction(ImageMode::Persona, PromptStyle::Tags, "Me").contains("Me (the user's character)"));
        assert!(instruction(ImageMode::Scene, PromptStyle::Tags, "").contains("current scene"));
        assert!(instruction(ImageMode::LastMessage, PromptStyle::Tags, "").contains("last message"));
    }

    #[test]
    fn sizes_follow_the_mode() {
        assert_eq!(size_for(ImageMode::Character, 832, 1216), (832, 1216));
        assert_eq!(size_for(ImageMode::Persona, 832, 1216), (832, 1216));
        assert_eq!(size_for(ImageMode::Face, 832, 1216), (1024, 1024));
        assert_eq!(size_for(ImageMode::Face, 512, 512), (512, 512));
        assert_eq!(size_for(ImageMode::Scene, 832, 1216), (1216, 832));
        assert_eq!(size_for(ImageMode::LastMessage, 1216, 832), (1216, 832));
    }

    #[test]
    fn cleans_common_model_chatter() {
        assert_eq!(clean_model_prompt("Sure! Here is the list:\n\"full body portrait, red hair, green eyes\""), "full body portrait, red hair, green eyes");
        assert_eq!(clean_model_prompt("<think>hmm</think>\n  close-up portrait, freckles  "), "close-up portrait, freckles");
        assert_eq!(clean_model_prompt("Prompt: scenery, forest,\nnight"), "scenery, forest, night");
        assert_eq!(clean_model_prompt("A woman stands in the rain."), "A woman stands in the rain.");
    }

    #[test]
    fn combines_prefixes_without_empty_parts_or_double_commas() {
        assert_eq!(combine("best quality,", "<lora:a:1>", "red hair"), "best quality, <lora:a:1>, red hair");
        assert_eq!(combine("", "  ", "red hair"), "red hair");
        assert_eq!(combine("masterpiece", "", ""), "masterpiece");
    }
}

//! What to ask the chat model for in each picture mode, and how its answer becomes the prompt the
//! image server gets. The modes follow SillyTavern's: the character full-body, their face, the
//! user's persona, the whole scene, or just the last message.

use serde::{Deserialize, Serialize};

use super::PromptStyle;
use crate::models::Character;
use crate::prompting::appearance::{self, AppearanceSubject};

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
eyes, what they are wearing right now as the story describes it (every piece of clothing and accessory, or none), \
then their pose, expression and the surroundings in the current scene"
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
build, skin, hair, eyes, what they are wearing right now as the story describes it (or nothing), then pose, \
expression and surroundings"
            ),
            "full body portrait",
        ),
        ImageMode::Scene => (
            "the current scene as a single picture: the location, time of day, lighting and mood, then each character present - \
what they look like, what they are wearing as the story describes it (if anything) and what they are doing"
                .to_string(),
            "scenery",
        ),
        ImageMode::LastMessage => (
            "only the moment in the last message as a single picture: the location, lighting, each character in it - what they \
look like, what they are wearing as the story describes it (if anything) and what they do - ignoring earlier events \
that are no longer visible"
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
    join_prompt(&[prefix, character_prefix, description])
}

/// Permanent visual identity for a picture: the character's base-look tags when set, otherwise the
/// appearance builder text, otherwise the written description. `lock` is true only for real image
/// tags (base look), which are safe to repeat after the shot so it cannot replace them.
pub fn visual_identity(base_look: &str, appearance: &str, description: &str) -> (String, bool) {
    let look = base_look.trim();
    if !look.is_empty() {
        return (look.to_string(), true);
    }
    let appearance = appearance.trim();
    if !appearance.is_empty() {
        return (appearance.to_string(), false);
    }
    let description = description.trim();
    if !description.is_empty() {
        return (description.to_string(), false);
    }
    (String::new(), false)
}

/// Resolves a character's permanent look for any picture prompt.
pub fn character_visual_identity(character: &Character) -> (String, bool) {
    let appearance = character
        .appearance
        .as_ref()
        .map(|a| with_character_name(&appearance::render(a, AppearanceSubject::Character), character.name.trim()))
        .unwrap_or_default();
    visual_identity(
        &character.picture_style.base_look,
        &appearance,
        &with_character_name(character.description.trim(), character.name.trim()),
    )
}

/// `{{char}}` in card text becomes this character's name.
fn with_character_name(text: &str, name: &str) -> String {
    if name.is_empty() {
        return text.to_string();
    }
    let lower = text.to_ascii_lowercase();
    let needle = "{{char}}";
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, _) in lower.match_indices(needle) {
        out.push_str(&text[last..start]);
        out.push_str(name);
        last = start + needle.len();
    }
    out.push_str(&text[last..]);
    out
}

/// Removes hair / eyes / skin / race / age tags from a shot so they cannot override a locked base
/// look. Pose, place, lighting and clothing stay.
pub fn scrub_appearance_overrides(shot: &str) -> String {
    shot.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty() && !looks_like_appearance_override(part))
        .collect::<Vec<_>>()
        .join(", ")
}

fn looks_like_appearance_override(part: &str) -> bool {
    let lower = part.to_ascii_lowercase();
    // Phrases first (substring), then single words on token boundaries so "fairly rainy" survives.
    const PHRASES: &[&str] = &[
        "middle eastern", "black woman", "black man", "white woman", "white man", "dark-skinned",
        "dark skinned", "light-skinned", "light skinned", "years old", "year old", "year-old",
        "middle-aged", "middle aged",
    ];
    if PHRASES.iter().any(|p| lower.contains(p)) {
        return true;
    }
    const WORDS: &[&str] = &[
        "blonde", "blond", "brunette", "redhead", "ginger", "hair", "eyes", "skin", "skinned",
        "freckles", "tan", "olive", "pale", "fair", "ebony", "caucasian", "european", "african",
        "asian", "latina", "latino", "hispanic", "korean", "japanese", "chinese", "indian", "arab",
        "teen", "teenage", "elderly",
    ];
    lower
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|w| !w.is_empty())
        .any(|w| WORDS.contains(&w))
}

/// A photo a character sent. When the card has real base-look tags, the shot is scrubbed of
/// appearance overrides and the look/outfit are placed at the end (where image models weight them
/// more). Doubling the look around a conflicting shot was getting ignored. Prose fallbacks stay
/// once, before the shot.
pub fn story_photo_prompt(prefix: &str, look: &str, outfit: &str, shot: &str, lock_look: bool) -> String {
    let look = look.trim();
    let outfit = outfit.trim();
    if lock_look {
        let shot = scrub_appearance_overrides(shot);
        // Identity last: prefix + cleaned shot + look + outfit.
        join_prompt(&[prefix, &shot, look, outfit])
    } else {
        join_prompt(&[prefix, look, outfit, shot.trim()])
    }
}

fn join_prompt(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| p.trim().trim_end_matches(',').trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What one picture in a character's set shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CharacterShot {
    /// A portrait for the expression pack.
    Mood { emotion: String },
    /// The character doing something, full body.
    Action { action: String },
}

/// Visual tags for an emotion name. Names beyond the common ones still work ("smug" → "smug
/// expression"), since a pack's emotion keys are free-form.
pub fn mood_tags(emotion: &str) -> String {
    let key = emotion.trim().to_lowercase();
    let tags = match key.as_str() {
        "neutral" | "calm" => "neutral expression, calm, relaxed",
        "happy" | "joy" => "happy, smiling",
        "sad" | "sadness" => "sad, downcast eyes, slight frown",
        "angry" | "anger" => "angry, furrowed brows, glaring",
        "surprised" | "surprise" => "surprised, wide eyes, open mouth",
        "scared" | "fear" | "afraid" => "scared, fearful expression, wide eyes",
        "disgusted" | "disgust" => "disgusted, grimace, wrinkled nose",
        "love" | "loving" => "loving gaze, soft smile, blushing",
        "embarrassed" => "embarrassed, blushing, looking away",
        "laughing" | "amused" | "amusement" => "laughing, eyes closed, open mouth smile",
        "crying" => "crying, tears streaming",
        "excited" | "excitement" => "excited, beaming smile, sparkling eyes",
        "confused" | "confusion" => "confused, tilted head, raised eyebrow",
        "thinking" | "curious" | "curiosity" => "thoughtful, hand on chin, curious look",
        "tired" | "sleepy" => "tired, drowsy, half-closed eyes",
        "smug" => "smug, smirk, half-lidded eyes",
        "nervous" | "anxious" | "nervousness" => "nervous, worried expression, sweat drop",
        "flirty" | "desire" => "flirty, sultry look, playful smile",
        "pain" | "hurt" => "pained expression, wincing",
        "proud" | "pride" => "proud, confident smile, chin up",
        _ => return if key.is_empty() { String::new() } else { format!("{key} expression") },
    };
    tags.to_string()
}

/// The prompt for one picture of a character's set: prefix, permanent look, clothing, then the
/// shot itself. Moods are upper-body portraits on a plain background so a set reads as a set.
pub fn character_picture_prompt(prefix: &str, base_look: &str, clothing: &str, shot: &CharacterShot) -> String {
    let shot_text = match shot {
        CharacterShot::Mood { emotion } => {
            format!("upper body portrait, looking at viewer, {}, simple background", mood_tags(emotion))
        }
        CharacterShot::Action { action } => format!("full body, {}", action.trim()),
    };
    let leading = combine(prefix, base_look, clothing);
    combine(&leading, "", &shot_text)
}

/// Mood portraits are square (they become the round chat avatar); actions use the portrait size.
pub fn character_picture_size(shot: &CharacterShot, width: u32, height: u32) -> (u32, u32) {
    match shot {
        CharacterShot::Mood { .. } => size_for(ImageMode::Face, width, height),
        CharacterShot::Action { .. } => (width, height),
    }
}

/// Splits the model's "LOOK: … / OUTFIT: …" answer. Without the labels, everything is the look.
pub fn parse_look(raw: &str) -> (String, String) {
    let text = clean_think(raw);
    let mut look = String::new();
    let mut outfit = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("look:").map(|_| &line[5..]) {
            look = clean_model_prompt(rest);
        } else if let Some(rest) = lower.strip_prefix("outfit:").map(|_| &line[7..]) {
            outfit = clean_model_prompt(rest);
        }
    }
    if look.is_empty() && outfit.is_empty() {
        look = clean_model_prompt(&text);
    }
    (look, outfit)
}

fn clean_think(raw: &str) -> String {
    let text = raw.trim();
    match text.rfind("</think>") {
        Some(end) => text[end + "</think>".len()..].trim().to_string(),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mood_tags_cover_the_suggested_emotions_and_any_other_name() {
        for emotion in ["neutral", "happy", "sad", "angry", "surprised", "scared", "disgusted", "love"] {
            let tags = mood_tags(emotion);
            assert!(!tags.is_empty() && !tags.ends_with(" expression") || emotion == "neutral", "{emotion}: {tags}");
        }
        assert_eq!(mood_tags(" Happy "), "happy, smiling");
        assert_eq!(mood_tags("Bored"), "bored expression");
        assert_eq!(mood_tags(" "), "");
    }

    #[test]
    fn character_pictures_put_the_look_and_clothing_before_the_shot() {
        let mood = CharacterShot::Mood { emotion: "happy".into() };
        assert_eq!(
            character_picture_prompt("best quality", "1girl, silver hair", "navy flight jacket", &mood),
            "best quality, 1girl, silver hair, navy flight jacket, upper body portrait, looking at viewer, happy, smiling, simple background"
        );
        let action = CharacterShot::Action { action: " reading by the fire ".into() };
        assert_eq!(character_picture_prompt("", "1girl", "", &action), "1girl, full body, reading by the fire");
        assert_eq!(character_picture_size(&mood, 832, 1216), (1024, 1024));
        assert_eq!(character_picture_size(&action, 832, 1216), (832, 1216));
    }

    #[test]
    fn shots_use_a_tagged_json_shape() {
        let shot: CharacterShot = serde_json::from_str(r#"{"kind":"mood","emotion":"sad"}"#).unwrap();
        assert_eq!(shot, CharacterShot::Mood { emotion: "sad".into() });
        let shot: CharacterShot = serde_json::from_str(r#"{"kind":"action","action":"waving"}"#).unwrap();
        assert_eq!(shot, CharacterShot::Action { action: "waving".into() });
    }

    #[test]
    fn parses_look_and_outfit_lines() {
        assert_eq!(
            parse_look("Sure!
LOOK: 1girl, silver hair, blue eyes
Outfit: \"navy flight jacket, boots\""),
            ("1girl, silver hair, blue eyes".to_string(), "navy flight jacket, boots".to_string())
        );
        assert_eq!(parse_look("<think>x</think>tall man, beard"), ("tall man, beard".to_string(), String::new()));
    }

    #[test]
    fn instructions_name_the_subject_and_ask_for_the_chosen_format() {
        let tags = instruction(ImageMode::Character, PromptStyle::Tags, "Alice");
        assert!(tags.starts_with("[System note:") && tags.ends_with(']'));
        assert!(tags.contains("Alice as they look right now"));
        assert!(tags.contains("as the story describes it"), "clothing follows the story rather than being assumed: {tags}");
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

    #[test]
    fn a_story_photo_puts_locked_look_last_and_scrubs_overrides_from_the_shot() {
        assert_eq!(
            story_photo_prompt("best quality", "1girl, dark skin, black hair", "navy jacket", "winking, blonde, blue eyes, cafe", true),
            "best quality, winking, cafe, 1girl, dark skin, black hair, navy jacket"
        );
        assert_eq!(
            story_photo_prompt("best quality", "an African woman with dark skin and black hair", "", "winking, cafe", false),
            "best quality, an African woman with dark skin and black hair, winking, cafe"
        );
        assert_eq!(story_photo_prompt("best quality", "", "", "winking", false), "best quality, winking");
        assert_eq!(story_photo_prompt("best quality", "1girl", "", "a sunset", true), "best quality, a sunset, 1girl");
    }

    #[test]
    fn scrub_appearance_overrides_keeps_pose_and_place() {
        assert_eq!(scrub_appearance_overrides("winking, blonde, cafe at night, blue eyes"), "winking, cafe at night");
        assert_eq!(scrub_appearance_overrides("sitting on a bench, rainy street"), "sitting on a bench, rainy street");
    }

    #[test]
    fn visual_identity_prefers_base_look_then_appearance_then_description() {
        assert_eq!(visual_identity("1girl, dark skin", "fair skin woman", "Asian man"), ("1girl, dark skin".into(), true));
        assert_eq!(visual_identity("", "a Black woman, dark skin", "ignored"), ("a Black woman, dark skin".into(), false));
        assert_eq!(visual_identity("", "", "Korean woman, black hair"), ("Korean woman, black hair".into(), false));
        assert_eq!(visual_identity(" ", "", ""), (String::new(), false));
    }
}

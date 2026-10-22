use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Appearance, Schedule};

/// One expression-pack image, keyed by a free-form emotion name (e.g. "happy") - matches
/// SillyTavern's sprite-folder convention. Any key is allowed, not just the suggested defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionImage {
    pub emotion: String,
    pub image_path: String,
}

/// How pictures of the character are drawn, so every mood, outfit and pose looks like the same
/// person: the same permanent features and the same seed each time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PictureStyle {
    /// Permanent physical features as image-prompt text (no clothing).
    pub base_look: String,
    /// What they wear when no other outfit is chosen.
    pub default_outfit: String,
    /// Extra positive tags for this character (after the global picture prefix).
    #[serde(default)]
    pub prompt_prefix: String,
    /// Extra negative tags for this character (after the global negative prompt).
    #[serde(default)]
    pub negative_prompt: String,
    /// Shared by all of the character's pictures; -1 until the first one is drawn.
    pub seed: i64,
}

impl Default for PictureStyle {
    fn default() -> Self {
        Self {
            base_look: String::new(),
            default_outfit: String::new(),
            prompt_prefix: String::new(),
            negative_prompt: String::new(),
            seed: -1,
        }
    }
}

/// A costume: its clothing and its own expression pack. A chat that puts the character in it shows
/// these portraits instead of the default ones (SillyTavern's `/costume`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outfit {
    pub id: Uuid,
    pub name: String,
    /// The clothing, as image-prompt text; also what the model is told they're wearing.
    pub clothing: String,
    #[serde(default)]
    pub expressions: Vec<ExpressionImage>,
}

/// A picture of the character doing something ("reading by the fire").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPicture {
    pub action: String,
    pub image_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    pub id: Uuid,
    pub name: String,
    pub avatar_path: Option<String>,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub first_messages: Vec<String>,
    pub example_dialogue: String,
    pub tags: Vec<String>,
    pub linked_lorebook_ids: Vec<Uuid>,
    /// Emotion-keyed portraits swapped in during chat based on each reply's detected emotion.
    pub expressions: Vec<ExpressionImage>,
    /// Extra reference images beyond the avatar and expression pack.
    pub gallery_image_paths: Vec<String>,
    /// The quick-builder picks behind the appearance paragraph in `description`, if it was used.
    pub appearance: Option<Appearance>,
    pub birth_date: Option<NaiveDate>,
    /// Weekly routine and home country; `None` when never filled in.
    pub schedule: Option<Schedule>,
    #[serde(default)]
    pub picture_style: PictureStyle,
    #[serde(default)]
    pub outfits: Vec<Outfit>,
    #[serde(default)]
    pub action_pictures: Vec<ActionPicture>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Default for Character {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            avatar_path: None,
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_messages: Vec::new(),
            example_dialogue: String::new(),
            tags: Vec::new(),
            linked_lorebook_ids: Vec::new(),
            expressions: Vec::new(),
            gallery_image_paths: Vec::new(),
            appearance: None,
            birth_date: None,
            schedule: None,
            picture_style: PictureStyle::default(),
            outfits: Vec::new(),
            action_pictures: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

impl Character {
    /// The character as a chat with `outfit_id` chosen sees them: the outfit's expression pack (when
    /// it has one) replaces the default, and the description says what they're wearing. An unknown
    /// or absent outfit leaves them as they are.
    pub fn dressed(mut self, outfit_id: Option<Uuid>) -> Self {
        let Some(outfit) = outfit_id.and_then(|id| self.outfits.iter().find(|o| o.id == id)).cloned() else {
            return self;
        };
        if outfit.expressions.iter().any(|e| !e.image_path.is_empty()) {
            self.expressions = outfit.expressions.clone();
        }
        let clothing = outfit.clothing.trim();
        let wearing = if clothing.is_empty() { outfit.name.trim() } else { clothing };
        if !wearing.is_empty() {
            let note = format!("Currently wearing: {wearing}");
            self.description =
                if self.description.trim().is_empty() { note } else { format!("{}

{note}", self.description.trim_end()) };
        }
        self
    }
}

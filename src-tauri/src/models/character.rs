use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Appearance;

/// One expression-pack image, keyed by a free-form emotion name (e.g. "happy") - matches
/// SillyTavern's sprite-folder convention. Any key is allowed, not just the suggested defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionImage {
    pub emotion: String,
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
    /// The quick-builder picks behind `appearance_text`, if the builder was used.
    pub appearance: Option<Appearance>,
    /// Rendered (or hand-edited / AI-polished) appearance, sent right after the description.
    pub appearance_text: String,
    pub birth_date: Option<NaiveDate>,
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
            appearance_text: String::new(),
            birth_date: None,
            created_at: now,
            updated_at: now,
        }
    }
}

impl Character {
    /// The description plus the appearance text, as the prompt sees them.
    pub fn full_description(&self) -> String {
        join_description(&self.description, &self.appearance_text)
    }
}

/// Joins a description and an appearance text with a blank line, skipping whichever is blank.
pub fn join_description(description: &str, appearance: &str) -> String {
    match (description.trim(), appearance.trim()) {
        (d, "") => d.to_string(),
        ("", a) => a.to_string(),
        (d, a) => format!("{d}\n\n{a}"),
    }
}

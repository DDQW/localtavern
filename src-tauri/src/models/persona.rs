use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{join_description, Appearance};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Persona {
    pub id: Uuid,
    pub name: String,
    pub avatar_path: Option<String>,
    pub description: String,
    pub is_default: bool,
    /// The quick-builder picks behind `appearance_text`, if the builder was used.
    pub appearance: Option<Appearance>,
    /// Rendered (or hand-edited / AI-polished) appearance, sent right after the description.
    pub appearance_text: String,
    pub birth_date: Option<NaiveDate>,
}

impl Default for Persona {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            avatar_path: None,
            description: String::new(),
            is_default: false,
            appearance: None,
            appearance_text: String::new(),
            birth_date: None,
        }
    }
}

impl Persona {
    /// The description plus the appearance text, as the prompt sees them.
    pub fn full_description(&self) -> String {
        join_description(&self.description, &self.appearance_text)
    }
}

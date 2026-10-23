use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A side character a story has introduced: someone who is not on a character card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryCharacter {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub name: String,
    /// Age in years when the story introduced them.
    pub age: Option<i32>,
    /// Who they are to the story ("Mia's landlord", "bartender at the Anchor").
    pub role: String,
    pub appearance: String,
    /// Explicit physical details the story has established.
    pub adult_details: String,
    /// The in-story time of the exchange that introduced them.
    pub introduced_at: Option<NaiveDateTime>,
    pub introduced_message_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl StoryCharacter {
    pub fn new(conversation_id: Uuid, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            name: name.into(),
            age: None,
            role: String::new(),
            appearance: String::new(),
            adult_details: String::new(),
            introduced_at: None,
            introduced_message_id: None,
            created_at: Utc::now(),
        }
    }

    /// Drops what must not be kept: trims name and validates age range.
    pub fn enforce_rules(&mut self) {
        self.name = self.name.trim().to_string();
        self.age = self.age.filter(|a| (0..=1000).contains(a));
    }
}

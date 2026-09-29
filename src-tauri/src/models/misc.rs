use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One durable fact the assistant has learned about the user, fed back into future Assistant-mode prompts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMemory {
    pub id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

impl UserMemory {
    pub fn new(content: impl Into<String>) -> Self {
        Self { id: Uuid::new_v4(), content: content.into(), created_at: Utc::now() }
    }
}

/// Something one specific character has learned during a conversation. Scoped per character so a
/// group chat's characters can each know different things.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterNotebookEntry {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub character_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

impl CharacterNotebookEntry {
    pub fn new(conversation_id: Uuid, character_id: Uuid, content: impl Into<String>) -> Self {
        Self { id: Uuid::new_v4(), conversation_id, character_id, content: content.into(), created_at: Utc::now() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestStatus {
    Active,
    Completed,
    Failed,
}
int_enum!(QuestStatus { Active = 0, Completed = 1, Failed = 2 });

/// A story objective tracked per conversation, maintained automatically after each reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestLogEntry {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub title: String,
    pub description: String,
    pub status: QuestStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl QuestLogEntry {
    pub fn new(conversation_id: Uuid, title: impl Into<String>, description: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            title: title.into(),
            description: description.into(),
            status: QuestStatus::Active,
            created_at: now,
            updated_at: now,
        }
    }
}

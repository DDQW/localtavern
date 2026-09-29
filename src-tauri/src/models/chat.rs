use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageRole {
    User,
    Character,
    System,
    /// A PowerShell tool call's result, fed back to the model as if the user had reported it.
    Tool,
}
int_enum!(MessageRole { User = 0, Character = 1, System = 2, Tool = 3 });

/// One generated variant of a message - "swiping" cycles through these without losing the others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageAlternate {
    pub text: String,
    pub reasoning_text: Option<String>,
    #[serde(default)]
    pub raw_prompt: Option<String>,
}

/// An image attached to a message - picked by the user to send, or returned by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageAttachment {
    pub image_path: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmUsage {
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub role: MessageRole,
    /// Which character spoke, when `role == Character` in a group chat.
    pub speaker_character_id: Option<Uuid>,
    /// The currently shown text. Once `alternates` exist this always equals the active alternate's
    /// text - use [`ChatMessage::set_text`] to change it so the two cannot desync.
    pub text: String,
    /// The model's reasoning trace, kept apart from `text` so the UI can hide it by default.
    pub reasoning_text: Option<String>,
    /// The exact prompt sent to the provider to produce this reply, for inspection.
    pub raw_prompt_text: Option<String>,
    /// Every generated variant (regeneration appends, swiping navigates).
    pub alternates: Vec<MessageAlternate>,
    pub active_alternate_index: i32,
    pub attachments: Vec<MessageAttachment>,
    /// Emotion detected in this reply when the speaking character has an expression pack.
    pub detected_emotion: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub usage: Option<LlmUsage>,
}

impl ChatMessage {
    pub fn new(conversation_id: Uuid, role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            role,
            speaker_character_id: None,
            text: text.into(),
            reasoning_text: None,
            raw_prompt_text: None,
            alternates: Vec::new(),
            active_alternate_index: 0,
            attachments: Vec::new(),
            detected_emotion: None,
            timestamp: Utc::now(),
            usage: None,
        }
    }

    fn active_index(&self) -> Option<usize> {
        if self.alternates.is_empty() {
            None
        } else {
            Some((self.active_alternate_index.max(0) as usize).min(self.alternates.len() - 1))
        }
    }

    /// The text that should be persisted/shown: the active alternate's when any exist, else `text`.
    pub fn effective_text(&self) -> &str {
        match self.active_index() {
            Some(i) => &self.alternates[i].text,
            None => &self.text,
        }
    }

    /// Sets the shown text, updating the active alternate too when there are any.
    pub fn set_text(&mut self, value: impl Into<String>) {
        let value = value.into();
        if let Some(i) = self.active_index() {
            self.alternates[i].text = value.clone();
        }
        self.text = value;
    }

    /// Re-establishes the invariant `text == active alternate's text`; called after loading a row.
    pub fn sync_text_from_alternates(&mut self) {
        if let Some(i) = self.active_index() {
            self.text = self.alternates[i].text.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alt(t: &str) -> MessageAlternate {
        MessageAlternate { text: t.into(), reasoning_text: None, raw_prompt: None }
    }

    #[test]
    fn set_text_updates_active_alternate() {
        let mut m = ChatMessage::new(Uuid::new_v4(), MessageRole::Character, "a");
        m.alternates = vec![alt("a"), alt("b")];
        m.active_alternate_index = 1;
        m.set_text("B!");
        assert_eq!(m.alternates[1].text, "B!");
        assert_eq!(m.alternates[0].text, "a");
        assert_eq!(m.text, "B!");
    }

    #[test]
    fn out_of_range_index_is_clamped() {
        let mut m = ChatMessage::new(Uuid::new_v4(), MessageRole::Character, "ignored");
        m.alternates = vec![alt("only")];
        m.active_alternate_index = 9;
        m.sync_text_from_alternates();
        assert_eq!(m.text, "only");
    }
}

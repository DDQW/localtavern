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
    /// A generated picture (text = the prompt it was drawn from). Shown in the chat but never sent
    /// to the model: see [`crate::chat::session::load_reply_context`].
    Image,
}
int_enum!(MessageRole { User = 0, Character = 1, System = 2, Tool = 3, Image = 4 });

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
    /// The story clock before this reply and what the reply did to it (roleplay replies).
    #[serde(default)]
    pub clock: Option<super::ClockSnapshot>,
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
            clock: None,
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

    /// Snapshots the current text/reasoning/prompt as alternate #1 - call once a brand-new reply has
    /// finished streaming.
    pub fn finalize_generation(&mut self) {
        self.alternates = vec![MessageAlternate {
            text: self.text.clone(),
            reasoning_text: self.reasoning_text.clone().filter(|r| !r.is_empty()),
            raw_prompt: self.raw_prompt_text.clone().filter(|p| !p.is_empty()),
        }];
        self.active_alternate_index = 0;
    }

    /// Appends a freshly regenerated variant and switches to showing it. A message that predates
    /// alternates keeps its original text as alternate #1 instead of losing it.
    pub fn add_alternate(&mut self, text: String, reasoning_text: Option<String>, raw_prompt: Option<String>) {
        if self.alternates.is_empty() {
            self.finalize_generation();
        }
        self.alternates.push(MessageAlternate { text, reasoning_text, raw_prompt });
        self.swipe_to(self.alternates.len() - 1);
    }

    /// Replaces the shown variant's text, reasoning and prompt in place (used by "Continue": it is
    /// still the same generation, just longer, so no new alternate is added).
    pub fn extend_active(&mut self, text: String, reasoning_text: Option<String>, raw_prompt: Option<String>) {
        if let Some(i) = self.active_index() {
            self.alternates[i].text = text.clone();
            self.alternates[i].reasoning_text = reasoning_text.clone();
            self.alternates[i].raw_prompt = raw_prompt.clone().or_else(|| self.alternates[i].raw_prompt.take());
        }
        self.text = text;
        self.reasoning_text = reasoning_text;
        self.raw_prompt_text = raw_prompt.or_else(|| self.raw_prompt_text.take());
    }

    /// Shows another alternate. Returns false (and changes nothing) when `index` is out of range.
    pub fn swipe_to(&mut self, index: usize) -> bool {
        let Some(alt) = self.alternates.get(index) else {
            return false;
        };
        self.text = alt.text.clone();
        self.reasoning_text = alt.reasoning_text.clone();
        self.raw_prompt_text = alt.raw_prompt.clone();
        self.active_alternate_index = index as i32;
        true
    }

    pub fn can_swipe_previous(&self) -> bool {
        self.active_alternate_index > 0
    }

    /// Whether swiping right lands on an already-generated alternate (true) or needs a new one.
    pub fn can_swipe_to_existing_next(&self) -> bool {
        (self.active_alternate_index.max(0) as usize) + 1 < self.alternates.len()
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

    #[test]
    fn add_alternate_preserves_the_original_and_shows_the_new_one() {
        let mut m = ChatMessage::new(Uuid::new_v4(), MessageRole::Character, "original");
        m.add_alternate("second".into(), Some("thoughts".into()), Some("prompt".into()));
        assert_eq!(m.alternates.len(), 2);
        assert_eq!(m.alternates[0].text, "original");
        assert_eq!(m.active_alternate_index, 1);
        assert_eq!(m.text, "second");
        assert_eq!(m.reasoning_text.as_deref(), Some("thoughts"));
        assert_eq!(m.raw_prompt_text.as_deref(), Some("prompt"));
    }

    #[test]
    fn extend_active_rewrites_the_shown_variant_without_adding_one() {
        let mut m = ChatMessage::new(Uuid::new_v4(), MessageRole::Character, "a");
        m.raw_prompt_text = Some("p1".into());
        m.finalize_generation();
        m.add_alternate("b".into(), None, Some("p2".into()));

        m.extend_active("b and more".into(), Some("why".into()), Some("p3".into()));
        assert_eq!(m.alternates.len(), 2);
        assert_eq!((m.text.as_str(), m.alternates[1].text.as_str()), ("b and more", "b and more"));
        assert_eq!(m.alternates[1].reasoning_text.as_deref(), Some("why"));
        assert_eq!((m.raw_prompt_text.as_deref(), m.alternates[1].raw_prompt.as_deref()), (Some("p3"), Some("p3")));
        assert_eq!(m.alternates[0].text, "a", "the other variant is untouched");

        // Without a new prompt the old one stays.
        m.extend_active("b and more!".into(), None, None);
        assert_eq!(m.raw_prompt_text.as_deref(), Some("p3"));
        assert_eq!(m.alternates[1].raw_prompt.as_deref(), Some("p3"));
    }

    #[test]
    fn swipe_to_moves_text_reasoning_and_prompt_together() {
        let mut m = ChatMessage::new(Uuid::new_v4(), MessageRole::Character, "a");
        m.reasoning_text = Some("ra".into());
        m.finalize_generation();
        m.add_alternate("b".into(), None, None);
        assert!(m.can_swipe_previous());
        assert!(!m.can_swipe_to_existing_next());

        assert!(m.swipe_to(0));
        assert_eq!((m.text.as_str(), m.reasoning_text.as_deref()), ("a", Some("ra")));
        assert!(!m.can_swipe_previous());
        assert!(m.can_swipe_to_existing_next());
        assert!(!m.swipe_to(5));
        assert_eq!(m.active_alternate_index, 0);
    }

    #[test]
    fn finalize_generation_drops_empty_reasoning_and_prompt() {
        let mut m = ChatMessage::new(Uuid::new_v4(), MessageRole::Character, "hi");
        m.reasoning_text = Some(String::new());
        m.finalize_generation();
        assert_eq!(m.alternates[0].reasoning_text, None);
        assert_eq!(m.alternates[0].raw_prompt, None);
    }
}

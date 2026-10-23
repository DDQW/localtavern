use std::sync::Mutex;

use serde::Serialize;
use uuid::Uuid;

use crate::models::{ChatMessage, StoryCharacter, WorldState};
use crate::providers::ProviderErrorKind;

/// What the engine tells the UI while a reply is being produced. Serialized as
/// `{ "type": "delta", "messageId": ..., ... }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChatEvent {
    /// A new message (the user's, or the still-empty reply that is about to stream).
    MessageAdded { message: ChatMessage },
    /// An existing message is being regenerated: clear its shown text and stream into it.
    Regenerating { message_id: Uuid },
    /// An existing message is being extended: keep its text and append the streamed deltas.
    Continuing { message_id: Uuid },
    /// Streamed text and reasoning since the last delta, coalesced to roughly 30 per second.
    Delta { message_id: Uuid, text: String, reasoning: String },
    /// The message's final (or restored) state.
    MessageUpdated { message: ChatMessage },
    MessageRemoved { message_id: Uuid },
    WorldStateChanged { conversation_id: Uuid, world_state: WorldState },
    /// The story's side-character list changed (someone new was introduced, or learned about).
    CastChanged { conversation_id: Uuid, cast: Vec<StoryCharacter> },
    TitleChanged { conversation_id: Uuid, title: String },
    /// The character sent the user a photo in the story (`[photo: ...]`); the app draws it.
    PhotoSent { conversation_id: Uuid, message_id: Uuid, character_id: Option<Uuid>, description: String },
    /// The character sent the user a video in the story (`[video: ...]`); the app renders it.
    VideoSent { conversation_id: Uuid, message_id: Uuid, character_id: Option<Uuid>, description: String },
    /// A group member took the user aside (`[private]`): `breakout_id` is their private talk, which
    /// the UI opens (and has `character_id` speak first in).
    BreakoutStarted { conversation_id: Uuid, breakout_id: Uuid, character_id: Uuid },
    /// A private talk's member ended it (`[back]`): the UI takes the user back to the group, where
    /// `character_id` speaks up.
    LeavingTalk { conversation_id: Uuid, character_id: Uuid },
    /// The group could see or hear part of a private talk: `message` is the note now in the group
    /// conversation `conversation_id`, and `reactors` are the members who would react to it (the UI
    /// has them answer there). Comes through the private talk's stream, possibly after it finished.
    Overheard { conversation_id: Uuid, message: ChatMessage, reactors: Vec<Uuid> },
    /// The request was getting too big for the model, so older messages are being summarized first.
    Compacting { conversation_id: Uuid },
    /// A compaction finished (or failed - an `Error` says so); the context meter is out of date.
    Compacted { conversation_id: Uuid },
    /// The reply waits for a picture to finish rendering: both need the one graphics card.
    WaitingForPicture { message_id: Uuid },
    /// The assistant wants to run a PowerShell command and needs the user's decision - answered
    /// through `ChatEngine::resolve_tool_approval` with this `request_id`.
    ToolApproval { request_id: Uuid, command: String, elevate: bool },
    Error { message: String, kind: Option<ProviderErrorKind> },
    /// Always the last event of one operation, success or not.
    Finished,
}

/// Receives events. The app implements this over a Tauri channel; tests collect into a vector.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: ChatEvent);
}

/// Collects events in memory.
#[derive(Default)]
pub struct VecSink(Mutex<Vec<ChatEvent>>);

impl VecSink {
    pub fn events(&self) -> Vec<ChatEvent> {
        self.0.lock().unwrap().clone()
    }
}

impl EventSink for VecSink {
    fn emit(&self, event: ChatEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_a_type_tag_and_camel_case_fields() {
        let id = Uuid::nil();
        let json = serde_json::to_value(ChatEvent::Delta { message_id: id, text: "hi".into(), reasoning: String::new() }).unwrap();
        assert_eq!(json["type"], "delta");
        assert_eq!(json["messageId"], id.to_string());
        assert_eq!(json["text"], "hi");

        let finished = serde_json::to_value(ChatEvent::Finished).unwrap();
        assert_eq!(finished, serde_json::json!({ "type": "finished" }));

        let error = serde_json::to_value(ChatEvent::Error { message: "x".into(), kind: Some(ProviderErrorKind::RateLimit) }).unwrap();
        assert_eq!(error["kind"], "rateLimit");
    }
}

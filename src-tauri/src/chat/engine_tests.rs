//! End-to-end tests of the chat engine against an in-memory database and a scripted provider.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use futures::StreamExt;
use uuid::Uuid;

use super::engine::ChatEngine;
use super::events::{ChatEvent, EventSink, VecSink};
use crate::db::{open_in_memory, Db};
use crate::error::Result;
use crate::llm::{LlmRequest, LlmRole};
use crate::models::{
    Character, ChatMessage, ConversationMode, ExpressionImage, GroupActivationMode, GroupChat, LlmUsage, MessageRole,
    Persona, WorldState,
};
use crate::providers::{
    ChunkStream, LlmProvider, LlmProviderRegistry, LlmResponseChunk, ProviderError, ProviderErrorKind,
};
use crate::repo::{characters, conversations, group_chats, personas as persona_repo};
use crate::settings;

// ---- a scripted provider ----

enum Script {
    /// Delivers these items in order, then ends.
    Items(Vec<std::result::Result<LlmResponseChunk, ProviderError>>),
    /// Delivers these chunks, then never finishes (until the stream is dropped).
    Hang(Vec<LlmResponseChunk>),
    /// The request itself is rejected before any chunk.
    FailToStart(ProviderError),
}

struct FakeProvider {
    scripts: Mutex<VecDeque<Script>>,
    requests: Mutex<Vec<LlmRequest>>,
}

impl FakeProvider {
    fn new(scripts: Vec<Script>) -> Arc<Self> {
        Arc::new(Self { scripts: Mutex::new(scripts.into()), requests: Mutex::new(vec![]) })
    }

    fn requests(&self) -> Vec<LlmRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmProvider for FakeProvider {
    fn provider_id(&self) -> &str {
        "fake"
    }

    async fn list_models(&self) -> std::result::Result<Vec<String>, ProviderError> {
        Ok(vec!["fake-model".into()])
    }

    async fn stream_completion(&self, request: LlmRequest) -> std::result::Result<ChunkStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        match self.scripts.lock().unwrap().pop_front().unwrap_or(Script::Items(vec![])) {
            Script::Items(items) => Ok(Box::pin(futures::stream::iter(items))),
            Script::FailToStart(e) => Err(e),
            Script::Hang(chunks) => Ok(Box::pin(
                futures::stream::iter(chunks.into_iter().map(Ok)).chain(futures::stream::pending()),
            )),
        }
    }
}

fn chunk(text: &str) -> std::result::Result<LlmResponseChunk, ProviderError> {
    Ok(LlmResponseChunk::text(text))
}

fn finish(reason: &str) -> std::result::Result<LlmResponseChunk, ProviderError> {
    Ok(LlmResponseChunk {
        is_final: true,
        finish_reason: Some(reason.into()),
        usage: Some(LlmUsage { prompt_tokens: Some(3), completion_tokens: Some(2), total_tokens: Some(5) }),
        ..Default::default()
    })
}

struct Fixture {
    engine: ChatEngine,
    provider: Arc<FakeProvider>,
    db: Db,
    _images: tempfile::TempDir,
}

fn fixture(scripts: Vec<Script>) -> Fixture {
    let provider = FakeProvider::new(scripts);
    let db = Db::new(open_in_memory().unwrap());
    let images = tempfile::tempdir().unwrap();
    let registry = LlmProviderRegistry::new(vec![provider.clone()]);
    let engine = ChatEngine::new(db.clone(), Arc::new(registry), images.path().to_path_buf());
    Fixture { engine, provider, db, _images: images }
}

fn add_character(db: &Db, name: &str, greeting: Option<&str>, emotions: &[&str]) -> Character {
    let character = Character {
        name: name.into(),
        first_messages: greeting.map(|g| vec![g.to_string()]).unwrap_or_default(),
        expressions: emotions
            .iter()
            .map(|e| ExpressionImage { emotion: e.to_string(), image_path: format!("{e}.png") })
            .collect(),
        ..Default::default()
    };
    db.with(|c| characters::upsert(c, &character)).unwrap();
    character
}

fn add_persona(db: &Db, name: &str) -> Persona {
    let persona = Persona { name: name.into(), is_default: true, ..Default::default() };
    db.with(|c| persona_repo::upsert(c, &persona)).unwrap();
    persona
}

/// An assistant chat with a model selected (a fresh conversation has none until one is picked).
fn ready_assistant(f: &Fixture) -> Uuid {
    let session = f.engine.open_assistant(false).unwrap();
    f.engine.set_model(session.conversation.id, "fake", "fake-model").unwrap();
    session.conversation.id
}

/// A roleplay with a greeting-less character. Unlike an Assistant chat it never makes the background
/// title request, so scripted providers see exactly the requests a test expects.
fn ready_roleplay(f: &Fixture) -> Uuid {
    let character = add_character(&f.db, "Nova", None, &[]);
    ready_character(f, &character)
}

fn ready_character(f: &Fixture, character: &Character) -> Uuid {
    let session = f.engine.open_character(character.id, false).unwrap();
    f.engine.set_model(session.conversation.id, "fake", "fake-model").unwrap();
    session.conversation.id
}

fn messages(f: &Fixture, conversation: Uuid) -> Vec<ChatMessage> {
    f.db.with(|c| conversations::get_messages(c, conversation)).unwrap()
}

async fn send(f: &Fixture, conversation: Uuid, text: &str) -> (Result<()>, Vec<ChatEvent>) {
    let sink = Arc::new(VecSink::default());
    let result = f.engine.send_message(conversation, text.into(), vec![], sink.clone()).await;
    (result, sink.events())
}

fn streamed_text(events: &[ChatEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Delta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn error_messages(events: &[ChatEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Error { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

/// Polls until `condition` holds (or fails the test after five seconds).
async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

// ---- opening ----

#[test]
fn opening_a_character_creates_a_conversation_with_the_greeting_then_resumes_it() {
    let f = fixture(vec![]);
    add_persona(&f.db, "Alex");
    let nova = add_character(&f.db, "Nova", Some("Hello {{user}}, I'm {{char}}."), &[]);

    let first = f.engine.open_character(nova.id, false).unwrap();
    assert_eq!(first.conversation.mode, ConversationMode::Roleplay);
    assert_eq!(first.conversation.title, "Chat with Nova");
    assert_eq!(first.messages.len(), 1);
    assert_eq!(first.messages[0].text, "Hello Alex, I'm Nova.");
    assert_eq!(first.messages[0].speaker_character_id, Some(nova.id));
    assert_eq!(first.messages[0].alternates.len(), 1, "the greeting is swipeable like any reply");
    assert_eq!(first.characters[0].id, nova.id);
    assert_eq!(first.persona.as_ref().unwrap().name, "Alex");
    assert_eq!(first.preset.name, "Roleplay (default)");

    let again = f.engine.open_character(nova.id, false).unwrap();
    assert_eq!(again.conversation.id, first.conversation.id);
    assert_eq!(again.messages.len(), 1);

    let fresh = f.engine.open_character(nova.id, true).unwrap();
    assert_ne!(fresh.conversation.id, first.conversation.id);
    assert_eq!(fresh.history.len(), 2, "both chats appear in this character's history");
}

#[test]
fn a_character_without_a_greeting_starts_empty_and_unknown_characters_are_rejected() {
    let f = fixture(vec![]);
    let quiet = add_character(&f.db, "Quiet", None, &[]);
    assert!(f.engine.open_character(quiet.id, false).unwrap().messages.is_empty());
    assert!(f.engine.open_character(Uuid::new_v4(), false).is_err());
}

#[test]
fn opening_the_assistant_resumes_the_latest_and_new_chat_starts_another() {
    let f = fixture(vec![]);
    let first = f.engine.open_assistant(false).unwrap();
    assert_eq!(first.conversation.mode, ConversationMode::Assistant);
    assert_eq!(first.conversation.title, "Assistant chat");
    assert_eq!(first.conversation.provider_id, "fake", "falls back to the first registered provider");
    assert_eq!(first.preset.name, "Assistant (default)");
    assert_eq!(first.providers, ["fake"]);

    assert_eq!(f.engine.open_assistant(false).unwrap().conversation.id, first.conversation.id);
    let second = f.engine.new_chat_like(first.conversation.id).unwrap();
    assert_ne!(second.conversation.id, first.conversation.id);
    assert_eq!(second.conversation.mode, ConversationMode::Assistant);
}

#[test]
fn new_chat_in_a_character_scope_stays_with_that_character() {
    let f = fixture(vec![]);
    let nova = add_character(&f.db, "Nova", Some("Hi"), &[]);
    let first = f.engine.open_character(nova.id, false).unwrap();
    let next = f.engine.new_chat_like(first.conversation.id).unwrap();
    assert_ne!(next.conversation.id, first.conversation.id);
    assert_eq!(next.conversation.character_id, Some(nova.id));
}

#[test]
fn new_conversations_default_to_the_last_selected_provider_and_model() {
    let f = fixture(vec![]);
    f.db.with(|c| {
        settings::set_last_selected_provider(c, "fake")?;
        settings::set_last_selected_model(c, "fake", "remembered-model")
    })
    .unwrap();
    assert_eq!(f.engine.open_assistant(false).unwrap().conversation.model, "remembered-model");

    // With nothing remembered, the first cached model is used.
    let g = fixture(vec![]);
    g.db.with(|c| settings::cache_models(c, "fake", &["m1".into(), "m2".into()])).unwrap();
    let session = g.engine.open_assistant(false).unwrap();
    assert_eq!(session.conversation.model, "m1");
    assert_eq!(session.models, ["m1", "m2"]);
}

// ---- sending ----

#[tokio::test]
async fn a_reply_streams_is_persisted_and_finalized_with_usage() {
    let f = fixture(vec![Script::Items(vec![chunk("Hel"), chunk("lo!"), finish("STOP")])]);
    let conv = ready_assistant(&f);

    let (result, events) = send(&f, conv, "  Hi there  ").await;
    result.unwrap();

    assert!(matches!(&events[0], ChatEvent::MessageAdded { message } if message.role == MessageRole::User && message.text == "Hi there"));
    assert!(matches!(&events[1], ChatEvent::MessageAdded { message } if message.role == MessageRole::Character && message.text.is_empty()));
    assert_eq!(streamed_text(&events), "Hello!");
    assert!(matches!(events.last(), Some(ChatEvent::Finished)));
    assert!(error_messages(&events).is_empty());

    let stored = messages(&f, conv);
    assert_eq!(stored.len(), 2);
    assert_eq!((stored[0].role, stored[0].text.as_str()), (MessageRole::User, "Hi there"));
    let reply = &stored[1];
    assert_eq!((reply.role, reply.text.as_str()), (MessageRole::Character, "Hello!"));
    assert_eq!(reply.alternates.len(), 1);
    assert_eq!(reply.usage.unwrap().total_tokens, Some(5));
    assert!(reply.raw_prompt_text.as_deref().unwrap().contains("=== User ===\nHi there"));

    // The final state is what the UI is told last.
    let updated = events.iter().rev().find_map(|e| match e {
        ChatEvent::MessageUpdated { message } => Some(message),
        _ => None,
    });
    assert_eq!(updated.unwrap().id, reply.id);

    let request = &f.provider.requests()[0];
    assert_eq!(request.model, "fake-model");
    assert_eq!(request.history.last().unwrap().text, "Hi there");
}

#[tokio::test]
async fn the_roleplay_prompt_includes_persona_character_and_earlier_messages() {
    let f = fixture(vec![Script::Items(vec![chunk("ok"), finish("STOP")])]);
    add_persona(&f.db, "Alex");
    let nova = add_character(&f.db, "Nova", Some("Welcome, {{user}}."), &[]);
    let conv = ready_character(&f, &nova);

    send(&f, conv, "Hello").await.0.unwrap();

    let request = &f.provider.requests()[0];
    assert!(request
        .system_instruction
        .as_deref()
        .unwrap()
        .contains("Write Nova's next reply in this fictional roleplay with Alex."));
    let history: Vec<_> = request.history.iter().map(|m| (m.role, m.text.as_str())).collect();
    assert_eq!(history, [(LlmRole::Model, "Welcome, Alex."), (LlmRole::User, "Hello")]);
}

#[tokio::test]
async fn sending_nothing_is_rejected_but_an_empty_box_after_a_user_message_retries_it() {
    let f = fixture(vec![
        Script::Items(vec![chunk("first"), finish("STOP")]),
        Script::Items(vec![chunk("second"), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);

    let (result, events) = send(&f, conv, "   ").await;
    assert!(result.is_err());
    assert_eq!(error_messages(&events), ["Nothing to send."]);
    assert!(matches!(events.last(), Some(ChatEvent::Finished)));

    send(&f, conv, "question").await.0.unwrap();
    // Delete the reply, leaving the user message unanswered, then "send" an empty box.
    let reply = messages(&f, conv)[1].id;
    f.engine.delete_message(reply).unwrap();
    let (result, events) = send(&f, conv, "").await;
    result.unwrap();

    let stored = messages(&f, conv);
    assert_eq!(stored.len(), 2, "the user message is reused, not duplicated");
    assert_eq!(stored[1].text, "second");
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::MessageAdded { message } if message.role == MessageRole::User)));
}

#[tokio::test]
async fn a_conversation_without_a_model_says_so_instead_of_calling_the_provider() {
    let f = fixture(vec![]);
    let conv = f.engine.open_assistant(false).unwrap().conversation.id; // model is empty
    let (result, events) = send(&f, conv, "hi").await;
    assert!(result.is_err());
    assert!(error_messages(&events)[0].contains("No model selected"));
    assert!(f.provider.requests().is_empty());
    assert!(messages(&f, conv).is_empty(), "nothing is saved for a send that never started");
}

#[tokio::test]
async fn attachments_are_saved_on_the_user_message_and_sent_with_it() {
    let f = fixture(vec![Script::Items(vec![chunk("nice"), finish("STOP")])]);
    let conv = ready_assistant(&f);
    let sink = Arc::new(VecSink::default());

    f.engine.send_message(conv, "look".into(), vec!["C:/pics/cat.png".into()], sink).await.unwrap();

    let user = &messages(&f, conv)[0];
    assert_eq!(user.attachments.len(), 1);
    assert_eq!(user.attachments[0].mime_type, "image/png");
    let sent = f.provider.requests()[0].history[0].attachments.clone().unwrap();
    assert_eq!(sent[0].image_path, "C:/pics/cat.png");
}

#[tokio::test]
async fn an_emotion_tag_is_stripped_and_recorded_for_characters_with_an_expression_pack() {
    let f = fixture(vec![Script::Items(vec![chunk("So happy to see you! "), chunk("[emotion: Happy]"), finish("STOP")])]);
    let nova = add_character(&f.db, "Nova", None, &["happy", "sad"]);
    let conv = ready_character(&f, &nova);

    send(&f, conv, "hi").await.0.unwrap();

    let reply = &messages(&f, conv)[1];
    assert_eq!(reply.text, "So happy to see you!");
    assert_eq!(reply.detected_emotion.as_deref(), Some("happy"));
    let system = f.provider.requests()[0].system_instruction.clone().unwrap();
    assert!(system.contains("chosen from exactly this list: happy, sad"), "{system}");
}

#[tokio::test]
async fn no_emotion_instruction_is_sent_without_an_expression_pack() {
    let f = fixture(vec![Script::Items(vec![chunk("hi [emotion: happy]"), finish("STOP")])]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = ready_character(&f, &nova);

    send(&f, conv, "hi").await.0.unwrap();

    assert!(!f.provider.requests()[0].system_instruction.clone().unwrap().contains("[emotion:"));
    assert_eq!(messages(&f, conv)[1].text, "hi [emotion: happy]", "left alone when the feature is off");
}

#[tokio::test]
async fn the_world_clock_advances_by_the_time_skip_tag_and_the_tag_is_hidden() {
    let f = fixture(vec![Script::Items(vec![chunk("They walked for a while. [time-skip: 3 hours]"), finish("STOP")])]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = ready_character(&f, &nova);
    let start = NaiveDate::from_ymd_opt(2025, 6, 14).unwrap().and_hms_opt(9, 0, 0).unwrap();
    f.db.with(|c| {
        conversations::update_world_state(c, conv, Some(&WorldState { fictional_date_time: start, ..WorldState::create_default() }))
    })
    .unwrap();

    let (result, events) = send(&f, conv, "go").await;
    result.unwrap();

    assert_eq!(messages(&f, conv)[1].text, "They walked for a while.");
    let stored = f.db.with(|c| conversations::get(c, conv)).unwrap().unwrap().world_state.unwrap();
    assert_eq!(stored.fictional_date_time, start + chrono::Duration::hours(3));
    assert!(events.iter().any(|e| matches!(e, ChatEvent::WorldStateChanged { world_state, .. } if world_state.fictional_date_time == stored.fictional_date_time)));
    assert!(f.provider.requests()[0].system_instruction.clone().unwrap().contains("[time-skip:"));
}

#[tokio::test]
async fn without_a_world_clock_no_time_skip_instruction_is_sent() {
    let f = fixture(vec![Script::Items(vec![chunk("ok"), finish("STOP")])]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = ready_character(&f, &nova);
    send(&f, conv, "hi").await.0.unwrap();
    assert!(!f.provider.requests()[0].system_instruction.clone().unwrap().contains("time-skip"));
}

// ---- failure, blocking, stopping ----

#[tokio::test]
async fn a_failure_mid_stream_keeps_the_partial_reply_and_reports_the_error() {
    let f = fixture(vec![Script::Items(vec![chunk("Partial answ"), Err(ProviderError::network("connection reset"))])]);
    let conv = ready_assistant(&f);

    let (result, events) = send(&f, conv, "hi").await;
    result.unwrap(); // the send itself worked; the failure is reported as an event

    assert_eq!(messages(&f, conv)[1].text, "Partial answ");
    assert_eq!(error_messages(&events), ["Network error: connection reset"]);
    assert!(matches!(events.last(), Some(ChatEvent::Finished)));
}

#[tokio::test]
async fn a_failure_before_any_text_removes_the_empty_bubble_and_saves_nothing() {
    let f = fixture(vec![Script::FailToStart(ProviderError::auth("bad key"))]);
    let conv = ready_assistant(&f);

    let (_, events) = send(&f, conv, "hi").await;

    assert!(events.iter().any(|e| matches!(e, ChatEvent::MessageRemoved { .. })));
    assert_eq!(messages(&f, conv).len(), 1, "only the user's message remains");
    assert_eq!(error_messages(&events), ["Authentication failed - check your API key in Settings."]);
}

#[tokio::test]
async fn a_blocked_reply_becomes_a_content_blocked_error_but_keeps_the_text_so_far() {
    let f = fixture(vec![Script::Items(vec![chunk("Once upon"), finish("SAFETY")])]);
    let conv = ready_assistant(&f);

    let (_, events) = send(&f, conv, "hi").await;

    assert_eq!(messages(&f, conv)[1].text, "Once upon");
    let errors: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Error { kind, message } => Some((*kind, message.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, Some(ProviderErrorKind::ContentBlocked));
    assert!(errors[0].1.contains("blocked the reply (reason: SAFETY)"));
}

#[tokio::test]
async fn an_unknown_provider_is_reported_not_a_panic() {
    let f = fixture(vec![]);
    let conv = ready_assistant(&f);
    f.engine.set_model(conv, "ghost", "m").unwrap();
    let (_, events) = send(&f, conv, "hi").await;
    assert!(error_messages(&events)[0].contains("ghost"));
}

#[tokio::test]
async fn stopping_keeps_what_arrived_and_is_not_reported_as_an_error() {
    let f = fixture(vec![Script::Hang(vec![LlmResponseChunk::text("Half a sent"), LlmResponseChunk::text("ence")])]);
    let conv = ready_assistant(&f);
    let sink = Arc::new(VecSink::default());

    let engine = f.engine.clone();
    let task_sink: Arc<dyn EventSink> = sink.clone();
    let task = tokio::spawn(async move { engine.send_message(conv, "talk".into(), vec![], task_sink).await });

    // Wait until the text has actually been streamed to the UI, then press Stop.
    wait_until("a streamed delta", || sink.events().iter().any(|e| matches!(e, ChatEvent::Delta { .. }))).await;
    assert!(f.engine.is_generating(conv));
    f.engine.stop(conv);
    tokio::time::timeout(Duration::from_secs(5), task).await.expect("stops promptly").unwrap().unwrap();

    let events = sink.events();
    assert_eq!(messages(&f, conv)[1].text, "Half a sentence");
    assert!(error_messages(&events).is_empty());
    assert!(matches!(events.last(), Some(ChatEvent::Finished)));
    assert!(!f.engine.is_generating(conv));
}

#[tokio::test]
async fn stopping_before_anything_arrives_leaves_no_empty_message() {
    let f = fixture(vec![Script::Hang(vec![])]);
    let conv = ready_assistant(&f);
    let sink = Arc::new(VecSink::default());

    let engine = f.engine.clone();
    let task_sink: Arc<dyn EventSink> = sink.clone();
    let task = tokio::spawn(async move { engine.send_message(conv, "talk".into(), vec![], task_sink).await });
    wait_until("the request to start", || f.engine.is_generating(conv) && !f.provider.requests().is_empty()).await;
    f.engine.stop(conv);
    task.await.unwrap().unwrap();

    assert_eq!(messages(&f, conv).len(), 1);
    assert!(sink.events().iter().any(|e| matches!(e, ChatEvent::MessageRemoved { .. })));
}

#[tokio::test]
async fn only_one_generation_runs_per_conversation() {
    let f = fixture(vec![Script::Hang(vec![LlmResponseChunk::text("busy")])]);
    let conv = ready_assistant(&f);
    let sink = Arc::new(VecSink::default());

    let engine = f.engine.clone();
    let task_sink: Arc<dyn EventSink> = sink.clone();
    let task = tokio::spawn(async move { engine.send_message(conv, "one".into(), vec![], task_sink).await });
    wait_until("the first generation", || f.engine.is_generating(conv)).await;

    let (second, _) = send(&f, conv, "two").await;
    assert!(second.unwrap_err().to_string().contains("already being generated"));

    f.engine.stop(conv);
    task.await.unwrap().unwrap();
}

// ---- swiping, editing, deleting ----

#[tokio::test]
async fn swiping_right_past_the_last_alternate_generates_a_new_one_and_left_goes_back() {
    let f = fixture(vec![
        Script::Items(vec![chunk("first"), finish("STOP")]),
        Script::Items(vec![chunk("second"), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    send(&f, conv, "hi").await.0.unwrap();
    let reply_id = messages(&f, conv)[1].id;

    let sink = Arc::new(VecSink::default());
    f.engine.swipe_next(conv, reply_id, sink.clone()).await.unwrap();
    let events = sink.events();
    assert!(matches!(events[0], ChatEvent::Regenerating { message_id } if message_id == reply_id));
    assert_eq!(streamed_text(&events), "second");

    let reply = f.db.with(|c| conversations::get_message(c, reply_id)).unwrap().unwrap();
    assert_eq!(reply.alternates.iter().map(|a| a.text.as_str()).collect::<Vec<_>>(), ["first", "second"]);
    assert_eq!((reply.text.as_str(), reply.active_alternate_index), ("second", 1));

    // The regeneration was prompted from the history *before* the reply, not including it.
    let request = &f.provider.requests()[1];
    assert_eq!(request.history.len(), 1);
    assert_eq!(request.history[0].text, "hi");

    let back = f.engine.swipe_previous(reply_id).unwrap();
    assert_eq!((back.text.as_str(), back.active_alternate_index), ("first", 0));
    assert_eq!(f.engine.swipe_previous(reply_id).unwrap().active_alternate_index, 0, "clamped at the first");

    // Swiping right now lands on the existing alternate without calling the model again.
    let sink = Arc::new(VecSink::default());
    f.engine.swipe_next(conv, reply_id, sink.clone()).await.unwrap();
    assert_eq!(f.provider.requests().len(), 2);
    assert!(matches!(&sink.events()[0], ChatEvent::MessageUpdated { message } if message.text == "second"));
}

#[tokio::test]
async fn a_failed_regeneration_leaves_the_original_reply_showing() {
    let f = fixture(vec![
        Script::Items(vec![chunk("original"), finish("STOP")]),
        Script::FailToStart(ProviderError::new("busy", ProviderErrorKind::RateLimit)),
    ]);
    let conv = ready_roleplay(&f);
    send(&f, conv, "hi").await.0.unwrap();
    let reply_id = messages(&f, conv)[1].id;

    let sink = Arc::new(VecSink::default());
    f.engine.swipe_next(conv, reply_id, sink.clone()).await.unwrap();

    let events = sink.events();
    assert_eq!(error_messages(&events), ["Rate limited - please wait a moment and try again."]);
    let restored = events.iter().rev().find_map(|e| match e {
        ChatEvent::MessageUpdated { message } => Some(message),
        _ => None,
    });
    assert_eq!(restored.unwrap().text, "original");
    let stored = &messages(&f, conv)[1];
    assert_eq!((stored.text.as_str(), stored.alternates.len()), ("original", 1));
}

#[tokio::test]
async fn stopping_a_regeneration_keeps_the_partial_as_its_own_alternate() {
    let f = fixture(vec![
        Script::Items(vec![chunk("original"), finish("STOP")]),
        Script::Hang(vec![LlmResponseChunk::text("partial regen")]),
    ]);
    let conv = ready_roleplay(&f);
    send(&f, conv, "hi").await.0.unwrap();
    let reply_id = messages(&f, conv)[1].id;

    let sink = Arc::new(VecSink::default());
    let engine = f.engine.clone();
    let task_sink: Arc<dyn EventSink> = sink.clone();
    let task = tokio::spawn(async move { engine.swipe_next(conv, reply_id, task_sink).await });
    wait_until("the regeneration to stream", || sink.events().iter().any(|e| matches!(e, ChatEvent::Delta { .. }))).await;
    f.engine.stop(conv);
    task.await.unwrap().unwrap();

    let stored = &messages(&f, conv)[1];
    assert_eq!(stored.alternates.iter().map(|a| a.text.as_str()).collect::<Vec<_>>(), ["original", "partial regen"]);
}

#[tokio::test]
async fn only_character_replies_can_be_regenerated() {
    let f = fixture(vec![Script::Items(vec![chunk("x"), finish("STOP")])]);
    let conv = ready_roleplay(&f);
    send(&f, conv, "hi").await.0.unwrap();
    let user_id = messages(&f, conv)[0].id;

    let sink = Arc::new(VecSink::default());
    assert!(f.engine.swipe_next(conv, user_id, sink).await.is_err());
}

#[tokio::test]
async fn editing_and_deleting_messages_persist() {
    let f = fixture(vec![Script::Items(vec![chunk("draft"), finish("STOP")])]);
    let conv = ready_roleplay(&f);
    send(&f, conv, "hi").await.0.unwrap();
    let reply_id = messages(&f, conv)[1].id;

    let edited = f.engine.edit_message(reply_id, "final wording").unwrap();
    assert_eq!(edited.text, "final wording");
    assert_eq!(edited.alternates[0].text, "final wording", "the active alternate follows the edit");
    assert_eq!(messages(&f, conv)[1].text, "final wording");

    f.engine.delete_message(reply_id).unwrap();
    assert_eq!(messages(&f, conv).len(), 1);
    assert!(f.engine.edit_message(reply_id, "ghost").is_err());
}

// ---- conversation settings ----

#[test]
fn choosing_a_model_is_remembered_for_new_chats() {
    let f = fixture(vec![]);
    let session = f.engine.open_assistant(false).unwrap();
    let conv = f.engine.set_model(session.conversation.id, "fake", "picked-model").unwrap();
    assert_eq!((conv.provider_id.as_str(), conv.model.as_str()), ("fake", "picked-model"));
    assert_eq!(f.engine.open_assistant(true).unwrap().conversation.model, "picked-model");
}

#[test]
fn per_conversation_settings_are_saved() {
    let f = fixture(vec![]);
    let alex = add_persona(&f.db, "Alex");
    let session = f.engine.open_assistant(false).unwrap();
    let id = session.conversation.id;

    let preset = session.presets.iter().find(|p| p.name == "Roleplay (default)").unwrap().id;
    assert_eq!(f.engine.set_preset(id, preset).unwrap().prompt_preset_id, Some(preset));
    assert_eq!(f.engine.set_persona(id, Some(alex.id)).unwrap().persona_id, Some(alex.id));
    assert_eq!(f.engine.set_persona(id, None).unwrap().persona_id, None);
    assert_eq!(
        f.engine.set_assistant_prompt(id, "Be terse.").unwrap().assistant_system_prompt.as_deref(),
        Some("Be terse.")
    );
    assert_eq!(f.engine.set_search_override(id, Some(true)).unwrap().enable_google_search, Some(true));

    let reloaded = f.engine.resume(id).unwrap();
    assert_eq!(reloaded.preset.id, preset);
    assert_eq!(reloaded.conversation.assistant_system_prompt.as_deref(), Some("Be terse."));
}

#[tokio::test]
async fn the_assistant_prompt_and_search_override_reach_the_request() {
    let f = fixture(vec![Script::Items(vec![chunk("ok"), finish("STOP")])]);
    let conv = ready_assistant(&f);
    f.engine.set_assistant_prompt(conv, "Answer in French.").unwrap();
    f.engine.set_search_override(conv, Some(true)).unwrap();

    send(&f, conv, "hi").await.0.unwrap();

    let request = &f.provider.requests()[0];
    assert!(request.system_instruction.clone().unwrap().contains("Answer in French."));
    assert_eq!(request.enable_web_search, Some(true));
}

#[test]
fn deleting_a_conversation_removes_it_and_its_messages() {
    let f = fixture(vec![]);
    let nova = add_character(&f.db, "Nova", Some("Hi"), &[]);
    let conv = f.engine.open_character(nova.id, false).unwrap().conversation.id;
    f.engine.delete_conversation(conv).unwrap();
    assert!(messages(&f, conv).is_empty());
    assert!(f.engine.resume(conv).is_err());
}

// ---- titles ----

#[tokio::test]
async fn the_first_assistant_exchange_gets_an_auto_generated_title() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Paris is the capital."), finish("STOP")]),
        Script::Items(vec![chunk("\"Capital of France.\""), finish("STOP")]),
    ]);
    let conv = ready_assistant(&f);
    let sink = Arc::new(VecSink::default());
    f.engine.send_message(conv, "What's the capital of France?".into(), vec![], sink.clone()).await.unwrap();

    wait_until("a generated title", || sink.events().iter().any(|e| matches!(e, ChatEvent::TitleChanged { .. }))).await;

    let title = f.db.with(|c| conversations::get(c, conv)).unwrap().unwrap().title;
    assert_eq!(title, "Capital of France");
    let prompt = &f.provider.requests()[1].history[0].text;
    assert!(prompt.contains("User: What's the capital of France?"));
    assert!(prompt.contains("Assistant: Paris is the capital."));
}

#[tokio::test]
async fn later_exchanges_and_roleplays_do_not_ask_for_a_title() {
    let f = fixture(vec![
        Script::Items(vec![chunk("one"), finish("STOP")]),
        Script::Items(vec![chunk("title?"), finish("STOP")]), // consumed by the first exchange's title request
        Script::Items(vec![chunk("two"), finish("STOP")]),
    ]);
    let conv = ready_assistant(&f);
    send(&f, conv, "a").await.0.unwrap();
    wait_until("the first exchange's title request", || f.provider.requests().len() == 2).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (_, events) = send(&f, conv, "b").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.provider.requests().len(), 3, "the second exchange makes exactly one request");
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::TitleChanged { .. })));

    let g = fixture(vec![Script::Items(vec![chunk("hi"), finish("STOP")])]);
    let nova = add_character(&g.db, "Nova", None, &[]);
    let rp = ready_character(&g, &nova);
    send(&g, rp, "hey").await.0.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(g.provider.requests().len(), 1, "no title request in roleplay");
}

// ---- group chats ----

fn add_group(f: &Fixture, mode: GroupActivationMode, members: &[&Character]) -> GroupChat {
    let group = GroupChat {
        name: "The Crew".into(),
        character_ids: members.iter().map(|c| c.id).collect(),
        activation_mode: mode,
        ..Default::default()
    };
    f.db.with(|c| group_chats::upsert(c, &group)).unwrap();
    group
}

fn ready_group(f: &Fixture, group: &GroupChat) -> Uuid {
    let session = f.engine.open_group(group.id, false).unwrap();
    f.engine.set_model(session.conversation.id, "fake", "fake-model").unwrap();
    session.conversation.id
}

fn speakers(f: &Fixture, conversation: Uuid) -> Vec<Option<Uuid>> {
    messages(f, conversation).iter().filter(|m| m.role == MessageRole::Character).map(|m| m.speaker_character_id).collect()
}

#[test]
fn opening_a_group_creates_a_roleplay_with_every_member_and_no_greeting() {
    let f = fixture(vec![]);
    let (alice, bob) = (add_character(&f.db, "Alice", Some("Hi!"), &[]), add_character(&f.db, "Bob", None, &[]));
    let group = add_group(&f, GroupActivationMode::Manual, &[&alice, &bob]);

    let session = f.engine.open_group(group.id, false).unwrap();

    assert_eq!(session.conversation.mode, ConversationMode::Roleplay);
    assert_eq!(session.conversation.group_chat_id, Some(group.id));
    assert_eq!(session.conversation.title, "The Crew");
    assert_eq!(session.group.as_ref().unwrap().activation_mode, GroupActivationMode::Manual);
    assert_eq!(session.characters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Alice", "Bob"]);
    assert!(session.messages.is_empty(), "groups don't open with anyone's greeting");

    assert_eq!(f.engine.open_group(group.id, false).unwrap().conversation.id, session.conversation.id);
    let fresh = f.engine.new_chat_like(session.conversation.id).unwrap();
    assert_ne!(fresh.conversation.id, session.conversation.id);
    assert_eq!(fresh.conversation.group_chat_id, Some(group.id));
    assert_eq!(fresh.history.len(), 2);
    assert!(f.engine.open_group(Uuid::new_v4(), false).is_err());
}

#[tokio::test]
async fn manual_groups_wait_for_the_user_to_pick_who_replies() {
    let f = fixture(vec![Script::Items(vec![chunk("Bob here."), finish("STOP")])]);
    let (alice, bob) = (add_character(&f.db, "Alice", None, &[]), add_character(&f.db, "Bob", None, &[]));
    let group = add_group(&f, GroupActivationMode::Manual, &[&alice, &bob]);
    let conv = ready_group(&f, &group);

    let (result, events) = send(&f, conv, "Hello both").await;
    result.unwrap();

    assert!(f.provider.requests().is_empty(), "nobody is asked to reply yet");
    assert!(error_messages(&events).is_empty());
    assert_eq!(messages(&f, conv).len(), 1);

    let sink = Arc::new(VecSink::default());
    f.engine.pick_speaker(conv, bob.id, sink.clone()).await.unwrap();

    assert_eq!(speakers(&f, conv), [Some(bob.id)]);
    assert_eq!(messages(&f, conv)[1].text, "Bob here.");
    assert!(matches!(sink.events().last(), Some(ChatEvent::Finished)));
}

#[tokio::test]
async fn round_robin_groups_hand_the_floor_to_the_next_member_each_turn() {
    let scripts = (0..3).map(|i| Script::Items(vec![chunk(&format!("reply {i}")), finish("STOP")])).collect();
    let f = fixture(scripts);
    let members: Vec<Character> = ["Alice", "Bob", "Cara"].iter().map(|n| add_character(&f.db, n, None, &[])).collect();
    let group = add_group(&f, GroupActivationMode::RoundRobin, &members.iter().collect::<Vec<_>>());
    let conv = ready_group(&f, &group);

    for turn in ["one", "two", "three"] {
        send(&f, conv, turn).await.0.unwrap();
    }

    let order: Vec<_> = speakers(&f, conv).into_iter().map(Option::unwrap).collect();
    assert_eq!(order, [members[0].id, members[1].id, members[2].id]);
}

#[tokio::test]
async fn trigger_word_groups_reply_in_the_order_names_are_mentioned_and_hear_each_other() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Cara speaking."), finish("STOP")]),
        Script::Items(vec![chunk("Alice speaking."), finish("STOP")]),
    ]);
    let (alice, cara) = (add_character(&f.db, "Alice", None, &[]), add_character(&f.db, "Cara", None, &[]));
    let group = add_group(&f, GroupActivationMode::TriggerWord, &[&alice, &cara]);
    let conv = ready_group(&f, &group);

    send(&f, conv, "Cara, then Alice - what do you two think?").await.0.unwrap();

    assert_eq!(speakers(&f, conv), [Some(cara.id), Some(alice.id)]);
    // The second speaker's prompt already contains the first one's reply, labelled with her name.
    let second = &f.provider.requests()[1];
    assert_eq!(second.history.last().unwrap().text, "Cara: Cara speaking.");
    assert_eq!(second.history.last().unwrap().role, LlmRole::Model);
}

#[tokio::test]
async fn a_failed_voice_ends_the_round_for_everyone_after_it() {
    let f = fixture(vec![Script::FailToStart(ProviderError::new("busy", ProviderErrorKind::RateLimit))]);
    let (alice, bob) = (add_character(&f.db, "Alice", None, &[]), add_character(&f.db, "Bob", None, &[]));
    let group = add_group(&f, GroupActivationMode::TriggerWord, &[&alice, &bob]);
    let conv = ready_group(&f, &group);

    let (_, events) = send(&f, conv, "Alice and Bob?").await;

    assert_eq!(f.provider.requests().len(), 1, "Bob is not asked once Alice's turn failed");
    assert_eq!(error_messages(&events).len(), 1);
}

#[tokio::test]
async fn stopping_mid_round_skips_the_remaining_speakers() {
    let f = fixture(vec![Script::Hang(vec![LlmResponseChunk::text("Alice starts")])]);
    let (alice, bob) = (add_character(&f.db, "Alice", None, &[]), add_character(&f.db, "Bob", None, &[]));
    let group = add_group(&f, GroupActivationMode::TriggerWord, &[&alice, &bob]);
    let conv = ready_group(&f, &group);
    let sink = Arc::new(VecSink::default());

    let engine = f.engine.clone();
    let task_sink: Arc<dyn EventSink> = sink.clone();
    let task = tokio::spawn(async move { engine.send_message(conv, "Alice, Bob!".into(), vec![], task_sink).await });
    wait_until("Alice to start", || sink.events().iter().any(|e| matches!(e, ChatEvent::Delta { .. }))).await;
    f.engine.stop(conv);
    task.await.unwrap().unwrap();

    assert_eq!(f.provider.requests().len(), 1);
    assert_eq!(speakers(&f, conv), [Some(alice.id)]);
}

#[tokio::test]
async fn picking_a_speaker_is_validated() {
    let f = fixture(vec![]);
    let (alice, outsider) = (add_character(&f.db, "Alice", None, &[]), add_character(&f.db, "Zed", None, &[]));
    let group = add_group(&f, GroupActivationMode::Manual, &[&alice]);
    let conv = ready_group(&f, &group);

    let sink = Arc::new(VecSink::default());
    let err = f.engine.pick_speaker(conv, outsider.id, sink.clone()).await.unwrap_err();
    assert!(err.to_string().contains("isn't in this group"));

    // A plain character chat is not a group.
    let solo = ready_character(&f, &alice);
    let err = f.engine.pick_speaker(solo, alice.id, sink).await.unwrap_err();
    assert!(err.to_string().contains("isn't a group chat"));
    assert!(f.provider.requests().is_empty());
}

#[tokio::test]
async fn deleting_a_group_keeps_its_conversations_usable() {
    let f = fixture(vec![Script::Items(vec![chunk("Still here."), finish("STOP")])]);
    let alice = add_character(&f.db, "Alice", None, &[]);
    let group = add_group(&f, GroupActivationMode::Manual, &[&alice]);
    let conv = ready_group(&f, &group);
    f.db.with(|c| group_chats::delete(c, group.id)).unwrap();

    // The chat loses its cast (the group reference is cleared) but keeps its history and still works.
    let session = f.engine.resume(conv).unwrap();
    assert!(session.group.is_none());
    assert!(session.characters.is_empty());

    send(&f, conv, "anyone?").await.0.unwrap();
    assert_eq!(messages(&f, conv).len(), 2);
    assert_eq!(messages(&f, conv)[1].text, "Still here.");
}

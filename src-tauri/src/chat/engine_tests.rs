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
    Persona, WeatherCondition, WorldState,
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
    /// What `context_window` reports (`None`: unknown, like most providers).
    window: Mutex<Option<i32>>,
}

impl FakeProvider {
    fn new(scripts: Vec<Script>) -> Arc<Self> {
        Arc::new(Self { scripts: Mutex::new(scripts.into()), requests: Mutex::new(vec![]), window: Mutex::new(None) })
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

    async fn context_window(&self, _model: &str) -> Option<i32> {
        *self.window.lock().unwrap()
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
    engine.set_extraction(false); // tests opt in, so scripted providers see only the requests they expect
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

/// Removes a roleplay's in-story clock, so a test can assert on the exact prompt (the clock adds a
/// trailing world-state turn).
fn no_clock(f: &Fixture, conversation: Uuid) {
    f.db.with(|c| conversations::update_world_state(c, conversation, None)).unwrap();
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
fn each_mode_uses_its_own_persona_and_opening_that_mode_selects_it() {
    let f = fixture(vec![]);
    let player = Persona {
        name: "Player".into(),
        description: "A daring adventurer.".into(),
        is_default: true,
        ..Default::default()
    };
    let coder = Persona { name: "Coder".into(), description: "A terse engineer.".into(), ..Default::default() };
    f.db
        .with(|c| {
            persona_repo::upsert(c, &player)?;
            persona_repo::upsert(c, &coder)?;
            settings::save_default_personas(
                c,
                &settings::DefaultPersonas { roleplay_persona_id: Some(player.id), assistant_persona_id: Some(coder.id) },
            )
        })
        .unwrap();

    let nova = add_character(&f.db, "Nova", Some("Hello {{user}}."), &[]);
    let roleplay = f.engine.open_character(nova.id, false).unwrap();
    assert_eq!(roleplay.conversation.persona_id, Some(player.id));
    assert_eq!(roleplay.messages[0].text, "Hello Player.");

    let assistant = f.engine.open_assistant(false).unwrap();
    assert_eq!(assistant.conversation.mode, ConversationMode::Assistant);
    assert_eq!(assistant.conversation.character_id, None, "a persona is not a character");
    assert_eq!(assistant.conversation.persona_id, Some(coder.id));
    assert_eq!(assistant.persona.as_ref().unwrap().description, "A terse engineer.");

    // The nav selects the mode's persona even after the header was switched.
    f.engine.set_persona(roleplay.conversation.id, Some(coder.id)).unwrap();
    let again = f.engine.open_last_roleplay().unwrap().unwrap();
    assert_eq!(again.conversation.id, roleplay.conversation.id);
    assert_eq!(again.conversation.persona_id, Some(player.id));

    f.engine.set_persona(assistant.conversation.id, Some(player.id)).unwrap();
    let assistant_again = f.engine.open_assistant(false).unwrap();
    assert_eq!(assistant_again.conversation.id, assistant.conversation.id);
    assert_eq!(assistant_again.conversation.persona_id, Some(coder.id));

    let fresh = f.engine.new_chat_like(assistant.conversation.id).unwrap();
    assert_eq!(fresh.conversation.mode, ConversationMode::Assistant);
    assert_eq!(fresh.conversation.persona_id, Some(coder.id));
    assert_eq!(fresh.conversation.character_id, None);
}

#[test]
fn without_a_mode_persona_the_global_default_is_used_and_a_per_chat_choice_is_kept() {
    let f = fixture(vec![]);
    let global = add_persona(&f.db, "Alex");
    let nova = add_character(&f.db, "Nova", None, &[]);
    let roleplay = f.engine.open_character(nova.id, false).unwrap();
    assert_eq!(roleplay.conversation.persona_id, Some(global.id));

    let other = Persona { name: "Other".into(), ..Default::default() };
    f.db.with(|c| persona_repo::upsert(c, &other)).unwrap();
    f.engine.set_persona(roleplay.conversation.id, Some(other.id)).unwrap();
    assert_eq!(f.engine.open_last_roleplay().unwrap().unwrap().conversation.persona_id, Some(other.id));
    assert_eq!(f.engine.open_character(nova.id, false).unwrap().conversation.persona_id, Some(other.id));
}

#[test]
fn a_deleted_mode_persona_falls_back_to_the_global_default() {
    let f = fixture(vec![]);
    let global = add_persona(&f.db, "Alex");
    let gone = Persona { name: "Gone".into(), ..Default::default() };
    f.db
        .with(|c| {
            persona_repo::upsert(c, &gone)?;
            settings::save_default_personas(
                c,
                &settings::DefaultPersonas { roleplay_persona_id: None, assistant_persona_id: Some(gone.id) },
            )?;
            persona_repo::delete(c, gone.id)
        })
        .unwrap();
    assert_eq!(f.engine.open_assistant(false).unwrap().conversation.persona_id, Some(global.id));
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
    no_clock(&f, conv);

    send(&f, conv, "Hello").await.0.unwrap();

    let request = &f.provider.requests()[0];
    let system = request.system_instruction.as_deref().unwrap();
    assert!(system.contains("Write Nova's next reply in this fictional roleplay with Alex."), "{system}");
    let history: Vec<_> = request.history.iter().map(|m| (m.role, m.text.as_str())).collect();
    assert_eq!(history, [(LlmRole::Model, "Welcome, Alex."), (LlmRole::User, "Hello")]);
}

#[tokio::test]
async fn every_roleplay_reply_says_who_is_who_even_with_an_edited_preset() {
    let f = fixture(vec![Script::Items(vec![chunk("ok"), finish("STOP")])]);
    add_persona(&f.db, "Alex");
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = ready_character(&f, &nova);
    let mut preset = f.engine.resume(conv).unwrap().preset;
    preset.blocks.retain(|b| b.block_type != crate::models::PromptBlockType::SystemPrompt);
    f.db.with(|c| crate::repo::prompt_presets::upsert(c, &preset)).unwrap();

    send(&f, conv, "*I hand you the map.*").await.0.unwrap();

    let system = f.provider.requests()[0].system_instruction.clone().unwrap();
    assert!(system.contains("Roles: the user plays Alex; you play Nova."), "{system}");
    assert!(system.contains("never write what Alex says, does, thinks or feels"), "{system}");
}

#[tokio::test]
async fn assistant_chats_get_no_roleplay_rules() {
    let f = fixture(vec![Script::Items(vec![chunk("ok"), finish("STOP")])]);
    let conv = ready_assistant(&f);
    send(&f, conv, "hi").await.0.unwrap();
    assert!(!f.provider.requests()[0].system_instruction.clone().unwrap_or_default().contains("Roles:"));
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
    no_clock(&f, conv);
    send(&f, conv, "hi").await.0.unwrap();
    assert!(!f.provider.requests()[0].system_instruction.clone().unwrap().contains("time-skip"));
}

fn world(f: &Fixture, conv: Uuid) -> Option<WorldState> {
    f.db.with(|c| conversations::get(c, conv)).unwrap().unwrap().world_state
}

#[test]
fn opening_a_roleplay_gives_it_the_default_clock_but_assistants_get_none() {
    let f = fixture(vec![]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let session = f.engine.open_character(nova.id, false).unwrap();
    assert_eq!(session.conversation.world_state, Some(WorldState::create_default()));

    // An edited clock survives reopening; a legacy roleplay without one gets the default.
    let edited = WorldState { weather: WeatherCondition::Fog, ..WorldState::create_default() };
    f.engine.set_world_state(session.conversation.id, edited.clone()).unwrap();
    assert_eq!(f.engine.resume(session.conversation.id).unwrap().conversation.world_state, Some(edited));
    f.db.with(|c| conversations::update_world_state(c, session.conversation.id, None)).unwrap();
    assert!(f.engine.resume(session.conversation.id).unwrap().conversation.world_state.is_some());

    assert!(f.engine.open_assistant(false).unwrap().conversation.world_state.is_none());
}

#[test]
fn editing_the_clock_saves_it_cleaned_up() {
    let f = fixture(vec![]);
    let conv = ready_roleplay(&f);
    let date = NaiveDate::from_ymd_opt(1999, 12, 31).unwrap().and_hms_opt(23, 30, 0).unwrap();
    let saved = f
        .engine
        .set_world_state(
            conv,
            WorldState {
                fictional_date_time: date,
                weather: WeatherCondition::Snow,
                weather_notes: Some("   ".into()),
                default_advance_minutes_per_message: 0,
                auto_drift_weather_enabled: true,
            },
        )
        .unwrap();
    assert_eq!(saved.weather_notes, None, "blank notes are dropped");
    assert_eq!(saved.default_advance_minutes_per_message, 1, "the tick is at least a minute");
    assert_eq!(world(&f, conv), Some(saved));
}

#[test]
fn only_roleplays_have_a_clock_to_edit() {
    let f = fixture(vec![]);
    let assistant = ready_assistant(&f);
    let err = f.engine.set_world_state(assistant, WorldState::create_default()).unwrap_err();
    assert!(err.to_string().contains("Only roleplays"));
    assert!(world(&f, assistant).is_none());
}

#[tokio::test]
async fn the_clock_cannot_be_edited_mid_reply_and_replies_continue_from_the_edited_time() {
    let f = fixture(vec![
        Script::Hang(vec![LlmResponseChunk::text("busy")]),
        Script::Items(vec![chunk("Later."), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    let sink = Arc::new(VecSink::default());
    let engine = f.engine.clone();
    let task_sink: Arc<dyn EventSink> = sink.clone();
    let task = tokio::spawn(async move { engine.send_message(conv, "one".into(), vec![], task_sink).await });
    wait_until("the reply to start", || f.engine.is_generating(conv)).await;
    let err = f.engine.set_world_state(conv, WorldState::create_default()).unwrap_err();
    assert!(err.to_string().contains("Wait for the reply"));
    f.engine.stop(conv);
    task.await.unwrap().unwrap();

    let date = NaiveDate::from_ymd_opt(2030, 5, 5).unwrap().and_hms_opt(12, 0, 0).unwrap();
    f.engine.set_world_state(conv, WorldState { fictional_date_time: date, ..WorldState::create_default() }).unwrap();
    send(&f, conv, "two").await.0.unwrap();
    assert_eq!(world(&f, conv).unwrap().fictional_date_time, date + chrono::Duration::minutes(5));
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
    no_clock(&f, conv);
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

#[tokio::test]
async fn the_assistant_persona_reaches_the_request_and_the_roleplay_persona_does_not() {
    let f = fixture(vec![Script::Items(vec![chunk("ok"), finish("STOP")])]);
    let coder = Persona { name: "Coder".into(), description: "Answers in one sentence.".into(), ..Default::default() };
    f.db
        .with(|c| {
            persona_repo::upsert(c, &coder)?;
            settings::save_default_personas(
                c,
                &settings::DefaultPersonas { roleplay_persona_id: None, assistant_persona_id: Some(coder.id) },
            )
        })
        .unwrap();
    let conv = f.engine.open_assistant(false).unwrap().conversation.id;
    f.engine.set_model(conv, "fake", "fake-model").unwrap();

    send(&f, conv, "hi").await.0.unwrap();

    let system = f.provider.requests()[0].system_instruction.clone().unwrap();
    assert!(system.contains("Coder's persona: Answers in one sentence."));
    assert!(!system.contains("fictional roleplay"));
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
    no_clock(&f, conv);

    send(&f, conv, "Cara, then Alice - what do you two think?").await.0.unwrap();

    assert_eq!(speakers(&f, conv), [Some(cara.id), Some(alice.id)]);
    // The second speaker's prompt already contains the first one's reply, labelled with her name.
    let second = &f.provider.requests()[1];
    assert_eq!(second.history.last().unwrap().text, "Cara: Cara speaking.");
    assert_eq!(second.history.last().unwrap().role, LlmRole::Model);
    // ...and is told it is now Alice's turn, by name - not a literal {{char}}.
    let system = second.system_instruction.clone().unwrap();
    assert!(system.starts_with("Write Alice's next reply"), "{system}");
    assert!(system.contains("this reply is Alice's") && system.contains("Don't write lines or actions for Cara"), "{system}");
    assert!(!system.contains("{{char}}"), "{system}");
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

// ---- continue ----

fn thought(text: &str) -> std::result::Result<LlmResponseChunk, ProviderError> {
    Ok(LlmResponseChunk { thought_delta_text: text.into(), ..Default::default() })
}

async fn continue_reply(f: &Fixture, conversation: Uuid) -> (Result<()>, Vec<ChatEvent>) {
    let sink = Arc::new(VecSink::default());
    let result = f.engine.continue_message(conversation, sink.clone()).await;
    (result, sink.events())
}

#[tokio::test]
async fn continue_appends_to_the_same_reply_without_adding_an_alternate() {
    let f = fixture(vec![
        Script::Items(vec![thought("hm"), chunk("The door creaks"), finish("STOP")]),
        Script::Items(vec![thought("more"), chunk(" open slowly."), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    no_clock(&f, conv);
    send(&f, conv, "I knock.").await.0.unwrap();
    let reply_id = messages(&f, conv)[1].id;

    let (result, events) = continue_reply(&f, conv).await;
    result.unwrap();
    assert!(matches!(events[0], ChatEvent::Continuing { message_id } if message_id == reply_id));
    assert_eq!(streamed_text(&events), " open slowly.", "only the new part is streamed");
    assert!(matches!(events.last(), Some(ChatEvent::Finished)));

    let all = messages(&f, conv);
    assert_eq!(all.len(), 2, "no new message");
    let reply = &all[1];
    assert_eq!(reply.text, "The door creaks open slowly.");
    assert_eq!(reply.alternates.len(), 1, "still one generation");
    assert_eq!(reply.alternates[0].text, "The door creaks open slowly.");
    assert_eq!(reply.reasoning_text.as_deref(), Some("hm\n\nmore"));

    // The model saw the reply, then the nudge as a trailing user turn marked as the app's, not the user's.
    let request = &f.provider.requests()[1];
    let tail: Vec<_> = request.history.iter().map(|m| (m.role, m.text.as_str())).collect();
    assert_eq!(
        tail,
        [
            (LlmRole::User, "I knock."),
            (LlmRole::Model, "The door creaks"),
            (LlmRole::User, "[System note: Continue your last message without repeating its original content.]"),
        ]
    );
    assert!(reply.raw_prompt_text.as_deref().unwrap().contains("[System note: Continue your last message"));
    assert!(request.system_instruction.as_deref().unwrap().contains("you play Nova"), "the roles still hold");
}

#[tokio::test]
async fn continuing_in_a_group_keeps_the_original_speaker() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Bob here"), finish("STOP")]),
        Script::Items(vec![chunk("."), finish("STOP")]),
    ]);
    let (alice, bob) = (add_character(&f.db, "Alice", None, &[]), add_character(&f.db, "Bob", None, &[]));
    let group = add_group(&f, GroupActivationMode::Manual, &[&alice, &bob]);
    let conv = ready_group(&f, &group);
    send(&f, conv, "Bob?").await.0.unwrap();
    f.engine.pick_speaker(conv, bob.id, Arc::new(VecSink::default())).await.unwrap();

    continue_reply(&f, conv).await.0.unwrap();

    let system = f.provider.requests()[1].system_instruction.clone().unwrap();
    assert!(system.starts_with("Write Bob's next reply"), "{system}");
    assert!(system.contains("this reply is Bob's"), "{system}");
}

#[tokio::test]
async fn continue_extends_only_the_shown_swipe() {
    let f = fixture(vec![
        Script::Items(vec![chunk("one"), finish("STOP")]),
        Script::Items(vec![chunk("two"), finish("STOP")]),
        Script::Items(vec![chunk("!"), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    send(&f, conv, "hi").await.0.unwrap();
    let reply_id = messages(&f, conv)[1].id;
    f.engine.swipe_next(conv, reply_id, Arc::new(VecSink::default())).await.unwrap();

    continue_reply(&f, conv).await.0.unwrap();
    let reply = &messages(&f, conv)[1];
    let texts: Vec<_> = reply.alternates.iter().map(|a| a.text.as_str()).collect();
    assert_eq!(texts, ["one", "two!"]);
    assert_eq!(reply.text, "two!");
}

#[tokio::test]
async fn continue_asks_for_no_emotion_or_time_skip_tags() {
    let f = fixture(vec![
        Script::Items(vec![chunk("She smiles.[emotion: joy]"), finish("STOP")]),
        Script::Items(vec![chunk(" Warmly."), finish("STOP")]),
    ]);
    let nova = add_character(&f.db, "Nova", None, &["joy"]);
    let conv = ready_character(&f, &nova);
    send(&f, conv, "hi").await.0.unwrap();
    assert!(f.provider.requests()[0].system_instruction.as_deref().unwrap().contains("emotion"));

    continue_reply(&f, conv).await.0.unwrap();
    let system = f.provider.requests()[1].system_instruction.clone().unwrap_or_default();
    assert!(!system.contains("[emotion:") && !system.contains("time-skip"), "{system}");
    assert_eq!(messages(&f, conv)[1].text, "She smiles. Warmly.");
    assert_eq!(messages(&f, conv)[1].detected_emotion.as_deref(), Some("joy"), "the earlier emotion stays");
}

#[tokio::test]
async fn stopping_a_continuation_keeps_the_part_that_arrived() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Start."), finish("STOP")]),
        Script::Hang(vec![LlmResponseChunk::text(" And on")]),
    ]);
    let conv = ready_roleplay(&f);
    send(&f, conv, "go").await.0.unwrap();

    let sink = Arc::new(VecSink::default());
    let engine = f.engine.clone();
    let task_sink: Arc<dyn EventSink> = sink.clone();
    let task = tokio::spawn(async move { engine.continue_message(conv, task_sink).await });
    wait_until("a streamed delta", || sink.events().iter().any(|e| matches!(e, ChatEvent::Delta { .. }))).await;
    f.engine.stop(conv);
    task.await.unwrap().unwrap();

    assert_eq!(messages(&f, conv)[1].text, "Start. And on");
    assert!(error_messages(&sink.events()).is_empty());
    assert!(!f.engine.is_generating(conv));
}

#[tokio::test]
async fn a_failed_continuation_rolls_back_to_the_original_text() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Original."), finish("STOP")]),
        Script::Items(vec![chunk(" partial"), Err(ProviderError::network("dropped"))]),
    ]);
    let conv = ready_roleplay(&f);
    send(&f, conv, "go").await.0.unwrap();

    let (result, events) = continue_reply(&f, conv).await;
    result.unwrap();
    assert_eq!(error_messages(&events).len(), 1);
    assert!(events.iter().any(|e| matches!(e, ChatEvent::MessageUpdated { message } if message.text == "Original.")));
    assert_eq!(messages(&f, conv)[1].text, "Original.");
}

#[tokio::test]
async fn continue_needs_a_character_reply() {
    let f = fixture(vec![]);
    let conv = ready_roleplay(&f);
    // Nothing to continue in an empty chat...
    let (result, events) = continue_reply(&f, conv).await;
    assert!(result.unwrap_err().to_string().contains("no reply to continue"));
    assert_eq!(error_messages(&events).len(), 1);

    // ...nor when the last message is the user's own.
    f.db.with(|c| conversations::add_message(c, &ChatMessage::new(conv, MessageRole::User, "hello"))).unwrap();
    assert!(continue_reply(&f, conv).await.0.is_err());
    assert!(f.provider.requests().is_empty());
}

// ---- impersonate ----

#[tokio::test]
async fn impersonate_drafts_a_user_message_without_saving_anything() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Hello there."), finish("STOP")]),
        Script::Items(vec![chunk("  I lean closer and whisper.\n"), finish("STOP")]),
    ]);
    add_persona(&f.db, "Alex");
    let conv = ready_roleplay(&f);
    no_clock(&f, conv);
    send(&f, conv, "hi").await.0.unwrap();
    let before = messages(&f, conv);

    let draft = f.engine.impersonate(conv).await.unwrap();
    assert_eq!(draft.as_deref(), Some("I lean closer and whisper."));
    assert_eq!(messages(&f, conv), before, "the draft is only returned, never stored");
    assert!(!f.engine.is_generating(conv));

    // The last turn was the character's, so the instruction is a new trailing user turn.
    let request = &f.provider.requests()[1];
    let last = request.history.last().unwrap();
    assert_eq!(last.role, LlmRole::User);
    assert!(last.text.starts_with("[System note: This time, write Alex's next message"), "{}", last.text);
    assert!(last.text.contains("Don't write anything Nova says or does"), "{}", last.text);
    assert_eq!(request.history.len(), 3);
    // Neither the preset's "write the character's reply" prompt nor the character-side roles remain.
    let system = request.system_instruction.clone().unwrap_or_default();
    assert!(!system.contains("Write Nova's next reply"), "{system}");
    assert!(!system.contains("Roles:"), "{system}");
}

#[tokio::test]
async fn impersonate_folds_the_instruction_into_a_trailing_user_message() {
    let f = fixture(vec![Script::Items(vec![chunk("Well?"), finish("STOP")])]);
    let conv = ready_roleplay(&f);
    no_clock(&f, conv);
    f.db.with(|c| conversations::add_message(c, &ChatMessage::new(conv, MessageRole::User, "Where were we?"))).unwrap();

    f.engine.impersonate(conv).await.unwrap();
    let request = &f.provider.requests()[0];
    assert_eq!(request.history.len(), 1, "no second user turn (providers reject two in a row)");
    assert!(request.history[0].text.starts_with("Where were we?\n\n[System note: This time, write User's next message"));
}

#[tokio::test]
async fn impersonate_in_a_group_names_the_other_characters() {
    let f = fixture(vec![Script::Items(vec![chunk("Right."), finish("STOP")])]);
    let alice = add_character(&f.db, "Alice", None, &[]);
    let bob = add_character(&f.db, "Bob", None, &[]);
    let group = add_group(&f, GroupActivationMode::Manual, &[&alice, &bob]);
    let conv = ready_group(&f, &group);
    no_clock(&f, conv);

    f.engine.impersonate(conv).await.unwrap();
    let last = f.provider.requests()[0].history.last().unwrap().text.clone();
    assert!(last.contains("Don't write anything Alice or Bob says or does"), "{last}");
}

#[tokio::test]
async fn impersonate_reports_provider_failures_and_can_be_stopped() {
    let f = fixture(vec![Script::FailToStart(ProviderError::auth("bad key")), Script::Hang(vec![])]);
    let conv = ready_roleplay(&f);
    let err = f.engine.impersonate(conv).await.unwrap_err();
    assert!(err.to_string().contains("Authentication failed"), "{err}");
    assert!(!f.engine.is_generating(conv), "the slot is freed after a failure");

    let engine = f.engine.clone();
    let task = tokio::spawn(async move { engine.impersonate(conv).await });
    wait_until("the request to start", || f.engine.is_generating(conv) && f.provider.requests().len() == 2).await;
    // Nothing else may generate meanwhile.
    assert!(send(&f, conv, "hi").await.0.unwrap_err().to_string().contains("already being generated"));
    f.engine.stop(conv);
    assert_eq!(task.await.unwrap().unwrap(), None);
    assert!(!f.engine.is_generating(conv));
}

// ---- background extraction (memories, quests, notebooks) ----

/// Answers each request by what it asks about, so the reply and the extraction requests (which run
/// concurrently) can't be confused by their order. The first rule whose needle occurs in the request
/// wins; nothing matching answers "NONE".
struct Routed {
    rules: Mutex<Vec<(&'static str, std::result::Result<String, ProviderError>)>>,
    requests: Mutex<Vec<String>>,
}

impl Routed {
    fn new(rules: Vec<(&'static str, std::result::Result<String, ProviderError>)>) -> Arc<Self> {
        Arc::new(Self { rules: Mutex::new(rules), requests: Mutex::new(vec![]) })
    }

    fn answer(&self, needle: &'static str, answer: &str) {
        let mut rules = self.rules.lock().unwrap();
        rules.retain(|(n, _)| *n != needle);
        rules.insert(0, (needle, Ok(answer.to_string())));
    }

    /// The prompts of requests that contained `needle`.
    fn asked(&self, needle: &str) -> Vec<String> {
        self.requests.lock().unwrap().iter().filter(|r| r.contains(needle)).cloned().collect()
    }
}

const NOTEBOOK_ASK: &str = "extracting what specific characters";
const QUEST_ASK: &str = "quest/objective log for an ongoing roleplay";
const MEMORY_ASK: &str = "extracting durable facts about the USER";
const CONDENSE_ASK: &str = "It has grown long";

#[async_trait]
impl LlmProvider for Routed {
    fn provider_id(&self) -> &str {
        "fake"
    }

    async fn list_models(&self) -> std::result::Result<Vec<String>, ProviderError> {
        Ok(vec!["fake-model".into()])
    }

    async fn stream_completion(&self, request: LlmRequest) -> std::result::Result<ChunkStream, ProviderError> {
        let prompt = request.history.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n");
        self.requests.lock().unwrap().push(prompt.clone());
        let answer = {
            let rules = self.rules.lock().unwrap();
            rules.iter().find(|(needle, _)| prompt.contains(needle)).map(|(_, a)| a.clone())
        };
        let text = answer.unwrap_or_else(|| Ok("NONE".into()))?;
        Ok(Box::pin(futures::stream::iter(vec![chunk(&text), finish("STOP")])))
    }
}

struct RoutedFixture {
    engine: ChatEngine,
    provider: Arc<Routed>,
    db: Db,
    _images: tempfile::TempDir,
}

/// Like [`fixture`], but with extraction switched on and a [`Routed`] provider.
fn routed(rules: Vec<(&'static str, std::result::Result<String, ProviderError>)>) -> RoutedFixture {
    let provider = Routed::new(rules);
    let db = Db::new(open_in_memory().unwrap());
    let images = tempfile::tempdir().unwrap();
    let registry = LlmProviderRegistry::new(vec![provider.clone()]);
    let engine = ChatEngine::new(db.clone(), Arc::new(registry), images.path().to_path_buf());
    engine.set_extraction(true);
    RoutedFixture { engine, provider, db, _images: images }
}

impl RoutedFixture {
    fn roleplay(&self, character: &Character) -> Uuid {
        let session = self.engine.open_character(character.id, false).unwrap();
        self.engine.set_model(session.conversation.id, "fake", "fake-model").unwrap();
        session.conversation.id
    }

    /// Sends a message and waits until the roleplay's background extraction has finished.
    async fn say(&self, conv: Uuid, text: &str) -> Vec<ChatEvent> {
        let sink = Arc::new(VecSink::default());
        self.engine.send_message(conv, text.into(), vec![], sink.clone()).await.unwrap();
        wait_until("story extraction", || sink.events().iter().any(|e| matches!(e, ChatEvent::StoryChanged { .. }))).await;
        sink.events()
    }

    fn notes(&self, conv: Uuid) -> Vec<String> {
        self.db.with(|c| crate::repo::notebook::get_by_conversation(c, conv)).unwrap().into_iter().map(|n| n.content).collect()
    }

    fn quests(&self, conv: Uuid) -> Vec<crate::models::QuestLogEntry> {
        self.db.with(|c| crate::repo::quest_log::get_by_conversation(c, conv)).unwrap()
    }
}

#[tokio::test]
async fn a_roleplay_reply_fills_the_notebook_and_quest_log_and_tells_the_ui() {
    let f = routed(vec![
        (NOTEBOOK_ASK, Ok("Nova: Alex is looking for the Lantern Belt\nnobody: ignored".into())),
        (QUEST_ASK, Ok("ADD: Reach the Lantern Belt :: Three days out.".into())),
        ("", Ok("We set course.".into())),
    ]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = f.roleplay(&nova);

    let events = f.say(conv, "Where to?").await;
    assert_eq!(f.notes(conv), ["Alex is looking for the Lantern Belt"]);
    let quests = f.quests(conv);
    assert_eq!((quests[0].title.as_str(), quests[0].description.as_str()), ("Reach the Lantern Belt", "Three days out."));
    assert_eq!(quests[0].status, crate::models::QuestStatus::Active);
    assert!(events.iter().any(|e| matches!(e, ChatEvent::StoryChanged { conversation_id } if *conversation_id == conv)));
    assert!(error_messages(&events).is_empty());

    // The model was shown the exchange, not just the reply, with each message under its author's name.
    let ask = &f.provider.asked(NOTEBOOK_ASK)[0];
    assert!(ask.contains("User (the player) wrote:\nWhere to?") && ask.contains("Nova replied:\nWe set course."), "{ask}");
    assert!(ask.contains("the player is User, never one of the characters"), "{ask}");
    assert!(ask.contains("Characters present: Nova") && ask.contains("Nova:\n(none yet)"), "{ask}");
}

#[tokio::test]
async fn quests_are_updated_completed_and_failed_by_exact_title() {
    let f = routed(vec![
        (QUEST_ASK, Ok("ADD: Find the key :: Somewhere in the vault.\nADD: Escape :: Before dawn.\nADD: Bribe the guard :: With coin.".into())),
        ("", Ok("ok".into())),
    ]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = f.roleplay(&nova);
    f.say(conv, "one").await;
    assert_eq!(f.quests(conv).len(), 3);

    f.provider.answer(QUEST_ASK, "UPDATE: Find the key :: Behind the painting.\nCOMPLETE: Escape\nFAIL: Bribe the guard\nCOMPLETE: No such quest");
    f.say(conv, "two").await;

    // The second request listed what was already tracked, statuses included.
    let ask = &f.provider.asked(QUEST_ASK)[1];
    assert!(ask.contains("- Find the key (Active): Somewhere in the vault."), "{ask}");

    let by_title = |t: &str| f.quests(conv).into_iter().find(|q| q.title == t).unwrap();
    assert_eq!(by_title("Find the key").description, "Behind the painting.");
    assert_eq!(by_title("Find the key").status, crate::models::QuestStatus::Active);
    assert_eq!(by_title("Escape").status, crate::models::QuestStatus::Completed);
    assert_eq!(by_title("Bribe the guard").status, crate::models::QuestStatus::Failed);
    assert_eq!(f.quests(conv).len(), 3, "an unknown title adds nothing");
}

#[tokio::test]
async fn known_notes_are_shown_to_the_model_so_it_does_not_repeat_them() {
    let f = routed(vec![(NOTEBOOK_ASK, Ok("Nova: likes tea".into())), ("", Ok("ok".into()))]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = f.roleplay(&nova);
    f.say(conv, "one").await;
    f.say(conv, "two").await;
    assert!(f.provider.asked(NOTEBOOK_ASK)[1].contains("Nova:\n- likes tea"));
}

#[tokio::test]
async fn in_a_group_each_character_gets_their_own_notes() {
    let f = routed(vec![
        (NOTEBOOK_ASK, Ok("bob: saw the thief\n- Alice: heard the alarm".into())),
        ("", Ok("ok".into())),
    ]);
    let alice = add_character(&f.db, "Alice", None, &[]);
    let bob = add_character(&f.db, "Bob", None, &[]);
    let group = crate::models::GroupChat {
        name: "Crew".into(),
        character_ids: vec![alice.id, bob.id],
        activation_mode: GroupActivationMode::Manual,
        ..Default::default()
    };
    f.db.with(|c| group_chats::upsert(c, &group)).unwrap();
    let session = f.engine.open_group(group.id, false).unwrap();
    let conv = session.conversation.id;
    f.engine.set_model(conv, "fake", "fake-model").unwrap();

    f.engine.send_message(conv, "who saw?".into(), vec![], Arc::new(VecSink::default())).await.unwrap();
    let sink = Arc::new(VecSink::default());
    f.engine.pick_speaker(conv, alice.id, sink.clone()).await.unwrap();
    wait_until("story extraction", || sink.events().iter().any(|e| matches!(e, ChatEvent::StoryChanged { .. }))).await;

    let notes = f.db.with(|c| crate::repo::notebook::get_by_conversation(c, conv)).unwrap();
    let of = |id| notes.iter().filter(|n| n.character_id == id).map(|n| n.content.as_str()).collect::<Vec<_>>();
    assert_eq!(of(alice.id), ["heard the alarm"]);
    assert_eq!(of(bob.id), ["saw the thief"]);
    assert!(f.provider.asked(NOTEBOOK_ASK)[0].contains("Characters present: Alice, Bob"));
}

#[tokio::test]
async fn a_notebook_that_grows_too_long_is_condensed_but_never_wiped() {
    let f = routed(vec![
        (NOTEBOOK_ASK, Ok("Nova: fact 21".into())),
        (CONDENSE_ASK, Ok("- merged one\n* merged two\n\n".into())),
        ("", Ok("ok".into())),
    ]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = f.roleplay(&nova);
    f.db.with(|c| {
        for i in 1..=20 {
            crate::repo::notebook::add(c, &crate::models::CharacterNotebookEntry::new(conv, nova.id, format!("fact {i}")))?;
        }
        Ok(())
    })
    .unwrap();

    f.say(conv, "hello").await;
    assert_eq!(f.notes(conv), ["merged one", "merged two"]);
    assert!(f.provider.asked(CONDENSE_ASK)[0].contains("- fact 21"), "the new fact is condensed too");

    // Twenty-one entries again, but the condensed answer has nothing usable: keep everything.
    f.db.with(|c| {
        for i in 1..=19 {
            crate::repo::notebook::add(c, &crate::models::CharacterNotebookEntry::new(conv, nova.id, format!("more {i}")))?;
        }
        Ok(())
    })
    .unwrap();
    f.provider.answer(CONDENSE_ASK, "  \n - \n");
    f.provider.answer(NOTEBOOK_ASK, "Nova: one more");
    f.say(conv, "again").await;
    assert_eq!(f.notes(conv).len(), 2 + 19 + 1);
}

#[tokio::test]
async fn assistant_chats_remember_new_facts_about_the_user() {
    let f = routed(vec![(MEMORY_ASK, Ok("- Lives in Berlin\n* Prefers Rust\nNONE\n".into())), ("", Ok("Noted.".into()))]);
    let session = f.engine.open_assistant(false).unwrap();
    let conv = session.conversation.id;
    f.engine.set_model(conv, "fake", "fake-model").unwrap();

    f.engine.send_message(conv, "I live in Berlin and like Rust.".into(), vec![], Arc::new(VecSink::default())).await.unwrap();
    let memories = || f.db.with(crate::repo::user_memories::get_all).unwrap();
    wait_until("the memories", || memories().len() == 2).await;
    let mut facts: Vec<_> = memories().into_iter().map(|m| m.content).collect();
    facts.sort();
    assert_eq!(facts, ["Lives in Berlin", "Prefers Rust"]);
    let ask = &f.provider.asked(MEMORY_ASK)[0];
    assert!(ask.contains("User: I live in Berlin and like Rust.") && ask.contains("Assistant: Noted."), "{ask}");
    assert!(ask.contains("(none yet)"));

    // The next exchange is told what is already known.
    f.provider.answer(MEMORY_ASK, "NONE");
    f.engine.send_message(conv, "thanks".into(), vec![], Arc::new(VecSink::default())).await.unwrap();
    wait_until("the second request", || f.provider.asked(MEMORY_ASK).len() == 2).await;
    assert!(f.provider.asked(MEMORY_ASK)[1].contains("- Lives in Berlin"));
    assert_eq!(memories().len(), 2, "NONE adds nothing");
}

#[tokio::test]
async fn extraction_failures_never_reach_the_chat() {
    let f = routed(vec![
        (NOTEBOOK_ASK, Err(ProviderError::network("down"))),
        (QUEST_ASK, Err(ProviderError::auth("nope"))),
        ("", Ok("Fine.".into())),
    ]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = f.roleplay(&nova);

    let events = f.say(conv, "hi").await;
    assert!(error_messages(&events).is_empty());
    assert_eq!(messages_of(&f.db, conv)[1].text, "Fine.");
    assert!(f.notes(conv).is_empty() && f.quests(conv).is_empty());
}

fn messages_of(db: &Db, conversation: Uuid) -> Vec<ChatMessage> {
    db.with(|c| conversations::get_messages(c, conversation)).unwrap()
}

#[tokio::test]
async fn nothing_is_extracted_from_replies_that_failed_or_were_stopped() {
    let f = fixture(vec![Script::Hang(vec![LlmResponseChunk::text("half")]), Script::Items(vec![chunk("x"), Err(ProviderError::network("cut"))])]);
    f.engine.set_extraction(true);
    let conv = ready_roleplay(&f);

    // Stopped mid-reply.
    let sink = Arc::new(VecSink::default());
    let engine = f.engine.clone();
    let task_sink: Arc<dyn EventSink> = sink.clone();
    let task = tokio::spawn(async move { engine.send_message(conv, "one".into(), vec![], task_sink).await });
    wait_until("a delta", || sink.events().iter().any(|e| matches!(e, ChatEvent::Delta { .. }))).await;
    f.engine.stop(conv);
    task.await.unwrap().unwrap();

    // Failed mid-reply.
    let (result, events) = send(&f, conv, "two").await;
    result.unwrap();
    assert_eq!(error_messages(&events).len(), 1);

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(f.provider.requests().len(), 2, "only the two replies were requested");
    assert!(!sink.events().iter().any(|e| matches!(e, ChatEvent::StoryChanged { .. })));
}

#[tokio::test]
async fn regenerating_and_continuing_extract_from_the_new_text() {
    let f = routed(vec![(QUEST_ASK, Ok("NONE".into())), ("", Ok("first".into()))]);
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = f.roleplay(&nova);
    f.say(conv, "go").await;
    let reply_id = messages_of(&f.db, conv)[1].id;

    f.provider.answer("", "second");
    let sink = Arc::new(VecSink::default());
    f.engine.swipe_next(conv, reply_id, sink.clone()).await.unwrap();
    wait_until("story extraction", || sink.events().iter().any(|e| matches!(e, ChatEvent::StoryChanged { .. }))).await;
    assert!(f.provider.asked(QUEST_ASK)[1].contains("User (the player) wrote:\ngo\n\nNova replied:\nsecond"));

    f.provider.answer("", " and more");
    let sink = Arc::new(VecSink::default());
    f.engine.continue_message(conv, sink.clone()).await.unwrap();
    wait_until("story extraction", || sink.events().iter().any(|e| matches!(e, ChatEvent::StoryChanged { .. }))).await;
    assert!(f.provider.asked(QUEST_ASK)[2].contains("Nova replied:\nsecond and more"), "the whole continued reply");
}

// ---- PowerShell tool ----

use crate::chat::ToolDecision;
use crate::powershell::exec::{CommandRunner, ExecError, ExecutionResult};
use crate::powershell::settings::{self as tool_settings, PowerShellToolOptions};
use tokio_util::sync::CancellationToken;

enum RunOutcome {
    /// Streams these chunks as live output, then finishes with this result.
    Done(ExecutionResult, Vec<&'static str>),
    Fail(ExecError),
    /// Runs until the generation is stopped.
    Hang,
}

/// Records the commands it is asked to run and answers from a script (default: exit 0, "ok").
struct FakeRunner {
    calls: Mutex<Vec<(String, bool)>>,
    outcomes: Mutex<VecDeque<RunOutcome>>,
}

impl FakeRunner {
    fn new(outcomes: Vec<RunOutcome>) -> Arc<Self> {
        Arc::new(Self { calls: Mutex::new(vec![]), outcomes: Mutex::new(outcomes.into()) })
    }

    fn calls(&self) -> Vec<(String, bool)> {
        self.calls.lock().unwrap().clone()
    }
}

fn exited(stdout: &str) -> ExecutionResult {
    ExecutionResult { exit_code: Some(0), std_out: stdout.into(), ..Default::default() }
}

#[async_trait]
impl CommandRunner for FakeRunner {
    async fn run(
        &self,
        command: &str,
        elevate: bool,
        on_output: &(dyn Fn(String) + Send + Sync),
        cancel: &CancellationToken,
    ) -> std::result::Result<ExecutionResult, ExecError> {
        self.calls.lock().unwrap().push((command.to_string(), elevate));
        let outcome = self.outcomes.lock().unwrap().pop_front().unwrap_or(RunOutcome::Done(exited("ok"), vec![]));
        match outcome {
            RunOutcome::Done(mut result, chunks) => {
                chunks.into_iter().for_each(|c| on_output(c.to_string()));
                result.command = command.to_string();
                result.elevated = elevate;
                Ok(result)
            }
            RunOutcome::Fail(error) => Err(error),
            RunOutcome::Hang => {
                cancel.cancelled().await;
                Err(ExecError::Cancelled)
            }
        }
    }
}

fn tool_call(command: &str) -> Script {
    Script::Items(vec![chunk(&format!("<powershell-run>{command}</powershell-run>")), finish("STOP")])
}

fn tools_on(f: &Fixture, auto_mode: bool, allowlist: &[&str]) {
    let options = PowerShellToolOptions {
        enabled: true,
        auto_mode,
        allowlist_patterns: allowlist.iter().map(|p| p.to_string()).collect(),
    };
    f.db.with(|c| tool_settings::save(c, &options)).unwrap();
}

fn roles(f: &Fixture, conv: Uuid) -> Vec<(MessageRole, String)> {
    messages(f, conv).into_iter().map(|m| (m.role, m.text)).collect()
}

fn approval_request(sink: &VecSink) -> Option<(Uuid, String, bool)> {
    sink.events().into_iter().find_map(|e| match e {
        ChatEvent::ToolApproval { request_id, command, elevate } => Some((request_id, command, elevate)),
        _ => None,
    })
}

#[tokio::test]
async fn auto_mode_runs_the_command_and_feeds_the_result_back() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Checking. <powershell-r"), chunk("un>Get-Da"), chunk("te</powershell-run>"), finish("STOP")]),
        Script::Items(vec![chunk("It is Monday."), finish("STOP")]),
    ]);
    tools_on(&f, true, &[]);
    let runner = FakeRunner::new(vec![RunOutcome::Done(exited("Monday"), vec!["Mon", "day\n"])]);
    f.engine.set_command_runner(runner.clone());
    let conv = ready_assistant(&f);

    let (result, events) = send(&f, conv, "what day is it?").await;
    result.unwrap();

    assert_eq!(runner.calls(), [("Get-Date".to_string(), false)]);
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::ToolApproval { .. })), "auto mode never asks");

    let saved = roles(&f, conv);
    assert_eq!(saved.len(), 4);
    assert_eq!(saved[1], (MessageRole::Character, "Checking. ".to_string()), "the tag is never stored");
    assert_eq!(saved[2].0, MessageRole::Tool);
    assert!(saved[2].1.contains("Command: Get-Date") && saved[2].1.contains("Output:\nMonday"), "{}", saved[2].1);
    assert_eq!(saved[3], (MessageRole::Character, "It is Monday.".to_string()));

    // The user never saw the tag, but did see the live output stream into the tool bubble.
    let tool_id = messages(&f, conv)[2].id;
    assert!(!streamed_text(&events).contains("powershell"), "{}", streamed_text(&events));
    let live: String = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Delta { message_id, text, .. } if *message_id == tool_id => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(live, "$ Get-Date\nMonday\n");

    // The second request carried the real output, and both offered the tool.
    let requests = f.provider.requests();
    assert!(requests[1].history.last().unwrap().text.contains("Output:\nMonday"));
    assert!(requests.iter().all(|r| r.system_instruction.as_deref().unwrap_or("").contains("<powershell-run>")));
    assert!(events.iter().filter(|e| matches!(e, ChatEvent::Finished)).count() == 1);
}

#[tokio::test]
async fn otherwise_the_user_is_asked_and_always_allow_remembers_the_pattern() {
    let f = fixture(vec![tool_call("Get-Date -Format o"), Script::Items(vec![chunk("Done."), finish("STOP")])]);
    tools_on(&f, false, &[]);
    let runner = FakeRunner::new(vec![]);
    f.engine.set_command_runner(runner.clone());
    let conv = ready_assistant(&f);

    let sink = Arc::new(VecSink::default());
    let (engine, task_sink) = (f.engine.clone(), sink.clone() as Arc<dyn EventSink>);
    let task = tokio::spawn(async move { engine.send_message(conv, "time?".into(), vec![], task_sink).await });
    wait_until("the approval request", || approval_request(&sink).is_some()).await;
    let (request_id, command, elevate) = approval_request(&sink).unwrap();
    assert_eq!((command.as_str(), elevate), ("Get-Date -Format o", false));
    assert!(runner.calls().is_empty(), "nothing runs before the user agrees");

    assert!(f.engine.resolve_tool_approval(request_id, ToolDecision { allow: true, always_pattern: Some("Get-Date*".into()) }));
    assert!(!f.engine.resolve_tool_approval(request_id, ToolDecision::default()), "an answer counts once");
    task.await.unwrap().unwrap();
    assert_eq!(runner.calls().len(), 1);
    let stored = f.db.with(tool_settings::load).unwrap();
    assert_eq!(stored.allowlist_patterns, ["Get-Date*"]);

    // The same kind of command now goes straight through.
    f.provider.scripts.lock().unwrap().extend([tool_call("Get-Date"), Script::Items(vec![chunk("Again."), finish("STOP")])]);
    let (result, events) = send(&f, conv, "and now?").await;
    result.unwrap();
    assert_eq!(runner.calls().len(), 2);
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::ToolApproval { .. })));
}

#[tokio::test]
async fn a_denied_command_is_not_run_and_the_model_is_told() {
    let f = fixture(vec![tool_call("Remove-Item x"), Script::Items(vec![chunk("Understood."), finish("STOP")])]);
    tools_on(&f, false, &[]);
    let runner = FakeRunner::new(vec![]);
    f.engine.set_command_runner(runner.clone());
    let conv = ready_assistant(&f);

    let sink = Arc::new(VecSink::default());
    let (engine, task_sink) = (f.engine.clone(), sink.clone() as Arc<dyn EventSink>);
    let task = tokio::spawn(async move { engine.send_message(conv, "clean up".into(), vec![], task_sink).await });
    wait_until("the approval request", || approval_request(&sink).is_some()).await;
    f.engine.resolve_tool_approval(approval_request(&sink).unwrap().0, ToolDecision { allow: false, always_pattern: Some("x*".into()) });
    task.await.unwrap().unwrap();

    assert!(runner.calls().is_empty());
    let saved = roles(&f, conv);
    assert_eq!(saved[1].0, MessageRole::Tool);
    assert!(saved[1].1.contains("declined") && saved[1].1.contains("Remove-Item x"), "{}", saved[1].1);
    assert_eq!(saved[2].1, "Understood.");
    assert!(f.db.with(tool_settings::load).unwrap().allowlist_patterns.is_empty(), "denying remembers nothing");
    assert!(f.provider.requests()[1].history.last().unwrap().text.contains("declined"));
}

#[tokio::test]
async fn elevation_always_asks_and_is_never_remembered() {
    let f = fixture(vec![
        Script::Items(vec![chunk("<powershell-run elevate=\"true\">Restart-Service Spooler</powershell-run>"), finish("STOP")]),
        Script::Items(vec![chunk("Restarted."), finish("STOP")]),
    ]);
    // Auto mode and a match-everything allowlist still don't cover elevation.
    tools_on(&f, true, &["*"]);
    let runner = FakeRunner::new(vec![]);
    f.engine.set_command_runner(runner.clone());
    let conv = ready_assistant(&f);

    let sink = Arc::new(VecSink::default());
    let (engine, task_sink) = (f.engine.clone(), sink.clone() as Arc<dyn EventSink>);
    let task = tokio::spawn(async move { engine.send_message(conv, "fix printing".into(), vec![], task_sink).await });
    wait_until("the approval request", || approval_request(&sink).is_some()).await;
    let (id, command, elevate) = approval_request(&sink).unwrap();
    assert_eq!((command.as_str(), elevate), ("Restart-Service Spooler", true));
    f.engine.resolve_tool_approval(id, ToolDecision { allow: true, always_pattern: Some("Restart*".into()) });
    task.await.unwrap().unwrap();

    assert_eq!(runner.calls(), [("Restart-Service Spooler".to_string(), true)]);
    assert_eq!(f.db.with(tool_settings::load).unwrap().allowlist_patterns, ["*"], "no elevated pattern was stored");
    assert!(roles(&f, conv)[1].1.contains("Elevated: yes"));
}

#[tokio::test]
async fn stopping_while_the_question_is_open_ends_the_turn_without_running_anything() {
    let f = fixture(vec![Script::Items(vec![chunk("Let me look. <powershell-run>ls</powershell-run>"), finish("STOP")])]);
    tools_on(&f, false, &[]);
    let runner = FakeRunner::new(vec![]);
    f.engine.set_command_runner(runner.clone());
    let conv = ready_assistant(&f);

    let sink = Arc::new(VecSink::default());
    let (engine, task_sink) = (f.engine.clone(), sink.clone() as Arc<dyn EventSink>);
    let task = tokio::spawn(async move { engine.send_message(conv, "list".into(), vec![], task_sink).await });
    wait_until("the approval request", || approval_request(&sink).is_some()).await;
    f.engine.stop(conv);
    tokio::time::timeout(Duration::from_secs(5), task).await.expect("stops promptly").unwrap().unwrap();

    assert!(runner.calls().is_empty());
    assert_eq!(roles(&f, conv).len(), 2, "just the user message and what the assistant said before the tag");
    assert!(error_messages(&sink.events()).is_empty());
    assert!(!f.engine.is_generating(conv));
    assert!(!f.engine.resolve_tool_approval(approval_request(&sink).unwrap().0, ToolDecision::default()), "the question is gone");
}

#[tokio::test]
async fn stopping_a_running_command_kills_it_and_drops_the_unfinished_bubble() {
    let f = fixture(vec![tool_call("Start-Sleep 60")]);
    tools_on(&f, true, &[]);
    f.engine.set_command_runner(FakeRunner::new(vec![RunOutcome::Hang]));
    let conv = ready_assistant(&f);

    let sink = Arc::new(VecSink::default());
    let (engine, task_sink) = (f.engine.clone(), sink.clone() as Arc<dyn EventSink>);
    let task = tokio::spawn(async move { engine.send_message(conv, "wait".into(), vec![], task_sink).await });
    wait_until("the tool bubble", || {
        sink.events().iter().any(|e| matches!(e, ChatEvent::MessageAdded { message } if message.role == MessageRole::Tool))
    })
    .await;
    f.engine.stop(conv);
    tokio::time::timeout(Duration::from_secs(5), task).await.expect("stops promptly").unwrap().unwrap();

    assert!(sink.events().iter().any(|e| matches!(e, ChatEvent::MessageRemoved { .. })));
    assert_eq!(roles(&f, conv).len(), 1, "only the user's message is left");
    assert_eq!(f.provider.requests().len(), 1, "the model isn't asked to continue");
}

#[tokio::test]
async fn a_runaway_tool_loop_is_cut_off_after_five_round_trips() {
    let f = fixture((0..7).map(|_| tool_call("Get-Date")).collect());
    tools_on(&f, true, &[]);
    let runner = FakeRunner::new(vec![]);
    f.engine.set_command_runner(runner.clone());
    let conv = ready_assistant(&f);

    let (result, events) = send(&f, conv, "loop forever").await;
    result.unwrap();
    assert_eq!(runner.calls().len(), 5);
    assert_eq!(f.provider.requests().len(), 6);
    let saved = roles(&f, conv);
    assert_eq!(saved.last().unwrap(), &(MessageRole::Tool, "Tool call limit reached for this turn.".to_string()));
    assert!(error_messages(&events).is_empty());
    assert!(!f.engine.is_generating(conv));
}

#[tokio::test]
async fn runner_problems_reach_the_model_as_tool_results() {
    let f = fixture(vec![
        tool_call("Get-Date"),
        Script::Items(vec![chunk("Can't."), finish("STOP")]),
        tool_call("Restart-Service x"),
        Script::Items(vec![chunk("Ok."), finish("STOP")]),
    ]);
    tools_on(&f, true, &[]);
    f.engine.set_command_runner(FakeRunner::new(vec![
        RunOutcome::Fail(ExecError::PwshNotFound),
        RunOutcome::Done(ExecutionResult { uac_declined: true, ..Default::default() }, vec![]),
    ]));
    let conv = ready_assistant(&f);

    send(&f, conv, "one").await.0.unwrap();
    assert!(roles(&f, conv)[1].1.contains("wasn't found on PATH"), "{}", roles(&f, conv)[1].1);

    // Elevation turned down at the Windows prompt is reported to the model as a decline.
    send(&f, conv, "two").await.0.unwrap();
    let last_tool = roles(&f, conv).into_iter().rev().find(|(role, _)| *role == MessageRole::Tool).unwrap();
    assert!(last_tool.1.contains("declined"), "{}", last_tool.1);
}

#[tokio::test]
async fn the_tool_is_only_offered_in_assistant_chats_when_switched_on() {
    // Off (the default): the instruction is absent and a tag in the reply is just text.
    let f = fixture(vec![tool_call("Get-Date")]);
    let runner = FakeRunner::new(vec![]);
    f.engine.set_command_runner(runner.clone());
    let conv = ready_assistant(&f);
    send(&f, conv, "hi").await.0.unwrap();
    assert!(!f.provider.requests()[0].system_instruction.clone().unwrap_or_default().contains("powershell"));
    assert_eq!(messages(&f, conv)[1].text, "<powershell-run>Get-Date</powershell-run>");
    assert!(runner.calls().is_empty());

    // On, but in a roleplay: still nothing.
    let f = fixture(vec![tool_call("Get-Date")]);
    tools_on(&f, true, &[]);
    let runner = FakeRunner::new(vec![]);
    f.engine.set_command_runner(runner.clone());
    let conv = ready_roleplay(&f);
    send(&f, conv, "hi").await.0.unwrap();
    assert!(!f.provider.requests()[0].system_instruction.clone().unwrap_or_default().contains("powershell"));
    assert!(runner.calls().is_empty());
    assert!(messages(&f, conv)[1].text.contains("<powershell-run>"));
}

#[tokio::test]
async fn a_malformed_tag_is_shown_as_written_and_nothing_runs() {
    let f = fixture(vec![Script::Items(vec![chunk("Try <powershell-run evil=\"1\">x</powershell-run> maybe"), finish("STOP")])]);
    tools_on(&f, true, &[]);
    let runner = FakeRunner::new(vec![]);
    f.engine.set_command_runner(runner.clone());
    let conv = ready_assistant(&f);
    let (result, events) = send(&f, conv, "hi").await;
    result.unwrap();
    assert!(runner.calls().is_empty());
    assert_eq!(streamed_text(&events), "Try <powershell-run evil=\"1\">x</powershell-run> maybe");
    assert_eq!(messages(&f, conv)[1].text, "Try <powershell-run evil=\"1\">x</powershell-run> maybe");
}

#[tokio::test]
async fn swiping_or_continuing_never_offers_the_tool() {
    let f = fixture(vec![
        Script::Items(vec![chunk("first"), finish("STOP")]),
        Script::Items(vec![chunk("second"), finish("STOP")]),
    ]);
    tools_on(&f, true, &[]);
    let conv = ready_assistant(&f);
    send(&f, conv, "hi").await.0.unwrap();
    let reply = messages(&f, conv)[1].id;
    f.engine.swipe_next(conv, reply, Arc::new(VecSink::default())).await.unwrap();
    assert!(!f.provider.requests()[1].system_instruction.clone().unwrap_or_default().contains("powershell"));
}

// ---- context window and compaction ----

/// A roleplay with `count` alternating player/character messages already in it ("m0", "m1", ...).
fn roleplay_with_history(f: &Fixture, count: usize, filler: usize) -> (Uuid, Vec<ChatMessage>) {
    let nova = add_character(&f.db, "Nova", None, &[]);
    let conv = ready_character(f, &nova);
    no_clock(f, conv);
    for i in 0..count {
        let mut m = if i % 2 == 0 {
            ChatMessage::new(conv, MessageRole::User, format!("m{i} {}", "x".repeat(filler)))
        } else {
            let mut reply = ChatMessage::new(conv, MessageRole::Character, format!("m{i} {}", "x".repeat(filler)));
            reply.speaker_character_id = Some(nova.id);
            reply
        };
        m.timestamp = chrono::Utc::now() + chrono::Duration::milliseconds(i as i64);
        f.db.with(|c| conversations::add_message(c, &m)).unwrap();
    }
    (conv, messages(f, conv))
}

fn history_texts(request: &LlmRequest) -> Vec<String> {
    request.history.iter().map(|m| m.text.clone()).collect()
}

#[tokio::test]
async fn compacting_summarizes_all_but_the_newest_messages_and_later_replies_send_the_summary_instead() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Alex and Nova met at the inn."), finish("STOP")]),
        Script::Items(vec![chunk("Welcome back."), finish("STOP")]),
    ]);
    add_persona(&f.db, "Alex");
    let (conv, history) = roleplay_with_history(&f, 10, 0);

    let status = f.engine.compact(conv).await.unwrap();

    let ask = &f.provider.requests()[0].history[0].text;
    assert!(ask.contains("You are compacting the history"), "{ask}");
    assert!(ask.contains("Alex:\nm0") && ask.contains("Nova:\nm1") && ask.contains("m3"), "{ask}");
    assert!(!ask.contains("m4"), "the newest six stay out of the summary: {ask}");
    assert_eq!(status.summarized_count, 4);
    assert_eq!(status.compactable_count, 0);
    let summary = status.summary.unwrap();
    assert_eq!(summary.text, "Alex and Nova met at the inn.");
    assert_eq!(summary.covers_through_message_id, history[3].id);
    // The chat itself keeps every message.
    assert_eq!(messages(&f, conv).len(), 10);

    send(&f, conv, "hi").await.0.unwrap();
    let reply = &f.provider.requests()[1];
    let system = reply.system_instruction.clone().unwrap_or_default();
    assert!(system.contains("The story so far (older messages are no longer shown; this summary replaces them):\nAlex and Nova met at the inn."), "{system}");
    let sent = history_texts(reply);
    assert!(sent.iter().all(|t| !t.starts_with("m0") && !t.starts_with("m3")), "{sent:?}");
    assert!(sent[0].starts_with("m4") && sent.last().unwrap() == "hi", "{sent:?}");
}

#[tokio::test]
async fn compacting_again_merges_the_earlier_summary_and_clearing_sends_everything_again() {
    let f = fixture(vec![
        Script::Items(vec![chunk("First part."), finish("STOP")]),
        Script::Items(vec![chunk("First and second part."), finish("STOP")]),
    ]);
    let (conv, _) = roleplay_with_history(&f, 8, 0);
    f.engine.compact(conv).await.unwrap();
    for i in 8..12 {
        let m = ChatMessage::new(conv, MessageRole::User, format!("m{i}"));
        f.db.with(|c| conversations::add_message(c, &m)).unwrap();
    }

    let status = f.engine.compact(conv).await.unwrap();

    let ask = &f.provider.requests()[1].history[0].text;
    assert!(ask.contains("Summary of everything before these messages:\nFirst part."), "{ask}");
    assert!(ask.contains("m2") && ask.contains("m5") && !ask.contains("m1\n") && !ask.contains("m6"), "{ask}");
    assert_eq!(status.summary.unwrap().text, "First and second part.");
    assert_eq!(status.summarized_count, 6);

    let cleared = f.engine.clear_summary(conv).await.unwrap();
    assert!(cleared.summary.is_none());
    assert_eq!(cleared.summarized_count, 0);
    assert_eq!(cleared.compactable_count, 6);
}

#[tokio::test]
async fn there_is_nothing_to_compact_in_a_short_chat() {
    let f = fixture(vec![]);
    let (conv, _) = roleplay_with_history(&f, 4, 0);
    let err = f.engine.compact(conv).await.unwrap_err();
    assert!(err.to_string().contains("Nothing to compact"), "{err}");
    assert!(f.provider.requests().is_empty());
}

#[tokio::test]
async fn a_request_nearing_the_window_is_compacted_before_the_reply() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Summary."), finish("STOP")]),
        Script::Items(vec![chunk("Reply."), finish("STOP")]),
    ]);
    // Ten 400-character messages are about 1000 tokens: well past 80% of a 1000-token window.
    let (conv, _) = roleplay_with_history(&f, 10, 400);
    *f.provider.window.lock().unwrap() = Some(1000);

    let before = f.engine.context_status(conv).await.unwrap();
    assert_eq!((before.window, before.window_source), (Some(1000), Some("provider")));
    assert!(before.estimated && before.used > 1000, "{before:?}");

    let (result, events) = send(&f, conv, "hi").await;
    result.unwrap();

    assert!(events.iter().any(|e| matches!(e, ChatEvent::Compacting { .. })), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Compacted { .. })), "{events:?}");
    let requests = f.provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].history[0].text.contains("You are compacting"));
    assert!(requests[1].system_instruction.clone().unwrap_or_default().contains("Summary."));
    // The user's new message is among the newest six, so it was sent in full, not summarized.
    assert_eq!(requests[1].history.last().unwrap().text, "hi");

    let after = f.engine.context_status(conv).await.unwrap();
    assert!(after.used < before.used, "{after:?}");
}

#[tokio::test]
async fn a_small_chat_is_never_compacted_and_a_failed_compaction_still_replies() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Reply."), finish("STOP")]),
        Script::FailToStart(ProviderError::new("boom", ProviderErrorKind::Unknown)),
        Script::Items(vec![chunk("Reply anyway."), finish("STOP")]),
    ]);
    let (conv, _) = roleplay_with_history(&f, 2, 0);
    *f.provider.window.lock().unwrap() = Some(100_000);
    let (_, events) = send(&f, conv, "hi").await;
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::Compacting { .. })));

    for i in 0..10 {
        let m = ChatMessage::new(conv, MessageRole::User, format!("long {i} {}", "y".repeat(400)));
        f.db.with(|c| conversations::add_message(c, &m)).unwrap();
    }
    *f.provider.window.lock().unwrap() = Some(1000);
    let (result, events) = send(&f, conv, "again").await;

    result.unwrap();
    assert!(error_messages(&events).iter().any(|m| m.starts_with("Couldn't compact the chat")), "{events:?}");
    assert_eq!(messages(&f, conv).last().unwrap().text, "Reply anyway.");
}

#[tokio::test]
async fn the_status_prefers_the_providers_count_unless_it_is_below_the_estimate() {
    let f = fixture(vec![]);
    let (conv, history) = roleplay_with_history(&f, 2, 0);
    let mut reply = history[1].clone();
    reply.usage = Some(LlmUsage { prompt_tokens: Some(4000), completion_tokens: Some(200), total_tokens: Some(4200) });
    f.db.with(|c| {
        c.execute("UPDATE ChatMessages SET PromptTokens = 4000, CompletionTokens = 200, TotalTokens = 4200 WHERE Id = ?1;", [reply.id.to_string()])?;
        Ok(())
    })
    .unwrap();

    let status = f.engine.context_status(conv).await.unwrap();
    assert_eq!((status.used, status.estimated), (4200, false));

    f.db.with(|c| {
        c.execute("UPDATE ChatMessages SET PromptTokens = 1, CompletionTokens = 1, TotalTokens = 2 WHERE Id = ?1;", [reply.id.to_string()])?;
        Ok(())
    })
    .unwrap();
    let status = f.engine.context_status(conv).await.unwrap();
    assert!(status.estimated && status.used > 2, "{status:?}");
    assert_eq!(status.window, None);
}
// ---- appearance polish ----

#[tokio::test]
async fn polish_appearance_sends_the_text_to_the_last_selected_model_and_returns_the_rewrite() {
    let f = fixture(vec![Script::Items(vec![chunk("  {{char}} is a slender woman with auburn waves.\n"), finish("STOP")])]);
    f.db.with(|c| {
        settings::set_last_selected_provider(c, "fake")?;
        settings::set_last_selected_model(c, "fake", "fake-model")
    })
    .unwrap();

    let polished = f.engine.polish_appearance("{{char}} is a woman. She has auburn hair (wavy).").await.unwrap();
    assert_eq!(polished, "{{char}} is a slender woman with auburn waves.");

    let request = &f.provider.requests()[0];
    assert_eq!(request.model, "fake-model");
    assert!(request.history[0].text.ends_with("{{char}} is a woman. She has auburn hair (wavy)."), "{}", request.history[0].text);
    assert!(request.history[0].text.contains("{{char}} and {{user}} exactly as written"), "{}", request.history[0].text);
}

#[tokio::test]
async fn polish_appearance_refuses_empty_text_and_empty_replies() {
    let f = fixture(vec![Script::Items(vec![chunk("   "), finish("STOP")])]);
    f.db.with(|c| settings::set_last_selected_model(c, "fake", "fake-model")).unwrap();
    assert!(f.engine.polish_appearance("  ").await.is_err());
    assert!(f.provider.requests().is_empty(), "nothing is sent for empty text");
    assert!(f.engine.polish_appearance("{{char}} is a man.").await.is_err());
}

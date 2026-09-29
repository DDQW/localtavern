//! The chat engine: opens and creates conversations, and generates, regenerates, swipes, edits and
//! deletes messages. It mirrors the C# `ChatViewModel`, minus everything UI-shaped - progress is
//! reported through an [`EventSink`] and the UI just renders what it's told.
//!
//! Not yet ported (later steps): group-chat speaker selection, Continue, Impersonate, the PowerShell
//! tool loop, and the background memory / quest-log / notebook extraction.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use rusqlite::Connection;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::events::{ChatEvent, EventSink};
use super::images;
use super::reply::{
    advance_world_state, append_instruction, check_blocked, extract_matched_emotion, format_prompt, random_unit,
    user_message,
};
use super::session::{build_session, default_persona, load_reply_context, ChatSession, ReplyContext};
use crate::db::Db;
use crate::error::{Error, Result};
use crate::llm::{LlmMessage, LlmRequest, LlmRole};
use crate::models::{
    Character, ChatMessage, Conversation, ConversationMode, LlmUsage, MessageAttachment, MessageRole,
};
use crate::prompting::{self, emotion_tag, time_skip_tag, world_state, KeywordWorldInfoScanner, PromptAssemblyContext};
use crate::providers::{LlmGeneratedImage, LlmProviderRegistry, ProviderError};
use crate::repo::{characters, conversations, notebook, quest_log, user_memories};
use crate::settings;

/// How often streamed text is pushed to the UI. Rendering faster than the display refreshes buys
/// nothing, so deltas accumulate and go out on a timer (SillyTavern's "streaming_fps").
const STREAM_FLUSH_INTERVAL: Duration = Duration::from_millis(33);

struct Inner {
    db: Db,
    providers: Arc<LlmProviderRegistry>,
    images_dir: PathBuf,
    active: Mutex<HashMap<Uuid, CancellationToken>>,
}

#[derive(Clone)]
pub struct ChatEngine {
    inner: Arc<Inner>,
}

/// How a streamed reply ended. Whatever text arrived before a stop or failure is kept.
enum StreamEnd {
    Completed,
    Cancelled,
    Failed(ProviderError),
}

struct StreamResult {
    text: String,
    reasoning: String,
    usage: Option<LlmUsage>,
    images: Vec<LlmGeneratedImage>,
    end: StreamEnd,
}

impl StreamResult {
    fn failed(error: ProviderError) -> Self {
        Self { text: String::new(), reasoning: String::new(), usage: None, images: vec![], end: StreamEnd::Failed(error) }
    }
}

fn nonempty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

impl ChatEngine {
    pub fn new(db: Db, providers: Arc<LlmProviderRegistry>, images_dir: PathBuf) -> Self {
        Self { inner: Arc::new(Inner { db, providers, images_dir, active: Mutex::new(HashMap::new()) }) }
    }

    fn db(&self) -> &Db {
        &self.inner.db
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.inner.providers.provider_ids()
    }

    // ------------------------------------------------------------------ opening

    fn session(&self, conversation_id: Uuid) -> Result<ChatSession> {
        let providers = self.provider_ids();
        self.db().with(|c| build_session(c, &providers, conversation_id))
    }

    /// The dedicated Assistant view: resumes your most recent Assistant-mode conversation, or starts
    /// a fresh one.
    pub fn open_assistant(&self, force_new: bool) -> Result<ChatSession> {
        let id = self.db().with(|c| {
            if !force_new {
                if let Some(existing) = conversations::get_most_recent_assistant(c)? {
                    return Ok(existing.id);
                }
            }
            let persona = default_persona(c)?;
            let (provider_id, model) = self.default_provider_and_model(c)?;
            let conversation = Conversation {
                title: "Assistant chat".into(),
                mode: ConversationMode::Assistant,
                persona_id: persona.map(|p| p.id),
                provider_id,
                model,
                ..Default::default()
            };
            conversations::upsert(c, &conversation)?;
            Ok(conversation.id)
        })?;
        self.session(id)
    }

    /// A 1:1 roleplay: resumes the latest conversation with this character, or starts one with the
    /// character's greeting already in place.
    pub fn open_character(&self, character_id: Uuid, force_new: bool) -> Result<ChatSession> {
        let id = self.db().with(|c| {
            if !force_new {
                if let Some(existing) = conversations::get_most_recent_by_character(c, character_id)? {
                    return Ok(existing.id);
                }
            }
            let character = characters::get(c, character_id)?.ok_or_else(|| Error::Other("Character not found.".into()))?;
            let persona = default_persona(c)?;
            let (provider_id, model) = self.default_provider_and_model(c)?;
            let conversation = Conversation {
                title: format!("Chat with {}", character.name),
                mode: ConversationMode::Roleplay,
                character_id: Some(character.id),
                persona_id: persona.as_ref().map(|p| p.id),
                provider_id,
                model,
                ..Default::default()
            };
            conversations::upsert(c, &conversation)?;

            if let Some(first) = character.first_messages.first() {
                let mut greeting = ChatMessage::new(
                    conversation.id,
                    MessageRole::Character,
                    prompting::apply_macros(first, persona.as_ref(), std::slice::from_ref(&character)),
                );
                greeting.speaker_character_id = Some(character.id);
                greeting.finalize_generation();
                conversations::add_message(c, &greeting)?;
            }
            Ok(conversation.id)
        })?;
        self.session(id)
    }

    pub fn resume(&self, conversation_id: Uuid) -> Result<ChatSession> {
        self.session(conversation_id)
    }

    /// "New chat" in whatever scope is currently open (the Assistant, or the same character).
    pub fn new_chat_like(&self, conversation_id: Uuid) -> Result<ChatSession> {
        let conversation = self
            .db()
            .with(|c| conversations::get(c, conversation_id))?
            .ok_or_else(|| Error::Other("Conversation not found.".into()))?;
        if let Some(character_id) = conversation.character_id {
            self.open_character(character_id, true)
        } else if conversation.group_chat_id.is_some() {
            Err(Error::Other("Group chats aren't supported in this build yet.".into()))
        } else {
            self.open_assistant(true)
        }
    }

    /// The user's last-selected provider (if still known) and model for a brand-new conversation.
    fn default_provider_and_model(&self, conn: &Connection) -> Result<(String, String)> {
        let ids = self.provider_ids();
        let provider_id = settings::last_selected_provider(conn)?
            .filter(|p| ids.contains(p))
            .or_else(|| ids.first().cloned())
            .unwrap_or_default();
        let model = match settings::last_selected_model(conn, &provider_id)? {
            Some(m) => m,
            None => settings::cached_models(conn, &provider_id)?.into_iter().next().unwrap_or_default(),
        };
        Ok((provider_id, model))
    }

    // ------------------------------------------------------------------ conversation settings

    fn update_conversation(&self, id: Uuid, change: impl FnOnce(&mut Conversation)) -> Result<Conversation> {
        self.db().with(|c| {
            let mut conversation = conversations::get(c, id)?.ok_or_else(|| Error::Other("Conversation not found.".into()))?;
            change(&mut conversation);
            conversations::upsert(c, &conversation)?;
            Ok(conversation)
        })
    }

    pub fn set_model(&self, conversation_id: Uuid, provider_id: &str, model: &str) -> Result<Conversation> {
        let conversation = self.update_conversation(conversation_id, |c| {
            c.provider_id = provider_id.to_string();
            c.model = model.to_string();
        })?;
        self.db().with(|c| {
            settings::set_last_selected_provider(c, provider_id)?;
            settings::set_last_selected_model(c, provider_id, model)
        })?;
        Ok(conversation)
    }

    pub fn set_preset(&self, conversation_id: Uuid, preset_id: Uuid) -> Result<Conversation> {
        self.update_conversation(conversation_id, |c| c.prompt_preset_id = Some(preset_id))
    }

    pub fn set_persona(&self, conversation_id: Uuid, persona_id: Option<Uuid>) -> Result<Conversation> {
        self.update_conversation(conversation_id, |c| c.persona_id = persona_id)
    }

    pub fn set_assistant_prompt(&self, conversation_id: Uuid, text: &str) -> Result<Conversation> {
        self.update_conversation(conversation_id, |c| c.assistant_system_prompt = Some(text.to_string()))
    }

    pub fn set_search_override(&self, conversation_id: Uuid, enabled: Option<bool>) -> Result<Conversation> {
        self.update_conversation(conversation_id, |c| c.enable_google_search = enabled)
    }

    pub fn delete_conversation(&self, conversation_id: Uuid) -> Result<()> {
        self.db().with(|c| conversations::delete(c, conversation_id))
    }

    // ------------------------------------------------------------------ message edits

    pub fn edit_message(&self, message_id: Uuid, text: &str) -> Result<ChatMessage> {
        self.db().with(|c| {
            let mut message = conversations::get_message(c, message_id)?.ok_or_else(|| Error::Other("Message not found.".into()))?;
            message.set_text(text);
            conversations::update_message(c, &message)?;
            Ok(message)
        })
    }

    pub fn delete_message(&self, message_id: Uuid) -> Result<()> {
        self.db().with(|c| conversations::delete_message(c, message_id))
    }

    pub fn swipe_previous(&self, message_id: Uuid) -> Result<ChatMessage> {
        self.db().with(|c| {
            let mut message = conversations::get_message(c, message_id)?.ok_or_else(|| Error::Other("Message not found.".into()))?;
            if message.can_swipe_previous() {
                message.swipe_to((message.active_alternate_index - 1) as usize);
                conversations::update_message(c, &message)?;
            }
            Ok(message)
        })
    }

    /// Stops the reply in flight for this conversation, keeping whatever already streamed in.
    pub fn stop(&self, conversation_id: Uuid) {
        if let Some(token) = self.inner.active.lock().unwrap().get(&conversation_id) {
            token.cancel();
        }
    }

    pub fn is_generating(&self, conversation_id: Uuid) -> bool {
        self.inner.active.lock().unwrap().contains_key(&conversation_id)
    }

    // ------------------------------------------------------------------ generation entry points

    /// Sends a message (or, when `text` is empty and the last message is the user's own, retries
    /// that one) and streams the reply.
    pub async fn send_message(
        &self,
        conversation_id: Uuid,
        text: String,
        attachments: Vec<String>,
        sink: Arc<dyn EventSink>,
    ) -> Result<()> {
        let engine = self.clone();
        let inner_sink = sink.clone();
        self.run_guarded(conversation_id, sink, move |token| async move {
            engine.send_inner(conversation_id, text, attachments, inner_sink, token).await
        })
        .await
    }

    /// Swiping right: moves to the next existing alternate, or generates a new one when already at
    /// the last (SillyTavern-style - there is no separate "Regenerate" button).
    pub async fn swipe_next(&self, conversation_id: Uuid, message_id: Uuid, sink: Arc<dyn EventSink>) -> Result<()> {
        let existing = self.db().with(|c| {
            let mut message = conversations::get_message(c, message_id)?.ok_or_else(|| Error::Other("Message not found.".into()))?;
            if message.can_swipe_to_existing_next() {
                message.swipe_to((message.active_alternate_index + 1) as usize);
                conversations::update_message(c, &message)?;
                return Ok(Some(message));
            }
            Ok(None)
        })?;
        if let Some(message) = existing {
            sink.emit(ChatEvent::MessageUpdated { message });
            sink.emit(ChatEvent::Finished);
            return Ok(());
        }

        let engine = self.clone();
        let inner_sink = sink.clone();
        self.run_guarded(conversation_id, sink, move |token| async move {
            engine.regenerate_inner(conversation_id, message_id, inner_sink, token).await
        })
        .await
    }

    /// Runs one operation as the conversation's only active generation, reporting failure and
    /// completion through the sink whatever happens.
    async fn run_guarded<F, Fut>(&self, conversation_id: Uuid, sink: Arc<dyn EventSink>, operation: F) -> Result<()>
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let token = CancellationToken::new();
        {
            let mut active = self.inner.active.lock().unwrap();
            if active.contains_key(&conversation_id) {
                return Err(Error::Other("A reply is already being generated for this chat.".into()));
            }
            active.insert(conversation_id, token.clone());
        }

        let result = operation(token).await;
        self.inner.active.lock().unwrap().remove(&conversation_id);

        if let Err(error) = &result {
            sink.emit(ChatEvent::Error { message: error.to_string(), kind: None });
        }
        sink.emit(ChatEvent::Finished);
        result
    }

    async fn send_inner(
        &self,
        conversation_id: Uuid,
        text: String,
        attachments: Vec<String>,
        sink: Arc<dyn EventSink>,
        token: CancellationToken,
    ) -> Result<()> {
        let ctx = self.db().with(|c| load_reply_context(c, conversation_id))?;
        if ctx.conversation.group_chat_id.is_some() {
            return Err(Error::Other("Group chats aren't supported in this build yet.".into()));
        }
        if ctx.conversation.model.trim().is_empty() {
            return Err(Error::Other("No model selected - pick one, or refresh the model list in Settings.".into()));
        }

        // An empty box with an unanswered user message means "retry this" (typically right after
        // deleting the character's reply) - reuse that message instead of requiring it be retyped.
        let awaiting_reply = ctx.messages.last().is_some_and(|m| m.role == MessageRole::User);
        let is_retry = text.trim().is_empty() && attachments.is_empty() && awaiting_reply;
        if !is_retry {
            if text.trim().is_empty() && attachments.is_empty() {
                return Err(Error::Other("Nothing to send.".into()));
            }
            let mut user = ChatMessage::new(conversation_id, MessageRole::User, text.trim());
            user.attachments = attachments
                .iter()
                .map(|p| MessageAttachment { image_path: p.clone(), mime_type: images::mime_from_extension(p).to_string() })
                .collect();
            self.db().with(|c| conversations::add_message(c, &user))?;
            sink.emit(ChatEvent::MessageAdded { message: user });
        }

        let speaker_id = (ctx.characters.len() == 1).then(|| ctx.characters[0].id);
        self.generate_new(conversation_id, speaker_id, &sink, &token).await
    }

    // ------------------------------------------------------------------ generation

    /// Builds the provider request for a turn: assembled prompt plus the per-turn instructions
    /// (emotion tag for characters with an expression pack, time-skip tag while a world clock runs).
    fn assemble(&self, ctx: &ReplyContext, history: &[ChatMessage], speaker: Option<&Character>) -> Result<LlmRequest> {
        let conversation = &ctx.conversation;
        let (memories, notebook_entries, quests) = self.db().with(|c| {
            let memories = if conversation.mode == ConversationMode::Assistant { user_memories::get_all(c)? } else { vec![] };
            let (notes, quests) = if conversation.mode == ConversationMode::Roleplay {
                (notebook::get_by_conversation(c, conversation.id)?, quest_log::get_by_conversation(c, conversation.id)?)
            } else {
                (vec![], vec![])
            };
            Ok((memories, notes, quests))
        })?;

        let world_text = (conversation.mode == ConversationMode::Roleplay)
            .then(|| conversation.world_state.as_ref().map(world_state::format))
            .flatten();

        let mut pctx = PromptAssemblyContext::new(conversation, &ctx.preset, history)
            .with_characters(&ctx.characters)
            .with_lorebooks(&ctx.lorebooks)
            .with_user_memories(&memories)
            .with_notebook_entries(&notebook_entries)
            .with_quest_log(&quests);
        if let Some(persona) = &ctx.persona {
            pctx = pctx.with_persona(persona);
        }
        if let Some(text) = world_text.as_deref() {
            pctx = pctx.with_world_state_text(text);
        }

        let mut request = prompting::assemble(&pctx, &KeywordWorldInfoScanner);

        if let Some(character) = speaker.filter(|c| !c.expressions.is_empty()) {
            let keys: Vec<String> = character.expressions.iter().map(|e| e.emotion.clone()).collect();
            append_instruction(&mut request, &emotion_tag::build_instruction(&character.name, &keys));
        }
        if conversation.mode == ConversationMode::Roleplay && conversation.world_state.is_some() {
            append_instruction(&mut request, time_skip_tag::build_instruction());
        }
        Ok(request)
    }

    /// Streams one request, pushing coalesced deltas to the sink. Whatever arrived before a stop or
    /// failure is returned along with how it ended.
    async fn stream_reply(
        &self,
        provider_id: &str,
        request: LlmRequest,
        message_id: Uuid,
        sink: &Arc<dyn EventSink>,
        token: &CancellationToken,
    ) -> StreamResult {
        let provider = match self.inner.providers.get(provider_id) {
            Ok(p) => p,
            Err(e) => return StreamResult::failed(e),
        };

        let started = tokio::select! {
            biased;
            _ = token.cancelled() => None,
            started = provider.stream_completion(request) => Some(started),
        };
        let mut stream = match started {
            None => {
                return StreamResult { end: StreamEnd::Cancelled, ..StreamResult::failed(ProviderError::unknown("")) };
            }
            Some(Err(e)) => return StreamResult::failed(e),
            Some(Ok(stream)) => stream,
        };

        let mut result = StreamResult {
            text: String::new(),
            reasoning: String::new(),
            usage: None,
            images: vec![],
            end: StreamEnd::Completed,
        };
        let (mut pending_text, mut pending_reasoning) = (String::new(), String::new());
        let mut finish_reason: Option<String> = None;
        let mut ticker = tokio::time::interval(STREAM_FLUSH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let flush = |text: &mut String, reasoning: &mut String| {
            if !text.is_empty() || !reasoning.is_empty() {
                sink.emit(ChatEvent::Delta {
                    message_id,
                    text: std::mem::take(text),
                    reasoning: std::mem::take(reasoning),
                });
            }
        };

        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => {
                    result.end = StreamEnd::Cancelled;
                    break;
                }
                _ = ticker.tick() => flush(&mut pending_text, &mut pending_reasoning),
                item = stream.next() => match item {
                    None => break,
                    Some(Err(e)) => {
                        result.end = StreamEnd::Failed(e);
                        break;
                    }
                    Some(Ok(chunk)) => {
                        result.text.push_str(&chunk.delta_text);
                        result.reasoning.push_str(&chunk.thought_delta_text);
                        pending_text.push_str(&chunk.delta_text);
                        pending_reasoning.push_str(&chunk.thought_delta_text);
                        if chunk.usage.is_some() {
                            result.usage = chunk.usage;
                        }
                        if chunk.finish_reason.is_some() {
                            finish_reason = chunk.finish_reason;
                        }
                        result.images.extend(chunk.images.into_iter().flatten());
                    }
                },
            }
        }
        flush(&mut pending_text, &mut pending_reasoning);

        // A blocked reply otherwise looks like an empty success; surface it as an error (keeping
        // any partial text).
        if matches!(result.end, StreamEnd::Completed) {
            if let Some(blocked) = check_blocked(provider_id, finish_reason.as_deref()) {
                result.end = StreamEnd::Failed(blocked);
            }
        }
        result
    }

    /// Saves model-generated images to local storage and returns them as attachments.
    fn save_generated_images(&self, generated: &[LlmGeneratedImage]) -> Vec<MessageAttachment> {
        generated
            .iter()
            .filter_map(|image| {
                images::store_bytes(&self.inner.images_dir, &image.data, &image.mime_type).ok().map(|path| MessageAttachment {
                    image_path: path.to_string_lossy().into_owned(),
                    mime_type: image.mime_type.clone(),
                })
            })
            .collect()
    }

    /// Strips the time-skip tag and, in a roleplay with a world clock, advances and persists it.
    fn apply_world_clock(&self, ctx: &ReplyContext, raw_text: String, sink: &Arc<dyn EventSink>) -> Result<String> {
        let Some(state) = ctx.conversation.world_state.as_ref().filter(|_| ctx.conversation.mode == ConversationMode::Roleplay) else {
            return Ok(raw_text);
        };
        let (clean, next) = advance_world_state(state, &raw_text, &mut random_unit);
        self.db().with(|c| conversations::update_world_state(c, ctx.conversation.id, Some(&next)))?;
        sink.emit(ChatEvent::WorldStateChanged { conversation_id: ctx.conversation.id, world_state: next });
        Ok(clean)
    }

    fn emit_failure(&self, error: &ProviderError, sink: &Arc<dyn EventSink>) {
        sink.emit(ChatEvent::Error { message: user_message(error), kind: Some(error.kind) });
    }

    async fn generate_new(
        &self,
        conversation_id: Uuid,
        speaker_id: Option<Uuid>,
        sink: &Arc<dyn EventSink>,
        token: &CancellationToken,
    ) -> Result<()> {
        let ctx = self.db().with(|c| load_reply_context(c, conversation_id))?;
        let speaker = speaker_id.and_then(|id| ctx.characters.iter().find(|c| c.id == id)).cloned();

        let mut message = ChatMessage::new(conversation_id, MessageRole::Character, "");
        message.speaker_character_id = speaker_id;
        sink.emit(ChatEvent::MessageAdded { message: message.clone() });

        let request = self.assemble(&ctx, &ctx.messages, speaker.as_ref())?;
        message.raw_prompt_text = Some(format_prompt(&request));
        self.update_conversation(conversation_id, |c| c.updated_at = Utc::now())?;

        let result = self.stream_reply(&ctx.conversation.provider_id, request, message.id, sink, token).await;
        let StreamResult { text, reasoning, usage, images: generated, end } = result;

        let succeeded = matches!(end, StreamEnd::Completed);
        if succeeded {
            message.usage = usage;
            let after_clock = self.apply_world_clock(&ctx, text, sink)?;
            let (clean, emotion) = extract_matched_emotion(speaker.as_ref(), &after_clock);
            message.text = clean;
            message.detected_emotion = emotion;
        } else {
            // Stopped or failed: keep whatever streamed in, untouched by tag processing.
            message.text = text;
        }
        message.reasoning_text = nonempty(reasoning);

        if succeeded && !generated.is_empty() {
            message.attachments = self.save_generated_images(&generated);
        }

        if message.text.is_empty() && message.reasoning_text.is_none() && message.attachments.is_empty() {
            sink.emit(ChatEvent::MessageRemoved { message_id: message.id });
        } else {
            message.finalize_generation();
            self.db().with(|c| conversations::add_message(c, &message))?;
            sink.emit(ChatEvent::MessageUpdated { message: message.clone() });
        }

        match end {
            StreamEnd::Failed(error) => self.emit_failure(&error, sink),
            StreamEnd::Cancelled => {}
            StreamEnd::Completed => {
                // Only the conversation's very first exchange gets auto-titled.
                if ctx.conversation.mode == ConversationMode::Assistant && ctx.messages.len() == 1 {
                    let user_text = ctx.messages.last().map(|m| m.text.clone()).unwrap_or_default();
                    self.spawn_title(&ctx.conversation, user_text, message.text.clone(), sink.clone());
                }
            }
        }
        Ok(())
    }

    async fn regenerate_inner(
        &self,
        conversation_id: Uuid,
        message_id: Uuid,
        sink: Arc<dyn EventSink>,
        token: CancellationToken,
    ) -> Result<()> {
        let ctx = self.db().with(|c| load_reply_context(c, conversation_id))?;
        let position = ctx
            .messages
            .iter()
            .position(|m| m.id == message_id)
            .ok_or_else(|| Error::Other("Message not found.".into()))?;
        let mut target = ctx.messages[position].clone();
        if target.role != MessageRole::Character {
            return Err(Error::Other("Only the character's replies can be regenerated.".into()));
        }
        if ctx.conversation.model.trim().is_empty() {
            return Err(Error::Other("No model selected - pick one, or refresh the model list in Settings.".into()));
        }

        let speaker = target.speaker_character_id.and_then(|id| ctx.characters.iter().find(|c| c.id == id)).cloned();
        let request = self.assemble(&ctx, &ctx.messages[..position], speaker.as_ref())?;
        let raw_prompt = format_prompt(&request);

        sink.emit(ChatEvent::Regenerating { message_id });
        let result = self.stream_reply(&ctx.conversation.provider_id, request, message_id, &sink, &token).await;
        let StreamResult { text, reasoning, usage, images: generated, end } = result;

        let succeeded = matches!(end, StreamEnd::Completed);
        let reasoning = nonempty(reasoning);
        if succeeded {
            target.usage = usage;
            let after_clock = self.apply_world_clock(&ctx, text, &sink)?;
            let (clean, emotion) = extract_matched_emotion(speaker.as_ref(), &after_clock);
            target.add_alternate(clean, reasoning, Some(raw_prompt));
            target.detected_emotion = emotion;
            if !generated.is_empty() {
                target.attachments = self.save_generated_images(&generated);
            }
        } else if !text.is_empty() || reasoning.is_some() {
            // Stopping (or failing) keeps the partial regeneration as its own alternate, so there is
            // still something to swipe between rather than silently reverting.
            target.add_alternate(text, reasoning, Some(raw_prompt));
        }

        self.db().with(|c| conversations::update_message(c, &target))?;
        // Also restores the original text on the UI when nothing new arrived.
        sink.emit(ChatEvent::MessageUpdated { message: target });

        if let StreamEnd::Failed(error) = end {
            self.emit_failure(&error, &sink);
        }
        Ok(())
    }

    /// Best-effort background enrichment after a conversation's first exchange: asks the model for a
    /// short topic title so the History picker shows what was discussed. Failures never surface.
    fn spawn_title(&self, conversation: &Conversation, user_text: String, assistant_text: String, sink: Arc<dyn EventSink>) {
        let engine = self.clone();
        let (conversation_id, provider_id, model) = (conversation.id, conversation.provider_id.clone(), conversation.model.clone());
        tokio::spawn(async move {
            let prompt = format!(
                "Summarize the TOPIC of this exchange in 3-6 words, suitable as a short chat title (like a browser tab title). \
No ending punctuation, no surrounding quotes, no \"Chat about\"/\"Discussion of\" prefix - just the topic itself.\n\n\
User: {user_text}\nAssistant: {assistant_text}"
            );
            let request = LlmRequest {
                model,
                system_instruction: None,
                history: vec![LlmMessage::new(LlmRole::User, prompt)],
                generation_config: Default::default(),
                enable_web_search: None,
            };
            let Ok(provider) = engine.inner.providers.get(&provider_id) else { return };
            let Ok(response) = provider.get_completion(request).await else { return };

            let title = response.delta_text.trim().trim_matches(['"', '\'', '.', ' ']).to_string();
            if title.is_empty() {
                return;
            }
            if engine.update_conversation(conversation_id, |c| c.title = title.clone()).is_ok() {
                sink.emit(ChatEvent::TitleChanged { conversation_id, title });
            }
        });
    }
}

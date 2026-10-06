//! The chat engine: opens and creates conversations, and generates, regenerates, swipes, edits and
//! deletes messages. It mirrors the C# `ChatViewModel`, minus everything UI-shaped - progress is
//! reported through an [`EventSink`] and the UI just renders what it's told.
//!
//! Not yet ported (later steps): the PowerShell tool loop and the background memory / quest-log /
//! notebook extraction.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::oneshot;
use rusqlite::Connection;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::events::{ChatEvent, EventSink};
use super::extraction::{Exchange, Extractor};
use super::images;
use super::reply::{
    advance_world_state, append_instruction, check_blocked, extract_matched_emotion, format_prompt, random_unit,
    user_message,
};
use super::session::{self, build_session, load_reply_context, ChatSession, ReplyContext};
use crate::db::Db;
use crate::error::{Error, Result};
use crate::group_chats::strategy_for;
use crate::llm::{LlmMessage, LlmRequest, LlmRole};
use crate::models::{
    Character, ChatMessage, Conversation, ConversationMode, LlmUsage, MessageAttachment, MessageRole, Persona,
    PromptBlockType, PromptPreset, WorldState,
};
use crate::powershell::exec::{CommandRunner, ExecError, ExecutionResult, Pwsh};
use crate::powershell::filter::ToolCallFilter;
use crate::powershell::settings as tool_settings;
use crate::prompting::powershell_tool::{self, PowerShellToolCallRequest, ToolResult};
use crate::prompting::{self, emotion_tag, time_skip_tag, world_state, KeywordWorldInfoScanner, PromptAssemblyContext};
use crate::providers::{LlmGeneratedImage, LlmProviderRegistry, ProviderError};
use crate::repo::{characters, conversations, group_chats, notebook, quest_log, user_memories};
use crate::settings;

/// How often streamed text is pushed to the UI. Rendering faster than the display refreshes buys
/// nothing, so deltas accumulate and go out on a timer (SillyTavern's "streaming_fps").
const STREAM_FLUSH_INTERVAL: Duration = Duration::from_millis(33);

struct Inner {
    db: Db,
    providers: Arc<LlmProviderRegistry>,
    images_dir: PathBuf,
    active: Mutex<HashMap<Uuid, CancellationToken>>,
    /// Whether finished replies trigger the background memory / quest / notebook requests.
    extraction: AtomicBool,
    /// Runs the assistant's PowerShell commands.
    runner: Mutex<Arc<dyn CommandRunner>>,
    /// Tool calls waiting for the user's yes/no, by request id.
    approvals: Mutex<HashMap<Uuid, oneshot::Sender<ToolDecision>>>,
}

/// The user's answer to a tool-call approval request.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDecision {
    pub allow: bool,
    /// With `allow`: also remember this pattern so matching commands run without asking next time.
    pub always_pattern: Option<String>,
}

/// Most tool round trips one turn may take before the assistant is cut off.
const MAX_TOOL_ITERATIONS: usize = 5;

/// What one model turn came to.
enum Turn {
    /// Finished (`true`) or stopped / failed (`false`).
    Done(bool),
    /// The reply ended in a tool call; the partial reply before it was kept.
    ToolCall(PowerShellToolCallRequest),
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
    /// The reply asked to run a command; the stream was stopped there.
    ToolCall(PowerShellToolCallRequest),
}

struct StreamResult {
    text: String,
    /// What was streamed to the UI - `text` minus a tool-call tag being withheld.
    shown_text: String,
    reasoning: String,
    usage: Option<LlmUsage>,
    images: Vec<LlmGeneratedImage>,
    end: StreamEnd,
}

impl StreamResult {
    fn failed(error: ProviderError) -> Self {
        Self {
            text: String::new(),
            shown_text: String::new(),
            reasoning: String::new(),
            usage: None,
            images: vec![],
            end: StreamEnd::Failed(error),
        }
    }
}

/// Marks a conversation as generating; dropping it (however the operation ends) frees the slot.
struct ActiveGuard<'a> {
    active: &'a Mutex<HashMap<Uuid, CancellationToken>>,
    conversation_id: Uuid,
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.active.lock().unwrap().remove(&self.conversation_id);
    }
}

/// Per-request choices for [`ChatEngine::assemble`].
#[derive(Default)]
struct AssembleOptions<'a> {
    /// Replaces the conversation's own preset.
    preset: Option<&'a PromptPreset>,
    /// Whose reply this is; a speaker with an expression pack is asked to tag its emotion.
    speaker: Option<&'a Character>,
    /// Ask for a `[time-skip: ...]` tag (only where the reply's tag is stripped again).
    time_skip: bool,
    /// Offer the PowerShell tool (Assistant chats only).
    tools: bool,
}

/// The nudge Continue appends as a trailing user turn, so the model extends rather than restarts.
const CONTINUE_NUDGE: &str = "[Continue your last message without repeating its original content.]";

/// An [`ExecutionResult`] as the text the model (and the tool bubble) gets.
fn format_execution(result: &ExecutionResult) -> String {
    let tool_result = ToolResult {
        command: &result.command,
        elevated: result.elevated,
        exit_code: result.exit_code,
        std_out: &result.std_out,
        std_err: &result.std_err,
        timed_out: result.timed_out,
        declined: result.uac_declined,
    };
    powershell_tool::format_result_for_model(&tool_result)
}

/// The most recent message the user wrote (what the reply after it was answering).
fn last_user_text(messages: &[ChatMessage]) -> String {
    messages.iter().rev().find(|m| m.role == MessageRole::User).map(|m| m.text.clone()).unwrap_or_default()
}

/// Background work has nobody to tell, so a failure only goes to stderr.
fn report(what: &str, result: Result<()>) {
    if let Err(error) = result {
        eprintln!("{what} failed: {error}");
    }
}

fn nonempty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// The character's opening line, with `{{user}}` / `{{char}}` filled in. No-op when the card has none.
fn push_greeting(
    conn: &Connection,
    conversation_id: Uuid,
    character: &Character,
    persona: Option<&Persona>,
) -> Result<()> {
    let Some(first) = character.first_messages.first() else {
        return Ok(());
    };
    let mut greeting = ChatMessage::new(
        conversation_id,
        MessageRole::Character,
        prompting::apply_macros(first, persona, std::slice::from_ref(character)),
    );
    greeting.speaker_character_id = Some(character.id);
    greeting.finalize_generation();
    conversations::add_message(conn, &greeting)
}

impl ChatEngine {
    pub fn new(db: Db, providers: Arc<LlmProviderRegistry>, images_dir: PathBuf) -> Self {
        Self { inner: Arc::new(Inner { db, providers, images_dir, active: Mutex::new(HashMap::new()),
                extraction: AtomicBool::new(true),
                runner: Mutex::new(Arc::new(Pwsh::default())),
                approvals: Mutex::new(HashMap::new()),
            }) }
    }

    fn db(&self) -> &Db {
        &self.inner.db
    }

    /// Turns the background memory / quest / notebook requests after each reply on or off (on by default).
    pub fn set_extraction(&self, enabled: bool) {
        self.inner.extraction.store(enabled, Ordering::Relaxed);
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.inner.providers.provider_ids()
    }

    // ------------------------------------------------------------------ opening

    fn session(&self, conversation_id: Uuid) -> Result<ChatSession> {
        let providers = self.provider_ids();
        self.db().with(|c| {
            // A roleplay always has an in-story clock: the first time one is opened it gets the default.
            if let Some(conversation) = conversations::get(c, conversation_id)? {
                if conversation.mode == ConversationMode::Roleplay && conversation.world_state.is_none() {
                    conversations::update_world_state(c, conversation_id, Some(&WorldState::create_default()))?;
                }
            }
            build_session(c, &providers, conversation_id)
        })
    }

    /// The Assistant view: resumes the latest Assistant chat, or starts one. When an assistant persona
    /// is configured, the chat you land in (new or resumed) is switched to it.
    pub fn open_assistant(&self, force_new: bool) -> Result<ChatSession> {
        let id = self.db().with(|c| {
            if !force_new {
                if let Some(existing) = conversations::get_most_recent_assistant(c)? {
                    session::apply_mode_persona(c, existing.id, ConversationMode::Assistant)?;
                    return Ok(existing.id);
                }
            }
            let persona = session::persona_for_mode(c, ConversationMode::Assistant)?;
            let (provider_id, model) = self.default_provider_and_model(c)?;
            let conversation = Conversation {
                title: "Assistant chat".into(),
                mode: ConversationMode::Assistant,
                persona_id: persona.as_ref().map(|p| p.id),
                provider_id,
                model,
                ..Default::default()
            };
            conversations::upsert(c, &conversation)?;
            Ok(conversation.id)
        })?;
        self.session(id)
    }

    /// A 1:1 roleplay: resumes the latest roleplay with this character, or starts one with the
    /// character's greeting already in place. Assistant chats bound to the same character are ignored.
    /// A new roleplay starts as the roleplay persona; resuming leaves a per-chat persona choice alone.
    pub fn open_character(&self, character_id: Uuid, force_new: bool) -> Result<ChatSession> {
        let id = self.db().with(|c| {
            if !force_new {
                if let Some(existing) =
                    conversations::get_most_recent_by_character_and_mode(c, character_id, ConversationMode::Roleplay)?
                {
                    return Ok(existing.id);
                }
            }
            let character = characters::get(c, character_id)?.ok_or_else(|| Error::Other("Character not found.".into()))?;
            let persona = session::persona_for_mode(c, ConversationMode::Roleplay)?;
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
            push_greeting(c, conversation.id, &character, persona.as_ref())?;
            Ok(conversation.id)
        })?;
        self.session(id)
    }

    /// The most recently active roleplay (1:1 or group), if there is one. When a roleplay persona is
    /// configured, that chat is switched to it — this is what the Roleplay nav item opens.
    pub fn open_last_roleplay(&self) -> Result<Option<ChatSession>> {
        let Some(id) = self.db().with(|c| {
            let Some(conversation) = conversations::get_most_recent_roleplay(c)? else {
                return Ok(None);
            };
            session::apply_mode_persona(c, conversation.id, ConversationMode::Roleplay)?;
            Ok(Some(conversation.id))
        })?
        else {
            return Ok(None);
        };
        self.session(id).map(Some)
    }

    /// A group chat: resumes the latest conversation with this group, or starts one.
    pub fn open_group(&self, group_id: Uuid, force_new: bool) -> Result<ChatSession> {
        let id = self.db().with(|c| {
            if !force_new {
                if let Some(existing) = conversations::get_most_recent_by_group_chat(c, group_id)? {
                    return Ok(existing.id);
                }
            }
            let group = group_chats::get(c, group_id)?.ok_or_else(|| Error::Other("Group chat not found.".into()))?;
            let persona = session::persona_for_mode(c, ConversationMode::Roleplay)?;
            let (provider_id, model) = self.default_provider_and_model(c)?;
            let conversation = Conversation {
                title: group.name.clone(),
                mode: ConversationMode::Roleplay,
                group_chat_id: Some(group.id),
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

    pub fn resume(&self, conversation_id: Uuid) -> Result<ChatSession> {
        self.session(conversation_id)
    }

    /// "New chat" stays in the mode of the chat that's open. Mode is what decides, not whether a
    /// character happens to be attached — an assistant chat must not turn into a roleplay.
    pub fn new_chat_like(&self, conversation_id: Uuid) -> Result<ChatSession> {
        let conversation = self
            .db()
            .with(|c| conversations::get(c, conversation_id))?
            .ok_or_else(|| Error::Other("Conversation not found.".into()))?;
        match conversation.mode {
            ConversationMode::Assistant => self.open_assistant(true),
            ConversationMode::Roleplay => {
                if let Some(character_id) = conversation.character_id {
                    self.open_character(character_id, true)
                } else if let Some(group_id) = conversation.group_chat_id {
                    self.open_group(group_id, true)
                } else {
                    Err(Error::Other("This roleplay isn't attached to a character or a group.".into()))
                }
            }
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

    /// Saves the in-story clock and weather as edited by the user. Refused while a reply is being
    /// generated, since that reply would otherwise advance from (and overwrite) the old clock.
    pub fn set_world_state(&self, conversation_id: Uuid, mut state: WorldState) -> Result<WorldState> {
        if self.is_generating(conversation_id) {
            return Err(Error::Other("Wait for the reply to finish before changing the clock.".into()));
        }
        state.default_advance_minutes_per_message = state.default_advance_minutes_per_message.max(1);
        state.weather_notes = state.weather_notes.filter(|n| !n.trim().is_empty());
        self.db().with(|c| {
            let conversation =
                conversations::get(c, conversation_id)?.ok_or_else(|| Error::Other("Conversation not found.".into()))?;
            if conversation.mode != ConversationMode::Roleplay {
                return Err(Error::Other("Only roleplays have an in-story clock.".into()));
            }
            conversations::update_world_state(c, conversation_id, Some(&state))
        })?;
        Ok(state)
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

    /// Group chats: has this character reply next (used when the group is in manual mode, or to add
    /// another voice to a conversation).
    pub async fn pick_speaker(&self, conversation_id: Uuid, character_id: Uuid, sink: Arc<dyn EventSink>) -> Result<()> {
        let engine = self.clone();
        let inner_sink = sink.clone();
        self.run_guarded(conversation_id, sink, move |token| async move {
            let ctx = engine.db().with(|c| load_reply_context(c, conversation_id))?;
            if ctx.group.is_none() {
                return Err(Error::Other("This isn't a group chat.".into()));
            }
            if !ctx.characters.iter().any(|c| c.id == character_id) {
                return Err(Error::Other("That character isn't in this group.".into()));
            }
            if ctx.conversation.model.trim().is_empty() {
                return Err(Error::Other("No model selected - pick one, or refresh the model list in Settings.".into()));
            }
            engine.generate_new(conversation_id, Some(character_id), &inner_sink, &token).await.map(|_| ())
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

    /// Extends the last reply instead of starting a new one (SillyTavern-style): the reply stays in
    /// the history, followed by a small nudge turn, and whatever comes back is appended to the same
    /// message - no new alternate. A stop keeps the partial continuation; a failure rolls back to
    /// the text as it was.
    pub async fn continue_message(&self, conversation_id: Uuid, sink: Arc<dyn EventSink>) -> Result<()> {
        let engine = self.clone();
        let inner_sink = sink.clone();
        self.run_guarded(conversation_id, sink, move |token| async move {
            engine.continue_inner(conversation_id, inner_sink, token).await
        })
        .await
    }

    async fn continue_inner(&self, conversation_id: Uuid, sink: Arc<dyn EventSink>, token: CancellationToken) -> Result<()> {
        let ctx = self.db().with(|c| load_reply_context(c, conversation_id))?;
        let mut target = match ctx.messages.last() {
            Some(m) if m.role == MessageRole::Character => m.clone(),
            _ => return Err(Error::Other("There is no reply to continue.".into())),
        };
        if ctx.conversation.model.trim().is_empty() {
            return Err(Error::Other("No model selected - pick one, or refresh the model list in Settings.".into()));
        }

        let mut history = ctx.messages.clone();
        history.push(ChatMessage::new(conversation_id, MessageRole::User, CONTINUE_NUDGE));
        // The world clock and emotion tags belong to whole replies; a continuation is neither.
        let request = self.assemble(&ctx, &history, AssembleOptions::default())?;
        let raw_prompt = format_prompt(&request);

        sink.emit(ChatEvent::Continuing { message_id: target.id });
        let result = self.stream_reply(&ctx.conversation.provider_id, request, target.id, &sink, &token, false).await;
        let StreamResult { text: added, reasoning: added_reasoning, usage, end, .. } = result;

        let failed = matches!(end, StreamEnd::Failed(_));
        if !failed && !(added.is_empty() && added_reasoning.is_empty()) {
            let reasoning = match target.reasoning_text.as_deref().filter(|r| !r.is_empty()) {
                Some(base) if !added_reasoning.is_empty() => Some(format!("{base}

{added_reasoning}")),
                Some(base) => Some(base.to_string()),
                None => nonempty(added_reasoning),
            };
            let text = format!("{}{added}", target.text);
            target.usage = usage.or(target.usage);
            target.extend_active(text, reasoning, Some(raw_prompt));
            self.db().with(|c| conversations::update_message(c, &target))?;
        }

        let reply_text = target.text.clone();
        // Also restores the original text on the UI after a failure.
        sink.emit(ChatEvent::MessageUpdated { message: target });
        match end {
            StreamEnd::Failed(error) => self.emit_failure(&error, &sink),
            // The clock is not advanced again (same exchange), but the added text may still matter.
            StreamEnd::Completed => self.spawn_enrichment(&ctx, last_user_text(&ctx.messages), reply_text, &sink),
            StreamEnd::Cancelled | StreamEnd::ToolCall(_) => {}
        }
        Ok(())
    }

    /// Drafts the user's next message for them to review - never sent automatically. The model is
    /// asked, in a trailing turn (Gemini rejects histories ending on a model turn, and a
    /// system-prompt nudge alone lost to strong character cards), to write in the user's voice.
    /// Returns `None` when stopped.
    pub async fn impersonate(&self, conversation_id: Uuid) -> Result<Option<String>> {
        let (token, _guard) = self.claim(conversation_id)?;
        let ctx = self.db().with(|c| load_reply_context(c, conversation_id))?;
        if ctx.conversation.model.trim().is_empty() {
            return Err(Error::Other("No model selected - pick one, or refresh the model list in Settings.".into()));
        }

        let user_name = ctx.persona.as_ref().map_or("the user", |p| p.name.as_str());
        let other_party = match ctx.characters.as_slice() {
            [] => "the assistant",
            [only] => only.name.as_str(),
            _ => "the other characters",
        };
        let instruction = format!(
            "[Write {user_name}'s next message in this conversation, using the chat history so far as a guideline for their writing style. Don't write as {other_party}. Reply with only the message text, nothing else.]"
        );

        let mut history = ctx.messages.clone();
        match history.last_mut() {
            Some(last) if last.role == MessageRole::User => {
                let mut folded = ChatMessage::new(conversation_id, MessageRole::User, format!("{}

{instruction}", last.text));
                folded.speaker_character_id = last.speaker_character_id;
                *last = folded;
            }
            _ => history.push(ChatMessage::new(conversation_id, MessageRole::User, instruction)),
        }

        // The preset's own system prompt ("Write {{char}}'s next reply...") would fight the instruction.
        let mut preset = ctx.preset.clone();
        preset.blocks.retain(|b| b.block_type != PromptBlockType::SystemPrompt);
        let request = self.assemble(&ctx, &history, AssembleOptions { preset: Some(&preset), ..Default::default() })?;

        let provider = self.inner.providers.get(&ctx.conversation.provider_id).map_err(|e| Error::Other(user_message(&e)))?;
        let outcome = tokio::select! {
            biased;
            _ = token.cancelled() => return Ok(None),
            outcome = provider.get_completion(request) => outcome,
        };
        let response = outcome.map_err(|e| Error::Other(user_message(&e)))?;
        Ok(Some(response.delta_text.trim().to_string()))
    }

    /// Claims the conversation's single generation slot; fails if a reply is already in flight.
    fn claim(&self, conversation_id: Uuid) -> Result<(CancellationToken, ActiveGuard<'_>)> {
        let token = CancellationToken::new();
        let mut active = self.inner.active.lock().unwrap();
        if active.contains_key(&conversation_id) {
            return Err(Error::Other("A reply is already being generated for this chat.".into()));
        }
        active.insert(conversation_id, token.clone());
        Ok((token, ActiveGuard { active: &self.inner.active, conversation_id }))
    }

    /// Runs one operation as the conversation's only active generation, reporting failure and
    /// completion through the sink whatever happens.
    async fn run_guarded<F, Fut>(&self, conversation_id: Uuid, sink: Arc<dyn EventSink>, operation: F) -> Result<()>
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let (token, guard) = self.claim(conversation_id)?;
        let result = operation(token).await;
        drop(guard);

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

        if let Some(group) = &ctx.group {
            // Who answers depends on the group's activation mode; an empty answer means "let the
            // user pick" (manual mode) - the UI offers the group's members.
            let history = self.db().with(|c| conversations::get_messages(c, conversation_id))?;
            let latest_user_text =
                history.iter().rev().find(|m| m.role == MessageRole::User).map(|m| m.text.clone()).unwrap_or_default();
            let speakers = strategy_for(group.activation_mode).select_next_speakers(group, &ctx.characters, &history, &latest_user_text);
            for speaker in speakers {
                if !self.generate_new(conversation_id, Some(speaker), &sink, &token).await? {
                    break; // a stop or failure ends the round, not just this voice
                }
            }
            return Ok(());
        }

        let speaker_id = (ctx.characters.len() == 1).then(|| ctx.characters[0].id);
        self.generate_new(conversation_id, speaker_id, &sink, &token).await.map(|_| ())
    }

    // ------------------------------------------------------------------ generation

    /// Builds the provider request for a turn: assembled prompt plus the per-turn instructions
    /// (emotion tag for characters with an expression pack, time-skip tag while a world clock runs).
    fn assemble(&self, ctx: &ReplyContext, history: &[ChatMessage], options: AssembleOptions<'_>) -> Result<LlmRequest> {
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

        let mut pctx = PromptAssemblyContext::new(conversation, options.preset.unwrap_or(&ctx.preset), history)
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

        if let Some(character) = options.speaker.filter(|c| !c.expressions.is_empty()) {
            let keys: Vec<String> = character.expressions.iter().map(|e| e.emotion.clone()).collect();
            append_instruction(&mut request, &emotion_tag::build_instruction(&character.name, &keys));
        }
        if options.tools && conversation.mode == ConversationMode::Assistant {
            append_instruction(&mut request, powershell_tool::build_instruction());
        }
        if options.time_skip && conversation.mode == ConversationMode::Roleplay && conversation.world_state.is_some() {
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
        tools: bool,
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
            shown_text: String::new(),
            reasoning: String::new(),
            usage: None,
            images: vec![],
            end: StreamEnd::Completed,
        };
        let (mut pending_text, mut pending_reasoning) = (String::new(), String::new());
        let mut finish_reason: Option<String> = None;
        // With the tool on, a `<powershell-run>` tag is held back from the UI and ends the stream.
        let mut tool_filter = tools.then(ToolCallFilter::new);
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
                        let shown = match tool_filter.as_mut() {
                            Some(filter) => filter.feed(&chunk.delta_text),
                            None => chunk.delta_text.clone(),
                        };
                        result.text.push_str(&chunk.delta_text);
                        result.shown_text.push_str(&shown);
                        result.reasoning.push_str(&chunk.thought_delta_text);
                        pending_text.push_str(&shown);
                        pending_reasoning.push_str(&chunk.thought_delta_text);
                        if chunk.usage.is_some() {
                            result.usage = chunk.usage;
                        }
                        if chunk.finish_reason.is_some() {
                            finish_reason = chunk.finish_reason;
                        }
                        result.images.extend(chunk.images.into_iter().flatten());
                        if let Some(call) = tool_filter.as_ref().and_then(|f| f.detected_call()) {
                            result.end = StreamEnd::ToolCall(call.clone());
                            break; // the command runs before the model says anything else
                        }
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

    /// Generates one character's reply. In Assistant chats with the PowerShell tool on, a reply may
    /// end in a tool call: the command is run (after the user's approval) and its result fed back
    /// for another turn, each turn its own bubble, up to [`MAX_TOOL_ITERATIONS`] round trips.
    /// Returns whether the exchange completed (`false`: stopped or failed).
    async fn generate_new(
        &self,
        conversation_id: Uuid,
        speaker_id: Option<Uuid>,
        sink: &Arc<dyn EventSink>,
        token: &CancellationToken,
    ) -> Result<bool> {
        let tools = self.db().with(|c| {
            let assistant = conversations::get(c, conversation_id)?.is_some_and(|conv| conv.mode == ConversationMode::Assistant);
            Ok(assistant && tool_settings::load(c)?.enabled)
        })?;

        for round_trips in 0.. {
            match self.generate_turn(conversation_id, speaker_id, sink, token, tools).await? {
                Turn::Done(completed) => return Ok(completed),
                Turn::ToolCall(call) => {
                    if round_trips >= MAX_TOOL_ITERATIONS {
                        self.append_tool_message(conversation_id, "Tool call limit reached for this turn.", sink)?;
                        return Ok(true);
                    }
                    if !self.run_tool_call(conversation_id, call, sink, token).await? {
                        return Ok(false);
                    }
                }
            }
        }
        unreachable!("the loop only ends by returning")
    }

    async fn generate_turn(
        &self,
        conversation_id: Uuid,
        speaker_id: Option<Uuid>,
        sink: &Arc<dyn EventSink>,
        token: &CancellationToken,
        tools: bool,
    ) -> Result<Turn> {
        let ctx = self.db().with(|c| load_reply_context(c, conversation_id))?;
        let speaker = speaker_id.and_then(|id| ctx.characters.iter().find(|c| c.id == id)).cloned();

        let mut message = ChatMessage::new(conversation_id, MessageRole::Character, "");
        message.speaker_character_id = speaker_id;
        sink.emit(ChatEvent::MessageAdded { message: message.clone() });

        let request = self.assemble(
            &ctx,
            &ctx.messages,
            AssembleOptions { speaker: speaker.as_ref(), time_skip: true, tools, ..Default::default() },
        )?;
        message.raw_prompt_text = Some(format_prompt(&request));
        self.update_conversation(conversation_id, |c| c.updated_at = Utc::now())?;

        let result = self.stream_reply(&ctx.conversation.provider_id, request, message.id, sink, token, tools).await;
        let StreamResult { text, shown_text, reasoning, usage, images: generated, end } = result;

        let succeeded = matches!(end, StreamEnd::Completed);
        if succeeded {
            message.usage = usage;
            let after_clock = self.apply_world_clock(&ctx, text, sink)?;
            let (clean, emotion) = extract_matched_emotion(speaker.as_ref(), &after_clock);
            message.text = clean;
            message.detected_emotion = emotion;
        } else if matches!(end, StreamEnd::ToolCall(_)) {
            // Keep what the user saw (everything before the tag); the tag itself is never stored.
            message.text = shown_text;
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
            StreamEnd::Failed(error) => {
                self.emit_failure(&error, sink);
                Ok(Turn::Done(false))
            }
            StreamEnd::Cancelled => Ok(Turn::Done(false)),
            StreamEnd::ToolCall(call) => Ok(Turn::ToolCall(call)),
            StreamEnd::Completed => {
                self.spawn_enrichment(&ctx, last_user_text(&ctx.messages), message.text.clone(), sink);
                // Only the conversation's very first exchange gets auto-titled.
                if ctx.conversation.mode == ConversationMode::Assistant && ctx.messages.len() == 1 {
                    let user_text = ctx.messages.last().map(|m| m.text.clone()).unwrap_or_default();
                    self.spawn_title(&ctx.conversation, user_text, message.text.clone(), sink.clone());
                }
                Ok(Turn::Done(true))
            }
        }
    }

    // ------------------------------------------------------------------ PowerShell tool

    /// Replaces how commands are run (tests use a fake).
    pub fn set_command_runner(&self, runner: Arc<dyn CommandRunner>) {
        *self.inner.runner.lock().unwrap() = runner;
    }

    /// Answers a [`ChatEvent::ToolApproval`]. Returns false if that request is no longer waiting.
    pub fn resolve_tool_approval(&self, request_id: Uuid, decision: ToolDecision) -> bool {
        match self.inner.approvals.lock().unwrap().remove(&request_id) {
            Some(waiting) => waiting.send(decision).is_ok(),
            None => false,
        }
    }

    /// Asks the user whether to run a command. `None` if generation was stopped meanwhile.
    async fn ask_to_run(
        &self,
        call: &PowerShellToolCallRequest,
        sink: &Arc<dyn EventSink>,
        token: &CancellationToken,
    ) -> Option<ToolDecision> {
        let request_id = Uuid::new_v4();
        let (send, receive) = oneshot::channel();
        self.inner.approvals.lock().unwrap().insert(request_id, send);
        sink.emit(ChatEvent::ToolApproval { request_id, command: call.command.clone(), elevate: call.elevate });

        let decision = tokio::select! {
            biased;
            _ = token.cancelled() => None,
            decision = receive => decision.ok(),
        };
        self.inner.approvals.lock().unwrap().remove(&request_id);
        decision
    }

    /// Approves (asking the user unless auto mode or the allowlist already covers it; elevation
    /// always asks), runs, and records one requested command as a Tool message the next turn sees.
    /// Returns false if the user stopped generation meanwhile.
    async fn run_tool_call(
        &self,
        conversation_id: Uuid,
        call: PowerShellToolCallRequest,
        sink: &Arc<dyn EventSink>,
        token: &CancellationToken,
    ) -> Result<bool> {
        let options = self.db().with(tool_settings::load)?;
        let pre_approved = !call.elevate && (options.auto_mode || options.allows(&call.command));
        if !pre_approved {
            let Some(decision) = self.ask_to_run(&call, sink, token).await else {
                return Ok(false);
            };
            if decision.allow {
                if let Some(pattern) = decision.always_pattern.filter(|_| !call.elevate) {
                    self.db().with(|c| tool_settings::add_allowlist_pattern(c, &pattern))?;
                }
            } else {
                let declined = ToolResult { command: &call.command, elevated: call.elevate, declined: true, ..Default::default() };
                self.append_tool_message(conversation_id, &powershell_tool::format_result_for_model(&declined), sink)?;
                return Ok(true);
            }
        }

        // Visible for the whole run: live output streams straight into the bubble, so it's clear the
        // assistant is doing something rather than stuck.
        let mut message = ChatMessage::new(conversation_id, MessageRole::Tool, "");
        sink.emit(ChatEvent::MessageAdded { message: message.clone() });
        let header = if call.elevate { format!("$ {}  (as Administrator)\n", call.command) } else { format!("$ {}\n", call.command) };
        sink.emit(ChatEvent::Delta { message_id: message.id, text: header, reasoning: String::new() });

        let runner = self.inner.runner.lock().unwrap().clone();
        let message_id = message.id;
        let live = sink.clone();
        let on_output = move |chunk: String| live.emit(ChatEvent::Delta { message_id, text: chunk, reasoning: String::new() });
        let outcome = runner.run(&call.command, call.elevate, &on_output, token).await;

        let text = match outcome {
            Err(ExecError::Cancelled) => {
                sink.emit(ChatEvent::MessageRemoved { message_id });
                return Ok(false);
            }
            // Not the model's fault and not a "declined" outcome, but it still needs to know.
            Err(error) => format!("[PowerShell tool result]\nCommand: {}\n{error}", call.command),
            Ok(result) => format_execution(&result),
        };

        // The clean, structured result replaces the raw live transcript.
        message.text = text;
        message.finalize_generation();
        self.db().with(|c| conversations::add_message(c, &message))?;
        sink.emit(ChatEvent::MessageUpdated { message });
        Ok(true)
    }

    /// Adds a Tool message that is already complete (a decline or a limit note).
    fn append_tool_message(&self, conversation_id: Uuid, text: &str, sink: &Arc<dyn EventSink>) -> Result<()> {
        let mut message = ChatMessage::new(conversation_id, MessageRole::Tool, text);
        message.finalize_generation();
        self.db().with(|c| conversations::add_message(c, &message))?;
        sink.emit(ChatEvent::MessageAdded { message });
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
        let request = self.assemble(
            &ctx,
            &ctx.messages[..position],
            AssembleOptions { speaker: speaker.as_ref(), time_skip: true, ..Default::default() },
        )?;
        let raw_prompt = format_prompt(&request);

        sink.emit(ChatEvent::Regenerating { message_id });
        let result = self.stream_reply(&ctx.conversation.provider_id, request, message_id, &sink, &token, false).await;
        let StreamResult { text, reasoning, usage, images: generated, end, .. } = result;

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
        let reply_text = target.text.clone();
        // Also restores the original text on the UI when nothing new arrived.
        sink.emit(ChatEvent::MessageUpdated { message: target });

        match end {
            StreamEnd::Failed(error) => self.emit_failure(&error, &sink),
            StreamEnd::Completed => {
                self.spawn_enrichment(&ctx, last_user_text(&ctx.messages[..position]), reply_text, &sink);
            }
            StreamEnd::Cancelled | StreamEnd::ToolCall(_) => {}
        }
        Ok(())
    }

    /// Fire-and-forget follow-up requests once a reply has completed: Assistant chats learn facts
    /// about the user; roleplays update the characters' notebooks and the quest log (and tell the UI
    /// when they are done). Failures are dropped - the reply itself already succeeded.
    fn spawn_enrichment(&self, ctx: &ReplyContext, user_text: String, reply_text: String, sink: &Arc<dyn EventSink>) {
        if !self.inner.extraction.load(Ordering::Relaxed) || user_text.trim().is_empty() {
            return;
        }
        let engine = self.clone();
        let (conversation, characters) = (ctx.conversation.clone(), ctx.characters.clone());
        let sink = sink.clone();
        tokio::spawn(async move {
            let extractor = Extractor {
                db: engine.db(),
                providers: &engine.inner.providers,
                provider_id: &conversation.provider_id,
                model: &conversation.model,
            };
            let exchange = Exchange { user_text: &user_text, reply_text: &reply_text };
            match conversation.mode {
                ConversationMode::Assistant => {
                    report("memory extraction", extractor.memories(&exchange).await);
                }
                ConversationMode::Roleplay => {
                    let (notes, quests) = tokio::join!(
                        extractor.notebooks(conversation.id, &characters, &exchange),
                        extractor.quests(conversation.id, &exchange)
                    );
                    report("notebook extraction", notes);
                    report("quest log extraction", quests);
                    sink.emit(ChatEvent::StoryChanged { conversation_id: conversation.id });
                }
            }
        });
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

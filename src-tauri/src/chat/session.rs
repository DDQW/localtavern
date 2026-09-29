//! Everything the chat page needs to render one conversation, loaded in one call.

use chrono::{DateTime, Local, Utc};
use rusqlite::Connection;
use serde::Serialize;
use uuid::Uuid;

use super::presets;
use crate::error::{Error, Result};
use crate::models::{Character, ChatMessage, Conversation, ConversationMode, Persona, PromptPreset};
use crate::repo::{characters, conversations, group_chats, lorebooks, personas, user_memories};
use crate::settings;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetSummary {
    pub id: Uuid,
    pub name: String,
}

/// One row of the "History" picker: a past conversation in the current scope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: Uuid,
    pub label: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    pub conversation: Conversation,
    pub messages: Vec<ChatMessage>,
    pub characters: Vec<Character>,
    pub persona: Option<Persona>,
    pub personas: Vec<Persona>,
    pub preset: PromptPreset,
    pub presets: Vec<PresetSummary>,
    pub history: Vec<HistoryEntry>,
    pub providers: Vec<String>,
    /// The cached model list for the conversation's provider.
    pub models: Vec<String>,
    pub memory_count: usize,
}

/// The loaded state a reply is generated from.
pub struct ReplyContext {
    pub conversation: Conversation,
    pub preset: PromptPreset,
    pub persona: Option<Persona>,
    pub characters: Vec<Character>,
    pub lorebooks: Vec<crate::models::Lorebook>,
    pub messages: Vec<ChatMessage>,
}

pub fn default_persona(conn: &Connection) -> Result<Option<Persona>> {
    let all = personas::get_all(conn)?;
    Ok(all.iter().find(|p| p.is_default).cloned().or_else(|| all.into_iter().next()))
}

pub fn load_reply_context(conn: &Connection, conversation_id: Uuid) -> Result<ReplyContext> {
    let conversation = conversations::get(conn, conversation_id)?
        .ok_or_else(|| Error::Other("Conversation not found.".into()))?;
    let preset = presets::resolve_for_conversation(conn, &conversation)?;
    let persona = match conversation.persona_id {
        Some(id) => personas::get(conn, id)?,
        None => None,
    };

    let mut cast = Vec::new();
    if let Some(id) = conversation.character_id {
        cast.extend(characters::get(conn, id)?);
    } else if let Some(group_id) = conversation.group_chat_id {
        if let Some(group) = group_chats::get(conn, group_id)? {
            for id in group.character_ids {
                cast.extend(characters::get(conn, id)?);
            }
        }
    }

    let mut lorebook_ids: Vec<Uuid> = cast.iter().flat_map(|c| c.linked_lorebook_ids.iter().copied()).collect();
    lorebook_ids.sort();
    lorebook_ids.dedup();
    let mut books = Vec::new();
    for id in lorebook_ids {
        books.extend(lorebooks::get(conn, id)?);
    }

    let messages = conversations::get_messages(conn, conversation_id)?;
    Ok(ReplyContext { conversation, preset, persona, characters: cast, lorebooks: books, messages })
}

pub fn build_session(conn: &Connection, provider_ids: &[String], conversation_id: Uuid) -> Result<ChatSession> {
    let ctx = load_reply_context(conn, conversation_id)?;

    let scope = if let Some(id) = ctx.conversation.character_id {
        conversations::get_by_character(conn, id)?
    } else if let Some(id) = ctx.conversation.group_chat_id {
        conversations::get_by_group_chat(conn, id)?
    } else {
        conversations::get_by_mode(conn, ConversationMode::Assistant)?
    };
    let history = scope.iter().map(history_entry).collect();

    let presets = presets::get_all_or_seed(conn)?
        .into_iter()
        .map(|p| PresetSummary { id: p.id, name: p.name })
        .collect();

    let models = settings::cached_models(conn, &ctx.conversation.provider_id)?;
    let memory_count = user_memories::get_all(conn)?.len();

    Ok(ChatSession {
        conversation: ctx.conversation,
        messages: ctx.messages,
        characters: ctx.characters,
        persona: ctx.persona,
        personas: personas::get_all(conn)?,
        preset: ctx.preset,
        presets,
        history,
        providers: provider_ids.to_vec(),
        models,
        memory_count,
    })
}

fn history_entry(c: &Conversation) -> HistoryEntry {
    let when = c.updated_at.with_timezone(&Local).format("%b %-d, %H:%M");
    let title = if c.title.trim().is_empty() { "Untitled chat" } else { c.title.trim() };
    HistoryEntry { id: c.id, label: format!("{title} · {when}"), updated_at: c.updated_at }
}

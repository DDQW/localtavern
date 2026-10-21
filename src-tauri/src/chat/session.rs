//! Everything the chat page needs to render one conversation, loaded in one call.

use chrono::{DateTime, Local, Utc};
use rusqlite::Connection;
use serde::Serialize;
use uuid::Uuid;

use super::presets;
use crate::error::{Error, Result};
use crate::models::{Character, ChatMessage, Conversation, ConversationMode, GroupChat, MessageRole, Persona, PromptPreset};
use crate::repo::{characters, conversations, group_chats, lorebooks, personas, summaries, user_memories};
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
    /// Set for group chats: who is in the group and how speakers are chosen.
    pub group: Option<GroupChat>,
    pub preset: PromptPreset,
    pub presets: Vec<PresetSummary>,
    pub history: Vec<HistoryEntry>,
    pub providers: Vec<String>,
    /// The cached model list for the conversation's provider.
    pub models: Vec<String>,
    pub memory_count: usize,
    /// For a private talk: the title of the group conversation it belongs to.
    pub parent_title: Option<String>,
}

/// The loaded state a reply is generated from.
pub struct ReplyContext {
    pub group: Option<GroupChat>,
    pub conversation: Conversation,
    pub preset: PromptPreset,
    pub persona: Option<Persona>,
    pub characters: Vec<Character>,
    pub lorebooks: Vec<crate::models::Lorebook>,
    pub messages: Vec<ChatMessage>,
    /// Set for a private talk: the group conversation it belongs to.
    pub breakout: Option<Breakout>,
    /// For a group conversation: its private talks, which only their own member remembers.
    pub private_talks: Vec<PrivateTalk>,
}

/// The group conversation a private talk belongs to, as the talk's prompt sees it.
pub struct Breakout {
    pub parent: Conversation,
    /// The group's name.
    pub group_name: String,
    /// Everyone in the group, this talk's character included (to name who said what).
    pub group_cast: Vec<Character>,
    pub group_messages: Vec<ChatMessage>,
    /// The group's summary text, when it has been compacted (`group_messages` are then only the
    /// ones it doesn't cover).
    pub group_summary: Option<String>,
}

/// One member's private talk with the user, inside a group conversation.
pub struct PrivateTalk {
    pub character_id: Uuid,
    pub messages: Vec<ChatMessage>,
    pub summary: Option<String>,
}

pub fn default_persona(conn: &Connection) -> Result<Option<Persona>> {
    let all = personas::get_all(conn)?;
    Ok(all.iter().find(|p| p.is_default).cloned().or_else(|| all.into_iter().next()))
}

/// The persona a new chat in `mode` starts as. A mode default wins when that persona still exists;
/// otherwise the global default (the one marked default, or the first persona).
pub fn persona_for_mode(conn: &Connection, mode: ConversationMode) -> Result<Option<Persona>> {
    if let Some(id) = settings::default_persona_for_mode(conn, mode)? {
        if let Some(persona) = personas::get(conn, id)? {
            return Ok(Some(persona));
        }
    }
    default_persona(conn)
}

/// Selects the mode's default persona on an existing chat. No-op when that mode has no default, or
/// the persona was deleted, so a per-chat choice is left alone until a default is actually set.
pub fn apply_mode_persona(conn: &Connection, conversation_id: Uuid, mode: ConversationMode) -> Result<()> {
    let Some(id) = settings::default_persona_for_mode(conn, mode)? else {
        return Ok(());
    };
    if personas::get(conn, id)?.is_none() {
        return Ok(());
    }
    let Some(mut conversation) = conversations::get(conn, conversation_id)? else {
        return Ok(());
    };
    if conversation.persona_id == Some(id) {
        return Ok(());
    }
    conversation.persona_id = Some(id);
    conversations::upsert(conn, &conversation)
}

pub fn load_reply_context(conn: &Connection, conversation_id: Uuid) -> Result<ReplyContext> {
    let conversation = conversations::get(conn, conversation_id)?
        .ok_or_else(|| Error::Other("Conversation not found.".into()))?;
    let preset = presets::resolve_for_conversation(conn, &conversation)?;
    let persona = match conversation.persona_id {
        Some(id) => personas::get(conn, id)?,
        None => None,
    };

    let group = match conversation.group_chat_id {
        Some(id) => group_chats::get(conn, id)?,
        None => None,
    };

    // The model sees each character in the outfit this story has them in.
    let outfits = conversation.world_state.as_ref().map(|w| &w.outfits);
    let cast: Vec<Character> = load_cast(conn, &conversation, group.as_ref())?
        .into_iter()
        .map(|c| {
            let outfit = outfits.and_then(|o| o.get(&c.id)).copied();
            c.dressed(outfit)
        })
        .collect();

    let mut lorebook_ids: Vec<Uuid> = cast.iter().flat_map(|c| c.linked_lorebook_ids.iter().copied()).collect();
    lorebook_ids.sort();
    lorebook_ids.dedup();
    let mut books = Vec::new();
    for id in lorebook_ids {
        books.extend(lorebooks::get(conn, id)?);
    }

    let messages = chat_history(conn, conversation_id)?;
    let breakout = match conversation.parent_conversation_id {
        Some(parent_id) => load_breakout(conn, parent_id)?,
        None => None,
    };
    let private_talks = if group.is_some() { load_private_talks(conn, conversation_id)? } else { Vec::new() };
    Ok(ReplyContext {
        group,
        conversation,
        preset,
        persona,
        characters: cast,
        lorebooks: books,
        messages,
        breakout,
        private_talks,
    })
}

fn load_breakout(conn: &Connection, parent_id: Uuid) -> Result<Option<Breakout>> {
    let Some(parent) = conversations::get(conn, parent_id)? else {
        return Ok(None);
    };
    let group = match parent.group_chat_id {
        Some(id) => group_chats::get(conn, id)?,
        None => None,
    };
    let group_cast = load_cast(conn, &parent, group.as_ref())?;
    let all = chat_history(conn, parent_id)?;
    let summary = summaries::get(conn, parent_id)?;
    let group_messages = summary.as_ref().map_or(&all[..], |s| s.uncovered(&all)).to_vec();
    let group_name = group.map(|g| g.name).unwrap_or_else(|| parent.title.clone());
    Ok(Some(Breakout { parent, group_name, group_cast, group_messages, group_summary: summary.map(|s| s.text) }))
}

fn load_private_talks(conn: &Connection, conversation_id: Uuid) -> Result<Vec<PrivateTalk>> {
    let mut talks = Vec::new();
    for talk in conversations::get_breakouts(conn, conversation_id)? {
        let Some(character_id) = talk.character_id else { continue };
        talks.push(PrivateTalk {
            character_id,
            messages: chat_history(conn, talk.id)?,
            summary: summaries::get(conn, talk.id)?.map(|s| s.text),
        });
    }
    Ok(talks)
}

/// The conversation's character(s) as stored.
fn load_cast(conn: &Connection, conversation: &Conversation, group: Option<&GroupChat>) -> Result<Vec<Character>> {
    let mut cast = Vec::new();
    if let Some(id) = conversation.character_id {
        cast.extend(characters::get(conn, id)?);
    } else if let Some(group) = group {
        for id in &group.character_ids {
            cast.extend(characters::get(conn, *id)?);
        }
    }
    Ok(cast)
}

/// The conversation as the model sees it: every message except generated pictures, which are only
/// for the user to look at (and would otherwise be re-uploaded to vision models on every turn).
pub fn chat_history(conn: &Connection, conversation_id: Uuid) -> Result<Vec<ChatMessage>> {
    let mut messages = conversations::get_messages(conn, conversation_id)?;
    messages.retain(|m| m.role != MessageRole::Image);
    Ok(messages)
}

pub fn build_session(conn: &Connection, provider_ids: &[String], conversation_id: Uuid) -> Result<ChatSession> {
    let mut ctx = load_reply_context(conn, conversation_id)?;
    // The chat view shows everything, pictures included, and gets the characters as stored (it
    // picks the outfit's portraits itself, so switching outfits needs no reload).
    ctx.messages = conversations::get_messages(conn, conversation_id)?;
    ctx.characters = load_cast(conn, &ctx.conversation, ctx.group.as_ref())?;

    // History stays inside the conversation's mode. An assistant bound to a character must not list
    // that character's roleplays, and a plain assistant chat must not list character-bound ones.
    let scope = match ctx.conversation.mode {
        ConversationMode::Assistant => match ctx.conversation.character_id {
            Some(id) => conversations::get_by_character_and_mode(conn, id, ConversationMode::Assistant)?,
            None => conversations::get_assistants_without_character(conn)?,
        },
        ConversationMode::Roleplay => {
            if let Some(parent) = ctx.conversation.parent_conversation_id {
                // A private talk lists its group conversation and that conversation's other talks.
                conversations::get(conn, parent)?.into_iter().chain(conversations::get_breakouts(conn, parent)?).collect()
            } else if let Some(id) = ctx.conversation.character_id {
                conversations::get_by_character_and_mode(conn, id, ConversationMode::Roleplay)?
            } else if let Some(id) = ctx.conversation.group_chat_id {
                conversations::get_by_group_chat(conn, id)?
            } else {
                Vec::new()
            }
        }
    };
    let history = scope.iter().map(history_entry).collect();

    let presets = presets::get_all_or_seed(conn)?
        .into_iter()
        .map(|p| PresetSummary { id: p.id, name: p.name })
        .collect();

    let models = settings::cached_models(conn, &ctx.conversation.provider_id)?;
    let memory_count = user_memories::get_all(conn)?.len();
    let parent_title = ctx.breakout.as_ref().map(|b| b.group_name.clone());

    Ok(ChatSession {
        conversation: ctx.conversation,
        messages: ctx.messages,
        characters: ctx.characters,
        persona: ctx.persona,
        personas: personas::get_all(conn)?,
        group: ctx.group,
        preset: ctx.preset,
        presets,
        history,
        providers: provider_ids.to_vec(),
        models,
        memory_count,
        parent_title,
    })
}

fn history_entry(c: &Conversation) -> HistoryEntry {
    let when = c.updated_at.with_timezone(&Local).format("%b %-d, %H:%M");
    let title = if c.title.trim().is_empty() { "Untitled chat" } else { c.title.trim() };
    HistoryEntry { id: c.id, label: format!("{title} · {when}"), updated_at: c.updated_at }
}

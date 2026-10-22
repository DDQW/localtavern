use rusqlite::{params, types::ToSql, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::db::{json, rows, time};
use crate::error::Result;
use crate::models::{
    ChatMessage, Conversation, ConversationMode, LlmUsage, MessageAttachment, MessageRole, WorldState,
};

fn map_conversation(row: &Row) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: rows::uuid(row, "Id")?,
        title: row.get("Title")?,
        mode: rows::int_enum(row, "Mode", ConversationMode::from_i32)?,
        character_id: rows::opt_uuid(row, "CharacterId")?,
        group_chat_id: rows::opt_uuid(row, "GroupChatId")?,
        persona_id: rows::opt_uuid(row, "PersonaId")?,
        prompt_preset_id: rows::opt_uuid(row, "PromptPresetId")?,
        provider_id: row.get("ProviderId")?,
        model: row.get("Model")?,
        assistant_system_prompt: row.get("AssistantSystemPrompt")?,
        enable_google_search: row.get("EnableGoogleSearch")?,
        world_state: rows::opt_blob(row, "WorldStateJson")?,
        parent_conversation_id: rows::opt_uuid(row, "ParentConversationId")?,
        created_at: rows::datetime(row, "CreatedAt")?,
        updated_at: rows::datetime(row, "UpdatedAt")?,
    })
}

fn map_message(row: &Row) -> rusqlite::Result<ChatMessage> {
    let prompt_tokens: Option<i32> = row.get("PromptTokens")?;
    let completion_tokens: Option<i32> = row.get("CompletionTokens")?;
    let total_tokens: Option<i32> = row.get("TotalTokens")?;
    let mut message = ChatMessage {
        id: rows::uuid(row, "Id")?,
        conversation_id: rows::uuid(row, "ConversationId")?,
        role: rows::int_enum(row, "Role", MessageRole::from_i32)?,
        speaker_character_id: rows::opt_uuid(row, "SpeakerCharacterId")?,
        text: row.get("Text")?,
        reasoning_text: row.get("ReasoningText")?,
        raw_prompt_text: row.get("RawPromptText")?,
        has_prompt: row.get("HasPrompt")?,
        alternates: rows::blob(row, "AlternatesJson")?,
        active_alternate_index: row.get("ActiveAlternateIndex")?,
        attachments: rows::blob(row, "AttachmentsJson")?,
        detected_emotion: row.get("DetectedEmotion")?,
        clock: rows::opt_blob(row, "ClockJson")?,
        timestamp: rows::datetime(row, "Timestamp")?,
        usage: if prompt_tokens.is_none() && completion_tokens.is_none() && total_tokens.is_none() {
            None
        } else {
            Some(LlmUsage { prompt_tokens, completion_tokens, total_tokens })
        },
    };
    message.sync_text_from_alternates();
    Ok(message)
}

// ---- conversations ----

pub fn get(conn: &Connection, id: Uuid) -> Result<Option<Conversation>> {
    Ok(conn
        .query_row("SELECT * FROM Conversations WHERE Id = ?1;", [id.to_string()], map_conversation)
        .optional()?)
}

fn all_where(conn: &Connection, clause: &str, param: &dyn ToSql) -> Result<Vec<Conversation>> {
    let sql = format!("SELECT * FROM Conversations WHERE {clause} ORDER BY UpdatedAt DESC;");
    let mut stmt = conn.prepare(&sql)?;
    let list = stmt.query_map([param], map_conversation)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

fn most_recent_where(conn: &Connection, clause: &str, param: &dyn ToSql) -> Result<Option<Conversation>> {
    Ok(all_where(conn, clause, param)?.into_iter().next())
}

/// The most recently updated plain Assistant-mode conversation, so the Assistant view can resume it.
pub fn get_most_recent_assistant(conn: &Connection) -> Result<Option<Conversation>> {
    most_recent_where(conn, "Mode = ?1", &ConversationMode::Assistant.to_i32())
}

/// The most recently updated roleplay conversation (1:1 or group).
pub fn get_most_recent_roleplay(conn: &Connection) -> Result<Option<Conversation>> {
    most_recent_where(conn, "Mode = ?1", &ConversationMode::Roleplay.to_i32())
}

/// A character's own chats leave out private talks inside a group (they belong to the group's chat).
pub fn get_most_recent_by_character(conn: &Connection, character_id: Uuid) -> Result<Option<Conversation>> {
    most_recent_where(conn, "CharacterId = ?1 AND ParentConversationId IS NULL", &character_id.to_string())
}

/// Conversations with this character in one mode, newest first. Roleplay and Assistant can share a
/// character id without sharing a thread: each listing stays inside its own mode. Private talks
/// inside a group are left out.
pub fn get_by_character_and_mode(
    conn: &Connection,
    character_id: Uuid,
    mode: ConversationMode,
) -> Result<Vec<Conversation>> {
    let mut stmt =
        conn.prepare("SELECT * FROM Conversations WHERE CharacterId = ?1 AND Mode = ?2 AND ParentConversationId IS NULL ORDER BY UpdatedAt DESC;")?;
    let list = stmt
        .query_map(params![character_id.to_string(), mode.to_i32()], map_conversation)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn get_most_recent_by_character_and_mode(
    conn: &Connection,
    character_id: Uuid,
    mode: ConversationMode,
) -> Result<Option<Conversation>> {
    Ok(get_by_character_and_mode(conn, character_id, mode)?.into_iter().next())
}

/// Assistant chats that aren't bound to a character. The History picker of a plain Assistant chat
/// uses this so it doesn't offer a character's assistant threads.
pub fn get_assistants_without_character(conn: &Connection) -> Result<Vec<Conversation>> {
    let mut stmt =
        conn.prepare("SELECT * FROM Conversations WHERE Mode = ?1 AND CharacterId IS NULL ORDER BY UpdatedAt DESC;")?;
    let list = stmt
        .query_map(params![ConversationMode::Assistant.to_i32()], map_conversation)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn get_most_recent_by_group_chat(conn: &Connection, group_chat_id: Uuid) -> Result<Option<Conversation>> {
    most_recent_where(conn, "GroupChatId = ?1", &group_chat_id.to_string())
}

pub fn get_by_character(conn: &Connection, character_id: Uuid) -> Result<Vec<Conversation>> {
    all_where(conn, "CharacterId = ?1 AND ParentConversationId IS NULL", &character_id.to_string())
}

/// The private talks (breakouts) inside a group conversation, newest first.
pub fn get_breakouts(conn: &Connection, parent_id: Uuid) -> Result<Vec<Conversation>> {
    all_where(conn, "ParentConversationId = ?1", &parent_id.to_string())
}

/// The private talk with `character_id` inside a group conversation, if there is one.
pub fn get_breakout(conn: &Connection, parent_id: Uuid, character_id: Uuid) -> Result<Option<Conversation>> {
    Ok(get_breakouts(conn, parent_id)?.into_iter().find(|c| c.character_id == Some(character_id)))
}

/// The conversation and every one tied to it: a group conversation and its private talks share
/// one story (and one clock), whichever of them `conversation_id` is.
pub fn family_ids(conn: &Connection, conversation_id: Uuid) -> Result<Vec<Uuid>> {
    let id = conversation_id.to_string();
    let root: String = conn
        .query_row("SELECT COALESCE(ParentConversationId, Id) FROM Conversations WHERE Id = ?1;", [&id], |r| r.get(0))
        .optional()?
        .unwrap_or(id);
    let mut stmt = conn.prepare("SELECT Id FROM Conversations WHERE Id = ?1 OR ParentConversationId = ?1;")?;
    let ids = stmt.query_map([&root], |r| rows::uuid(r, "Id"))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

pub fn get_by_group_chat(conn: &Connection, group_chat_id: Uuid) -> Result<Vec<Conversation>> {
    all_where(conn, "GroupChatId = ?1", &group_chat_id.to_string())
}

pub fn get_by_mode(conn: &Connection, mode: ConversationMode) -> Result<Vec<Conversation>> {
    all_where(conn, "Mode = ?1", &mode.to_i32())
}

pub fn get_all(conn: &Connection) -> Result<Vec<Conversation>> {
    let mut stmt = conn.prepare("SELECT * FROM Conversations ORDER BY UpdatedAt DESC;")?;
    let list = stmt.query_map([], map_conversation)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn upsert(conn: &Connection, c: &Conversation) -> Result<()> {
    let world_state = c.world_state.as_ref().map(json::to_blob).transpose()?;
    conn.execute(
        "INSERT INTO Conversations
            (Id, Title, Mode, CharacterId, GroupChatId, PersonaId, PromptPresetId, ProviderId, Model,
             AssistantSystemPrompt, EnableGoogleSearch, WorldStateJson, CreatedAt, UpdatedAt, ParentConversationId)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(Id) DO UPDATE SET
            Title = ?2, Mode = ?3, CharacterId = ?4, GroupChatId = ?5, PersonaId = ?6, PromptPresetId = ?7,
            ProviderId = ?8, Model = ?9, AssistantSystemPrompt = ?10, EnableGoogleSearch = ?11,
            WorldStateJson = ?12, UpdatedAt = ?14, ParentConversationId = ?15;",
        params![
            c.id.to_string(),
            c.title,
            c.mode.to_i32(),
            c.character_id.map(|i| i.to_string()),
            c.group_chat_id.map(|i| i.to_string()),
            c.persona_id.map(|i| i.to_string()),
            c.prompt_preset_id.map(|i| i.to_string()),
            c.provider_id,
            c.model,
            c.assistant_system_prompt,
            c.enable_google_search,
            world_state,
            time::format(&c.created_at),
            time::format(&c.updated_at),
            c.parent_conversation_id.map(|i| i.to_string()),
        ],
    )?;
    Ok(())
}

/// Persists only the world-state column, so a sidebar edit or clock auto-advance doesn't re-upsert the row.
/// A group conversation and its private talks share the state: it is written to all of them.
pub fn update_world_state(conn: &Connection, conversation_id: Uuid, state: Option<&WorldState>) -> Result<()> {
    let blob = state.map(json::to_blob).transpose()?;
    for id in family_ids(conn, conversation_id)? {
        conn.execute("UPDATE Conversations SET WorldStateJson = ?1 WHERE Id = ?2;", params![blob, id.to_string()])?;
    }
    Ok(())
}

/// Whether a message anywhere in the conversation's family, newer than `after`, moved the story
/// clock - then an older reply's clock can no longer be put back.
pub fn clock_moved_after(conn: &Connection, conversation_id: Uuid, after: &ChatMessage) -> Result<bool> {
    for id in family_ids(conn, conversation_id)? {
        let later = get_messages(conn, id)?
            .into_iter()
            .filter(|m| m.id != after.id && m.clock.is_some())
            .any(|m| m.timestamp > after.timestamp);
        if later {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM Conversations WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

// ---- messages ----

/// Every message column, plus whether a prompt was saved for it (the prompts themselves stay in
/// `MessagePrompts` until one is asked for).
const MESSAGE_COLUMNS: &str =
    "m.*, EXISTS (SELECT 1 FROM MessagePrompts p WHERE p.MessageId = m.Id) AS HasPrompt";

/// Only a picture keeps its draft in the row; a reply's prompt goes to `MessagePrompts`.
fn row_prompt(m: &ChatMessage) -> Option<&str> {
    (m.role == MessageRole::Image).then_some(m.raw_prompt_text.as_deref()).flatten()
}

/// Saves the prompts the message carries in memory, one per variant. A loaded message carries none,
/// and the ones already saved stay.
fn save_prompts(conn: &Connection, m: &ChatMessage) -> Result<()> {
    if m.role == MessageRole::Image {
        return Ok(());
    }
    let mut prompts: Vec<(usize, &str)> =
        m.alternates.iter().enumerate().filter_map(|(i, a)| Some((i, a.raw_prompt.as_deref()?))).collect();
    if m.alternates.is_empty() {
        prompts.extend(m.raw_prompt_text.as_deref().map(|p| (0, p)));
    }
    for (index, prompt) in prompts.into_iter().filter(|(_, p)| !p.is_empty()) {
        conn.execute(
            "INSERT OR REPLACE INTO MessagePrompts (MessageId, AlternateIndex, Prompt) VALUES (?1, ?2, ?3);",
            params![m.id.to_string(), index as i64, prompt],
        )?;
    }
    Ok(())
}

/// The prompt the message's shown variant was generated from, if one was saved.
pub fn get_prompt(conn: &Connection, message_id: Uuid) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT p.Prompt FROM ChatMessages m
             JOIN MessagePrompts p ON p.MessageId = m.Id AND p.AlternateIndex = MAX(m.ActiveAlternateIndex, 0)
             WHERE m.Id = ?1;",
            [message_id.to_string()],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn add_message(conn: &Connection, m: &ChatMessage) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO ChatMessages
            (Id, ConversationId, Role, SpeakerCharacterId, Text, ReasoningText, RawPromptText, AlternatesJson,
             ActiveAlternateIndex, AttachmentsJson, DetectedEmotion, Timestamp, PromptTokens, CompletionTokens, TotalTokens,
             ClockJson)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16);",
        params![
            m.id.to_string(),
            m.conversation_id.to_string(),
            m.role.to_i32(),
            m.speaker_character_id.map(|i| i.to_string()),
            m.effective_text(),
            m.reasoning_text,
            row_prompt(m),
            json::to_blob(&m.alternates)?,
            m.active_alternate_index,
            json::to_blob(&m.attachments)?,
            m.detected_emotion,
            time::format(&m.timestamp),
            m.usage.and_then(|u| u.prompt_tokens),
            m.usage.and_then(|u| u.completion_tokens),
            m.usage.and_then(|u| u.total_tokens),
            m.clock.as_ref().map(json::to_blob).transpose()?,
        ],
    )?;
    save_prompts(&tx, m)?;
    tx.commit()?;
    Ok(())
}

/// Updates an existing message's editable/regenerable fields - edit, swipe and regenerate all land here.
pub fn update_message(conn: &Connection, m: &ChatMessage) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE ChatMessages SET
            Text = ?1, ReasoningText = ?2, RawPromptText = ?3, AlternatesJson = ?4, ActiveAlternateIndex = ?5,
            AttachmentsJson = ?6, DetectedEmotion = ?7, ClockJson = ?8
         WHERE Id = ?9;",
        params![
            m.effective_text(),
            m.reasoning_text,
            row_prompt(m),
            json::to_blob(&m.alternates)?,
            m.active_alternate_index,
            json::to_blob(&m.attachments)?,
            m.detected_emotion,
            m.clock.as_ref().map(json::to_blob).transpose()?,
            m.id.to_string(),
        ],
    )?;
    save_prompts(&tx, m)?;
    tx.commit()?;
    Ok(())
}

/// Rewrites only a message's attachments - a background write (a picture's description) can't then
/// undo an edit or swipe that landed meanwhile.
pub fn set_attachments(conn: &Connection, id: Uuid, attachments: &[MessageAttachment]) -> Result<()> {
    conn.execute(
        "UPDATE ChatMessages SET AttachmentsJson = ?1 WHERE Id = ?2;",
        params![json::to_blob(&attachments)?, id.to_string()],
    )?;
    Ok(())
}

pub fn delete_message(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM ChatMessages WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

pub fn get_message(conn: &Connection, id: Uuid) -> Result<Option<ChatMessage>> {
    Ok(conn
        .query_row(&format!("SELECT {MESSAGE_COLUMNS} FROM ChatMessages m WHERE m.Id = ?1;"), [id.to_string()], map_message)
        .optional()?)
}

pub fn get_messages(conn: &Connection, conversation_id: Uuid) -> Result<Vec<ChatMessage>> {
    let mut stmt = conn.prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM ChatMessages m WHERE m.ConversationId = ?1 ORDER BY m.RowId;"))?;
    let list = stmt.query_map([conversation_id.to_string()], map_message)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

/// Full-text search over all chat messages, most recent match first.
pub fn search_messages(conn: &Connection, query: &str, limit: i64) -> Result<Vec<ChatMessage>> {
    let Some(fts) = build_fts_query(query) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM ChatMessages m
         JOIN ChatMessagesFts fts ON fts.rowid = m.RowId
         WHERE ChatMessagesFts MATCH ?1
         ORDER BY m.RowId DESC
         LIMIT ?2;"
    ))?;
    let list = stmt.query_map(params![fts, limit], map_message)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

/// The query is user input bound into an FTS5 MATCH expression, whose syntax (", *, NEAR, AND/OR...)
/// throws on innocent text like `what's "new"`. Quote each whitespace-separated token as a phrase
/// (doubling embedded quotes) so the input matches as literal text - an implicit AND of phrases.
fn build_fts_query(query: &str) -> Option<String> {
    let tokens: Vec<String> =
        query.split_whitespace().map(|t| format!("\"{}\"", t.replace('"', "\"\""))).collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::models::{MessageAlternate, MessageAttachment, WeatherCondition};
    use chrono::{Duration, Utc};

    fn conversation(conn: &Connection) -> Conversation {
        let c = Conversation { title: "t".into(), ..Default::default() };
        upsert(conn, &c).unwrap();
        c
    }

    #[test]
    fn conversation_round_trips_including_world_state() {
        let conn = open_in_memory().unwrap();
        let mut ws = WorldState::create_default();
        ws.weather = WeatherCondition::PartlyCloudy;
        ws.weather_notes = Some("breezy".into());
        let c = Conversation {
            mode: ConversationMode::Roleplay,
            character_id: None,
            enable_google_search: Some(true),
            world_state: Some(ws.clone()),
            ..Default::default()
        };
        upsert(&conn, &c).unwrap();
        let loaded = get(&conn, c.id).unwrap().unwrap();
        assert_eq!(loaded.world_state, Some(ws));
        assert_eq!(loaded.enable_google_search, Some(true));
        assert_eq!(loaded.mode, ConversationMode::Roleplay);
    }

    #[test]
    fn world_state_is_stored_in_the_format_the_csharp_app_wrote() {
        let conn = open_in_memory().unwrap();
        let c = Conversation { world_state: Some(WorldState::create_default()), ..Default::default() };
        upsert(&conn, &c).unwrap();
        let raw: String = conn
            .query_row("SELECT WorldStateJson FROM Conversations;", [], |r| r.get(0))
            .unwrap();
        assert!(raw.contains("\"FictionalDateTime\":\"2025-01-01T09:00:00\""), "{raw}");
        assert!(raw.contains("\"Weather\":\"Clear\""), "{raw}");
        assert!(raw.contains("\"DefaultAdvanceMinutesPerMessage\":5"), "{raw}");
    }

    #[test]
    fn update_world_state_only_touches_that_column() {
        let conn = open_in_memory().unwrap();
        let c = conversation(&conn);
        update_world_state(&conn, c.id, Some(&WorldState::create_default())).unwrap();
        assert!(get(&conn, c.id).unwrap().unwrap().world_state.is_some());
        update_world_state(&conn, c.id, None).unwrap();
        assert!(get(&conn, c.id).unwrap().unwrap().world_state.is_none());
        assert_eq!(get(&conn, c.id).unwrap().unwrap().title, "t");
    }

    #[test]
    fn most_recent_and_listings_sort_by_updated_at_desc() {
        let conn = open_in_memory().unwrap();
        let ch = Uuid::new_v4();
        // No FK target needed once we bypass: insert characters first.
        crate::repo::characters::upsert(&conn, &crate::models::Character { id: ch, ..Default::default() }).unwrap();
        let old = Conversation {
            character_id: Some(ch),
            mode: ConversationMode::Roleplay,
            updated_at: Utc::now() - Duration::hours(2),
            ..Default::default()
        };
        let new = Conversation { character_id: Some(ch), mode: ConversationMode::Roleplay, ..Default::default() };
        upsert(&conn, &old).unwrap();
        upsert(&conn, &new).unwrap();
        let plain = Conversation::default(); // assistant, no character
        upsert(&conn, &plain).unwrap();
        let assistant_with_character = Conversation {
            character_id: Some(ch),
            mode: ConversationMode::Assistant,
            updated_at: Utc::now() + Duration::minutes(1),
            ..Default::default()
        };
        upsert(&conn, &assistant_with_character).unwrap();

        assert_eq!(get_most_recent_by_character(&conn, ch).unwrap().unwrap().id, assistant_with_character.id);
        assert_eq!(
            get_most_recent_by_character_and_mode(&conn, ch, ConversationMode::Roleplay).unwrap().unwrap().id,
            new.id,
            "an assistant chat with the same character is not that character's roleplay"
        );
        assert_eq!(
            get_by_character_and_mode(&conn, ch, ConversationMode::Assistant).unwrap().iter().map(|c| c.id).collect::<Vec<_>>(),
            [assistant_with_character.id]
        );
        assert_eq!(get_by_character(&conn, ch).unwrap().len(), 3);
        assert_eq!(get_by_mode(&conn, ConversationMode::Roleplay).unwrap().len(), 2);
        assert_eq!(get_by_mode(&conn, ConversationMode::Assistant).unwrap().len(), 2);
        assert_eq!(get_assistants_without_character(&conn).unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [plain.id]);
        assert_eq!(get_most_recent_roleplay(&conn).unwrap().unwrap().id, new.id);
        assert_eq!(get_most_recent_assistant(&conn).unwrap().unwrap().id, assistant_with_character.id);
        assert!(get_most_recent_by_group_chat(&conn, Uuid::new_v4()).unwrap().is_none());
        assert_eq!(get_all(&conn).unwrap().len(), 4);
    }

    #[test]
    fn deleting_a_character_keeps_conversations_with_null_reference() {
        let conn = open_in_memory().unwrap();
        let ch = Uuid::new_v4();
        crate::repo::characters::upsert(&conn, &crate::models::Character { id: ch, ..Default::default() }).unwrap();
        let c = Conversation { character_id: Some(ch), ..Default::default() };
        upsert(&conn, &c).unwrap();
        crate::repo::characters::delete(&conn, ch).unwrap();
        assert_eq!(get(&conn, c.id).unwrap().unwrap().character_id, None);
    }

    #[test]
    fn messages_round_trip_with_alternates_attachments_and_usage() {
        let conn = open_in_memory().unwrap();
        let c = conversation(&conn);
        let mut m = ChatMessage::new(c.id, MessageRole::Character, "first");
        m.alternates = vec![
            MessageAlternate { text: "first".into(), reasoning_text: Some("r".into()), raw_prompt: None },
            MessageAlternate { text: "second".into(), reasoning_text: None, raw_prompt: Some("p".into()) },
        ];
        m.active_alternate_index = 1;
        m.attachments = vec![MessageAttachment { image_path: "a.png".into(), mime_type: "image/png".into(), description: Some("A cat.".into()) }];
        m.detected_emotion = Some("happy".into());
        m.usage = Some(LlmUsage { prompt_tokens: Some(1), completion_tokens: Some(2), total_tokens: Some(3) });
        add_message(&conn, &m).unwrap();

        let loaded = get_messages(&conn, c.id).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].text, "second"); // the active alternate wins
        assert_eq!(loaded[0].alternates[0], m.alternates[0]);
        assert_eq!(loaded[0].alternates[1].raw_prompt, None, "prompts aren't loaded with the message");
        assert!(loaded[0].has_prompt);
        assert_eq!(get_prompt(&conn, m.id).unwrap().as_deref(), Some("p"));
        assert_eq!(loaded[0].attachments, m.attachments);
        assert_eq!(loaded[0].usage, m.usage);
    }

    #[test]
    fn prompts_follow_the_shown_variant_and_survive_updates_of_a_loaded_message() {
        let conn = open_in_memory().unwrap();
        let c = conversation(&conn);
        let mut m = ChatMessage::new(c.id, MessageRole::Character, "one");
        m.raw_prompt_text = Some("prompt one".into());
        m.finalize_generation();
        add_message(&conn, &m).unwrap();

        // Regenerating a loaded message (which carries no prompts) adds the new one beside the old.
        let mut loaded = get_message(&conn, m.id).unwrap().unwrap();
        loaded.add_alternate("two".into(), None, Some("prompt two".into()));
        update_message(&conn, &loaded).unwrap();
        assert_eq!(get_prompt(&conn, m.id).unwrap().as_deref(), Some("prompt two"));

        let mut loaded = get_message(&conn, m.id).unwrap().unwrap();
        loaded.swipe_to(0);
        update_message(&conn, &loaded).unwrap();
        assert_eq!(get_prompt(&conn, m.id).unwrap().as_deref(), Some("prompt one"));

        let greeting = ChatMessage::new(c.id, MessageRole::Character, "Hello");
        add_message(&conn, &greeting).unwrap();
        assert!(!get_message(&conn, greeting.id).unwrap().unwrap().has_prompt);
        assert_eq!(get_prompt(&conn, greeting.id).unwrap(), None);

        delete_message(&conn, m.id).unwrap();
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM MessagePrompts;", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "a message's prompts go with it");
    }

    #[test]
    fn a_picture_keeps_its_draft_in_the_row() {
        let conn = open_in_memory().unwrap();
        let c = conversation(&conn);
        let mut picture = ChatMessage::new(c.id, MessageRole::Image, "a cat");
        picture.raw_prompt_text = Some("{\"prompt\":\"a cat\"}".into());
        add_message(&conn, &picture).unwrap();
        let loaded = get_message(&conn, picture.id).unwrap().unwrap();
        assert_eq!(loaded.raw_prompt_text, picture.raw_prompt_text);
        assert!(!loaded.has_prompt);
    }

    #[test]
    fn update_message_persists_edit_and_swipe() {
        let conn = open_in_memory().unwrap();
        let c = conversation(&conn);
        let mut m = ChatMessage::new(c.id, MessageRole::User, "hi");
        add_message(&conn, &m).unwrap();
        m.set_text("hello");
        m.reasoning_text = Some("because".into());
        update_message(&conn, &m).unwrap();
        let loaded = get_messages(&conn, c.id).unwrap().remove(0);
        assert_eq!(loaded.text, "hello");
        assert_eq!(loaded.reasoning_text.as_deref(), Some("because"));
        assert_eq!(loaded.usage, None);
    }

    #[test]
    fn messages_come_back_in_insertion_order_and_cascade_on_delete() {
        let conn = open_in_memory().unwrap();
        let c = conversation(&conn);
        for t in ["one", "two", "three"] {
            add_message(&conn, &ChatMessage::new(c.id, MessageRole::User, t)).unwrap();
        }
        let texts: Vec<_> = get_messages(&conn, c.id).unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, ["one", "two", "three"]);
        delete(&conn, c.id).unwrap();
        assert!(get_messages(&conn, c.id).unwrap().is_empty());
    }

    #[test]
    fn delete_message_removes_only_that_message() {
        let conn = open_in_memory().unwrap();
        let c = conversation(&conn);
        let a = ChatMessage::new(c.id, MessageRole::User, "a");
        let b = ChatMessage::new(c.id, MessageRole::User, "b");
        add_message(&conn, &a).unwrap();
        add_message(&conn, &b).unwrap();
        delete_message(&conn, a.id).unwrap();
        assert_eq!(get_messages(&conn, c.id).unwrap()[0].id, b.id);
    }

    #[test]
    fn search_finds_messages_and_tracks_edits_and_deletes() {
        let conn = open_in_memory().unwrap();
        let c = conversation(&conn);
        let mut m = ChatMessage::new(c.id, MessageRole::User, "the dragon sleeps");
        add_message(&conn, &m).unwrap();
        add_message(&conn, &ChatMessage::new(c.id, MessageRole::User, "nothing here")).unwrap();

        assert_eq!(search_messages(&conn, "dragon", 10).unwrap().len(), 1);
        m.set_text("the wyrm sleeps");
        update_message(&conn, &m).unwrap();
        assert!(search_messages(&conn, "dragon", 10).unwrap().is_empty());
        assert_eq!(search_messages(&conn, "wyrm", 10).unwrap().len(), 1);
        delete_message(&conn, m.id).unwrap();
        assert!(search_messages(&conn, "wyrm", 10).unwrap().is_empty());
    }

    #[test]
    fn search_treats_fts_syntax_as_literal_text() {
        let conn = open_in_memory().unwrap();
        let c = conversation(&conn);
        add_message(&conn, &ChatMessage::new(c.id, MessageRole::User, "what's new today")).unwrap();
        // Would be an FTS5 syntax error if passed through raw.
        assert!(search_messages(&conn, "\"what's", 10).is_ok());
        assert!(search_messages(&conn, "NEAR( AND *", 10).is_ok());
        assert_eq!(search_messages(&conn, "new today", 10).unwrap().len(), 1);
        assert!(search_messages(&conn, "   ", 10).unwrap().is_empty());
    }
}

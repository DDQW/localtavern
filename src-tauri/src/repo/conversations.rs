use rusqlite::{params, types::ToSql, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::db::{json, rows, time};
use crate::error::Result;
use crate::models::{
    ChatMessage, Conversation, ConversationMode, LlmUsage, MessageRole, WorldState,
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
        alternates: rows::blob(row, "AlternatesJson")?,
        active_alternate_index: row.get("ActiveAlternateIndex")?,
        attachments: rows::blob(row, "AttachmentsJson")?,
        detected_emotion: row.get("DetectedEmotion")?,
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

pub fn get_most_recent_by_character(conn: &Connection, character_id: Uuid) -> Result<Option<Conversation>> {
    most_recent_where(conn, "CharacterId = ?1", &character_id.to_string())
}

pub fn get_most_recent_by_group_chat(conn: &Connection, group_chat_id: Uuid) -> Result<Option<Conversation>> {
    most_recent_where(conn, "GroupChatId = ?1", &group_chat_id.to_string())
}

pub fn get_by_character(conn: &Connection, character_id: Uuid) -> Result<Vec<Conversation>> {
    all_where(conn, "CharacterId = ?1", &character_id.to_string())
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
             AssistantSystemPrompt, EnableGoogleSearch, WorldStateJson, CreatedAt, UpdatedAt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(Id) DO UPDATE SET
            Title = ?2, Mode = ?3, CharacterId = ?4, GroupChatId = ?5, PersonaId = ?6, PromptPresetId = ?7,
            ProviderId = ?8, Model = ?9, AssistantSystemPrompt = ?10, EnableGoogleSearch = ?11,
            WorldStateJson = ?12, UpdatedAt = ?14;",
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
        ],
    )?;
    Ok(())
}

/// Persists only the world-state column, so a sidebar edit or clock auto-advance doesn't re-upsert the row.
pub fn update_world_state(conn: &Connection, conversation_id: Uuid, state: Option<&WorldState>) -> Result<()> {
    let blob = state.map(json::to_blob).transpose()?;
    conn.execute(
        "UPDATE Conversations SET WorldStateJson = ?1 WHERE Id = ?2;",
        params![blob, conversation_id.to_string()],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM Conversations WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

// ---- messages ----

pub fn add_message(conn: &Connection, m: &ChatMessage) -> Result<()> {
    conn.execute(
        "INSERT INTO ChatMessages
            (Id, ConversationId, Role, SpeakerCharacterId, Text, ReasoningText, RawPromptText, AlternatesJson,
             ActiveAlternateIndex, AttachmentsJson, DetectedEmotion, Timestamp, PromptTokens, CompletionTokens, TotalTokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15);",
        params![
            m.id.to_string(),
            m.conversation_id.to_string(),
            m.role.to_i32(),
            m.speaker_character_id.map(|i| i.to_string()),
            m.effective_text(),
            m.reasoning_text,
            m.raw_prompt_text,
            json::to_blob(&m.alternates)?,
            m.active_alternate_index,
            json::to_blob(&m.attachments)?,
            m.detected_emotion,
            time::format(&m.timestamp),
            m.usage.and_then(|u| u.prompt_tokens),
            m.usage.and_then(|u| u.completion_tokens),
            m.usage.and_then(|u| u.total_tokens),
        ],
    )?;
    Ok(())
}

/// Updates an existing message's editable/regenerable fields - edit, swipe and regenerate all land here.
pub fn update_message(conn: &Connection, m: &ChatMessage) -> Result<()> {
    conn.execute(
        "UPDATE ChatMessages SET
            Text = ?1, ReasoningText = ?2, RawPromptText = ?3, AlternatesJson = ?4, ActiveAlternateIndex = ?5,
            AttachmentsJson = ?6, DetectedEmotion = ?7
         WHERE Id = ?8;",
        params![
            m.effective_text(),
            m.reasoning_text,
            m.raw_prompt_text,
            json::to_blob(&m.alternates)?,
            m.active_alternate_index,
            json::to_blob(&m.attachments)?,
            m.detected_emotion,
            m.id.to_string(),
        ],
    )?;
    Ok(())
}

pub fn delete_message(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM ChatMessages WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

pub fn get_message(conn: &Connection, id: Uuid) -> Result<Option<ChatMessage>> {
    Ok(conn
        .query_row("SELECT * FROM ChatMessages WHERE Id = ?1;", [id.to_string()], map_message)
        .optional()?)
}

pub fn get_messages(conn: &Connection, conversation_id: Uuid) -> Result<Vec<ChatMessage>> {
    let mut stmt = conn.prepare("SELECT * FROM ChatMessages WHERE ConversationId = ?1 ORDER BY RowId;")?;
    let list = stmt.query_map([conversation_id.to_string()], map_message)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

/// Full-text search over all chat messages, most recent match first.
pub fn search_messages(conn: &Connection, query: &str, limit: i64) -> Result<Vec<ChatMessage>> {
    let Some(fts) = build_fts_query(query) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT m.* FROM ChatMessages m
         JOIN ChatMessagesFts fts ON fts.rowid = m.RowId
         WHERE ChatMessagesFts MATCH ?1
         ORDER BY m.RowId DESC
         LIMIT ?2;",
    )?;
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
        upsert(&conn, &Conversation::default()).unwrap(); // assistant, no character

        assert_eq!(get_most_recent_by_character(&conn, ch).unwrap().unwrap().id, new.id);
        assert_eq!(get_by_character(&conn, ch).unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [new.id, old.id]);
        assert_eq!(get_by_mode(&conn, ConversationMode::Roleplay).unwrap().len(), 2);
        assert_eq!(get_by_mode(&conn, ConversationMode::Assistant).unwrap().len(), 1);
        assert_eq!(get_most_recent_roleplay(&conn).unwrap().unwrap().id, new.id);
        assert!(get_most_recent_by_group_chat(&conn, Uuid::new_v4()).unwrap().is_none());
        assert_eq!(get_all(&conn).unwrap().len(), 3);
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
        m.attachments = vec![MessageAttachment { image_path: "a.png".into(), mime_type: "image/png".into() }];
        m.detected_emotion = Some("happy".into());
        m.usage = Some(LlmUsage { prompt_tokens: Some(1), completion_tokens: Some(2), total_tokens: Some(3) });
        add_message(&conn, &m).unwrap();

        let loaded = get_messages(&conn, c.id).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].text, "second"); // the active alternate wins
        assert_eq!(loaded[0].alternates, m.alternates);
        assert_eq!(loaded[0].attachments, m.attachments);
        assert_eq!(loaded[0].usage, m.usage);
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

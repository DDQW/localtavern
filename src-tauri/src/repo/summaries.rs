use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::db::{rows, time};
use crate::error::Result;
use crate::models::ConversationSummary;

fn map(row: &Row) -> rusqlite::Result<ConversationSummary> {
    Ok(ConversationSummary {
        conversation_id: rows::uuid(row, "ConversationId")?,
        text: row.get("SummaryText")?,
        covers_through_message_id: rows::uuid(row, "CoversThroughMessageId")?,
        covers_through_timestamp: rows::datetime(row, "CoversThroughTimestamp")?,
        updated_at: rows::datetime(row, "UpdatedAt")?,
    })
}

pub fn get(conn: &Connection, conversation_id: Uuid) -> Result<Option<ConversationSummary>> {
    Ok(conn
        .query_row("SELECT * FROM ConversationSummaries WHERE ConversationId = ?1;", [conversation_id.to_string()], map)
        .optional()?)
}

pub fn set(conn: &Connection, s: &ConversationSummary) -> Result<()> {
    conn.execute(
        "INSERT INTO ConversationSummaries (ConversationId, SummaryText, CoversThroughMessageId, CoversThroughTimestamp, UpdatedAt)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(ConversationId) DO UPDATE SET
            SummaryText = excluded.SummaryText,
            CoversThroughMessageId = excluded.CoversThroughMessageId,
            CoversThroughTimestamp = excluded.CoversThroughTimestamp,
            UpdatedAt = excluded.UpdatedAt;",
        params![
            s.conversation_id.to_string(),
            s.text,
            s.covers_through_message_id.to_string(),
            time::format(&s.covers_through_timestamp),
            time::format(&s.updated_at),
        ],
    )?;
    Ok(())
}

pub fn clear(conn: &Connection, conversation_id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM ConversationSummaries WHERE ConversationId = ?1;", [conversation_id.to_string()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::models::{ChatMessage, Conversation, MessageRole};
    use crate::repo::conversations;

    fn summary(conversation_id: Uuid, text: &str, through: &ChatMessage) -> ConversationSummary {
        ConversationSummary {
            conversation_id,
            text: text.into(),
            covers_through_message_id: through.id,
            covers_through_timestamp: through.timestamp,
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn set_replaces_and_clear_removes_and_deleting_the_conversation_cascades() {
        let conn = open_in_memory().unwrap();
        let conversation = Conversation::default();
        conversations::upsert(&conn, &conversation).unwrap();
        let message = ChatMessage::new(conversation.id, MessageRole::User, "hi");

        set(&conn, &summary(conversation.id, "first", &message)).unwrap();
        set(&conn, &summary(conversation.id, "second", &message)).unwrap();
        let loaded = get(&conn, conversation.id).unwrap().unwrap();
        assert_eq!(loaded.text, "second");
        assert_eq!(loaded.covers_through_message_id, message.id);

        clear(&conn, conversation.id).unwrap();
        assert!(get(&conn, conversation.id).unwrap().is_none());

        set(&conn, &summary(conversation.id, "again", &message)).unwrap();
        conversations::delete(&conn, conversation.id).unwrap();
        assert!(get(&conn, conversation.id).unwrap().is_none());
    }

    #[test]
    fn uncovered_starts_after_the_cut_or_falls_back_to_its_timestamp() {
        let conversation_id = Uuid::new_v4();
        let mut messages: Vec<ChatMessage> =
            (0..4).map(|i| ChatMessage::new(conversation_id, MessageRole::User, format!("m{i}"))).collect();
        for (i, m) in messages.iter_mut().enumerate() {
            m.timestamp = chrono::Utc::now() + chrono::Duration::seconds(i as i64);
        }
        let cut = summary(conversation_id, "s", &messages[1]);

        let texts = |list: &[ChatMessage]| list.iter().map(|m| m.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(cut.uncovered(&messages)), ["m2", "m3"]);

        messages.remove(1); // the cut message is gone; its timestamp still marks the spot
        assert_eq!(texts(cut.uncovered(&messages)), ["m2", "m3"]);
    }
}

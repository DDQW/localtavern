use rusqlite::{params, Connection, Row};
use uuid::Uuid;

use crate::db::{rows, time};
use crate::error::Result;
use crate::models::{QuestLogEntry, QuestStatus};

fn map(row: &Row) -> rusqlite::Result<QuestLogEntry> {
    Ok(QuestLogEntry {
        id: rows::uuid(row, "Id")?,
        conversation_id: rows::uuid(row, "ConversationId")?,
        title: row.get("Title")?,
        description: row.get("Description")?,
        status: rows::int_enum(row, "Status", QuestStatus::from_i32)?,
        created_at: rows::datetime(row, "CreatedAt")?,
        updated_at: rows::datetime(row, "UpdatedAt")?,
    })
}

pub fn get_by_conversation(conn: &Connection, conversation_id: Uuid) -> Result<Vec<QuestLogEntry>> {
    let mut stmt = conn.prepare("SELECT * FROM QuestLogEntries WHERE ConversationId = ?1 ORDER BY CreatedAt;")?;
    let list = stmt.query_map([conversation_id.to_string()], map)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn add(conn: &Connection, e: &QuestLogEntry) -> Result<()> {
    conn.execute(
        "INSERT INTO QuestLogEntries (Id, ConversationId, Title, Description, Status, CreatedAt, UpdatedAt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);",
        params![
            e.id.to_string(),
            e.conversation_id.to_string(),
            e.title,
            e.description,
            e.status.to_i32(),
            time::format(&e.created_at),
            time::format(&e.updated_at),
        ],
    )?;
    Ok(())
}

/// Only description and status change after creation (title is the entry's identity for the extractor).
pub fn update(conn: &Connection, e: &QuestLogEntry) -> Result<()> {
    conn.execute(
        "UPDATE QuestLogEntries SET Description = ?1, Status = ?2, UpdatedAt = ?3 WHERE Id = ?4;",
        params![e.description, e.status.to_i32(), time::format(&e.updated_at), e.id.to_string()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_update_and_scope_by_conversation() {
        let conn = crate::db::open_in_memory().unwrap();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut q = QuestLogEntry::new(a, "Find the key", "in the cellar");
        add(&conn, &q).unwrap();
        add(&conn, &QuestLogEntry::new(b, "Other", "x")).unwrap();

        q.status = QuestStatus::Completed;
        q.description = "found".into();
        update(&conn, &q).unwrap();

        let loaded = get_by_conversation(&conn, a).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].status, QuestStatus::Completed);
        assert_eq!(loaded[0].description, "found");
    }
}

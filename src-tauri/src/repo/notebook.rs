use rusqlite::{Connection, Row};
use uuid::Uuid;

use crate::db::{rows, time};
use crate::error::Result;
use crate::models::CharacterNotebookEntry;

fn map(row: &Row) -> rusqlite::Result<CharacterNotebookEntry> {
    Ok(CharacterNotebookEntry {
        id: rows::uuid(row, "Id")?,
        conversation_id: rows::uuid(row, "ConversationId")?,
        character_id: rows::uuid(row, "CharacterId")?,
        content: row.get("Content")?,
        created_at: rows::datetime(row, "CreatedAt")?,
    })
}

pub fn get_by_conversation(conn: &Connection, conversation_id: Uuid) -> Result<Vec<CharacterNotebookEntry>> {
    let mut stmt =
        conn.prepare("SELECT * FROM CharacterNotebookEntries WHERE ConversationId = ?1 ORDER BY CreatedAt;")?;
    let list = stmt.query_map([conversation_id.to_string()], map)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn add(conn: &Connection, e: &CharacterNotebookEntry) -> Result<()> {
    conn.execute(
        "INSERT INTO CharacterNotebookEntries (Id, ConversationId, CharacterId, Content, CreatedAt)
         VALUES (?1, ?2, ?3, ?4, ?5);",
        [
            e.id.to_string(),
            e.conversation_id.to_string(),
            e.character_id.to_string(),
            e.content.clone(),
            time::format(&e.created_at),
        ],
    )?;
    Ok(())
}

pub fn update(conn: &Connection, e: &CharacterNotebookEntry) -> Result<()> {
    conn.execute(
        "UPDATE CharacterNotebookEntries SET Content = ?1 WHERE Id = ?2;",
        [e.content.clone(), e.id.to_string()],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM CharacterNotebookEntries WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

pub fn delete_all_for_character(conn: &Connection, conversation_id: Uuid, character_id: Uuid) -> Result<()> {
    conn.execute(
        "DELETE FROM CharacterNotebookEntries WHERE ConversationId = ?1 AND CharacterId = ?2;",
        [conversation_id.to_string(), character_id.to_string()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_scoped_per_conversation_and_character() {
        let conn = crate::db::open_in_memory().unwrap();
        let (conv, other_conv) = (Uuid::new_v4(), Uuid::new_v4());
        let (ada, bob) = (Uuid::new_v4(), Uuid::new_v4());
        let mut e = CharacterNotebookEntry::new(conv, ada, "knows the secret");
        add(&conn, &e).unwrap();
        add(&conn, &CharacterNotebookEntry::new(conv, bob, "knows nothing")).unwrap();
        add(&conn, &CharacterNotebookEntry::new(other_conv, ada, "elsewhere")).unwrap();

        e.content = "edited".into();
        update(&conn, &e).unwrap();
        assert_eq!(get_by_conversation(&conn, conv).unwrap().len(), 2);

        delete_all_for_character(&conn, conv, ada).unwrap();
        let left = get_by_conversation(&conn, conv).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].character_id, bob);
        assert_eq!(get_by_conversation(&conn, other_conv).unwrap().len(), 1);
    }
}

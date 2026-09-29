//! Hand edits to a roleplay's character notebooks (the quest log is maintained automatically and has
//! no manual editing, as in the C# app).

use rusqlite::Connection;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::models::CharacterNotebookEntry;
use crate::repo::{characters, conversations, notebook};

/// Adds a note to `character_id`'s notebook in this conversation.
pub fn add_note(conn: &Connection, conversation_id: Uuid, character_id: Uuid, content: &str) -> Result<CharacterNotebookEntry> {
    let content = content.trim();
    if content.is_empty() {
        return Err(Error::Other("A note can't be empty.".into()));
    }
    if conversations::get(conn, conversation_id)?.is_none() {
        return Err(Error::Other("Conversation not found.".into()));
    }
    if characters::get(conn, character_id)?.is_none() {
        return Err(Error::Other("Character not found.".into()));
    }
    let entry = CharacterNotebookEntry::new(conversation_id, character_id, content);
    notebook::add(conn, &entry)?;
    Ok(entry)
}

/// Rewrites a note's text.
pub fn update_note(conn: &Connection, id: Uuid, content: &str) -> Result<CharacterNotebookEntry> {
    let content = content.trim();
    if content.is_empty() {
        return Err(Error::Other("A note can't be empty - delete it instead.".into()));
    }
    let mut entry = notebook::get(conn, id)?.ok_or_else(|| Error::Other("That note no longer exists.".into()))?;
    entry.content = content.to_string();
    notebook::update(conn, &entry)?;
    Ok(entry)
}

pub fn delete_note(conn: &Connection, id: Uuid) -> Result<()> {
    notebook::delete(conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Character, Conversation};

    fn setup() -> (Connection, Uuid, Uuid) {
        let conn = crate::db::open_in_memory().unwrap();
        let character = Character { name: "Nova".into(), ..Default::default() };
        characters::upsert(&conn, &character).unwrap();
        let conversation = Conversation::default();
        conversations::upsert(&conn, &conversation).unwrap();
        (conn, conversation.id, character.id)
    }

    #[test]
    fn notes_can_be_added_edited_and_deleted() {
        let (conn, conv, nova) = setup();
        let note = add_note(&conn, conv, nova, "  Alex is afraid of heights  ").unwrap();
        assert_eq!(note.content, "Alex is afraid of heights");

        let edited = update_note(&conn, note.id, "Alex hates heights").unwrap();
        assert_eq!(edited.created_at, note.created_at);
        assert_eq!(notebook::get_by_conversation(&conn, conv).unwrap()[0].content, "Alex hates heights");

        delete_note(&conn, note.id).unwrap();
        assert!(notebook::get_by_conversation(&conn, conv).unwrap().is_empty());
        assert!(update_note(&conn, note.id, "x").is_err(), "it's gone");
    }

    #[test]
    fn empty_or_orphaned_notes_are_refused() {
        let (conn, conv, nova) = setup();
        assert!(add_note(&conn, conv, nova, "   ").is_err());
        assert!(add_note(&conn, Uuid::new_v4(), nova, "x").is_err());
        assert!(add_note(&conn, conv, Uuid::new_v4(), "x").is_err());
        let note = add_note(&conn, conv, nova, "keep").unwrap();
        assert!(update_note(&conn, note.id, "  ").is_err());
        assert_eq!(notebook::get_by_conversation(&conn, conv).unwrap()[0].content, "keep");
    }
}

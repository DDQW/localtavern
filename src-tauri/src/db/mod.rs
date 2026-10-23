//! SQLite access. The schema is shared with the original C# app: the migration scripts in
//! `migrations/` are copied verbatim and tracked by the same `SchemaVersion` table, so a database
//! created by either app opens in the other.

pub mod date;
pub mod json;
pub mod rows;
pub mod time;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{Error, Result};

/// A shared handle to the single SQLite connection. SQLite calls are short and synchronous, so
/// callers lock it for one closure and never hold it across an `.await`.
#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

impl Db {
    pub fn new(conn: Connection) -> Self {
        Self(Arc::new(Mutex::new(conn)))
    }

    pub fn with<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let guard = self.0.lock().map_err(|_| Error::Other("The database lock was poisoned by an earlier crash.".into()))?;
        f(&guard)
    }
}

/// Ordered migration scripts. Scripts are NOT idempotent (later ones use plain ALTER TABLE) - the
/// SchemaVersion gate is what makes re-running safe.
const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../../migrations/0001_initial.sql")),
    (2, include_str!("../../migrations/0002_app_settings.sql")),
    (3, include_str!("../../migrations/0003_chat_message_reasoning.sql")),
    (4, include_str!("../../migrations/0004_chat_message_alternates.sql")),
    (5, include_str!("../../migrations/0005_conversation_search_override.sql")),
    (6, include_str!("../../migrations/0006_user_memories.sql")),
    (7, include_str!("../../migrations/0007_sampling.sql")),
    (8, include_str!("../../migrations/0008_drop_raw_import_json.sql")),
    (9, include_str!("../../migrations/0009_context_size.sql")),
    (10, include_str!("../../migrations/0010_chat_message_raw_prompt.sql")),
    (11, include_str!("../../migrations/0011_expressions_and_pictures.sql")),
    (12, include_str!("../../migrations/0012_world_state_notebooks_quests.sql")),
    (13, include_str!("../../migrations/0013_reasoning_effort.sql")),
    (14, include_str!("../../migrations/0014_conversation_summaries.sql")),
    (15, include_str!("../../migrations/0015_appearance_birthdays.sql")),
    (16, include_str!("../../migrations/0016_schedules.sql")),
    (17, include_str!("../../migrations/0017_appearance_into_description.sql")),
    (18, include_str!("../../migrations/0018_character_pictures.sql")),
    (19, include_str!("../../migrations/0019_drop_notebooks_quests.sql")),
    (20, include_str!("../../migrations/0020_story_characters.sql")),
    (21, include_str!("../../migrations/0021_message_clock.sql")),
    (22, include_str!("../../migrations/0022_group_joint_replies.sql")),
    (23, include_str!("../../migrations/0023_breakouts.sql")),
    (24, include_str!("../../migrations/0024_group_scenario_setting.sql")),
    (25, include_str!("../../migrations/0025_message_prompts.sql")),
    (26, include_str!("../../migrations/0026_message_clips.sql")),
];

/// The migration that moved the saved prompts out of ChatMessages. The space they freed is only
/// given back to the disk by a VACUUM, which can't run inside the migration's transaction.
const PROMPTS_MOVED: i32 = 25;

/// Opens (creating if needed) the database at `path`, enables foreign keys and applies pending migrations.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Other(e.to_string()))?;
    }
    let mut conn = Connection::open(path)?;
    prepare(&mut conn)?;
    Ok(conn)
}

/// An in-memory database with the full schema - used by tests.
pub fn open_in_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    prepare(&mut conn)?;
    Ok(conn)
}

fn prepare(conn: &mut Connection) -> Result<()> {
    // foreign_keys is per-connection in SQLite, so it must be set on every connection.
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(conn)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS SchemaVersion (Version INTEGER NOT NULL);")?;
    let current: i32 = conn
        .query_row("SELECT Version FROM SchemaVersion LIMIT 1;", [], |r| r.get(0))
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(0),
            other => Err(other),
        })?;

    for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute("DELETE FROM SchemaVersion;", [])?;
        tx.execute("INSERT INTO SchemaVersion(Version) VALUES (?1);", [version])?;
        tx.commit()?;
    }
    if current > 0 && current < PROMPTS_MOVED {
        conn.execute_batch("VACUUM;")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_database_reaches_latest_version() {
        let conn = open_in_memory().unwrap();
        let v: i32 = conn.query_row("SELECT Version FROM SchemaVersion;", [], |r| r.get(0)).unwrap();
        assert_eq!(v, MIGRATIONS.last().unwrap().0);
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        drop(open(&path).unwrap());
        let conn = open(&path).unwrap();
        let count: i32 = conn.query_row("SELECT COUNT(*) FROM SchemaVersion;", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn appearance_text_is_folded_into_the_description() {
        // A database as version 16 left it, with text in the old AppearanceText column.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE SchemaVersion (Version INTEGER NOT NULL);").unwrap();
        for (_, sql) in MIGRATIONS.iter().filter(|(v, _)| *v <= 16) {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute("INSERT INTO SchemaVersion(Version) VALUES (16);", []).unwrap();
        let character = "INSERT INTO Characters (Id, Name, Description, Personality, Scenario, FirstMessagesJson, ExampleDialogue, \
            TagsJson, CreatedAt, UpdatedAt, AppearanceText) VALUES (?1, 'x', ?2, '', '', '[]', '', '[]', 't', 't', ?3);";
        conn.execute(character, ["a", "A baker.  ", "{{char}} is tall."]).unwrap();
        conn.execute(character, ["b", " ", "{{char}} is short."]).unwrap();
        conn.execute(character, ["c", "Untouched.", "  "]).unwrap();
        conn.execute(character, ["d", "Pasted.\n\n{{char}} is tall.\nMore.", "{{char}} is tall."]).unwrap();
        conn.execute("INSERT INTO Personas (Id, Name, Description, IsDefault, AppearanceText) VALUES ('p', 'Me', 'Kind.', 0, '{{user}} is a man.');", [])
            .unwrap();

        migrate(&mut conn).unwrap();

        let description = |table: &str, id: &str| -> String {
            conn.query_row(&format!("SELECT Description FROM {table} WHERE Id = ?1;"), [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(description("Characters", "a"), "A baker.\n\n{{char}} is tall.");
        assert_eq!(description("Characters", "b"), "{{char}} is short.");
        assert_eq!(description("Characters", "c"), "Untouched.");
        assert_eq!(description("Characters", "d"), "Pasted.\n\n{{char}} is tall.\nMore.", "already there: not added twice");
        assert_eq!(description("Personas", "p"), "Kind.\n\n{{user}} is a man.");
        assert!(conn.prepare("SELECT AppearanceText FROM Characters;").is_err(), "the column is gone");
    }

    #[test]
    fn notebook_and_quest_blocks_and_tables_are_dropped() {
        // A database as version 18 left it, with a preset holding notebook (12) and quest log (13) blocks.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE SchemaVersion (Version INTEGER NOT NULL);").unwrap();
        for (_, sql) in MIGRATIONS.iter().filter(|(v, _)| *v <= 18) {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute("INSERT INTO SchemaVersion(Version) VALUES (18);", []).unwrap();
        let preset = crate::prompting::defaults::create_roleplay_preset();
        crate::repo::prompt_presets::upsert(&conn, &preset).unwrap();
        for (id, block_type) in [("n", 12), ("q", 13)] {
            let sql = "INSERT INTO PromptBlocks (Id, PresetId, Type, Enabled, OrderIndex) VALUES (?1, ?2, ?3, 1, 99);";
            conn.execute(sql, rusqlite::params![id, preset.id.to_string(), block_type]).unwrap();
        }

        migrate(&mut conn).unwrap();

        let loaded = crate::repo::prompt_presets::get(&conn, preset.id).unwrap().unwrap();
        assert_eq!(loaded.blocks.len(), preset.blocks.len(), "the old blocks are gone and the preset loads");
        assert!(conn.prepare("SELECT * FROM CharacterNotebookEntries;").is_err());
        assert!(conn.prepare("SELECT * FROM QuestLogEntries;").is_err());
    }

    #[test]
    fn saved_prompts_move_out_of_the_message_rows() {
        // A database as version 24 left it: prompts in RawPromptText and inside AlternatesJson.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE SchemaVersion (Version INTEGER NOT NULL);").unwrap();
        for (_, sql) in MIGRATIONS.iter().filter(|(v, _)| *v <= 24) {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute("INSERT INTO SchemaVersion(Version) VALUES (24);", []).unwrap();
        conn.execute("INSERT INTO Conversations (Id, Title, Mode, CreatedAt, UpdatedAt) VALUES ('c', 't', 1, 't', 't');", [])
            .unwrap();
        let message = "INSERT INTO ChatMessages (Id, ConversationId, Role, Text, Timestamp, RawPromptText, AlternatesJson, \
            ActiveAlternateIndex) VALUES (?1, 'c', ?2, ?3, '2025-01-01T00:00:00Z', ?4, ?5, ?6);";
        let swiped = r#"[{"Text":"one","ReasoningText":null,"RawPrompt":"p1"},{"Text":"two","ReasoningText":"r","RawPrompt":"p2"}]"#;
        conn.execute(message, rusqlite::params!["a", 1, "two", "p2", swiped, 1]).unwrap();
        conn.execute(message, rusqlite::params!["b", 1, "old", "p-old", "[]", 0]).unwrap();
        conn.execute(message, rusqlite::params!["pic", 4, "a cat", "{\"prompt\":\"a cat\"}", "[]", 0]).unwrap();

        migrate(&mut conn).unwrap();

        let prompts: Vec<(String, i64, String)> = conn
            .prepare("SELECT MessageId, AlternateIndex, Prompt FROM MessagePrompts ORDER BY MessageId, AlternateIndex;")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let expected = [("a", 0, "p1"), ("a", 1, "p2"), ("b", 0, "p-old")].map(|(m, i, p)| (m.to_string(), i, p.to_string()));
        assert_eq!(prompts, expected);

        let row = |id: &str| -> (Option<String>, String) {
            conn.query_row("SELECT RawPromptText, AlternatesJson FROM ChatMessages WHERE Id = ?1;", [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
        };
        let (raw, alternates) = row("a");
        assert_eq!(raw, None);
        assert!(!alternates.contains("RawPrompt"), "{alternates}");
        let loaded: Vec<crate::models::MessageAlternate> = json::from_blob(&alternates).unwrap();
        assert_eq!(loaded.iter().map(|a| a.text.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert_eq!(loaded[1].reasoning_text.as_deref(), Some("r"));
        assert_eq!(row("pic").0.as_deref(), Some("{\"prompt\":\"a cat\"}"), "a picture keeps its draft");
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let conn = open_in_memory().unwrap();
        let result = conn.execute(
            "INSERT INTO CharacterLorebooks (CharacterId, LorebookId) VALUES ('nope', 'nope');",
            [],
        );
        assert!(result.is_err());
    }
}

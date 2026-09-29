//! SQLite access. The schema is shared with the original C# app: the migration scripts in
//! `migrations/` are copied verbatim and tracked by the same `SchemaVersion` table, so a database
//! created by either app opens in the other.

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
];

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
    fn foreign_keys_are_enforced() {
        let conn = open_in_memory().unwrap();
        let result = conn.execute(
            "INSERT INTO CharacterLorebooks (CharacterId, LorebookId) VALUES ('nope', 'nope');",
            [],
        );
        assert!(result.is_err());
    }
}

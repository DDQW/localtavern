use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::db::{json, rows};
use crate::error::Result;
use crate::models::{Lorebook, WorldInfoEntry, WorldInfoPosition};

fn map_entry(row: &Row) -> rusqlite::Result<WorldInfoEntry> {
    Ok(WorldInfoEntry {
        id: rows::uuid(row, "Id")?,
        keys: rows::blob(row, "KeysJson")?,
        secondary_keys: rows::opt_blob(row, "SecondaryKeysJson")?,
        content: row.get("Content")?,
        case_sensitive: row.get("CaseSensitive")?,
        enabled: row.get("Enabled")?,
        insertion_order: row.get("InsertionOrder")?,
        position: rows::int_enum(row, "Position", WorldInfoPosition::from_i32)?,
        depth: row.get("Depth")?,
    })
}

fn map_book(row: &Row) -> rusqlite::Result<Lorebook> {
    Ok(Lorebook {
        id: rows::uuid(row, "Id")?,
        name: row.get("Name")?,
        entries: Vec::new(), // filled in by the caller
        scan_depth: row.get("ScanDepth")?,
        token_budget: row.get("TokenBudget")?,
    })
}

fn entries_of(conn: &Connection, id: Uuid) -> Result<Vec<WorldInfoEntry>> {
    let mut stmt = conn.prepare("SELECT * FROM WorldInfoEntries WHERE LorebookId = ?1 ORDER BY InsertionOrder;")?;
    let list = stmt.query_map([id.to_string()], map_entry)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn get(conn: &Connection, id: Uuid) -> Result<Option<Lorebook>> {
    let found = conn
        .query_row("SELECT * FROM Lorebooks WHERE Id = ?1;", [id.to_string()], map_book)
        .optional()?;
    match found {
        Some(mut b) => {
            b.entries = entries_of(conn, id)?;
            Ok(Some(b))
        }
        None => Ok(None),
    }
}

pub fn get_all(conn: &Connection) -> Result<Vec<Lorebook>> {
    let mut stmt = conn.prepare("SELECT * FROM Lorebooks ORDER BY Name COLLATE NOCASE;")?;
    let mut books = stmt.query_map([], map_book)?.collect::<rusqlite::Result<Vec<_>>>()?;
    for b in &mut books {
        b.entries = entries_of(conn, b.id)?;
    }
    Ok(books)
}

pub fn upsert(conn: &Connection, book: &Lorebook) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let id = book.id.to_string();
    tx.execute(
        "INSERT INTO Lorebooks (Id, Name, ScanDepth, TokenBudget) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(Id) DO UPDATE SET Name = ?2, ScanDepth = ?3, TokenBudget = ?4;",
        params![id, book.name, book.scan_depth, book.token_budget],
    )?;
    tx.execute("DELETE FROM WorldInfoEntries WHERE LorebookId = ?1;", [&id])?;
    for e in &book.entries {
        let secondary = e.secondary_keys.as_ref().map(json::to_blob).transpose()?;
        tx.execute(
            "INSERT INTO WorldInfoEntries
                (Id, LorebookId, KeysJson, SecondaryKeysJson, Content, CaseSensitive, Enabled, InsertionOrder, Position, Depth)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10);",
            params![
                e.id.to_string(),
                id,
                json::to_blob(&e.keys)?,
                secondary,
                e.content,
                e.case_sensitive,
                e.enabled,
                e.insertion_order,
                e.position.to_i32(),
                e.depth,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM Lorebooks WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn entry(order: i32, key: &str) -> WorldInfoEntry {
        WorldInfoEntry {
            keys: vec![key.into()],
            content: format!("about {key}"),
            insertion_order: order,
            ..Default::default()
        }
    }

    #[test]
    fn entries_round_trip_in_insertion_order() {
        let conn = open_in_memory().unwrap();
        let mut second = entry(2, "b");
        second.secondary_keys = Some(vec!["x".into()]);
        second.position = WorldInfoPosition::AtDepth;
        second.depth = 3;
        let book = Lorebook { name: "L".into(), entries: vec![second.clone(), entry(1, "a")], ..Default::default() };
        upsert(&conn, &book).unwrap();

        let loaded = get(&conn, book.id).unwrap().unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].keys, ["a"]);
        assert_eq!(loaded.entries[1], second);
        assert_eq!(loaded.entries[0].secondary_keys, None);
    }

    #[test]
    fn upsert_replaces_entries() {
        let conn = open_in_memory().unwrap();
        let mut book = Lorebook { entries: vec![entry(1, "a"), entry(2, "b")], ..Default::default() };
        upsert(&conn, &book).unwrap();
        book.entries.truncate(1);
        upsert(&conn, &book).unwrap();
        assert_eq!(get(&conn, book.id).unwrap().unwrap().entries.len(), 1);
    }

    #[test]
    fn deleting_a_lorebook_cascades_to_entries() {
        let conn = open_in_memory().unwrap();
        let book = Lorebook { entries: vec![entry(1, "a")], ..Default::default() };
        upsert(&conn, &book).unwrap();
        delete(&conn, book.id).unwrap();
        let n: i32 = conn.query_row("SELECT COUNT(*) FROM WorldInfoEntries;", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}

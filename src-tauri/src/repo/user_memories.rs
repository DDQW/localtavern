use rusqlite::{Connection, Row};
use uuid::Uuid;

use crate::db::{rows, time};
use crate::error::Result;
use crate::models::UserMemory;

fn map(row: &Row) -> rusqlite::Result<UserMemory> {
    Ok(UserMemory { id: rows::uuid(row, "Id")?, content: row.get("Content")?, created_at: rows::datetime(row, "CreatedAt")? })
}

pub fn get_all(conn: &Connection) -> Result<Vec<UserMemory>> {
    let mut stmt = conn.prepare("SELECT * FROM UserMemories ORDER BY CreatedAt;")?;
    let list = stmt.query_map([], map)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn add(conn: &Connection, m: &UserMemory) -> Result<()> {
    conn.execute(
        "INSERT INTO UserMemories (Id, Content, CreatedAt) VALUES (?1, ?2, ?3);",
        [m.id.to_string(), m.content.clone(), time::format(&m.created_at)],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM UserMemories WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

pub fn delete_all(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM UserMemories;", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_list_delete() {
        let conn = crate::db::open_in_memory().unwrap();
        let a = UserMemory::new("likes tea");
        let b = UserMemory::new("has a cat");
        add(&conn, &a).unwrap();
        add(&conn, &b).unwrap();
        assert_eq!(get_all(&conn).unwrap().len(), 2);
        delete(&conn, a.id).unwrap();
        assert_eq!(get_all(&conn).unwrap()[0].content, "has a cat");
        delete_all(&conn).unwrap();
        assert!(get_all(&conn).unwrap().is_empty());
    }
}

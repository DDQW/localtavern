use rusqlite::{Connection, OptionalExtension};

use crate::error::Result;

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT Value FROM AppSettings WHERE Key = ?1;", [key], |r| r.get(0))
        .optional()?)
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO AppSettings (Key, Value) VALUES (?1, ?2) ON CONFLICT(Key) DO UPDATE SET Value = ?2;",
        [key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_get_and_overwrite() {
        let conn = crate::db::open_in_memory().unwrap();
        assert_eq!(get(&conn, "k").unwrap(), None);
        set(&conn, "k", "v1").unwrap();
        set(&conn, "k", "v2").unwrap();
        assert_eq!(get(&conn, "k").unwrap().as_deref(), Some("v2"));
    }
}

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::db::{date, json, rows};
use crate::error::Result;
use crate::models::Persona;

fn map(row: &Row) -> rusqlite::Result<Persona> {
    Ok(Persona {
        id: rows::uuid(row, "Id")?,
        name: row.get("Name")?,
        avatar_path: row.get("AvatarPath")?,
        description: row.get("Description")?,
        is_default: row.get("IsDefault")?,
        appearance: rows::opt_blob(row, "AppearanceJson")?,
        birth_date: rows::opt_date(row, "BirthDate")?,
    })
}

pub fn get(conn: &Connection, id: Uuid) -> Result<Option<Persona>> {
    Ok(conn.query_row("SELECT * FROM Personas WHERE Id = ?1;", [id.to_string()], map).optional()?)
}

pub fn get_all(conn: &Connection) -> Result<Vec<Persona>> {
    let mut stmt = conn.prepare("SELECT * FROM Personas ORDER BY Name COLLATE NOCASE;")?;
    let list = stmt.query_map([], map)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn upsert(conn: &Connection, p: &Persona) -> Result<()> {
    conn.execute(
        "INSERT INTO Personas (Id, Name, AvatarPath, Description, IsDefault, AppearanceJson, BirthDate)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(Id) DO UPDATE SET Name = ?2, AvatarPath = ?3, Description = ?4, IsDefault = ?5,
            AppearanceJson = ?6, BirthDate = ?7;",
        params![
            p.id.to_string(),
            p.name,
            p.avatar_path,
            p.description,
            p.is_default,
            p.appearance.as_ref().map(json::to_blob).transpose()?,
            date::format_opt(p.birth_date),
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM Personas WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_update_delete() {
        let conn = crate::db::open_in_memory().unwrap();
        let mut p = Persona { name: "Me".into(), is_default: true, ..Default::default() };
        upsert(&conn, &p).unwrap();
        assert_eq!(get(&conn, p.id).unwrap().unwrap(), p);
        p.description = "d".into();
        upsert(&conn, &p).unwrap();
        assert_eq!(get_all(&conn).unwrap()[0].description, "d");
        delete(&conn, p.id).unwrap();
        assert!(get(&conn, p.id).unwrap().is_none());
    }

    #[test]
    fn round_trips_appearance_and_birth_date() {
        use crate::models::{Appearance, Sex};
        let conn = crate::db::open_in_memory().unwrap();
        let p = Persona {
            name: "Alex".into(),
            appearance: Some(Appearance { sex: Some(Sex::Male), eye_color: Some("grey".into()), ..Default::default() }),
            birth_date: chrono::NaiveDate::from_ymd_opt(1996, 2, 29),
            ..Default::default()
        };
        upsert(&conn, &p).unwrap();
        assert_eq!(get(&conn, p.id).unwrap().unwrap(), p);
    }
}

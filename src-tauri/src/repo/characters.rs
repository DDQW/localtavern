use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::db::{date, json, rows, time};
use crate::error::Result;
use crate::models::Character;

fn map(row: &Row) -> rusqlite::Result<Character> {
    Ok(Character {
        id: rows::uuid(row, "Id")?,
        name: row.get("Name")?,
        avatar_path: row.get("AvatarPath")?,
        description: row.get("Description")?,
        personality: row.get("Personality")?,
        scenario: row.get("Scenario")?,
        first_messages: rows::blob(row, "FirstMessagesJson")?,
        example_dialogue: row.get("ExampleDialogue")?,
        tags: rows::blob(row, "TagsJson")?,
        linked_lorebook_ids: Vec::new(), // filled in by the caller
        expressions: rows::blob(row, "ExpressionsJson")?,
        gallery_image_paths: rows::blob(row, "GalleryImagePathsJson")?,
        appearance: rows::opt_blob(row, "AppearanceJson")?,
        appearance_text: row.get("AppearanceText")?,
        birth_date: rows::opt_date(row, "BirthDate")?,
        schedule: rows::opt_blob(row, "ScheduleJson")?,
        created_at: rows::datetime(row, "CreatedAt")?,
        updated_at: rows::datetime(row, "UpdatedAt")?,
    })
}

fn linked_lorebooks(conn: &Connection, id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt = conn.prepare("SELECT LorebookId FROM CharacterLorebooks WHERE CharacterId = ?1;")?;
    let ids = stmt
        .query_map([id.to_string()], |r| rows::uuid(r, "LorebookId"))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

pub fn get(conn: &Connection, id: Uuid) -> Result<Option<Character>> {
    let found = conn
        .query_row("SELECT * FROM Characters WHERE Id = ?1;", [id.to_string()], map)
        .optional()?;
    match found {
        Some(mut c) => {
            c.linked_lorebook_ids = linked_lorebooks(conn, id)?;
            Ok(Some(c))
        }
        None => Ok(None),
    }
}

pub fn get_all(conn: &Connection) -> Result<Vec<Character>> {
    let mut stmt = conn.prepare("SELECT * FROM Characters ORDER BY Name COLLATE NOCASE;")?;
    let mut characters = stmt.query_map([], map)?.collect::<rusqlite::Result<Vec<_>>>()?;
    for c in &mut characters {
        c.linked_lorebook_ids = linked_lorebooks(conn, c.id)?;
    }
    Ok(characters)
}

pub fn upsert(conn: &Connection, c: &Character) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let id = c.id.to_string();
    tx.execute(
        "INSERT INTO Characters
            (Id, Name, AvatarPath, Description, Personality, Scenario, FirstMessagesJson, ExampleDialogue, TagsJson, ExpressionsJson, GalleryImagePathsJson, CreatedAt, UpdatedAt, AppearanceJson, AppearanceText, BirthDate, ScheduleJson)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(Id) DO UPDATE SET
            Name = ?2, AvatarPath = ?3, Description = ?4, Personality = ?5, Scenario = ?6,
            FirstMessagesJson = ?7, ExampleDialogue = ?8, TagsJson = ?9, ExpressionsJson = ?10,
            GalleryImagePathsJson = ?11, UpdatedAt = ?13, AppearanceJson = ?14, AppearanceText = ?15,
            BirthDate = ?16, ScheduleJson = ?17;",
        params![
            id,
            c.name,
            c.avatar_path,
            c.description,
            c.personality,
            c.scenario,
            json::to_blob(&c.first_messages)?,
            c.example_dialogue,
            json::to_blob(&c.tags)?,
            json::to_blob(&c.expressions)?,
            json::to_blob(&c.gallery_image_paths)?,
            time::format(&c.created_at),
            time::format(&c.updated_at),
            c.appearance.as_ref().map(json::to_blob).transpose()?,
            c.appearance_text,
            date::format_opt(c.birth_date),
            c.schedule.as_ref().map(json::to_blob).transpose()?,
        ],
    )?;
    tx.execute("DELETE FROM CharacterLorebooks WHERE CharacterId = ?1;", [&id])?;
    for lorebook_id in &c.linked_lorebook_ids {
        tx.execute(
            "INSERT INTO CharacterLorebooks (CharacterId, LorebookId) VALUES (?1, ?2);",
            [&id, &lorebook_id.to_string()],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM Characters WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::models::{ExpressionImage, Lorebook};
    use crate::repo::lorebooks;

    #[test]
    fn round_trips_all_fields_including_linked_lorebooks() {
        let conn = open_in_memory().unwrap();
        let book = Lorebook { name: "World".into(), ..Default::default() };
        lorebooks::upsert(&conn, &book).unwrap();

        let c = Character {
            name: "Ada".into(),
            avatar_path: Some("C:\\a.png".into()),
            first_messages: vec!["Hi".into(), "Hello \"there\"".into()],
            tags: vec!["x".into()],
            expressions: vec![ExpressionImage { emotion: "happy".into(), image_path: "h.png".into() }],
            gallery_image_paths: vec!["g.png".into()],
            linked_lorebook_ids: vec![book.id],
            ..Default::default()
        };
        upsert(&conn, &c).unwrap();

        let loaded = get(&conn, c.id).unwrap().unwrap();
        assert_eq!(loaded.name, c.name);
        assert_eq!(loaded.first_messages, c.first_messages);
        assert_eq!(loaded.expressions, c.expressions);
        assert_eq!(loaded.linked_lorebook_ids, vec![book.id]);
        assert_eq!(loaded.avatar_path, c.avatar_path);
    }

    #[test]
    fn round_trips_appearance_and_birth_date() {
        use crate::models::{Appearance, Sex};
        let conn = open_in_memory().unwrap();
        let mut c = Character {
            name: "Mira".into(),
            appearance: Some(Appearance {
                sex: Some(Sex::Female),
                height_cm: Some(168),
                hair_color: Some("auburn".into()),
                marks: vec!["freckles".into()],
                ..Default::default()
            }),
            appearance_text: "{{char}} is a woman.".into(),
            birth_date: chrono::NaiveDate::from_ymd_opt(2001, 3, 3),
            ..Default::default()
        };
        c.appearance.as_mut().unwrap().female.cup_size = Some("C".into());
        upsert(&conn, &c).unwrap();
        let loaded = get(&conn, c.id).unwrap().unwrap();
        assert_eq!(loaded.appearance, c.appearance);
        assert_eq!(loaded.appearance_text, c.appearance_text);
        assert_eq!(loaded.birth_date, c.birth_date);

        let mut schedule = crate::models::Schedule { home_country: Some("DE".into()), ..Default::default() };
        schedule.days[0].kind = crate::models::DayKind::Work;
        c.schedule = Some(schedule);
        upsert(&conn, &c).unwrap();
        assert_eq!(get(&conn, c.id).unwrap().unwrap().schedule, c.schedule);

        c.appearance = None;
        c.birth_date = None;
        upsert(&conn, &c).unwrap();
        let cleared = get(&conn, c.id).unwrap().unwrap();
        assert_eq!((cleared.appearance, cleared.birth_date), (None, None));
    }

    #[test]
    fn upsert_updates_existing_and_replaces_links() {
        let conn = open_in_memory().unwrap();
        let mut c = Character { name: "A".into(), ..Default::default() };
        upsert(&conn, &c).unwrap();
        c.name = "B".into();
        upsert(&conn, &c).unwrap();
        assert_eq!(get_all(&conn).unwrap().len(), 1);
        assert_eq!(get(&conn, c.id).unwrap().unwrap().name, "B");
    }

    #[test]
    fn get_all_sorts_case_insensitively() {
        let conn = open_in_memory().unwrap();
        for n in ["bob", "Alice", "carl"] {
            upsert(&conn, &Character { name: n.into(), ..Default::default() }).unwrap();
        }
        let names: Vec<_> = get_all(&conn).unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Alice", "bob", "carl"]);
    }

    #[test]
    fn missing_returns_none_and_delete_removes() {
        let conn = open_in_memory().unwrap();
        assert!(get(&conn, Uuid::new_v4()).unwrap().is_none());
        let c = Character::default();
        upsert(&conn, &c).unwrap();
        delete(&conn, c.id).unwrap();
        assert!(get(&conn, c.id).unwrap().is_none());
    }
}

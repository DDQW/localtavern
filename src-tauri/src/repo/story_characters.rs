use rusqlite::{params, Connection, Row};
use uuid::Uuid;

use crate::db::{rows, time};
use crate::error::Result;
use crate::models::StoryCharacter;

const STORY_TIME: &str = "%Y-%m-%dT%H:%M:%S";

fn map(row: &Row) -> rusqlite::Result<StoryCharacter> {
    let introduced_at: Option<String> = row.get("IntroducedAt")?;
    Ok(StoryCharacter {
        id: rows::uuid(row, "Id")?,
        conversation_id: rows::uuid(row, "ConversationId")?,
        name: row.get("Name")?,
        age: row.get("Age")?,
        role: row.get("Role")?,
        appearance: row.get("Appearance")?,
        adult_details: row.get("AdultDetails")?,
        introduced_at: introduced_at.and_then(|s| chrono::NaiveDateTime::parse_from_str(&s, STORY_TIME).ok()),
        introduced_message_id: rows::opt_uuid(row, "IntroducedMessageId")?,
        created_at: rows::datetime(row, "CreatedAt")?,
    })
}

/// In the order they were introduced.
pub fn for_conversation(conn: &Connection, conversation_id: Uuid) -> Result<Vec<StoryCharacter>> {
    let mut stmt = conn.prepare("SELECT * FROM StoryCharacters WHERE ConversationId = ?1 ORDER BY CreatedAt, rowid;")?;
    let list = stmt.query_map([conversation_id.to_string()], map)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn upsert(conn: &Connection, c: &StoryCharacter) -> Result<()> {
    conn.execute(
        "INSERT INTO StoryCharacters (Id, ConversationId, Name, Age, Role, Appearance, AdultDetails, IntroducedAt, IntroducedMessageId, CreatedAt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(Id) DO UPDATE SET
            Name = excluded.Name, Age = excluded.Age, Role = excluded.Role, Appearance = excluded.Appearance,
            AdultDetails = excluded.AdultDetails, IntroducedAt = excluded.IntroducedAt;",
        params![
            c.id.to_string(),
            c.conversation_id.to_string(),
            c.name,
            c.age,
            c.role,
            c.appearance,
            c.adult_details,
            c.introduced_at.map(|t| t.format(STORY_TIME).to_string()),
            c.introduced_message_id.map(|id| id.to_string()),
            time::format(&c.created_at),
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM StoryCharacters WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::models::Conversation;
    use crate::repo::conversations;

    #[test]
    fn round_trips_and_cascades_with_the_conversation() {
        let conn = open_in_memory().unwrap();
        let conversation = Conversation::default();
        conversations::upsert(&conn, &conversation).unwrap();
        let mut c = StoryCharacter::new(conversation.id, "Tom");
        c.age = Some(34);
        c.appearance = "tall, red beard".into();
        c.introduced_at = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap().and_hms_opt(18, 30, 0);
        upsert(&conn, &c).unwrap();
        assert_eq!(for_conversation(&conn, conversation.id).unwrap(), vec![c.clone()]);

        c.role = "bartender".into();
        upsert(&conn, &c).unwrap();
        assert_eq!(for_conversation(&conn, conversation.id).unwrap()[0].role, "bartender");

        conversations::delete(&conn, conversation.id).unwrap();
        assert!(for_conversation(&conn, conversation.id).unwrap().is_empty());
    }

    #[test]
    fn adult_details_are_no_longer_filtered_by_age() {
        let mut c = StoryCharacter::new(Uuid::new_v4(), " Eve ");
        c.adult_details = "x".into();
        c.enforce_rules();
        assert_eq!(c.adult_details, "x");
        assert_eq!(c.name, "Eve");
        c.age = Some(25);
        c.adult_details = "y".into();
        c.enforce_rules();
        assert_eq!(c.adult_details, "y");
        c.age = Some(17);
        c.adult_details = "z".into();
        c.enforce_rules();
        assert_eq!(c.adult_details, "z");
    }
}

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::db::{json, rows};
use crate::error::Result;
use crate::models::{GroupActivationMode, GroupChat};

fn map(row: &Row) -> rusqlite::Result<GroupChat> {
    Ok(GroupChat {
        id: rows::uuid(row, "Id")?,
        name: row.get("Name")?,
        character_ids: Vec::new(), // filled in by the caller
        activation_mode: rows::int_enum(row, "ActivationMode", GroupActivationMode::from_i32)?,
        allow_self_responses: row.get("AllowSelfResponses")?,
        joint_replies: row.get("JointReplies")?,
        private_talks: row.get("PrivateTalks")?,
        scenario: row.get("Scenario")?,
        setting: rows::opt_blob(row, "SettingJson")?.unwrap_or_default(),
    })
}

fn members(conn: &Connection, id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt =
        conn.prepare("SELECT CharacterId FROM GroupChatMembers WHERE GroupChatId = ?1 ORDER BY OrderIndex;")?;
    let ids = stmt
        .query_map([id.to_string()], |r| rows::uuid(r, "CharacterId"))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

pub fn get(conn: &Connection, id: Uuid) -> Result<Option<GroupChat>> {
    let found = conn.query_row("SELECT * FROM GroupChats WHERE Id = ?1;", [id.to_string()], map).optional()?;
    match found {
        Some(mut g) => {
            g.character_ids = members(conn, id)?;
            Ok(Some(g))
        }
        None => Ok(None),
    }
}

pub fn get_all(conn: &Connection) -> Result<Vec<GroupChat>> {
    let mut stmt = conn.prepare("SELECT * FROM GroupChats ORDER BY Name COLLATE NOCASE;")?;
    let mut groups = stmt.query_map([], map)?.collect::<rusqlite::Result<Vec<_>>>()?;
    for g in &mut groups {
        g.character_ids = members(conn, g.id)?;
    }
    Ok(groups)
}

pub fn upsert(conn: &Connection, g: &GroupChat) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let id = g.id.to_string();
    tx.execute(
        "INSERT INTO GroupChats (Id, Name, ActivationMode, AllowSelfResponses, JointReplies, PrivateTalks, Scenario, SettingJson)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(Id) DO UPDATE SET Name = ?2, ActivationMode = ?3, AllowSelfResponses = ?4, JointReplies = ?5,
            PrivateTalks = ?6, Scenario = ?7, SettingJson = ?8;",
        params![
            id,
            g.name,
            g.activation_mode.to_i32(),
            g.allow_self_responses,
            g.joint_replies,
            g.private_talks,
            g.scenario,
            json::to_blob(&g.setting)?,
        ],
    )?;
    tx.execute("DELETE FROM GroupChatMembers WHERE GroupChatId = ?1;", [&id])?;
    for (index, character_id) in g.character_ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO GroupChatMembers (GroupChatId, CharacterId, OrderIndex) VALUES (?1, ?2, ?3);",
            params![id, character_id.to_string(), index as i32],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM GroupChats WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::models::{Character, GroupSetting};
    use crate::repo::characters;

    fn character(conn: &Connection, name: &str) -> Uuid {
        let c = Character { name: name.into(), ..Default::default() };
        characters::upsert(conn, &c).unwrap();
        c.id
    }

    #[test]
    fn member_order_is_preserved() {
        let conn = open_in_memory().unwrap();
        let ids = [character(&conn, "c"), character(&conn, "a"), character(&conn, "b")];
        let g = GroupChat {
            name: "G".into(),
            character_ids: ids.to_vec(),
            activation_mode: GroupActivationMode::RoundRobin,
            allow_self_responses: true,
            joint_replies: true,
            private_talks: false,
            scenario: "Shipwrecked on {{user}}'s island.".into(),
            setting: GroupSetting {
                country_code: Some("DE".into()),
                region_code: Some("DE-BY".into()),
                story_start: chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap().and_hms_opt(18, 30, 0),
            },
            ..Default::default()
        };
        upsert(&conn, &g).unwrap();
        assert_eq!(get(&conn, g.id).unwrap().unwrap(), g);
    }

    #[test]
    fn deleting_a_character_removes_it_from_groups() {
        let conn = open_in_memory().unwrap();
        let keep = character(&conn, "keep");
        let drop = character(&conn, "drop");
        let g = GroupChat { character_ids: vec![keep, drop], ..Default::default() };
        upsert(&conn, &g).unwrap();
        characters::delete(&conn, drop).unwrap();
        assert_eq!(get(&conn, g.id).unwrap().unwrap().character_ids, vec![keep]);
    }
}

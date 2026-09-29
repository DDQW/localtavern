use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::db::rows;
use crate::error::Result;
use crate::models::{PromptBlock, PromptBlockType, PromptPreset, ReasoningEffort};

fn map_preset(row: &Row) -> rusqlite::Result<PromptPreset> {
    Ok(PromptPreset {
        id: rows::uuid(row, "Id")?,
        name: row.get("Name")?,
        blocks: Vec::new(), // filled in by the caller
        temperature: row.get("Temperature")?,
        top_p: row.get("TopP")?,
        top_k: row.get("TopK")?,
        max_output_tokens: row.get("MaxOutputTokens")?,
        reasoning_effort: rows::opt_int_enum(row, "ReasoningEffort", ReasoningEffort::from_i32)?,
        max_context_tokens: row.get("MaxContextTokens")?,
    })
}

fn map_block(row: &Row) -> rusqlite::Result<PromptBlock> {
    Ok(PromptBlock {
        id: rows::uuid(row, "Id")?,
        block_type: rows::int_enum(row, "Type", PromptBlockType::from_i32)?,
        enabled: row.get("Enabled")?,
        order: row.get("OrderIndex")?,
        custom_text: row.get("CustomText")?,
        injection_depth: row.get("InjectionDepth")?,
    })
}

fn blocks_of(conn: &Connection, id: Uuid) -> Result<Vec<PromptBlock>> {
    let mut stmt = conn.prepare("SELECT * FROM PromptBlocks WHERE PresetId = ?1 ORDER BY OrderIndex;")?;
    let list = stmt.query_map([id.to_string()], map_block)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(list)
}

pub fn get(conn: &Connection, id: Uuid) -> Result<Option<PromptPreset>> {
    let found = conn
        .query_row("SELECT * FROM PromptPresets WHERE Id = ?1;", [id.to_string()], map_preset)
        .optional()?;
    match found {
        Some(mut p) => {
            p.blocks = blocks_of(conn, id)?;
            Ok(Some(p))
        }
        None => Ok(None),
    }
}

pub fn get_all(conn: &Connection) -> Result<Vec<PromptPreset>> {
    let mut stmt = conn.prepare("SELECT * FROM PromptPresets ORDER BY Name COLLATE NOCASE;")?;
    let mut presets = stmt.query_map([], map_preset)?.collect::<rusqlite::Result<Vec<_>>>()?;
    for p in &mut presets {
        p.blocks = blocks_of(conn, p.id)?;
    }
    Ok(presets)
}

pub fn upsert(conn: &Connection, p: &PromptPreset) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let id = p.id.to_string();
    tx.execute(
        "INSERT INTO PromptPresets (Id, Name, Temperature, TopP, TopK, MaxOutputTokens, MaxContextTokens, ReasoningEffort)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(Id) DO UPDATE SET
            Name = ?2, Temperature = ?3, TopP = ?4, TopK = ?5, MaxOutputTokens = ?6,
            MaxContextTokens = ?7, ReasoningEffort = ?8;",
        params![
            id,
            p.name,
            p.temperature,
            p.top_p,
            p.top_k,
            p.max_output_tokens,
            p.max_context_tokens,
            p.reasoning_effort.map(ReasoningEffort::to_i32),
        ],
    )?;
    tx.execute("DELETE FROM PromptBlocks WHERE PresetId = ?1;", [&id])?;
    for b in &p.blocks {
        tx.execute(
            "INSERT INTO PromptBlocks (Id, PresetId, Type, Enabled, OrderIndex, CustomText, InjectionDepth)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);",
            params![b.id.to_string(), id, b.block_type.to_i32(), b.enabled, b.order, b.custom_text, b.injection_depth],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn delete(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM PromptPresets WHERE Id = ?1;", [id.to_string()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn preset_with_sampling_and_blocks_round_trips() {
        let conn = open_in_memory().unwrap();
        let mut custom = PromptBlock::new(PromptBlockType::Custom, 1);
        custom.custom_text = Some("hi".into());
        custom.injection_depth = 2;
        custom.enabled = false;
        let p = PromptPreset {
            name: "P".into(),
            blocks: vec![PromptBlock::new(PromptBlockType::SystemPrompt, 0), custom],
            temperature: Some(0.7),
            top_p: Some(0.9),
            top_k: Some(40),
            max_output_tokens: Some(2048),
            reasoning_effort: Some(ReasoningEffort::High),
            max_context_tokens: Some(8000),
            ..Default::default()
        };
        upsert(&conn, &p).unwrap();
        assert_eq!(get(&conn, p.id).unwrap().unwrap(), p);
    }

    #[test]
    fn null_sampling_stays_null_and_blocks_are_ordered() {
        let conn = open_in_memory().unwrap();
        let p = PromptPreset {
            blocks: vec![PromptBlock::new(PromptBlockType::ChatHistory, 5), PromptBlock::new(PromptBlockType::Scenario, 1)],
            ..Default::default()
        };
        upsert(&conn, &p).unwrap();
        let loaded = get(&conn, p.id).unwrap().unwrap();
        assert_eq!(loaded.temperature, None);
        assert_eq!(loaded.reasoning_effort, None);
        assert_eq!(loaded.blocks[0].block_type, PromptBlockType::Scenario);
    }

    #[test]
    fn upsert_replaces_blocks() {
        let conn = open_in_memory().unwrap();
        let mut p = PromptPreset { blocks: vec![PromptBlock::new(PromptBlockType::Scenario, 0)], ..Default::default() };
        upsert(&conn, &p).unwrap();
        p.blocks.clear();
        upsert(&conn, &p).unwrap();
        assert!(get(&conn, p.id).unwrap().unwrap().blocks.is_empty());
    }
}

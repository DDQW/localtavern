//! Prompt-preset resolution for chats. Seeds the two built-in presets ("Assistant (default)",
//! "Roleplay (default)") into an empty table on first use; conversations without an explicit preset
//! pick the one matching their mode. Older installs get missing blocks backfilled in place.

use rusqlite::Connection;

use crate::error::Result;
use crate::models::{Conversation, ConversationMode, PromptBlock, PromptBlockType, PromptPreset};
use crate::prompting::defaults::{create_assistant_preset, create_roleplay_preset};
use crate::repo::prompt_presets;

const ASSISTANT_DEFAULT: &str = "Assistant (default)";
const ROLEPLAY_DEFAULT: &str = "Roleplay (default)";

pub fn get_all_or_seed(conn: &Connection) -> Result<Vec<PromptPreset>> {
    let all = prompt_presets::get_all(conn)?;
    if all.is_empty() {
        prompt_presets::upsert(conn, &create_assistant_preset())?;
        prompt_presets::upsert(conn, &create_roleplay_preset())?;
        return prompt_presets::get_all(conn);
    }

    let all = backfill_persona_block(conn, all)?;
    backfill_world_system_blocks(conn, all)
}

/// "Assistant (default)" presets seeded before persona support don't have a PersonaDescription
/// block, so a persona picked in Assistant mode never reached the model - add it in place, once.
fn backfill_persona_block(conn: &Connection, all: Vec<PromptPreset>) -> Result<Vec<PromptPreset>> {
    let Some(mut preset) = all.iter().find(|p| p.name == ASSISTANT_DEFAULT).cloned() else {
        return Ok(all);
    };
    if preset.blocks.iter().any(|b| b.block_type == PromptBlockType::PersonaDescription) {
        return Ok(all);
    }

    let system_order = preset
        .blocks
        .iter()
        .find(|b| b.block_type == PromptBlockType::SystemPrompt)
        .map_or(-1, |b| b.order);
    preset.blocks.push(PromptBlock::new(PromptBlockType::PersonaDescription, system_order + 1));
    prompt_presets::upsert(conn, &preset)?;
    prompt_presets::get_all(conn)
}

/// "Roleplay (default)" presets seeded before the world-state/notebook/quest-log feature don't have
/// those blocks - add whichever are missing, once.
fn backfill_world_system_blocks(conn: &Connection, all: Vec<PromptPreset>) -> Result<Vec<PromptPreset>> {
    let Some(mut preset) = all.iter().find(|p| p.name == ROLEPLAY_DEFAULT).cloned() else {
        return Ok(all);
    };
    let missing: Vec<_> = [PromptBlockType::WorldState, PromptBlockType::CharacterNotebook, PromptBlockType::QuestLog]
        .into_iter()
        .filter(|t| preset.blocks.iter().all(|b| b.block_type != *t))
        .collect();
    if missing.is_empty() {
        return Ok(all);
    }

    let mut next_order = preset.blocks.iter().map(|b| b.order).max().map_or(0, |m| m + 1);
    for block_type in missing {
        preset.blocks.push(PromptBlock::new(block_type, next_order));
        next_order += 1;
    }
    prompt_presets::upsert(conn, &preset)?;
    prompt_presets::get_all(conn)
}

pub fn resolve_for_conversation(conn: &Connection, conversation: &Conversation) -> Result<PromptPreset> {
    if let Some(id) = conversation.prompt_preset_id {
        // A deleted preset falls through to the mode default below.
        if let Some(preset) = prompt_presets::get(conn, id)? {
            return Ok(preset);
        }
    }

    let all = get_all_or_seed(conn)?;
    let default_name = if conversation.mode == ConversationMode::Assistant { ASSISTANT_DEFAULT } else { ROLEPLAY_DEFAULT };
    if let Some(preset) = all.iter().find(|p| p.name == default_name) {
        return Ok(preset.clone());
    }
    Ok(all.into_iter().next().unwrap_or_else(|| {
        if conversation.mode == ConversationMode::Assistant {
            create_assistant_preset()
        } else {
            create_roleplay_preset()
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn an_empty_table_is_seeded_with_both_defaults() {
        let conn = open_in_memory().unwrap();
        let all = get_all_or_seed(&conn).unwrap();
        let names: Vec<_> = all.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, [ASSISTANT_DEFAULT, ROLEPLAY_DEFAULT]);
        assert_eq!(get_all_or_seed(&conn).unwrap().len(), 2, "seeding twice must not duplicate");
    }

    #[test]
    fn resolves_the_mode_default_when_no_preset_is_pinned() {
        let conn = open_in_memory().unwrap();
        let assistant = Conversation { mode: ConversationMode::Assistant, ..Default::default() };
        let roleplay = Conversation { mode: ConversationMode::Roleplay, ..Default::default() };
        assert_eq!(resolve_for_conversation(&conn, &assistant).unwrap().name, ASSISTANT_DEFAULT);
        assert_eq!(resolve_for_conversation(&conn, &roleplay).unwrap().name, ROLEPLAY_DEFAULT);
    }

    #[test]
    fn a_pinned_preset_wins_and_a_deleted_one_falls_back() {
        let conn = open_in_memory().unwrap();
        let custom = PromptPreset { name: "Mine".into(), ..Default::default() };
        prompt_presets::upsert(&conn, &custom).unwrap();

        let mut conv = Conversation { mode: ConversationMode::Roleplay, prompt_preset_id: Some(custom.id), ..Default::default() };
        assert_eq!(resolve_for_conversation(&conn, &conv).unwrap().name, "Mine");

        prompt_presets::delete(&conn, custom.id).unwrap();
        conv.prompt_preset_id = Some(custom.id);
        assert_eq!(resolve_for_conversation(&conn, &conv).unwrap().name, ROLEPLAY_DEFAULT);
    }

    #[test]
    fn old_roleplay_defaults_get_the_world_system_blocks_backfilled_once() {
        let conn = open_in_memory().unwrap();
        let mut old = create_roleplay_preset();
        old.blocks.retain(|b| {
            !matches!(b.block_type, PromptBlockType::WorldState | PromptBlockType::CharacterNotebook | PromptBlockType::QuestLog)
        });
        prompt_presets::upsert(&conn, &old).unwrap();

        let all = get_all_or_seed(&conn).unwrap();
        let roleplay = all.iter().find(|p| p.name == ROLEPLAY_DEFAULT).unwrap();
        for t in [PromptBlockType::WorldState, PromptBlockType::CharacterNotebook, PromptBlockType::QuestLog] {
            assert_eq!(roleplay.blocks.iter().filter(|b| b.block_type == t).count(), 1, "{t:?}");
        }
        let again = get_all_or_seed(&conn).unwrap();
        assert_eq!(again.iter().find(|p| p.name == ROLEPLAY_DEFAULT).unwrap().blocks.len(), roleplay.blocks.len());
    }

    #[test]
    fn old_assistant_defaults_get_a_persona_block_right_after_the_system_prompt() {
        let conn = open_in_memory().unwrap();
        let mut old = create_assistant_preset();
        old.blocks.retain(|b| b.block_type != PromptBlockType::PersonaDescription);
        prompt_presets::upsert(&conn, &old).unwrap();

        let all = get_all_or_seed(&conn).unwrap();
        let assistant = all.iter().find(|p| p.name == ASSISTANT_DEFAULT).unwrap();
        let persona = assistant.blocks.iter().find(|b| b.block_type == PromptBlockType::PersonaDescription).unwrap();
        assert_eq!(persona.order, 1);
    }
}

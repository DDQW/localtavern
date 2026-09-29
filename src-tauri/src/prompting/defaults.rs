use crate::models::{PromptBlock, PromptBlockType, PromptPreset};

pub const DEFAULT_ROLEPLAY_SYSTEM_PROMPT: &str =
    "Write {{char}}'s next reply in this fictional roleplay with {{user}}. Stay in character and be descriptive.";

fn block(block_type: PromptBlockType, order: i32) -> PromptBlock {
    PromptBlock::new(block_type, order)
}

pub fn create_assistant_preset() -> PromptPreset {
    PromptPreset {
        name: "Assistant (default)".into(),
        blocks: vec![
            block(PromptBlockType::SystemPrompt, 0),
            block(PromptBlockType::PersonaDescription, 1),
            block(PromptBlockType::UserMemory, 2),
            block(PromptBlockType::ChatHistory, 3),
        ],
        ..Default::default()
    }
}

pub fn create_roleplay_preset() -> PromptPreset {
    let mut system = block(PromptBlockType::SystemPrompt, 0);
    system.custom_text = Some(DEFAULT_ROLEPLAY_SYSTEM_PROMPT.into());
    PromptPreset {
        name: "Roleplay (default)".into(),
        blocks: vec![
            system,
            block(PromptBlockType::PersonaDescription, 1),
            block(PromptBlockType::CharacterDescription, 2),
            block(PromptBlockType::Scenario, 3),
            block(PromptBlockType::ExampleDialogue, 4),
            block(PromptBlockType::WorldInfo, 5),
            block(PromptBlockType::ChatHistory, 6),
            block(PromptBlockType::PostHistoryInstructions, 7),
            block(PromptBlockType::WorldState, 8),
            block(PromptBlockType::CharacterNotebook, 9),
            block(PromptBlockType::QuestLog, 10),
        ],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_the_expected_block_layout() {
        let assistant = create_assistant_preset();
        assert_eq!(assistant.blocks.len(), 4);
        assert_eq!(assistant.blocks[3].block_type, PromptBlockType::ChatHistory);

        let roleplay = create_roleplay_preset();
        assert_eq!(roleplay.blocks.len(), 11);
        assert_eq!(roleplay.blocks[0].custom_text.as_deref(), Some(DEFAULT_ROLEPLAY_SYSTEM_PROMPT));
        assert!(roleplay.blocks.iter().enumerate().all(|(i, b)| b.order == i as i32));
    }
}

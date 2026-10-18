//! Save rules for the library editors (characters, personas, lorebooks, presets, group chats), and
//! character import/export. The editors send whatever the form holds; these functions normalize it
//! the way the original app's view models did before it reaches the database.

use std::path::Path;

use chrono::Utc;
use rusqlite::Connection;

use crate::chat::images;
use crate::error::{Error, Result};
use crate::import;
use crate::models::{Appearance, Character, ExpressionImage, GroupChat, Lorebook, Persona, PromptPreset};
use crate::repo::{characters, group_chats, lorebooks, personas, prompt_presets};

const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];

/// Trims the appearance text and drops builder picks that hold nothing.
fn clean_appearance(appearance: Option<Appearance>, text: &mut String) -> Option<Appearance> {
    *text = text.trim().to_string();
    appearance.filter(|a| *a != Appearance::default())
}

fn clean_list(items: Vec<String>) -> Vec<String> {
    items.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

pub fn save_character(conn: &Connection, mut character: Character) -> Result<Character> {
    character.name = character.name.trim().to_string();
    if character.name.is_empty() {
        return Err(Error::Other("A character needs a name.".into()));
    }
    character.first_messages.retain(|m| !m.trim().is_empty());
    character.tags = clean_list(character.tags);
    character.expressions = character
        .expressions
        .into_iter()
        .map(|e| ExpressionImage { emotion: e.emotion.trim().to_lowercase(), image_path: e.image_path })
        .filter(|e| !e.emotion.is_empty() && !e.image_path.trim().is_empty())
        .collect();
    character.gallery_image_paths.retain(|p| !p.trim().is_empty());
    character.avatar_path = character.avatar_path.filter(|p| !p.trim().is_empty());
    character.appearance = clean_appearance(character.appearance.take(), &mut character.appearance_text);
    character.schedule = character.schedule.take().filter(|s| !s.is_empty());
    character.updated_at = Utc::now();
    characters::upsert(conn, &character)?;
    Ok(character)
}

/// Only one persona may be the default: saving one as default clears the flag on every other.
pub fn save_persona(conn: &Connection, mut persona: Persona) -> Result<Persona> {
    persona.name = persona.name.trim().to_string();
    if persona.name.is_empty() {
        return Err(Error::Other("A persona needs a name.".into()));
    }
    persona.avatar_path = persona.avatar_path.filter(|p| !p.trim().is_empty());
    persona.appearance = clean_appearance(persona.appearance.take(), &mut persona.appearance_text);

    if persona.is_default {
        for mut other in personas::get_all(conn)? {
            if other.id != persona.id && other.is_default {
                other.is_default = false;
                personas::upsert(conn, &other)?;
            }
        }
    }
    personas::upsert(conn, &persona)?;
    Ok(persona)
}

pub fn save_lorebook(conn: &Connection, mut lorebook: Lorebook) -> Result<Lorebook> {
    lorebook.name = lorebook.name.trim().to_string();
    if lorebook.name.is_empty() {
        return Err(Error::Other("A lorebook needs a name.".into()));
    }
    lorebook.scan_depth = lorebook.scan_depth.max(0);
    lorebook.token_budget = lorebook.token_budget.max(0);
    for entry in &mut lorebook.entries {
        entry.keys = clean_list(std::mem::take(&mut entry.keys));
        entry.secondary_keys =
            entry.secondary_keys.take().map(clean_list).filter(|keys| !keys.is_empty());
        entry.depth = entry.depth.max(0);
    }
    lorebooks::upsert(conn, &lorebook)?;
    Ok(lorebook)
}

pub fn save_preset(conn: &Connection, mut preset: PromptPreset) -> Result<PromptPreset> {
    preset.name = if preset.name.trim().is_empty() { "Untitled preset".to_string() } else { preset.name.trim().to_string() };
    // The editor lets people reorder freely; storing 0..n keeps the order column tidy and unambiguous.
    preset.blocks.sort_by_key(|b| b.order);
    for (index, block) in preset.blocks.iter_mut().enumerate() {
        block.order = index as i32;
        block.custom_text = block.custom_text.take().filter(|t| !t.trim().is_empty());
        block.injection_depth = block.injection_depth.max(0);
    }
    prompt_presets::upsert(conn, &preset)?;
    Ok(preset)
}

pub fn save_group_chat(conn: &Connection, mut group: GroupChat) -> Result<GroupChat> {
    group.name = group.name.trim().to_string();
    if group.name.is_empty() {
        return Err(Error::Other("A group chat needs a name.".into()));
    }
    let mut seen = std::collections::HashSet::new();
    group.character_ids.retain(|id| seen.insert(*id));
    group_chats::upsert(conn, &group)?;
    Ok(group)
}

/// Imports a SillyTavern `.json`/`.png` card, persisting its embedded lorebook (if any) and the
/// character. A PNG card's image is copied into the app's storage so it outlives the original file.
pub fn import_character(conn: &Connection, images_dir: &Path, path: &Path) -> Result<Character> {
    let mut result = import::import_file(path)?;

    let is_png = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if is_png {
        if let Ok(stored) = images::store_file(images_dir, path) {
            result.character.avatar_path = Some(stored.to_string_lossy().into_owned());
        }
    }

    if let Some(lorebook) = &result.lorebook {
        lorebooks::upsert(conn, lorebook)?;
    }
    characters::upsert(conn, &result.character)?;
    Ok(result.character)
}

/// The card JSON for the character as currently edited (not necessarily saved). A card can embed
/// only one lorebook, so when several are linked the first one is exported.
pub fn export_character_card(conn: &Connection, character: &Character) -> Result<String> {
    let mut book = None;
    for id in &character.linked_lorebook_ids {
        if let Some(found) = lorebooks::get(conn, *id)? {
            book = Some(found);
            break;
        }
    }
    import::export_card_json(character, book.as_ref())
}

/// Imports a SillyTavern-style sprite folder: one image per emotion, named `<emotion>.png` (etc.)
/// directly inside the folder. Each file's name (without extension) becomes its emotion key.
pub fn import_expression_folder(images_dir: &Path, folder: &Path) -> Result<Vec<ExpressionImage>> {
    let io = |e: std::io::Error| Error::Other(format!("Couldn't read {}: {e}", folder.display()));

    let mut files: Vec<_> = std::fs::read_dir(folder)
        .map_err(io)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .collect();
    files.sort();

    let mut imported: Vec<ExpressionImage> = Vec::new();
    for file in files {
        let emotion = file.file_stem().and_then(|s| s.to_str()).unwrap_or("").trim().to_lowercase();
        if emotion.is_empty() {
            continue;
        }
        let stored = images::store_file(images_dir, &file)?.to_string_lossy().into_owned();
        match imported.iter_mut().find(|e| e.emotion == emotion) {
            Some(existing) => existing.image_path = stored,
            None => imported.push(ExpressionImage { emotion, image_path: stored }),
        }
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::models::{PromptBlock, PromptBlockType, WorldInfoEntry};
    use uuid::Uuid;

    #[test]
    fn saving_a_character_normalizes_the_form() {
        let conn = open_in_memory().unwrap();
        let saved = save_character(
            &conn,
            Character {
                name: "  Nova  ".into(),
                first_messages: vec!["Hi".into(), "   ".into(), "".into(), "Hello again".into()],
                tags: vec![" scifi ".into(), "".into(), "explorer".into()],
                expressions: vec![
                    ExpressionImage { emotion: " Happy ".into(), image_path: "h.png".into() },
                    ExpressionImage { emotion: "".into(), image_path: "x.png".into() },
                    ExpressionImage { emotion: "sad".into(), image_path: "  ".into() },
                ],
                avatar_path: Some("  ".into()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(saved.name, "Nova");
        assert_eq!(saved.first_messages, ["Hi", "Hello again"]);
        assert_eq!(saved.tags, ["scifi", "explorer"]);
        assert_eq!(saved.expressions.len(), 1);
        assert_eq!(saved.expressions[0].emotion, "happy");
        assert_eq!(saved.avatar_path, None);
        assert_eq!(characters::get(&conn, saved.id).unwrap().unwrap().first_messages, ["Hi", "Hello again"]);
    }

    #[test]
    fn saving_bumps_updated_at_but_keeps_created_at() {
        let conn = open_in_memory().unwrap();
        let mut original = Character { name: "A".into(), ..Default::default() };
        original.updated_at = original.created_at - chrono::Duration::days(1);
        let saved = save_character(&conn, original.clone()).unwrap();
        assert!(saved.updated_at > original.updated_at);
        assert_eq!(saved.created_at, original.created_at);
    }

    #[test]
    fn unnamed_things_are_rejected() {
        let conn = open_in_memory().unwrap();
        assert!(save_character(&conn, Character { name: "  ".into(), ..Default::default() }).is_err());
        assert!(save_persona(&conn, Persona::default()).is_err());
        assert!(save_lorebook(&conn, Lorebook::default()).is_err());
        assert!(save_group_chat(&conn, GroupChat::default()).is_err());
        assert!(characters::get_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn only_one_persona_can_be_the_default() {
        let conn = open_in_memory().unwrap();
        let a = save_persona(&conn, Persona { name: "A".into(), is_default: true, ..Default::default() }).unwrap();
        let b = save_persona(&conn, Persona { name: "B".into(), is_default: true, ..Default::default() }).unwrap();

        let all = personas::get_all(&conn).unwrap();
        assert!(!all.iter().find(|p| p.id == a.id).unwrap().is_default);
        assert!(all.iter().find(|p| p.id == b.id).unwrap().is_default);

        // Saving a non-default persona leaves the default alone.
        save_persona(&conn, Persona { name: "C".into(), ..Default::default() }).unwrap();
        assert_eq!(personas::get_all(&conn).unwrap().iter().filter(|p| p.is_default).count(), 1);
    }

    #[test]
    fn lorebook_entries_get_clean_key_lists() {
        let conn = open_in_memory().unwrap();
        let saved = save_lorebook(
            &conn,
            Lorebook {
                name: " World ".into(),
                scan_depth: -3,
                entries: vec![WorldInfoEntry {
                    keys: vec![" dragon ".into(), "".into(), "wyrm".into()],
                    secondary_keys: Some(vec!["  ".into()]),
                    depth: -1,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(saved.name, "World");
        assert_eq!(saved.scan_depth, 0);
        assert_eq!(saved.entries[0].keys, ["dragon", "wyrm"]);
        assert_eq!(saved.entries[0].secondary_keys, None, "blank secondary keys mean no secondary condition");
        assert_eq!(saved.entries[0].depth, 0);
    }

    #[test]
    fn presets_get_a_default_name_and_contiguous_block_order() {
        let conn = open_in_memory().unwrap();
        let mut history = PromptBlock::new(PromptBlockType::ChatHistory, 40);
        history.custom_text = Some("   ".into());
        history.injection_depth = -2;
        let saved = save_preset(
            &conn,
            PromptPreset {
                name: "   ".into(),
                blocks: vec![history, PromptBlock::new(PromptBlockType::SystemPrompt, 7)],
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(saved.name, "Untitled preset");
        assert_eq!(saved.blocks[0].block_type, PromptBlockType::SystemPrompt);
        assert_eq!(saved.blocks.iter().map(|b| b.order).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(saved.blocks[1].custom_text, None);
        assert_eq!(saved.blocks[1].injection_depth, 0);
        assert_eq!(prompt_presets::get(&conn, saved.id).unwrap().unwrap().blocks.len(), 2);
    }

    #[test]
    fn group_members_are_deduplicated_keeping_order() {
        let conn = open_in_memory().unwrap();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        for id in [a, b] {
            characters::upsert(&conn, &Character { id, name: "x".into(), ..Default::default() }).unwrap();
        }
        let saved = save_group_chat(&conn, GroupChat { name: "G".into(), character_ids: vec![b, a, b], ..Default::default() }).unwrap();
        assert_eq!(saved.character_ids, [b, a]);
    }

    // ---- import / export ----

    #[test]
    fn importing_a_json_card_saves_the_character_and_its_lorebook() {
        let conn = open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let card = dir.path().join("nova.json");
        std::fs::write(
            &card,
            r#"{"spec":"chara_card_v2","data":{"name":"Nova","first_mes":"Hi","character_book":{"entries":[{"keys":["ship"],"content":"A ship."}]}}}"#,
        )
        .unwrap();

        let character = import_character(&conn, &dir.path().join("images"), &card).unwrap();

        assert_eq!(character.name, "Nova");
        let stored = characters::get(&conn, character.id).unwrap().unwrap();
        assert_eq!(stored.linked_lorebook_ids.len(), 1);
        let book = lorebooks::get(&conn, stored.linked_lorebook_ids[0]).unwrap().unwrap();
        assert_eq!(book.entries[0].content, "A ship.");
    }

    #[test]
    fn importing_a_png_card_copies_the_image_into_app_storage() {
        use crate::import::png::testing::{build, text_chunk};
        use base64::Engine;

        let conn = open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let images = dir.path().join("images");
        let card = dir.path().join("rook.png");
        let payload = base64::engine::general_purpose::STANDARD.encode(r#"{"name":"Rook"}"#);
        std::fs::write(&card, build(&[("tEXt", text_chunk("chara", &payload))])).unwrap();

        let character = import_character(&conn, &images, &card).unwrap();

        let avatar = std::path::PathBuf::from(character.avatar_path.unwrap());
        assert!(avatar.starts_with(&images), "the avatar lives in app storage, not at the original path");
        assert!(avatar.exists());
        assert!(card.exists());
    }

    #[test]
    fn a_bad_card_imports_nothing() {
        let conn = open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let card = dir.path().join("bad.json");
        std::fs::write(&card, "not json at all").unwrap();
        assert!(import_character(&conn, dir.path(), &card).is_err());
        assert!(characters::get_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn exporting_includes_the_first_linked_lorebook() {
        let conn = open_in_memory().unwrap();
        let book = Lorebook {
            name: "Lore".into(),
            entries: vec![WorldInfoEntry { keys: vec!["k".into()], content: "c".into(), ..Default::default() }],
            ..Default::default()
        };
        lorebooks::upsert(&conn, &book).unwrap();
        let character = Character { name: "Nova".into(), linked_lorebook_ids: vec![Uuid::new_v4(), book.id], ..Default::default() };

        let json = export_character_card(&conn, &character).unwrap();

        let back = import::map_card_json(&json).unwrap();
        assert_eq!(back.lorebook.unwrap().entries[0].content, "c");
        assert_eq!(back.character.name, "Nova");
    }

    #[test]
    fn a_sprite_folder_becomes_one_expression_per_image() {
        let dir = tempfile::tempdir().unwrap();
        let sprites = dir.path().join("sprites");
        std::fs::create_dir_all(&sprites).unwrap();
        for name in ["Happy.PNG", "sad.jpg", "notes.txt", ".png"] {
            std::fs::write(sprites.join(name), [1, 2, 3]).unwrap();
        }
        std::fs::create_dir_all(sprites.join("nested.png")).unwrap(); // a directory named like an image
        let images = dir.path().join("images");

        let imported = import_expression_folder(&images, &sprites).unwrap();

        let emotions: Vec<_> = imported.iter().map(|e| e.emotion.as_str()).collect();
        assert_eq!(emotions, ["happy", "sad"]);
        for e in &imported {
            assert!(std::path::Path::new(&e.image_path).starts_with(&images));
        }
        assert!(sprites.join("Happy.PNG").exists(), "originals are left alone");
        assert!(import_expression_folder(&images, &dir.path().join("missing")).is_err());
    }
}

//! SillyTavern character cards: v1 (flat) and v2/v3 (`data` wrapper) JSON in, v2 JSON out. The same
//! mapper serves plain `.json` files and cards embedded in a `.png`.

use std::path::Path;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::png;
use crate::error::{Error, Result};
use crate::models::{Appearance, Character, Schedule, Lorebook, WorldInfoEntry, WorldInfoPosition};

/// A character plus the lorebook its card embedded (already linked from the character), if any.
/// The caller just needs to persist both.
#[derive(Debug, Clone)]
pub struct CharacterImportResult {
    pub character: Character,
    pub lorebook: Option<Lorebook>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CardBookEntry {
    keys: Option<Vec<String>>,
    secondary_keys: Option<Vec<String>>,
    content: Option<String>,
    case_sensitive: Option<bool>,
    enabled: Option<bool>,
    insertion_order: Option<i32>,
    position: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CardBook {
    entries: Option<Vec<CardBookEntry>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CardData {
    name: Option<String>,
    description: Option<String>,
    personality: Option<String>,
    scenario: Option<String>,
    first_mes: Option<String>,
    mes_example: Option<String>,
    alternate_greetings: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    character_book: Option<CardBook>,
    /// Free-form per-app data; LocalTavern keeps its builder picks and birthday under `localtavern`.
    extensions: Option<Value>,
}

/// What `data.extensions.localtavern` holds. The card's description already includes the
/// appearance text, so other apps see it; on import it is split off again.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LocalTavernExtension {
    appearance: Option<Appearance>,
    appearance_text: String,
    birth_date: Option<chrono::NaiveDate>,
    schedule: Option<Schedule>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    spec: Option<String>,
    data: Option<Value>,
}

#[derive(Debug, Serialize)]
struct ExportEnvelope {
    spec: &'static str,
    spec_version: &'static str,
    data: CardData,
}

fn bad_card(e: serde_json::Error) -> Error {
    Error::Other(format!("This doesn't look like a character card: {e}"))
}

/// Maps card JSON (v1 flat, or v2 `chara_card_v2` with a `data` wrapper) into domain objects.
pub fn map_card_json(json: &str) -> Result<CharacterImportResult> {
    let json = json.trim_start_matches('\u{feff}');
    let envelope: Envelope = serde_json::from_str(json).map_err(bad_card)?;

    let data: CardData = match (envelope.spec, envelope.data) {
        (Some(_), Some(data)) => serde_json::from_value(data).map_err(bad_card)?,
        _ => serde_json::from_str(json).map_err(bad_card)?,
    };

    let mut first_messages = Vec::new();
    if let Some(first) = data.first_mes.filter(|m| !m.is_empty()) {
        first_messages.push(first);
    }
    first_messages.extend(data.alternate_greetings.unwrap_or_default());

    let ours: LocalTavernExtension = data
        .extensions
        .as_ref()
        .and_then(|e| e.get("localtavern"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let mut description = data.description.unwrap_or_default();
    let appearance_text = ours.appearance_text.trim();
    if !appearance_text.is_empty() {
        if let Some(rest) = description.trim_end().strip_suffix(appearance_text) {
            description = rest.trim_end().to_string();
        }
    }

    let mut character = Character {
        name: data.name.unwrap_or_default(),
        description,
        appearance: ours.appearance,
        appearance_text: appearance_text.to_string(),
        birth_date: ours.birth_date,
        schedule: ours.schedule,
        personality: data.personality.unwrap_or_default(),
        scenario: data.scenario.unwrap_or_default(),
        first_messages,
        example_dialogue: data.mes_example.unwrap_or_default(),
        tags: data.tags.unwrap_or_default(),
        ..Default::default()
    };

    let mut lorebook = None;
    if let Some(entries) = data.character_book.and_then(|b| b.entries).filter(|e| !e.is_empty()) {
        let book = Lorebook {
            name: format!("{} - World Info", character.name),
            entries: entries
                .into_iter()
                .map(|e| WorldInfoEntry {
                    keys: e.keys.unwrap_or_default(),
                    secondary_keys: e.secondary_keys,
                    content: e.content.unwrap_or_default(),
                    case_sensitive: e.case_sensitive.unwrap_or(false),
                    enabled: e.enabled.unwrap_or(true),
                    insertion_order: e.insertion_order.unwrap_or(0),
                    position: if e.position.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("after_char")) {
                        WorldInfoPosition::AfterCharacterDescription
                    } else {
                        WorldInfoPosition::BeforeCharacterDescription
                    },
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        character.linked_lorebook_ids = vec![book.id];
        lorebook = Some(book);
    }

    Ok(CharacterImportResult { character, lorebook })
}

/// Serializes a character (optionally with one linked lorebook) as a `chara_card_v2` JSON document,
/// for sharing outside LocalTavern. Builds fresh JSON from the current field values, so edits made
/// after import are reflected.
pub fn export_card_json(character: &Character, lorebook: Option<&Lorebook>) -> Result<String> {
    let book = lorebook.filter(|l| !l.entries.is_empty()).map(|l| CardBook {
        entries: Some(
            l.entries
                .iter()
                .map(|e| CardBookEntry {
                    keys: Some(e.keys.clone()),
                    secondary_keys: e.secondary_keys.clone(),
                    content: Some(e.content.clone()),
                    case_sensitive: Some(e.case_sensitive),
                    enabled: Some(e.enabled),
                    insertion_order: Some(e.insertion_order),
                    position: Some(
                        if e.position == WorldInfoPosition::AfterCharacterDescription { "after_char" } else { "before_char" }
                            .to_string(),
                    ),
                })
                .collect(),
        ),
    });

    let data = CardData {
        name: Some(character.name.clone()),
        description: Some(character.full_description()),
        personality: Some(character.personality.clone()),
        scenario: Some(character.scenario.clone()),
        first_mes: Some(character.first_messages.first().cloned().unwrap_or_default()),
        mes_example: Some(character.example_dialogue.clone()),
        alternate_greetings: (character.first_messages.len() > 1).then(|| character.first_messages[1..].to_vec()),
        tags: (!character.tags.is_empty()).then(|| character.tags.clone()),
        character_book: book,
        extensions: Some(serde_json::json!({
            "localtavern": LocalTavernExtension {
                appearance: character.appearance.clone(),
                appearance_text: character.appearance_text.clone(),
                birth_date: character.birth_date,
                schedule: character.schedule.clone(),
            }
        })),
    };

    Ok(serde_json::to_string_pretty(&ExportEnvelope { spec: "chara_card_v2", spec_version: "2.0", data })?)
}

/// Extracts and maps the card embedded in a PNG, without touching the filesystem.
pub fn import_png_bytes(bytes: &[u8]) -> Result<CharacterImportResult> {
    let encoded = png::read_text_chunk(bytes, "chara")?
        .or(png::read_text_chunk(bytes, "ccv3")?)
        .ok_or_else(|| {
            Error::Other("No embedded SillyTavern character data ('chara'/'ccv3' metadata chunk) found in this PNG.".into())
        })?;

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| Error::Other(format!("The embedded character data isn't valid base64: {e}")))?;
    let json = String::from_utf8(decoded).map_err(|_| Error::Other("The embedded character data isn't valid UTF-8 text.".into()))?;
    map_card_json(&json)
}

/// Imports a `.json` or `.png` card from disk. A PNG card's own image becomes the avatar.
pub fn import_file(path: &Path) -> Result<CharacterImportResult> {
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let io = |e: std::io::Error| Error::Other(format!("Couldn't read {}: {e}", path.display()));

    match extension.as_deref() {
        Some("json") => map_card_json(&std::fs::read_to_string(path).map_err(io)?),
        Some("png") => {
            let mut result = import_png_bytes(&std::fs::read(path).map_err(io)?)?;
            result.character.avatar_path = Some(path.to_string_lossy().into_owned());
            Ok(result)
        }
        _ => Err(Error::Other("Unsupported file type - use a SillyTavern .json or .png character card.".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::super::png::testing::{build, text_chunk};
    use super::*;

    // ---- mapping ----

    #[test]
    fn maps_a_v2_card_with_the_data_wrapper() {
        let json = r#"{
            "spec": "chara_card_v2", "spec_version": "2.0",
            "data": {
                "name": "Nova",
                "description": "A wandering star-mapper.",
                "personality": "Curious, calm.",
                "scenario": "Aboard a research vessel.",
                "first_mes": "Hello, traveler.",
                "mes_example": "<START>\n{{user}}: Hi\n{{char}}: Hello!",
                "alternate_greetings": ["Oh! You're here.", "Welcome aboard."],
                "tags": ["scifi", "explorer"]
            }
        }"#;

        let result = map_card_json(json).unwrap();

        let c = &result.character;
        assert_eq!(c.name, "Nova");
        assert_eq!(c.description, "A wandering star-mapper.");
        assert_eq!(c.personality, "Curious, calm.");
        assert_eq!(c.scenario, "Aboard a research vessel.");
        assert_eq!(c.first_messages, ["Hello, traveler.", "Oh! You're here.", "Welcome aboard."]);
        assert_eq!(c.tags, ["scifi", "explorer"]);
        assert_eq!(c.example_dialogue, "<START>\n{{user}}: Hi\n{{char}}: Hello!");
        assert!(result.lorebook.is_none());
        assert!(c.linked_lorebook_ids.is_empty());
    }

    #[test]
    fn maps_a_v1_flat_card_without_a_wrapper() {
        let json = r#"{"name":"Rook","description":"A gruff mercenary.","personality":"Blunt.","scenario":"A tavern brawl.","first_mes":"You lost, kid.","mes_example":""}"#;
        let result = map_card_json(json).unwrap();
        assert_eq!(result.character.name, "Rook");
        assert_eq!(result.character.description, "A gruff mercenary.");
        assert_eq!(result.character.first_messages, ["You lost, kid."]);
    }

    #[test]
    fn maps_v3_cards_the_same_way() {
        let json = r#"{"spec":"chara_card_v3","spec_version":"3.0","data":{"name":"Vex","first_mes":"Hi"}}"#;
        let result = map_card_json(json).unwrap();
        assert_eq!((result.character.name.as_str(), result.character.first_messages.len()), ("Vex", 1));
    }

    #[test]
    fn maps_an_embedded_character_book_into_a_linked_lorebook() {
        let json = r#"{
            "spec": "chara_card_v2",
            "data": {
                "name": "Nova",
                "character_book": { "entries": [
                    { "keys": ["starship", "vessel"], "secondary_keys": ["engine"],
                      "content": "The starship's engine hums with quantum energy.",
                      "case_sensitive": false, "enabled": true, "insertion_order": 2, "position": "after_char" },
                    { "keys": ["captain"], "content": "The captain is stern but fair.", "insertion_order": 1 }
                ] }
            }
        }"#;

        let result = map_card_json(json).unwrap();

        let book = result.lorebook.expect("a lorebook");
        assert_eq!(result.character.linked_lorebook_ids, [book.id]);
        assert_eq!(book.name, "Nova - World Info");
        assert_eq!(book.entries.len(), 2);

        let starship = book.entries.iter().find(|e| e.keys.contains(&"starship".to_string())).unwrap();
        assert_eq!(starship.secondary_keys.as_deref(), Some(&["engine".to_string()][..]));
        assert_eq!(starship.position, WorldInfoPosition::AfterCharacterDescription);
        assert_eq!(starship.insertion_order, 2);

        let captain = book.entries.iter().find(|e| e.keys.contains(&"captain".to_string())).unwrap();
        assert_eq!(captain.position, WorldInfoPosition::BeforeCharacterDescription);
        assert!(captain.enabled);
        assert_eq!(captain.secondary_keys, None);
    }

    #[test]
    fn missing_fields_fall_back_to_empty_defaults_instead_of_failing() {
        let result = map_card_json("{}").unwrap();
        assert_eq!(result.character.name, "");
        assert!(result.character.first_messages.is_empty());
        assert!(result.character.tags.is_empty());
        assert!(result.lorebook.is_none());
    }

    #[test]
    fn nulls_unknown_fields_and_a_byte_order_mark_are_tolerated() {
        let json = "\u{feff}{\"name\":\"N\",\"tags\":null,\"alternate_greetings\":null,\"extensions\":{\"x\":1},\"character_book\":{\"entries\":[]}}";
        let result = map_card_json(json).unwrap();
        assert_eq!(result.character.name, "N");
        assert!(result.lorebook.is_none(), "an empty book links nothing");
    }

    #[test]
    fn an_empty_first_message_is_not_a_greeting() {
        let result = map_card_json(r#"{"first_mes":"","alternate_greetings":["Hi"]}"#).unwrap();
        assert_eq!(result.character.first_messages, ["Hi"]);
    }

    #[test]
    fn non_card_json_is_a_friendly_error() {
        let err = map_card_json("[1,2,3]").unwrap_err().to_string();
        assert!(err.contains("doesn't look like a character card"), "{err}");
        assert!(map_card_json("not json").is_err());
    }

    // ---- export ----

    #[test]
    fn exports_core_fields_as_a_chara_card_v2() {
        let character = Character {
            name: "Nova".into(),
            description: "A wandering star-mapper.".into(),
            personality: "Curious, calm.".into(),
            scenario: "Aboard a research vessel.".into(),
            first_messages: vec!["Hello, traveler.".into(), "Oh! You're here.".into()],
            example_dialogue: "Nova: Greetings.".into(),
            tags: vec!["scifi".into(), "explorer".into()],
            ..Default::default()
        };

        let json = export_card_json(&character, None).unwrap();

        assert!(json.contains("\"spec\": \"chara_card_v2\""), "{json}");
        assert!(json.contains("\"name\": \"Nova\""));
        assert!(json.contains("\"first_mes\": \"Hello, traveler.\""));
        assert!(json.contains("Oh! You're here."));
        assert!(json.contains("\"scifi\""));
    }

    #[test]
    fn round_trips_through_the_importer_unchanged() {
        let original = Character {
            name: "Rook".into(),
            description: "A gruff mercenary.".into(),
            personality: "Blunt.".into(),
            scenario: "A tavern brawl.".into(),
            first_messages: vec!["You lost, kid.".into(), "Alt greeting".into()],
            example_dialogue: "Rook: Move along.".into(),
            tags: vec!["mercenary".into()],
            ..Default::default()
        };

        let reimported = map_card_json(&export_card_json(&original, None).unwrap()).unwrap().character;

        assert_eq!(reimported.name, original.name);
        assert_eq!(reimported.description, original.description);
        assert_eq!(reimported.personality, original.personality);
        assert_eq!(reimported.scenario, original.scenario);
        assert_eq!(reimported.first_messages, original.first_messages);
        assert_eq!(reimported.example_dialogue, original.example_dialogue);
        assert_eq!(reimported.tags, original.tags);
    }

    #[test]
    fn appearance_and_birthday_survive_a_round_trip_and_other_apps_see_the_appearance() {
        use crate::models::Sex;
        let original = Character {
            name: "Mira".into(),
            description: "A cartographer.".into(),
            appearance: Some(Appearance { sex: Some(Sex::Female), hair_color: Some("auburn".into()), ..Default::default() }),
            appearance_text: "{{char}} is a woman. She has auburn hair.".into(),
            birth_date: chrono::NaiveDate::from_ymd_opt(2001, 3, 3),
            ..Default::default()
        };

        let json = export_card_json(&original, None).unwrap();
        let card: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(card["data"]["description"], "A cartographer.

{{char}} is a woman. She has auburn hair.");
        assert_eq!(card["data"]["extensions"]["localtavern"]["birthDate"], "2001-03-03");

        let reimported = map_card_json(&json).unwrap().character;
        assert_eq!(reimported.description, original.description);
        assert_eq!(reimported.appearance_text, original.appearance_text);
        assert_eq!(reimported.appearance, original.appearance);
        assert_eq!(reimported.birth_date, original.birth_date);
    }

    #[test]
    fn an_edited_description_keeps_the_appearance_text_inside_it() {
        let card = r#"{"spec":"chara_card_v2","data":{"name":"N","description":"Rewritten by another app.",
            "extensions":{"localtavern":{"appearanceText":"{{char}} is tall."}}}}"#;
        let character = map_card_json(card).unwrap().character;
        assert_eq!(character.description, "Rewritten by another app.");
        assert_eq!(character.appearance_text, "{{char}} is tall.");
    }

    #[test]
    fn exports_a_linked_lorebook_as_the_character_book() {
        let character = Character { name: "Nova".into(), ..Default::default() };
        let lorebook = Lorebook {
            entries: vec![WorldInfoEntry {
                keys: vec!["starship".into()],
                secondary_keys: Some(vec!["engine".into()]),
                content: "The engine hums.".into(),
                position: WorldInfoPosition::AfterCharacterDescription,
                ..Default::default()
            }],
            ..Default::default()
        };

        let reimported = map_card_json(&export_card_json(&character, Some(&lorebook)).unwrap()).unwrap();

        let book = reimported.lorebook.expect("lorebook survives");
        assert_eq!(book.entries.len(), 1);
        let entry = &book.entries[0];
        assert_eq!(entry.keys, ["starship"]);
        assert_eq!(entry.secondary_keys.as_deref(), Some(&["engine".to_string()][..]));
        assert_eq!(entry.content, "The engine hums.");
        assert_eq!(entry.position, WorldInfoPosition::AfterCharacterDescription);
    }

    #[test]
    fn an_empty_lorebook_is_not_exported() {
        let character = Character { name: "Nova".into(), ..Default::default() };
        let json = export_card_json(&character, Some(&Lorebook::default())).unwrap();
        assert!(map_card_json(&json).unwrap().lorebook.is_none());
    }

    // ---- PNG cards ----

    fn png_with(keyword: &str, json: &str) -> Vec<u8> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(json);
        build(&[("tEXt", text_chunk(keyword, &encoded)), ("IEND", vec![])])
    }

    #[test]
    fn imports_a_character_embedded_in_a_chara_chunk() {
        let png = png_with("chara", r#"{"name":"Nova","description":"A wandering star-mapper."}"#);
        let result = import_png_bytes(&png).unwrap();
        assert_eq!(result.character.name, "Nova");
        assert_eq!(result.character.description, "A wandering star-mapper.");
    }

    #[test]
    fn falls_back_to_the_ccv3_chunk_when_chara_is_absent() {
        assert_eq!(import_png_bytes(&png_with("ccv3", r#"{"name":"Rook"}"#)).unwrap().character.name, "Rook");
    }

    #[test]
    fn a_png_without_character_data_is_a_clear_error() {
        let png = build(&[("tEXt", text_chunk("Comment", "just a photo"))]);
        let err = import_png_bytes(&png).unwrap_err().to_string();
        assert!(err.contains("No embedded SillyTavern character data"), "{err}");
    }

    #[test]
    fn corrupt_embedded_data_is_reported_not_a_panic() {
        let bad_base64 = build(&[("tEXt", text_chunk("chara", "!!!not base64!!!"))]);
        assert!(import_png_bytes(&bad_base64).unwrap_err().to_string().contains("base64"));

        let not_utf8 = build(&[("tEXt", text_chunk("chara", &base64::engine::general_purpose::STANDARD.encode([0xff, 0xfe, 0xfd])))]);
        assert!(import_png_bytes(&not_utf8).unwrap_err().to_string().contains("UTF-8"));

        assert!(import_png_bytes(b"definitely not a png").is_err());
    }

    // ---- files ----

    #[test]
    fn imports_json_and_png_files_and_rejects_other_types() {
        let dir = tempfile::tempdir().unwrap();

        let json_path = dir.path().join("card.JSON");
        std::fs::write(&json_path, r#"{"name":"FromJson"}"#).unwrap();
        assert_eq!(import_file(&json_path).unwrap().character.name, "FromJson");

        let png_path = dir.path().join("card.png");
        std::fs::write(&png_path, png_with("chara", r#"{"name":"FromPng"}"#)).unwrap();
        let png = import_file(&png_path).unwrap();
        assert_eq!(png.character.name, "FromPng");
        assert_eq!(png.character.avatar_path.as_deref(), Some(png_path.to_string_lossy().as_ref()));

        let txt = dir.path().join("card.txt");
        std::fs::write(&txt, "x").unwrap();
        assert!(import_file(&txt).unwrap_err().to_string().contains("Unsupported file type"));
        assert!(import_file(&dir.path().join("missing.json")).is_err());
    }
}

//! Character notebooks: the `<CharacterName>: <fact>` line format the extraction LLM answers with,
//! and the formatter that groups entries per character for the prompt.

use uuid::Uuid;

use crate::models::{Character, CharacterNotebookEntry};

/// One new fact a specific character learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterNotebookAddition {
    pub character_id: Uuid,
    pub content: String,
}

pub fn parse(response: &str, characters: &[Character]) -> Vec<CharacterNotebookAddition> {
    let mut additions = Vec::new();
    for raw in response.split('\n').map(str::trim).filter(|l| !l.is_empty()) {
        let line = raw.trim_start_matches(['-', '*', ' ']);
        if line.is_empty() || line.eq_ignore_ascii_case("NONE") {
            continue;
        }
        let Some(sep) = line.find(':').filter(|&i| i > 0 && i != line.len() - 1) else {
            continue;
        };
        let name = line[..sep].trim();
        let content = line[sep + 1..].trim();
        let wanted = name.to_uppercase();
        let Some(character) = characters.iter().find(|c| c.name.to_uppercase() == wanted) else {
            continue;
        };
        if content.is_empty() {
            continue;
        }
        additions.push(CharacterNotebookAddition { character_id: character.id, content: content.to_string() });
    }
    additions
}

/// Groups entries by character and labels each group with that character's name, so the model keeps
/// information asymmetry straight in a group chat - one character's private notes are never implied
/// to be known by another.
pub fn format(entries: &[CharacterNotebookEntry], characters: &[Character]) -> String {
    characters
        .iter()
        .filter_map(|character| {
            let own: Vec<_> = entries.iter().filter(|e| e.character_id == character.id).collect();
            if own.is_empty() {
                return None;
            }
            let lines: Vec<_> = own.iter().map(|e| format!("- {}", e.content)).collect();
            Some(format!("What {} privately knows/recalls:\n{}", character.name, lines.join("\n")))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast() -> Vec<Character> {
        vec![
            Character { name: "Mira".into(), ..Default::default() },
            Character { name: "Dorian".into(), ..Default::default() },
        ]
    }

    #[test]
    fn matches_character_names_case_insensitively() {
        let cast = cast();
        let additions = parse("mira: The player is afraid of heights.", &cast);
        assert_eq!(additions.len(), 1);
        assert_eq!(additions[0].character_id, cast[0].id);
        assert_eq!(additions[0].content, "The player is afraid of heights.");
    }

    #[test]
    fn handles_multiple_lines_for_different_characters() {
        let cast = cast();
        let additions = parse("Mira: The player is afraid of heights.\nDorian: The bridge collapsed.", &cast);
        assert_eq!(additions.len(), 2);
        assert!(additions.iter().any(|a| a.character_id == cast[0].id));
        assert!(additions.iter().any(|a| a.character_id == cast[1].id));
    }

    #[test]
    fn ignores_unknown_names_and_empty_facts() {
        let cast = cast();
        assert!(parse("Stranger: Something happened.", &cast).is_empty());
        assert!(parse("Mira:", &cast).is_empty());
        assert!(parse("Mira:   ", &cast).is_empty());
    }

    #[test]
    fn none_or_blank_means_no_additions() {
        let cast = cast();
        for response in ["NONE", "", "   "] {
            assert!(parse(response, &cast).is_empty(), "{response:?}");
        }
    }

    #[test]
    fn format_groups_by_character_and_skips_those_without_entries() {
        let cast = cast();
        let conv = Uuid::new_v4();
        let entries = vec![
            CharacterNotebookEntry::new(conv, cast[1].id, "the bridge collapsed"),
            CharacterNotebookEntry::new(conv, cast[1].id, "the player lied"),
        ];
        assert_eq!(
            format(&entries, &cast),
            "What Dorian privately knows/recalls:\n- the bridge collapsed\n- the player lied"
        );
        assert_eq!(format(&[], &cast), "");
    }
}

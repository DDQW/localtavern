//! Reading the arousal / climax meters off the story. After each roleplay reply the model is asked,
//! in a separate request, what the passage did to each character's meters - so they follow what was
//! actually written, whatever kind of story it is, instead of depending on the roleplay model to
//! remember a tag. The app applies the answer (see `Meter::apply`).

use crate::models::Meter;
use crate::prompting::meter_tag::{self, MeterReading};

/// What the question starts with (tests look for it).
pub const ASK: &str = "You keep two meters for the characters of a story";

/// The question: the current values, the user's move and the model's reply.
pub fn prompt(cast: &[(&str, Meter)], user_name: &str, user_text: &str, reply_text: &str) -> String {
    let current = cast
        .iter()
        .map(|(name, m)| format!("{name}: arousal {}, climax {}", m.arousal, m.climax))
        .collect::<Vec<_>>()
        .join("\n");
    let names = cast.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ");
    format!(
        "{ASK}. Read the passage below and update them from what it shows (actions, touch, words, body language, \
thoughts).\n\n\
arousal (0-100): how sexually or romantically aroused the character is right now. Flirting, a charged look, a kiss, \
touch or intimacy raise it a little to a lot, depending on the story; explicit sexual activity raises it high. When the \
scene calms down or moves on to something unrelated it falls back toward 0.\n\
climax (0-100): how close the character is to orgasm. It only moves while there is sexual stimulation, and it is 0 otherwise.\n\
orgasm (yes/no): yes only if the passage actually shows the character reaching orgasm. If it does, say yes even if the climax \
number was lower.\n\n\
Judge only from the passage, for these characters: {names}. Most scenes are not sexual at all - then arousal stays near 0 and \
climax is 0. Do not invent arousal the passage does not support, and do not keep it high once the scene has moved on.\n\n\
Current values:\n{current}\n\n\
{user_name}'s message:\n{user_text}\n\n\
The reply:\n{reply_text}\n\n\
Answer with one line per character, exactly like `Name: arousal <0-100>, climax <0-100>, orgasm <yes|no>`, and nothing else."
    )
}

/// `(index into cast, reading)` for each character line in the answer.
pub fn parse(answer: &str, names: &[&str]) -> Vec<(usize, MeterReading)> {
    let mut found: Vec<(usize, MeterReading)> = Vec::new();
    for line in answer.lines() {
        let line = line.trim().trim_start_matches(['-', '*', '#', ' ']).trim_start_matches("**");
        let lower = line.to_lowercase();
        // The longest name that starts the line, so "Anna Marie" isn't taken for "Anna".
        let Some((index, name)) = names
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.trim().is_empty() && lower.starts_with(&n.trim().to_lowercase()))
            .max_by_key(|(_, n)| n.trim().len())
        else {
            continue;
        };
        // Skipped by characters: the match was made in lowercase, which can change a name's byte length.
        let rest: String = line.chars().skip(name.trim().chars().count()).collect();
        let rest = rest.as_str();
        if let Some(reading) = meter_tag::parse_values(rest) {
            found.retain(|(i, _)| *i != index);
            found.push((index, reading));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_carries_the_values_names_and_both_texts() {
        let cast = [("Nova", Meter { arousal: 40, climax: 10, orgasms: 2 }), ("Mira", Meter::default())];
        let text = prompt(&cast, "Alex", "I kiss her.", "She kisses back.");
        assert!(text.starts_with(ASK));
        for needle in ["Nova: arousal 40, climax 10", "Mira: arousal 0, climax 0", "Nova, Mira", "I kiss her.", "She kisses back.", "Alex's message"] {
            assert!(text.contains(needle), "{needle}");
        }
    }

    #[test]
    fn answers_are_matched_to_characters_in_any_shape() {
        let answer = "Nova: arousal 55, climax 20, orgasm no\n- **Mira**: Arousal=10 Climax=0 orgasm: yes\nSomeone else: arousal 99\nnoise";
        let found = parse(answer, &["Nova", "Mira"]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], (0, MeterReading { arousal: Some(55), climax: Some(20), orgasm: false }));
        assert_eq!(found[1], (1, MeterReading { arousal: Some(10), climax: Some(0), orgasm: true }));
    }

    #[test]
    fn the_longest_matching_name_wins_and_garbage_gives_nothing() {
        let found = parse("Anna Marie: arousal 30, climax 0, orgasm no", &["Anna", "Anna Marie"]);
        assert_eq!(found.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [1]);
        assert!(parse("I cannot help with that.", &["Nova"]).is_empty());
    }

    #[test]
    fn a_name_whose_case_changes_its_length_doesnt_break_the_line() {
        // "ẞ" is 3 bytes, its lowercase "ß" 2: slicing the line by the name's byte length split a character.
        let found = parse("ẞ: arousal 30, climax 0, orgasm no", &["ß"]);
        assert_eq!(found, [(0, MeterReading { arousal: Some(30), climax: Some(0), orgasm: false })]);
    }
}

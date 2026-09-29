//! Lorebook keyword scanning. Scans the last N messages (per-lorebook `scan_depth`) for keyword
//! matches against enabled entries, then returns matches (lowest `insertion_order` first) until the
//! token budget runs out. Token counting is a chars/4 heuristic - exactness isn't the goal here.

use crate::models::{ChatMessage, Lorebook, WorldInfoEntry, WorldInfoPosition};

const CHARS_PER_TOKEN: usize = 4;

/// One lorebook entry that matched; `position`/`depth` steer where the assembler places it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldInfoMatch {
    pub content: String,
    pub position: WorldInfoPosition,
    pub depth: i32,
}

/// Seam for the assembler so tests can substitute a fixed match set.
pub trait WorldInfoScanner {
    fn scan(&self, lorebooks: &[Lorebook], recent_history: &[ChatMessage]) -> Vec<WorldInfoMatch>;
}

/// The real keyword scanner.
pub struct KeywordWorldInfoScanner;

impl WorldInfoScanner for KeywordWorldInfoScanner {
    fn scan(&self, lorebooks: &[Lorebook], recent_history: &[ChatMessage]) -> Vec<WorldInfoMatch> {
        let mut matches: Vec<(i32, WorldInfoMatch)> = Vec::new();

        for lorebook in lorebooks {
            let depth = lorebook.scan_depth.max(0) as usize;
            let start = recent_history.len().saturating_sub(depth);
            let scan_text = recent_history[start..]
                .iter()
                .map(ChatMessage::effective_text)
                .collect::<Vec<_>>()
                .join("\n");
            let scan_lower = scan_text.to_lowercase();

            for entry in lorebook.entries.iter().filter(|e| e.enabled && !e.content.trim().is_empty()) {
                if is_match(entry, &scan_text, &scan_lower) {
                    matches.push((
                        entry.insertion_order,
                        WorldInfoMatch { content: entry.content.clone(), position: entry.position, depth: entry.depth },
                    ));
                }
            }
        }

        matches.sort_by_key(|(order, _)| *order); // stable, so ties keep lorebook/entry order

        let char_budget = lorebooks.iter().map(|l| l.token_budget).max().unwrap_or(0).max(0) as usize * CHARS_PER_TOKEN;
        let mut result = Vec::new();
        let mut used = 0usize;
        for (_, m) in matches {
            let len = m.content.chars().count();
            if char_budget > 0 && used + len > char_budget {
                break;
            }
            used += len;
            result.push(m);
        }
        result
    }
}

fn contains_key(entry: &WorldInfoEntry, scan_text: &str, scan_lower: &str, key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    if entry.case_sensitive {
        scan_text.contains(key)
    } else {
        scan_lower.contains(&key.to_lowercase())
    }
}

fn is_match(entry: &WorldInfoEntry, scan_text: &str, scan_lower: &str) -> bool {
    let primary = !entry.keys.is_empty() && entry.keys.iter().any(|k| contains_key(entry, scan_text, scan_lower, k));
    if !primary {
        return false;
    }
    match entry.secondary_keys.as_deref() {
        Some(secondary) if !secondary.is_empty() => {
            secondary.iter().any(|k| contains_key(entry, scan_text, scan_lower, k))
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MessageRole;
    use uuid::Uuid;

    fn msg(text: &str) -> ChatMessage {
        ChatMessage::new(Uuid::new_v4(), MessageRole::User, text)
    }

    fn entry(keys: &[&str], content: &str) -> WorldInfoEntry {
        WorldInfoEntry { keys: keys.iter().map(|k| k.to_string()).collect(), content: content.into(), ..Default::default() }
    }

    fn book(entries: Vec<WorldInfoEntry>) -> Lorebook {
        Lorebook { entries, ..Default::default() }
    }

    fn scan(books: &[Lorebook], history: &[ChatMessage]) -> Vec<WorldInfoMatch> {
        KeywordWorldInfoScanner.scan(books, history)
    }

    #[test]
    fn matches_entry_whose_keyword_appears_in_recent_history() {
        let result = scan(&[book(vec![entry(&["dragon"], "Dragons breathe fire.")])], &[msg("Tell me about the dragon.")]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "Dragons breathe fire.");
        assert_eq!(result[0].position, WorldInfoPosition::BeforeCharacterDescription);
        assert_eq!(result[0].depth, 0);
    }

    #[test]
    fn carries_entry_position_and_depth() {
        let mut e = entry(&["dragon"], "Lore.");
        e.position = WorldInfoPosition::AtDepth;
        e.depth = 3;
        let result = scan(&[book(vec![e])], &[msg("a dragon flew by")]);
        assert_eq!(result[0].position, WorldInfoPosition::AtDepth);
        assert_eq!(result[0].depth, 3);
    }

    #[test]
    fn does_not_match_without_a_keyword() {
        assert!(scan(&[book(vec![entry(&["dragon"], "x")])], &[msg("What's the weather like?")]).is_empty());
    }

    #[test]
    fn respects_scan_depth_cutoff() {
        let lorebook = Lorebook { scan_depth: 1, entries: vec![entry(&["dragon"], "x")], ..Default::default() };
        let history = [msg("A dragon appeared."), msg("Just kidding, never mind.")];
        assert!(scan(&[lorebook], &history).is_empty());
    }

    #[test]
    fn zero_scan_depth_scans_nothing() {
        let lorebook = Lorebook { scan_depth: 0, entries: vec![entry(&["dragon"], "x")], ..Default::default() };
        assert!(scan(&[lorebook], &[msg("dragon")]).is_empty());
    }

    #[test]
    fn case_sensitive_entry_requires_exact_case() {
        let mut e = entry(&["Dragon"], "X");
        e.case_sensitive = true;
        let books = [book(vec![e])];
        assert!(scan(&books, &[msg("a dragon flew by")]).is_empty());
        assert_eq!(scan(&books, &[msg("a Dragon flew by")]).len(), 1);
    }

    #[test]
    fn secondary_keys_require_an_additional_match() {
        let mut e = entry(&["dragon"], "Fire dragon lore.");
        e.secondary_keys = Some(vec!["fire".into()]);
        let books = [book(vec![e])];
        assert!(scan(&books, &[msg("a dragon flew by")]).is_empty());
        assert_eq!(scan(&books, &[msg("a fire-breathing dragon flew by")])[0].content, "Fire dragon lore.");
    }

    #[test]
    fn empty_secondary_key_list_is_ignored() {
        let mut e = entry(&["dragon"], "x");
        e.secondary_keys = Some(vec![]);
        assert_eq!(scan(&[book(vec![e])], &[msg("dragon")]).len(), 1);
    }

    #[test]
    fn disabled_entries_are_ignored() {
        let mut e = entry(&["dragon"], "X");
        e.enabled = false;
        assert!(scan(&[book(vec![e])], &[msg("a dragon flew by")]).is_empty());
    }

    #[test]
    fn matches_are_ordered_by_insertion_order() {
        let mut second = entry(&["dragon"], "Second");
        second.insertion_order = 2;
        let mut first = entry(&["dragon"], "First");
        first.insertion_order = 1;
        let result = scan(&[book(vec![second, first])], &[msg("a dragon flew by")]);
        assert_eq!(result.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
    }

    #[test]
    fn stops_adding_entries_once_the_token_budget_is_exceeded() {
        let mut fits = entry(&["dragon"], "1234");
        fits.insertion_order = 0;
        let mut too_big = entry(&["dragon"], "should not fit");
        too_big.insertion_order = 1;
        let lorebook = Lorebook { token_budget: 1, entries: vec![fits, too_big], ..Default::default() }; // ~4 chars
        let result = scan(&[lorebook], &[msg("a dragon flew by")]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "1234");
    }

    #[test]
    fn scans_the_active_alternate_of_a_swiped_message() {
        use crate::models::MessageAlternate;
        let mut m = msg("old text");
        m.alternates = vec![
            MessageAlternate { text: "old text".into(), reasoning_text: None, raw_prompt: None },
            MessageAlternate { text: "a dragon appears".into(), reasoning_text: None, raw_prompt: None },
        ];
        m.active_alternate_index = 1;
        assert_eq!(scan(&[book(vec![entry(&["dragon"], "x")])], &[m]).len(), 1);
    }
}

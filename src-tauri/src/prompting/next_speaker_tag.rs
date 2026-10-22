//! The trailing `[next: ...]` tag a group reply may end with when another member should answer the
//! same message of the user's too. The model decides: the app only offers the members who haven't
//! answered that message yet, and has the one named reply right after.

use std::sync::LazyLock;

use regex::Regex;

use super::instructions::{self, Instructions};
use super::roles::join_names;

/// One line for the system instruction. `others` are the members who may still answer the user's
/// latest message. Leaving the tag out is the default, so a plain exchange stays one reply.
pub fn build_instruction(ins: &Instructions, speaker: &str, others: &[&str], user: &str) -> String {
    let others = join_names(others, "or");
    ins.render(instructions::NEXT_SPEAKER, &[("char", speaker), ("user", user), ("others", &others)])
}

/// `[next: ...]` / `(next speaker = ...)` and the like, anywhere in `tail`.
static ANY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[\[(]\s*next(?:[-_ ]?(?:speaker|up))?\s*[:=]([^\[\])]*)[\])]").unwrap());

/// The name in a next-speaker tag anywhere in `tail` (the trailing tags peeled off a reply).
pub fn parse_tail(tail: &str) -> Option<String> {
    let name = ANY.captures(tail)?.get(1)?.as_str();
    Some(name.trim().trim_matches(['"', '*', '_', '<', '>']).trim().to_string()).filter(|n| !n.is_empty())
}

/// Which of `candidates` (id, name) `name` means: the full name, else the one whose first name it is.
/// Ignoring case, and nobody when it fits none ("none", a non-member) or more than one.
pub fn resolve<T: Copy>(name: &str, candidates: &[(T, &str)]) -> Option<T> {
    let wanted = name.trim().to_lowercase();
    if let Some((id, _)) = candidates.iter().find(|(_, n)| n.trim().to_lowercase() == wanted) {
        return Some(*id);
    }
    let first = |n: &str| n.split_whitespace().next().map(str::to_lowercase);
    let mut by_first = candidates.iter().filter(|(_, n)| first(n).is_some() && first(n) == first(&wanted));
    match (by_first.next(), by_first.next()) {
        (Some((id, _)), None) => Some(*id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_names_the_speaker_the_candidates_and_the_format() {
        let line = build_instruction(&Instructions::default(), "Alice", &["Bob", "Cara"], "Alex");
        assert!(line.starts_with("Several members may answer the same message of Alex's. Leave it at Alice's reply"), "{line}");
        assert!(line.contains("Only when Bob or Cara would naturally speak up"), "{line}");
        assert!(line.contains("square brackets included: [next: <name>], naming exactly one of Bob or Cara."), "{line}");
        assert!(line.ends_with("they answer in their own turn."), "{line}");
    }

    #[test]
    fn reads_the_name_from_the_usual_spellings() {
        assert_eq!(parse_tail("[next: Bob]").as_deref(), Some("Bob"));
        assert_eq!(parse_tail("**[Next Speaker: \"Cara\"]**").as_deref(), Some("Cara"));
        assert_eq!(parse_tail("(next = <Captain Jack>)").as_deref(), Some("Captain Jack"));
        assert_eq!(parse_tail("[emotion: happy] [next-up: Bob)").as_deref(), Some("Bob"));
        assert_eq!(parse_tail("[next: ]"), None);
        assert_eq!(parse_tail("[emotion: happy]"), None);
    }

    #[test]
    fn resolves_full_names_then_unique_first_names() {
        let cast = [(1, "Bob"), (2, "Cara Lynn"), (3, "Captain Jack"), (4, "Captain Hook")];
        assert_eq!(resolve("bob", &cast), Some(1));
        assert_eq!(resolve("Cara", &cast), Some(2));
        assert_eq!(resolve("CAPTAIN JACK", &cast), Some(3));
        assert_eq!(resolve("Captain", &cast), None, "two members are a Captain");
        assert_eq!(resolve("none", &cast), None);
        assert_eq!(resolve("Dave", &cast), None);
    }
}

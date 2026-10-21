//! Private talks inside a group chat ("breakouts"): the user and one member, away from the others.
//! A private talk sees the group conversation it belongs to; the group never sees the talk, and
//! neither does any other private talk. Only the member who was in it remembers it in the group.

use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::Regex;

use super::roles::{join_names, system_note};
use crate::models::{ChatMessage, MessageRole};

/// Roughly how much of a private talk a member carries back into the group (the newest part).
const MEMORY_CHARS: usize = 8000;

/// One line for a group reply's system instruction: the speaker may take the user aside. Framed as
/// ordinary behaviour, not a rare exception: people step aside for a quiet word all the time.
pub fn private_tag_instruction(character: &str, user: &str) -> String {
    format!(
        "{character} can take {user} aside whenever that would come naturally: to say something not meant for the others' ears (a \
remark about someone, a secret, a worry, a confession, a bit of flirting, a plan) or just for a moment alone with {user}. Then show \
{character} drawing {user} aside, keep the private part itself for later, and end the reply with one last line in exactly this form, \
square brackets included: [private]. The app then takes {character} and {user} to a private conversation, where {character} goes on. \
Leave the line out when everything {character} wants to say can be said in front of the others."
    )
}

/// `[private]`, `[private: ...]` or `(private)`.
static PRIVATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[\[(]\s*private\b[^\[\])]*[\])]").unwrap());

/// Whether the trailing tags peeled off a reply ask for a private talk.
pub fn parse_tail(tail: &str) -> bool {
    PRIVATE.is_match(tail)
}

/// What the group chat shows (and its model reads, as a system note) where a private talk happened.
pub fn marker_text(user: &str, character: &str) -> String {
    format!("{user} and {character} stepped away from the group to talk in private.")
}

/// The system-instruction rules for a private talk: who is there, and what the group notes are.
pub fn rules(character: &str, user: &str, group: &str, others: &[&str]) -> String {
    let others = if others.is_empty() { String::new() } else { format!(" with {}", join_names(others, "and")) };
    format!(
        "This is a private conversation between {user} and {character}, away from the group chat \"{group}\"{others}. Nobody else can \
hear it. Notes in the form [System note: In the group chat, ...] are what was said in the group chat itself, where {character} was \
present; they are not part of this private conversation, so don't answer them here."
    )
}

/// The group's own summary, for a private talk whose group chat has been compacted.
pub fn group_summary_instruction(summary: &str) -> String {
    format!("What happened earlier in the group chat (a summary; older group messages are no longer shown):\n{}", summary.trim())
}

/// A group message, as a private talk's history shows it.
fn group_note(message: &ChatMessage, name: &str) -> Option<String> {
    let text = message.effective_text().trim();
    match message.role {
        MessageRole::User | MessageRole::Character if !text.is_empty() => {
            Some(system_note(&format!("In the group chat, {name}: {text}")))
        }
        MessageRole::System if !text.is_empty() => Some(system_note(&format!("In the group chat: {text}"))),
        _ => None,
    }
}

/// A private talk's history: its own messages with the group's in between, in the order they were
/// written. `name` says who wrote a group message. Group messages from `cutoff` on are left out
/// (regenerating an older reply sees the group as it was then).
pub fn merge_history(
    own: &[ChatMessage],
    group: &[ChatMessage],
    name: impl Fn(&ChatMessage) -> String,
    cutoff: Option<DateTime<Utc>>,
) -> Vec<ChatMessage> {
    let notes = group.iter().filter(|m| cutoff.is_none_or(|t| m.timestamp < t)).filter_map(|m| {
        let mut note = ChatMessage::new(m.conversation_id, MessageRole::User, group_note(m, &name(m))?);
        note.timestamp = m.timestamp;
        Some(note)
    });

    let mut merged: Vec<ChatMessage> = Vec::with_capacity(own.len() + group.len());
    let mut notes = notes.peekable();
    for message in own {
        while let Some(note) = notes.next_if(|n| n.timestamp <= message.timestamp) {
            merged.push(note);
        }
        merged.push(message.clone());
    }
    merged.extend(notes);
    merged
}

/// What a member remembers of their private talk with the user, for that member's group turns
/// only: the talk's summary (if it was compacted) and its newest messages.
pub fn private_memory(character: &str, user: &str, summary: Option<&str>, talk: &[ChatMessage]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0;
    for message in talk.iter().rev() {
        let who = match message.role {
            MessageRole::User => user,
            MessageRole::Character => character,
            _ => continue,
        };
        let text = message.effective_text().trim();
        if text.is_empty() {
            continue;
        }
        if used + text.len() > MEMORY_CHARS && !lines.is_empty() {
            break;
        }
        used += text.len();
        lines.push(format!("{who}: {text}"));
    }
    lines.reverse();
    let summary = summary.map(str::trim).filter(|s| !s.is_empty());
    if lines.is_empty() && summary.is_none() {
        return None;
    }

    let mut text = format!(
        "{character} and {user} have talked in private, away from the group. Only the two of them know what was said: {character} \
remembers it, but the others don't know it, and {character} doesn't bring it up in front of them unless {character} would choose \
to share it."
    );
    if let Some(summary) = summary {
        text.push_str(&format!("\nEarlier in the private talk (summary): {summary}"));
    }
    if !lines.is_empty() {
        text.push_str(&format!("\nThe private talk:\n{}", lines.join("\n")));
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use uuid::Uuid;

    use super::*;

    fn at(role: MessageRole, text: &str, minute: i64) -> ChatMessage {
        let mut m = ChatMessage::new(Uuid::new_v4(), role, text);
        m.timestamp = DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(minute);
        m
    }

    #[test]
    fn the_private_tag_is_found_in_its_usual_spellings() {
        assert!(parse_tail("[private]"));
        assert!(parse_tail("**[Private: she wants a word]**"));
        assert!(parse_tail("[emotion: shy] (private)"));
        assert!(!parse_tail("[privately: no]"));
        assert!(!parse_tail("[emotion: happy]"));
    }

    #[test]
    fn group_messages_are_merged_in_time_order_as_notes() {
        let group = [at(MessageRole::User, "Hi all", 0), at(MessageRole::Character, "Hey!", 1), at(MessageRole::System, "Alex and Mira stepped away.", 2), at(MessageRole::Character, "Back so soon?", 10)];
        let own = [at(MessageRole::User, "So, the letter?", 3), at(MessageRole::Character, "Not here.", 4)];
        let name = |m: &ChatMessage| if m.role == MessageRole::User { "Alex".to_string() } else { "Cara".to_string() };

        let merged = merge_history(&own, &group, name, None);
        let texts: Vec<&str> = merged.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "[System note: In the group chat, Alex: Hi all]",
                "[System note: In the group chat, Cara: Hey!]",
                "[System note: In the group chat: Alex and Mira stepped away.]",
                "So, the letter?",
                "Not here.",
                "[System note: In the group chat, Cara: Back so soon?]",
            ]
        );
        assert!(merged.iter().take(3).all(|m| m.role == MessageRole::User));
        assert_eq!(merged[4].role, MessageRole::Character);

        let before = merge_history(&own[..1], &group, name, Some(own[1].timestamp));
        assert_eq!(before.len(), 4, "a cutoff leaves out the group's later messages");
    }

    #[test]
    fn memory_keeps_the_newest_part_of_the_talk_and_the_summary() {
        let talk = [at(MessageRole::User, "The letter?", 0), at(MessageRole::Character, "I burned it.", 1)];
        let memory = private_memory("Mira", "Alex", Some("They argued."), &talk).unwrap();
        assert!(memory.starts_with("Mira and Alex have talked in private"), "{memory}");
        assert!(memory.contains("the others don't know it"), "{memory}");
        assert!(memory.contains("Earlier in the private talk (summary): They argued."), "{memory}");
        assert!(memory.ends_with("The private talk:\nAlex: The letter?\nMira: I burned it."), "{memory}");
        assert_eq!(private_memory("Mira", "Alex", None, &[]), None);

        let long = vec![at(MessageRole::User, &"x".repeat(5000), 0), at(MessageRole::Character, &"y".repeat(5000), 1)];
        let memory = private_memory("Mira", "Alex", None, &long).unwrap();
        assert!(memory.contains("Mira: yyy") && !memory.contains("Alex: xxx"), "only the newest part fits");
    }

    #[test]
    fn rules_name_the_group_and_the_others() {
        let rules = rules("Mira", "Alex", "The Crew", &["Bob", "Cara"]);
        assert!(rules.starts_with("This is a private conversation between Alex and Mira, away from the group chat \"The Crew\" with Bob and Cara."), "{rules}");
        let aside = private_tag_instruction("Mira", "Alex");
        assert!(aside.starts_with("Mira can take Alex aside whenever that would come naturally: to say something not meant for the others' ears"), "{aside}");
        assert!(aside.contains("keep the private part itself for later"), "{aside}");
        assert!(aside.contains("square brackets included: [private]."), "{aside}");
        assert!(!aside.contains("Most replies") && !aside.contains("Only when"), "not framed as a rare exception: {aside}");
        assert_eq!(marker_text("Alex", "Mira"), "Alex and Mira stepped away from the group to talk in private.");
    }
}

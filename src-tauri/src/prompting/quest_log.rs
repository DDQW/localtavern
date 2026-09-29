//! Quest log: the "ADD:/UPDATE:/COMPLETE:/FAIL:" line grammar the extraction LLM answers with, and
//! the formatter that injects the log back into prompts. Kept pure (no LLM calls) so it's unit-testable.

use uuid::Uuid;

use crate::models::{QuestLogEntry, QuestStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestLogActionType {
    Add,
    Update,
    Complete,
    Fail,
}

/// One line of the extraction response, resolved to a concrete action. `existing_quest_id` is set
/// for Update/Complete/Fail once matched against a known title; Add never has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestLogAction {
    pub action_type: QuestLogActionType,
    pub title: String,
    pub description: Option<String>,
    pub existing_quest_id: Option<Uuid>,
}

pub fn parse(response: &str, existing: &[QuestLogEntry]) -> Vec<QuestLogAction> {
    let mut actions = Vec::new();
    for raw in response.split('\n').map(str::trim).filter(|l| !l.is_empty()) {
        let line = raw.trim_start_matches(['-', '*', ' ']);
        if line.is_empty() || line.eq_ignore_ascii_case("NONE") {
            continue;
        }
        let Some(colon) = line.find(':').filter(|&i| i > 0) else {
            continue;
        };
        let verb = line[..colon].trim();
        let rest = line[colon + 1..].trim();
        if rest.is_empty() {
            continue;
        }

        if verb.eq_ignore_ascii_case("ADD") {
            let (title, description) = split_title_and_description(rest);
            if !title.is_empty() && !description.is_empty() {
                actions.push(QuestLogAction {
                    action_type: QuestLogActionType::Add,
                    title: title.to_string(),
                    description: Some(description.to_string()),
                    existing_quest_id: None,
                });
            }
        } else if verb.eq_ignore_ascii_case("UPDATE") {
            let (title, description) = split_title_and_description(rest);
            if let Some(quest) = find_by_title(existing, title).filter(|_| !description.is_empty()) {
                actions.push(QuestLogAction {
                    action_type: QuestLogActionType::Update,
                    title: quest.title.clone(),
                    description: Some(description.to_string()),
                    existing_quest_id: Some(quest.id),
                });
            }
        } else if verb.eq_ignore_ascii_case("COMPLETE") || verb.eq_ignore_ascii_case("FAIL") {
            if let Some(quest) = find_by_title(existing, rest) {
                actions.push(QuestLogAction {
                    action_type: if verb.eq_ignore_ascii_case("COMPLETE") {
                        QuestLogActionType::Complete
                    } else {
                        QuestLogActionType::Fail
                    },
                    title: quest.title.clone(),
                    description: None,
                    existing_quest_id: Some(quest.id),
                });
            }
        }
    }
    actions
}

/// `<title> :: <description>` split on the first `::`; empty description if the separator is missing.
fn split_title_and_description(text: &str) -> (&str, &str) {
    match text.find("::") {
        Some(i) => (text[..i].trim(), text[i + 2..].trim()),
        None => (text.trim(), ""),
    }
}

fn find_by_title<'a>(quests: &'a [QuestLogEntry], title: &str) -> Option<&'a QuestLogEntry> {
    let wanted = title.trim().to_uppercase();
    quests.iter().find(|q| q.title.trim().to_uppercase() == wanted)
}

/// Active quests are always listed; completed ones are kept to the 3 most recently updated, for
/// continuity without bloating the prompt.
pub fn format(entries: &[QuestLogEntry]) -> String {
    let active: Vec<_> = entries.iter().filter(|e| e.status == QuestStatus::Active).collect();
    let mut completed: Vec<_> = entries.iter().filter(|e| e.status == QuestStatus::Completed).collect();
    completed.sort_by_key(|q| std::cmp::Reverse(q.updated_at));
    completed.truncate(3);

    let mut parts = Vec::new();
    if !active.is_empty() {
        let lines: Vec<_> = active.iter().map(|q| format!("- {}: {}", q.title, q.description)).collect();
        parts.push(format!("Active quests:\n{}", lines.join("\n")));
    }
    if !completed.is_empty() {
        let lines: Vec<_> = completed.iter().map(|q| format!("- {}", q.title)).collect();
        parts.push(format!("Recently completed:\n{}", lines.join("\n")));
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn quest(title: &str) -> QuestLogEntry {
        QuestLogEntry::new(Uuid::new_v4(), title, "original description")
    }

    #[test]
    fn add_always_yields_a_new_entry_with_no_existing_id() {
        let actions = parse("ADD: Find the lantern :: Explore the cave to find the lost lantern.", &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, QuestLogActionType::Add);
        assert_eq!(actions[0].title, "Find the lantern");
        assert_eq!(actions[0].description.as_deref(), Some("Explore the cave to find the lost lantern."));
        assert_eq!(actions[0].existing_quest_id, None);
    }

    #[test]
    fn update_resolves_to_existing_id_when_title_matches_case_insensitively() {
        let q = quest("Find the lantern");
        let actions = parse("UPDATE: find the lantern :: Found a clue pointing north.", std::slice::from_ref(&q));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, QuestLogActionType::Update);
        assert_eq!(actions[0].existing_quest_id, Some(q.id));
        assert_eq!(actions[0].title, "Find the lantern"); // the stored title, not the model's casing
        assert_eq!(actions[0].description.as_deref(), Some("Found a clue pointing north."));
    }

    #[test]
    fn complete_and_fail_resolve_to_existing_id() {
        let q = quest("Find the lantern");
        let complete = parse("COMPLETE: Find the lantern", std::slice::from_ref(&q));
        assert_eq!(complete[0].action_type, QuestLogActionType::Complete);
        assert_eq!(complete[0].existing_quest_id, Some(q.id));
        let fail = parse("FAIL: Find the lantern", std::slice::from_ref(&q));
        assert_eq!(fail[0].action_type, QuestLogActionType::Fail);
    }

    #[test]
    fn drops_update_complete_fail_when_title_does_not_match() {
        let q = [quest("Find the lantern")];
        for line in ["UPDATE: Unknown quest :: new description", "COMPLETE: Unknown quest", "FAIL: Unknown quest"] {
            assert!(parse(line, &q).is_empty(), "{line}");
        }
    }

    #[test]
    fn none_or_blank_means_no_actions() {
        for response in ["NONE", "", "   "] {
            assert!(parse(response, &[]).is_empty(), "{response:?}");
        }
    }

    #[test]
    fn tolerates_bullets_and_handles_several_lines() {
        let q = quest("Find the lantern");
        let actions = parse(
            "- ADD: Cross the bridge :: Get to the far side.\n* COMPLETE: find the lantern\nnot a line",
            std::slice::from_ref(&q),
        );
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Cross the bridge");
        assert_eq!(actions[1].action_type, QuestLogActionType::Complete);
    }

    #[test]
    fn add_without_description_is_dropped() {
        assert!(parse("ADD: Just a title", &[]).is_empty());
    }

    #[test]
    fn format_lists_active_and_the_three_latest_completed() {
        let now = Utc::now();
        let mut entries = vec![quest("Active one")];
        for (i, title) in ["c1", "c2", "c3", "c4"].iter().enumerate() {
            let mut q = quest(title);
            q.status = QuestStatus::Completed;
            q.updated_at = now + Duration::minutes(i as i64);
            entries.push(q);
        }
        let mut failed = quest("failed one");
        failed.status = QuestStatus::Failed;
        entries.push(failed);

        let text = format(&entries);
        assert!(text.starts_with("Active quests:\n- Active one: original description"));
        assert!(text.contains("Recently completed:\n- c4\n- c3\n- c2"));
        assert!(!text.contains("c1"));
        assert!(!text.contains("failed one"));
    }

    #[test]
    fn format_is_empty_with_nothing_to_show() {
        assert_eq!(format(&[]), "");
    }
}

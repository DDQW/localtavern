//! Who is who in a roleplay prompt. Providers only see two roles - user and model - so everything
//! the user's character does arrives as a user-role message, and a line like "*I hand you the
//! sword*" is easy for a model to misread as the character's own doing, or to answer by narrating
//! the user's next move as well. These texts spell out, by name, whose message is whose.

use super::instructions::{self, Instructions};
use crate::models::{Character, Persona};

/// The name `{{user}}` stands for: the persona's, or "User" without one.
pub fn user_name(persona: Option<&Persona>) -> &str {
    persona.map(|p| p.name.as_str()).filter(|n| !n.trim().is_empty()).unwrap_or("User")
}

/// "Alice", "Alice and Bob", "Alice, Bob and Cara" (or "or" in place of "and").
pub fn join_names(names: &[&str], conjunction: &str) -> String {
    match names {
        [] => String::new(),
        [only] => only.to_string(),
        [rest @ .., last] => format!("{} {conjunction} {last}", rest.join(", ")),
    }
}

/// Marks text the app slips into the history as a user-role turn (author's note, world state,
/// lore at depth, the Continue nudge...). Unmarked it would read as something the user's character
/// said, since every other user-role message is theirs.
pub fn system_note(text: &str) -> String {
    instructions::defaults().note(text)
}

/// Tells the model the user's latest message carries picture(s) it can see, and to answer them.
pub fn picture_rule(ins: &Instructions, count: usize, roleplay: bool) -> String {
    let what = if count == 1 { "a picture".to_string() } else { format!("{count} pictures") };
    let key = if roleplay { instructions::PICTURE_SHOWN } else { instructions::PICTURE_SHOWN_ASSISTANT };
    ins.render(key, &[("what", &what)])
}

/// Stands in for pictures on an earlier user message once each has a description: the model saw them
/// on that turn, and resending the images made it decode them again on every later reply.
pub fn picture_note(ins: &Instructions, descriptions: &[&str]) -> String {
    let text = match descriptions {
        [one] => ins.render(instructions::PICTURE_NOTE_ONE, &[("description", one.trim())]),
        many => {
            let each: Vec<String> = many.iter().enumerate().map(|(i, d)| format!("Picture {}: {}", i + 1, d.trim())).collect();
            ins.render(instructions::PICTURE_NOTE_MANY, &[("count", &many.len().to_string()), ("descriptions", &each.join(" "))])
        }
    };
    ins.note(&text)
}

/// The rules sent with every roleplay reply, whatever the preset says (a preset's own system prompt
/// is the user's to edit, and may be anything). `speaker` is whose reply this is; a one-character
/// chat falls back to that character. `None` when there is no character to play.
///
/// Says nothing about side characters either way - whether the model may voice extras is the
/// preset's call. The one hard line is the user's character, plus, in a group, the other members
/// (they get turns of their own).
pub fn reply_rules(ins: &Instructions, user: &str, characters: &[Character], speaker: Option<&Character>) -> Option<String> {
    let speaker = speaker.or(match characters {
        [only] => Some(only),
        _ => None,
    });

    // Who plays whom, the rule about the other group members (if any), and whom the user's
    // character's actions must never be credited to.
    let (roles, members, actor) = if characters.len() > 1 {
        let names: Vec<&str> = characters.iter().map(|c| c.name.as_str()).collect();
        let cast = join_names(&names, "and");
        // Without a speaker (only for estimating the request's size) the whole cast stands in.
        let me = speaker.map_or(cast.as_str(), |c| c.name.as_str());
        let roles = ins.render(instructions::RULES_GROUP, &[("user", user), ("cast", &cast), ("char", me)]);
        let others: Vec<&str> = names.iter().copied().filter(|n| speaker.is_some_and(|me| me.name != *n)).collect();
        let members = match (others.is_empty(), speaker) {
            (false, Some(me)) => {
                Some(ins.render(instructions::RULES_GROUP_MEMBERS, &[("others", &join_names(&others, "or")), ("char", &me.name)]))
            }
            _ => None,
        };
        (roles, members, "a character")
    } else {
        let me = speaker?.name.as_str();
        (ins.render(instructions::RULES_SOLO, &[("user", user), ("char", me)]), None, me)
    };

    let common = ins.render(instructions::RULES_COMMON, &[("user", user)]);
    let user_only = ins.render(instructions::RULES_USER_ONLY, &[("user", user), ("actor", actor)]);
    let parts = [Some(roles), Some(common), members, Some(user_only)];
    Some(parts.into_iter().flatten().filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join(" "))
}

/// The last turn of a group reply's request: whose reply this is, said where the model reads it
/// last. Without it the request ends on the previous member's reply, and a model plans (and
/// reasons through) the whole scene - every member's answer - to write one of them.
pub fn group_turn_note(ins: &Instructions, speaker: &str, others: &[&str]) -> String {
    let others = join_names(others, "and");
    system_note(&ins.render(instructions::GROUP_TURN, &[("char", speaker), ("others", &others)]))
}

/// The trailing note asking for the user's own next message (Impersonate). It goes into the last
/// user turn, not the system prompt - a system-prompt nudge alone lost to strong character cards -
/// and says outright that the usual sides are swapped for this one message.
pub fn impersonation_request(ins: &Instructions, user: &str, characters: &[Character]) -> String {
    let names: Vec<&str> = characters.iter().map(|c| c.name.as_str()).collect();
    let others = if names.is_empty() { "the assistant".to_string() } else { join_names(&names, "or") };
    ins.note(&ins.render(instructions::IMPERSONATE, &[("user", user), ("others", &others)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast(names: &[&str]) -> Vec<Character> {
        names.iter().map(|n| Character { name: n.to_string(), ..Default::default() }).collect()
    }

    #[test]
    fn user_name_falls_back_to_user() {
        assert_eq!(user_name(None), "User");
        assert_eq!(user_name(Some(&Persona { name: "  ".into(), ..Default::default() })), "User");
        assert_eq!(user_name(Some(&Persona { name: "Alex".into(), ..Default::default() })), "Alex");
    }

    #[test]
    fn names_are_joined_like_a_sentence() {
        assert_eq!(join_names(&[], "and"), "");
        assert_eq!(join_names(&["Alice"], "and"), "Alice");
        assert_eq!(join_names(&["Alice", "Bob"], "or"), "Alice or Bob");
        assert_eq!(join_names(&["Alice", "Bob", "Cara"], "and"), "Alice, Bob and Cara");
    }

    #[test]
    fn picture_notes_carry_each_description() {
        assert_eq!(picture_note(instructions::defaults(), &["A cat. "]), "[System note: The user sent a picture with this message. It showed: A cat.]");
        assert_eq!(
            picture_note(instructions::defaults(), &["A cat.", "A dog."]),
            "[System note: The user sent 2 pictures with this message. Picture 1: A cat. Picture 2: A dog.]"
        );
    }

    #[test]
    fn system_notes_are_bracketed_and_labelled() {
        assert_eq!(system_note("  Remember the twist.\n"), "[System note: Remember the twist.]");
    }

    #[test]
    fn one_on_one_rules_name_both_sides() {
        let nova = cast(&["Nova"]);
        let rules = reply_rules(instructions::defaults(), "Alex", &nova, Some(&nova[0])).unwrap();

        assert!(rules.starts_with("Roles: the user plays Alex; you play Nova."), "{rules}");
        assert!(rules.contains("Every user-role message is written by Alex"), "{rules}");
        assert!(rules.contains("*actions* included, is what Alex says, does, thinks or feels - never Nova."), "{rules}");
        assert!(rules.contains("\"I\", \"me\" and \"my\" mean Alex, and \"you\" usually means Nova."), "{rules}");
        assert!(rules.contains("[System note: ...] comes from the app, not from Alex"), "{rules}");
        assert!(rules.contains("never write what Alex says, does, thinks or feels"), "{rules}");
        assert!(rules.contains("never retell Alex's actions as if Nova did them"), "{rules}");
        assert!(!rules.contains("{{") && !rules.contains("\n"), "{rules}");
        // Side characters are the preset's business; the rules neither ask for nor forbid them.
        assert!(!rules.contains("only Nova") && !rules.contains("other characters"), "{rules}");
    }

    #[test]
    fn a_one_character_chat_without_a_known_speaker_uses_that_character() {
        let nova = cast(&["Nova"]);
        assert_eq!(reply_rules(instructions::defaults(), "Alex", &nova, None), reply_rules(instructions::defaults(), "Alex", &nova, Some(&nova[0])));
    }

    #[test]
    fn group_rules_say_whose_turn_it_is_and_keep_other_members_out() {
        let crew = cast(&["Alice", "Bob", "Cara"]);
        let rules = reply_rules(instructions::defaults(), "Alex", &crew, Some(&crew[0])).unwrap();

        assert!(rules.starts_with("Roles: the user plays Alex; you play Alice, Bob and Cara, one reply at a time, and this reply is Alice's."), "{rules}");
        assert!(rules.contains("never one of the characters"), "{rules}");
        assert!(rules.contains("starts with the name of the character who spoke it"), "{rules}");
        assert!(rules.contains("Don't write lines or actions for Bob or Cara: they speak in their own turns, so end the reply where Alice's part ends."), "{rules}");
        assert!(rules.contains("without \"Alice:\" in front"), "{rules}");
        assert!(rules.contains("never retell Alex's actions as if a character did them"), "{rules}");
        assert!(!rules.contains("\"you\" usually means"), "{rules}");
    }

    #[test]
    fn group_rules_without_a_speaker_name_the_whole_cast() {
        let crew = cast(&["Alice", "Bob"]);
        let rules = reply_rules(instructions::defaults(), "Alex", &crew, None).unwrap();
        assert!(rules.starts_with("Roles: the user plays Alex; you play Alice and Bob,"), "{rules}");
        assert!(!rules.contains("Don't write lines or actions for"), "{rules}");
    }

    #[test]
    fn no_character_means_no_rules() {
        assert_eq!(reply_rules(instructions::defaults(), "Alex", &[], None), None);
    }

    #[test]
    fn the_group_turn_note_names_the_speaker_and_leaves_the_others_out() {
        let note = group_turn_note(&Instructions::default(), "Alice", &["Bob", "Cara"]);
        assert!(note.starts_with("[System note: Now write Alice's reply, and only Alice's. Any reply from Bob and Cara comes in a message of its own"), "{note}");
        assert!(note.contains("don't plan, think through or write their replies"), "{note}");
        assert!(note.ends_with("think only about what Alice says and does next.]"), "{note}");
    }

    #[test]
    fn impersonation_swaps_sides_explicitly() {
        let note = impersonation_request(instructions::defaults(), "Alex", &cast(&["Nova"]));
        assert!(note.starts_with("[System note: This time, write Alex's next message instead of a reply."), "{note}");
        assert!(note.contains("the sides are swapped: write only what Alex says and does"), "{note}");
        assert!(note.contains("Don't write anything Nova says or does"), "{note}");
        assert!(note.ends_with("nothing else.]"), "{note}");

        assert!(impersonation_request(instructions::defaults(), "Alex", &cast(&["Alice", "Bob"])).contains("Don't write anything Alice or Bob says"));
        assert!(impersonation_request(instructions::defaults(), "User", &[]).contains("Don't write anything the assistant says"));
    }
}

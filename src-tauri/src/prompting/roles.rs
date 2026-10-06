//! Who is who in a roleplay prompt. Providers only see two roles - user and model - so everything
//! the user's character does arrives as a user-role message, and a line like "*I hand you the
//! sword*" is easy for a model to misread as the character's own doing, or to answer by narrating
//! the user's next move as well. These texts spell out, by name, whose message is whose.

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
    format!("[System note: {}]", text.trim())
}

/// The rules sent with every roleplay reply, whatever the preset says (a preset's own system prompt
/// is the user's to edit, and may be anything). `speaker` is whose reply this is; a one-character
/// chat falls back to that character. `None` when there is no character to play.
///
/// Says nothing about side characters either way - whether the model may voice extras is the
/// preset's call. The one hard line is the user's character, plus, in a group, the other members
/// (they get turns of their own).
pub fn reply_rules(user: &str, characters: &[Character], speaker: Option<&Character>) -> Option<String> {
    let speaker = speaker.or(match characters {
        [only] => Some(only),
        _ => None,
    });

    // Who plays whom, the rule about the other group members (if any), and whom the user's
    // character's actions must never be credited to.
    let (roles, members, actor) = if characters.len() > 1 {
        let names: Vec<&str> = characters.iter().map(|c| c.name.as_str()).collect();
        let whose_turn = speaker.map(|me| format!(", one reply at a time, and this reply is {}'s", me.name)).unwrap_or_default();
        let roles = format!(
            "Roles: the user plays {user}; you play {cast}{whose_turn}. Every user-role message is written by {user}: everything in it, *actions* included, is what {user} says, does, thinks or feels - never one of the characters. In {user}'s messages \"I\", \"me\" and \"my\" mean {user}. Each of your earlier messages starts with the name of the character who spoke it.",
            cast = join_names(&names, "and"),
        );
        let others: Vec<&str> = names.iter().copied().filter(|n| speaker.is_some_and(|me| me.name != *n)).collect();
        let members = if others.is_empty() {
            String::new()
        } else {
            format!(" Don't write lines or actions for {}: they speak in their own turns.", join_names(&others, "or"))
        };
        (roles, members, "a character")
    } else {
        let me = speaker?.name.as_str();
        let roles = format!(
            "Roles: the user plays {user}; you play {me}. Every user-role message is written by {user}: everything in it, *actions* included, is what {user} says, does, thinks or feels - never {me}. In {user}'s messages \"I\", \"me\" and \"my\" mean {user}, and \"you\" usually means {me}. Your own messages are {me}'s side of the story."
        );
        (roles, String::new(), me)
    };

    Some(format!(
        "{roles} Text in *asterisks* is action or narration and text in \"quotes\" is speech; both belong to whoever wrote that message. A message in the form [System note: ...] comes from the app, not from {user}: follow it without mentioning it.{members} {user} belongs to the user alone: never write what {user} says, does, thinks or feels, never retell {user}'s actions as if {actor} did them, and don't repeat what {user} just did - react to it and move the scene forward."
    ))
}

/// The trailing note asking for the user's own next message (Impersonate). It goes into the last
/// user turn, not the system prompt - a system-prompt nudge alone lost to strong character cards -
/// and says outright that the usual sides are swapped for this one message.
pub fn impersonation_request(user: &str, characters: &[Character]) -> String {
    let names: Vec<&str> = characters.iter().map(|c| c.name.as_str()).collect();
    let others = if names.is_empty() { "the assistant".to_string() } else { join_names(&names, "or") };
    system_note(&format!(
        "This time, write {user}'s next message instead of a reply. {user} is the user's character, so for this one message the sides are swapped: write only what {user} says and does, from {user}'s point of view, matching the person, tense and style of {user}'s earlier messages. Don't write anything {others} says or does, and don't continue the last reply. Reply with only the message text, nothing else."
    ))
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
    fn system_notes_are_bracketed_and_labelled() {
        assert_eq!(system_note("  Remember the twist.\n"), "[System note: Remember the twist.]");
    }

    #[test]
    fn one_on_one_rules_name_both_sides() {
        let nova = cast(&["Nova"]);
        let rules = reply_rules("Alex", &nova, Some(&nova[0])).unwrap();

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
        assert_eq!(reply_rules("Alex", &nova, None), reply_rules("Alex", &nova, Some(&nova[0])));
    }

    #[test]
    fn group_rules_say_whose_turn_it_is_and_keep_other_members_out() {
        let crew = cast(&["Alice", "Bob", "Cara"]);
        let rules = reply_rules("Alex", &crew, Some(&crew[0])).unwrap();

        assert!(rules.starts_with("Roles: the user plays Alex; you play Alice, Bob and Cara, one reply at a time, and this reply is Alice's."), "{rules}");
        assert!(rules.contains("never one of the characters"), "{rules}");
        assert!(rules.contains("starts with the name of the character who spoke it"), "{rules}");
        assert!(rules.contains("Don't write lines or actions for Bob or Cara: they speak in their own turns."), "{rules}");
        assert!(rules.contains("never retell Alex's actions as if a character did them"), "{rules}");
        assert!(!rules.contains("\"you\" usually means"), "{rules}");
    }

    #[test]
    fn group_rules_without_a_speaker_name_the_whole_cast() {
        let crew = cast(&["Alice", "Bob"]);
        let rules = reply_rules("Alex", &crew, None).unwrap();
        assert!(rules.starts_with("Roles: the user plays Alex; you play Alice and Bob."), "{rules}");
        assert!(!rules.contains("Don't write lines or actions for"), "{rules}");
    }

    #[test]
    fn no_character_means_no_rules() {
        assert_eq!(reply_rules("Alex", &[], None), None);
    }

    #[test]
    fn impersonation_swaps_sides_explicitly() {
        let note = impersonation_request("Alex", &cast(&["Nova"]));
        assert!(note.starts_with("[System note: This time, write Alex's next message instead of a reply."), "{note}");
        assert!(note.contains("the sides are swapped: write only what Alex says and does"), "{note}");
        assert!(note.contains("Don't write anything Nova says or does"), "{note}");
        assert!(note.ends_with("nothing else.]"), "{note}");

        assert!(impersonation_request("Alex", &cast(&["Alice", "Bob"])).contains("Don't write anything Alice or Bob says"));
        assert!(impersonation_request("User", &[]).contains("Don't write anything the assistant says"));
    }
}

//! Who speaks up after a group reply. A model asked to write one member's reply, and to say in a tag
//! whether another member should answer too, mostly leaves the tag out - and copies that from its
//! own earlier turns. So when a reply names nobody, the model is asked the question on its own: of
//! the members who haven't answered the user's latest message, would one react to what was just
//! said? (See `ChatEngine::group_round`.)

use crate::prompting::next_speaker_tag;

/// What the question starts with (tests look for it).
pub const ASK: &str = "You decide who speaks next in a group scene";

/// The question: the newest lines of the scene, and the members who may still speak.
pub fn prompt(user: &str, candidates: &[&str], transcript: &str) -> String {
    let names = candidates.join(", ");
    format!(
        "{ASK}. {user} is the user's character; the members of the group take turns answering. The newest lines come last.\n\n\
Decide whether one of these members would naturally speak up or react right now: {names}. They haven't said anything yet in answer \
to {user}'s latest message. Someone would if they were addressed or mentioned, have something to add, would disagree or be affected, \
or simply wouldn't let the moment pass in a shared scene. People in a scene usually react to what happens around them, but nobody \
speaks just to fill space: if the moment is complete as it stands, nobody does.\n\n\
The scene:\n{transcript}\n\n\
Answer with exactly one name from {names}, or NONE."
    )
}

/// The member the answer names (an index into `candidates`), or `None`.
pub fn parse(answer: &str, candidates: &[&str]) -> Option<usize> {
    let line = answer.lines().map(str::trim).find(|l| !l.is_empty())?;
    let name = line.trim_matches(|c: char| !c.is_alphanumeric() && c != ' ');
    let indexed: Vec<(usize, &str)> = candidates.iter().copied().enumerate().collect();
    next_speaker_tag::resolve(name, &indexed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_names_the_members_and_carries_the_scene() {
        let text = prompt("Alex", &["Bob", "Cara"], "Alex: Hi\nAlice: Hello!");
        assert!(text.starts_with(ASK));
        for needle in ["react right now: Bob, Cara.", "answer to Alex's latest message", "Alex: Hi\nAlice: Hello!", "one name from Bob, Cara, or NONE"] {
            assert!(text.contains(needle), "{needle}: {text}");
        }
    }

    #[test]
    fn the_answer_is_a_candidate_or_nobody() {
        let cast = ["Bob Stone", "Cara"];
        assert_eq!(parse("Cara", &cast), Some(1));
        assert_eq!(parse("**Bob**\nBecause he was mentioned.", &cast), Some(0));
        assert_eq!(parse("\"bob stone\".", &cast), Some(0));
        assert_eq!(parse("NONE", &cast), None);
        assert_eq!(parse("Dave", &cast), None);
        assert_eq!(parse("", &cast), None);
    }
}

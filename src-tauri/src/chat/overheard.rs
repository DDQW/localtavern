//! What a private talk gives away. A private talk usually happens right inside the scene (a corner
//! of the same room, the next table, the back seat), so the others can hear a raised voice or see a
//! kiss, and a talk over a phone or behind a closed door gives nothing away. After each reply in
//! one, the model is asked in a separate request what the others could perceive, and which of them
//! would react to it. The app notes that in the group chat and lets those members answer (see
//! `ChatEngine::spawn_overheard`).

/// What the question starts with (tests look for it).
pub const ASK: &str = "You decide what the rest of a group notices of a private conversation";

/// The most members who react to one thing: more reads like a chorus.
const MAX_REACTORS: usize = 2;

/// What the question is about: a private talk, and the scene around it.
pub struct Scene<'a> {
    pub user: &'a str,
    /// The member in the private talk.
    pub talker: &'a str,
    /// Everyone else in the group.
    pub others: &'a [&'a str],
    /// The group chat's newest lines, one per line.
    pub group: &'a str,
    /// The private talk before the newest exchange, one line per message.
    pub talk: &'a str,
    pub user_text: &'a str,
    pub reply_text: &'a str,
}

/// The question for the newest exchange of a private talk.
pub fn prompt(scene: &Scene<'_>) -> String {
    fn or_nothing(text: &str) -> &str {
        if text.trim().is_empty() { "(nothing yet)" } else { text }
    }
    let Scene { user, talker, user_text, reply_text, .. } = scene;
    let others = scene.others.join(", ");
    let (group, talk) = (or_nothing(scene.group), or_nothing(scene.talk));
    format!(
        "{ASK}. {user} and {talker} are talking in private inside a group scene. The rest of the group ({others}) is not part of the \
conversation.\n\n\
Judge only the newest exchange. The others notice something only when the story puts them where they could perceive it: the two \
are in the same room or space, within earshot or sight (a corner of the same room, the next table, a party, a shared car, a camp). \
They notice nothing when the two are clearly apart - another room with the door closed, somewhere far away, or talking over a \
phone, messages or a chat window - unless something is loud or dramatic enough to carry (shouting, a crash, a scream). When the \
story doesn't say where the talk is, judge from the group chat and the talk itself.\n\n\
What the others can notice is the outward surface only: raised or suddenly hushed voices, laughter, crying, gestures, a kiss or \
other touch, one of the two storming off or coming back flustered, noises. Quietly spoken words stay private: never reveal what \
was whispered, thought or confided, or anything the others couldn't actually see or hear, and don't invent anything the newest \
exchange doesn't show. Small things that would pass unnoticed don't count, and neither does what the group chat already noted.\n\n\
Then say which of the others would react to it right now, out loud or visibly: only those it would plausibly affect or who would \
plainly notice. Most of the time that is nobody or one person.\n\n\
The group chat so far:\n{group}\n\n\
The private talk so far:\n{talk}\n\n\
The newest exchange:\n{user}: {user_text}\n{talker}: {reply_text}\n\n\
Answer with exactly two lines and nothing else:\n\
NOTICED: <one or two short sentences of plain third-person narration, in the past tense, of only what the others could hear or \
see, naming {user} and {talker} - or NONE>\n\
REACT: <the names, separated by commas, of those of {others} who would react - or NONE>\n\
If NOTICED is NONE, REACT is NONE."
    )
}

/// What the others perceived, and who of them (indexes into the question's `others`) reacts.
#[derive(Debug, PartialEq)]
pub struct Overheard {
    pub note: String,
    pub reactors: Vec<usize>,
}

/// The value after `label:` on a line, ignoring bullets, bold and case.
fn field<'a>(answer: &'a str, label: &str) -> Option<&'a str> {
    answer.lines().find_map(|line| {
        let line = line.trim().trim_start_matches(['-', '*', '#', ' ']).trim_start_matches("**");
        let head = line.get(..label.len())?;
        let rest = line[label.len()..].trim_start_matches("**").trim_start();
        (head.eq_ignore_ascii_case(label) && rest.starts_with(':')).then(|| rest[1..].trim().trim_start_matches("**").trim())
    })
}

fn is_none(text: &str) -> bool {
    let text = text.trim_matches(|c: char| !c.is_alphanumeric());
    text.is_empty() || text.eq_ignore_ascii_case("none") || text.eq_ignore_ascii_case("nobody") || text.eq_ignore_ascii_case("n/a")
}

/// The member a name in the answer means: the whole name, or its first word when that is unambiguous.
fn member(name: &str, others: &[&str]) -> Option<usize> {
    let name = name.trim().trim_matches(['"', '*', '.']).to_lowercase();
    if name.is_empty() {
        return None;
    }
    let exact = others.iter().position(|o| o.trim().to_lowercase() == name);
    exact.or_else(|| {
        let first = |s: &str| s.split_whitespace().next().map(str::to_lowercase);
        let mut matching = others.iter().enumerate().filter(|(_, o)| first(o) == first(&name));
        match (matching.next(), matching.next()) {
            (Some((index, _)), None) => Some(index),
            _ => None,
        }
    })
}

/// The answer to [`prompt`]; `None` when the others noticed nothing.
pub fn parse(answer: &str, others: &[&str]) -> Option<Overheard> {
    let note = field(answer, "NOTICED").filter(|n| !is_none(n))?;
    let mut reactors: Vec<usize> = Vec::new();
    let named = field(answer, "REACT").filter(|r| !is_none(r)).unwrap_or_default().replace(" and ", ",");
    for name in named.split([',', ';']) {
        if let Some(index) = member(name, others).filter(|i| !reactors.contains(i)) {
            reactors.push(index);
        }
    }
    reactors.truncate(MAX_REACTORS);
    Some(Overheard { note: note.to_string(), reactors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_carries_the_scene_and_the_newest_exchange() {
        let scene = Scene {
            user: "Alex",
            talker: "Mira",
            others: &["Bob", "Cara"],
            group: "Bob: Nice party.",
            talk: "Alex: Not here.",
            user_text: "You burned it?!",
            reply_text: "Keep your voice down.",
        };
        let text = prompt(&scene);
        assert!(text.starts_with(ASK));
        for needle in ["Alex and Mira are talking in private", "The rest of the group (Bob, Cara)", "Bob: Nice party.", "Alex: Not here.", "Alex: You burned it?!\nMira: Keep your voice down.", "phone"] {
            assert!(text.contains(needle), "{needle}");
        }
    }

    #[test]
    fn a_noticed_thing_and_its_reactors_are_read_in_any_shape() {
        let others = ["Bob Stone", "Cara"];
        let answer = "**NOTICED:** Alex's voice rose across the room and Mira pulled him toward the window.\n- REACT: bob, Cara and nobody";
        let heard = parse(answer, &others).unwrap();
        assert_eq!(heard.note, "Alex's voice rose across the room and Mira pulled him toward the window.");
        assert_eq!(heard.reactors, [0, 1]);

        let again = parse("noticed: A door slammed.\nreact: Cara, Cara, Bob Stone", &others).unwrap();
        assert_eq!(again.reactors, [1, 0], "named once each, in the order given");
    }

    #[test]
    fn nothing_noticed_means_nothing_to_note_or_react_to() {
        let others = ["Bob", "Cara"];
        assert_eq!(parse("NOTICED: NONE\nREACT: NONE", &others), None);
        assert_eq!(parse("NOTICED: none.\nREACT: Bob", &others), None);
        assert_eq!(parse("Sorry, I can't tell.", &others), None);
        let quiet = parse("NOTICED: Alex laughed loudly.\nREACT: NONE", &others).unwrap();
        assert!(quiet.reactors.is_empty(), "a noise can be noted without anyone answering it");
        assert!(parse("NOTICED: Alex laughed loudly.\nREACT: Dave", &others).unwrap().reactors.is_empty(), "strangers don't react");
    }

    #[test]
    fn at_most_two_members_react_and_ambiguous_first_names_are_skipped() {
        let others = ["Ann Lee", "Ann Ray", "Bob", "Cara"];
        let heard = parse("NOTICED: Glass broke.\nREACT: Ann, Bob, Cara", &others).unwrap();
        assert_eq!(heard.reactors, [2, 3], "'Ann' could be either, so she isn't picked");
        assert_eq!(parse("NOTICED: Glass broke.\nREACT: Bob, Cara, Ann Ray", &others).unwrap().reactors, [2, 3]);
    }
}

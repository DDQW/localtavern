//! Group chat speaker selection. An empty result means "let the user pick manually".

use regex::RegexBuilder;
use uuid::Uuid;

use crate::models::{Character, ChatMessage, GroupActivationMode, GroupChat, MessageRole};

pub trait GroupChatActivationStrategy {
    /// Returns which character(s) should speak next, in order. An empty list means "let the user
    /// pick manually" (used by [`ManualActivationStrategy`]).
    fn select_next_speakers(
        &self,
        group: &GroupChat,
        characters: &[Character],
        history: &[ChatMessage],
        latest_user_message: &str,
    ) -> Vec<Uuid>;
}

/// Always defers to the user - the UI shows a speaker-picker after every user turn.
pub struct ManualActivationStrategy;

impl GroupChatActivationStrategy for ManualActivationStrategy {
    fn select_next_speakers(&self, _: &GroupChat, _: &[Character], _: &[ChatMessage], _: &str) -> Vec<Uuid> {
        Vec::new()
    }
}

/// Cycles through the group's characters in list order, skipping the last speaker unless
/// `allow_self_responses`.
pub struct RoundRobinActivationStrategy;

impl GroupChatActivationStrategy for RoundRobinActivationStrategy {
    fn select_next_speakers(&self, group: &GroupChat, _: &[Character], history: &[ChatMessage], _: &str) -> Vec<Uuid> {
        let order = &group.character_ids;
        if order.is_empty() {
            return Vec::new();
        }

        let last_speaker = history
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::Character)
            .and_then(|m| m.speaker_character_id);
        let start_index: isize = last_speaker
            .and_then(|id| order.iter().position(|c| *c == id))
            .map_or(-1, |i| i as isize);

        let n = order.len() as isize;
        for offset in 1..=n {
            let candidate = order[((start_index + offset + n) % n) as usize];
            if group.allow_self_responses || Some(candidate) != last_speaker {
                return vec![candidate];
            }
        }
        vec![order[0]]
    }
}

/// Scans the latest user message for whole-word mentions of member character names (in mention
/// order). Falls back to round-robin when no character is mentioned.
pub struct TriggerWordActivationStrategy {
    pub fallback: RoundRobinActivationStrategy,
}

impl GroupChatActivationStrategy for TriggerWordActivationStrategy {
    fn select_next_speakers(
        &self,
        group: &GroupChat,
        characters: &[Character],
        history: &[ChatMessage],
        latest_user_message: &str,
    ) -> Vec<Uuid> {
        let mut mentioned: Vec<(usize, Uuid)> = characters
            .iter()
            .filter(|c| group.character_ids.contains(&c.id) && !c.name.trim().is_empty())
            .filter_map(|c| index_of_mention(latest_user_message, &c.name).map(|i| (i, c.id)))
            .collect();
        mentioned.sort_by_key(|(index, _)| *index); // stable

        if mentioned.is_empty() {
            self.fallback.select_next_speakers(group, characters, history, latest_user_message)
        } else {
            mentioned.into_iter().map(|(_, id)| id).collect()
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Finds the first whole-word mention of `name`. A plain substring search lets short names like
/// "Al" fire on "always", so the name must not be flanked by word characters. (The `regex` crate has
/// no lookaround, so the boundary check is done by hand.)
fn index_of_mention(message: &str, name: &str) -> Option<usize> {
    let re = RegexBuilder::new(&regex::escape(name)).case_insensitive(true).build().ok()?;
    let mut from = 0;
    while let Some(m) = re.find_at(message, from) {
        let before_ok = message[..m.start()].chars().next_back().is_none_or(|c| !is_word_char(c));
        let after_ok = message[m.end()..].chars().next().is_none_or(|c| !is_word_char(c));
        if before_ok && after_ok {
            return Some(m.start());
        }
        // Resume one character later so overlapping candidates aren't skipped.
        from = m.start() + message[m.start()..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// The strategy for a group's configured activation mode.
pub fn strategy_for(mode: GroupActivationMode) -> Box<dyn GroupChatActivationStrategy + Send + Sync> {
    match mode {
        GroupActivationMode::Manual => Box::new(ManualActivationStrategy),
        GroupActivationMode::RoundRobin => Box::new(RoundRobinActivationStrategy),
        GroupActivationMode::TriggerWord => {
            Box::new(TriggerWordActivationStrategy { fallback: RoundRobinActivationStrategy })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character(name: &str) -> Character {
        Character { name: name.into(), ..Default::default() }
    }

    fn spoke(id: Uuid) -> ChatMessage {
        let mut m = ChatMessage::new(Uuid::new_v4(), MessageRole::Character, "");
        m.speaker_character_id = Some(id);
        m
    }

    struct Cast {
        alice: Character,
        bob: Character,
        carol: Character,
    }

    impl Cast {
        fn new() -> Self {
            Self { alice: character("Alice"), bob: character("Bob"), carol: character("Carol") }
        }
        fn all(&self) -> Vec<Character> {
            vec![self.alice.clone(), self.bob.clone(), self.carol.clone()]
        }
        fn group(&self) -> GroupChat {
            GroupChat { character_ids: vec![self.alice.id, self.bob.id, self.carol.id], ..Default::default() }
        }
    }

    fn trigger() -> TriggerWordActivationStrategy {
        TriggerWordActivationStrategy { fallback: RoundRobinActivationStrategy }
    }

    #[test]
    fn manual_always_returns_empty() {
        let cast = Cast::new();
        assert!(ManualActivationStrategy.select_next_speakers(&cast.group(), &cast.all(), &[], "hello").is_empty());
    }

    #[test]
    fn round_robin_picks_the_first_member_when_no_one_has_spoken() {
        let cast = Cast::new();
        let result = RoundRobinActivationStrategy.select_next_speakers(&cast.group(), &cast.all(), &[], "hello");
        assert_eq!(result, [cast.alice.id]);
    }

    #[test]
    fn round_robin_advances_after_the_last_speaker() {
        let cast = Cast::new();
        let history = [spoke(cast.alice.id)];
        let result = RoundRobinActivationStrategy.select_next_speakers(&cast.group(), &cast.all(), &history, "hello");
        assert_eq!(result, [cast.bob.id]);
    }

    #[test]
    fn round_robin_wraps_around() {
        let cast = Cast::new();
        let history = [spoke(cast.carol.id)];
        let result = RoundRobinActivationStrategy.select_next_speakers(&cast.group(), &cast.all(), &history, "hello");
        assert_eq!(result, [cast.alice.id]);
    }

    #[test]
    fn round_robin_skips_the_last_speaker_when_self_responses_are_disallowed() {
        let cast = Cast::new();
        let group = GroupChat { character_ids: vec![cast.alice.id, cast.bob.id], allow_self_responses: false, ..Default::default() };
        let history = [spoke(cast.alice.id)];
        let result = RoundRobinActivationStrategy.select_next_speakers(&group, &cast.all(), &history, "hello");
        assert_eq!(result, [cast.bob.id]);
    }

    #[test]
    fn round_robin_with_a_single_member_and_no_self_responses_still_answers() {
        let cast = Cast::new();
        let group = GroupChat { character_ids: vec![cast.alice.id], allow_self_responses: false, ..Default::default() };
        let history = [spoke(cast.alice.id)];
        let result = RoundRobinActivationStrategy.select_next_speakers(&group, &cast.all(), &history, "hello");
        assert_eq!(result, [cast.alice.id]);
    }

    #[test]
    fn round_robin_of_an_empty_group_is_empty() {
        let group = GroupChat::default();
        assert!(RoundRobinActivationStrategy.select_next_speakers(&group, &[], &[], "hi").is_empty());
    }

    #[test]
    fn trigger_word_matches_a_mentioned_character() {
        let cast = Cast::new();
        let result = trigger().select_next_speakers(&cast.group(), &cast.all(), &[], "Hey Bob, what do you think?");
        assert_eq!(result, [cast.bob.id]);
    }

    #[test]
    fn trigger_word_returns_multiple_mentions_in_mention_order() {
        let cast = Cast::new();
        let result = trigger().select_next_speakers(&cast.group(), &cast.all(), &[], "Carol and Alice, both weigh in.");
        assert_eq!(result, [cast.carol.id, cast.alice.id]);
    }

    #[test]
    fn trigger_word_falls_back_to_round_robin_when_no_one_is_mentioned() {
        let cast = Cast::new();
        let result = trigger().select_next_speakers(&cast.group(), &cast.all(), &[], "What's everyone up to?");
        assert_eq!(result, [cast.alice.id]);
    }

    #[test]
    fn trigger_word_ignores_names_that_are_only_substrings_of_other_words() {
        let al = character("Al");
        let bob = character("Bob");
        let group = GroupChat { character_ids: vec![al.id, bob.id], ..Default::default() };
        // "Al" appears inside "always" - not a mention, so round-robin picks the first member.
        let result = trigger().select_next_speakers(&group, &[al.clone(), bob], &[], "I always enjoy this.");
        assert_eq!(result, [al.id]);
    }

    #[test]
    fn trigger_word_matches_a_short_name_at_a_word_boundary() {
        let al = character("Al");
        let bob = character("Bob");
        let group = GroupChat { character_ids: vec![bob.id, al.id], ..Default::default() };
        // Round-robin would start with Bob, so getting Al proves the mention matched.
        let result = trigger().select_next_speakers(&group, &[al.clone(), bob], &[], "What about you, Al?");
        assert_eq!(result, [al.id]);
    }

    #[test]
    fn trigger_word_finds_a_later_valid_mention_after_an_invalid_one() {
        let al = character("Al");
        let bob = character("Bob");
        let group = GroupChat { character_ids: vec![bob.id, al.id], ..Default::default() };
        let result = trigger().select_next_speakers(&group, &[al.clone(), bob], &[], "always... anyway, Al?");
        assert_eq!(result, [al.id]);
    }

    #[test]
    fn trigger_word_matches_multi_word_names_with_punctuation() {
        let jack = character("Captain Jack (The Third)");
        let bob = character("Bob");
        let group = GroupChat { character_ids: vec![bob.id, jack.id], ..Default::default() };
        let result =
            trigger().select_next_speakers(&group, &[jack.clone(), bob], &[], "Ask Captain Jack (The Third) about it.");
        assert_eq!(result, [jack.id]);
    }

    #[test]
    fn trigger_word_is_case_insensitive_and_only_considers_members() {
        let cast = Cast::new();
        let outsider = character("Dave");
        let group = GroupChat { character_ids: vec![cast.alice.id, cast.bob.id], ..Default::default() };
        let mut everyone = cast.all();
        everyone.push(outsider);
        let result = trigger().select_next_speakers(&group, &everyone, &[], "bob? or DAVE? or carol?");
        assert_eq!(result, [cast.bob.id]);
    }

    #[test]
    fn strategy_for_maps_each_mode() {
        let cast = Cast::new();
        let group = cast.group();
        assert!(strategy_for(GroupActivationMode::Manual).select_next_speakers(&group, &cast.all(), &[], "x").is_empty());
        assert_eq!(
            strategy_for(GroupActivationMode::RoundRobin).select_next_speakers(&group, &cast.all(), &[], "x"),
            [cast.alice.id]
        );
        assert_eq!(
            strategy_for(GroupActivationMode::TriggerWord).select_next_speakers(&group, &cast.all(), &[], "hi Carol"),
            [cast.carol.id]
        );
    }
}

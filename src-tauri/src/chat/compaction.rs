//! Keeping a long conversation inside the model's context window: how full a request is, and
//! compacting older messages into a running summary that is sent in their place.
//!
//! Token counts here are estimates (about 4 characters per token, the same rule the preset's
//! history limit uses) - close enough to decide when to compact without the provider's tokenizer.
//! Real counts come from the provider's usage report on each reply.

use serde::Serialize;

use crate::llm::LlmRequest;
use crate::models::{Character, ChatMessage, ConversationSummary, MessageRole};

const APPROX_CHARS_PER_TOKEN: usize = 4;

/// A request is compacted before sending once it would fill this share of the window, leaving room
/// for the reply.
pub const AUTO_COMPACT_PERCENT: i32 = 80;

/// The newest messages are always sent in full, never summarized.
pub const KEEP_RECENT_MESSAGES: usize = 6;

/// Each summarizing request gets at most this share of the window for the messages it condenses,
/// so condensing a huge backlog still fits; a longer backlog is condensed in several passes.
const CHUNK_SHARE_PERCENT: usize = 50;

/// Chunk size when the window is unknown.
const DEFAULT_CHUNK_TOKENS: usize = 24_000;

/// How full the conversation's context is, for the meter above the chat.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextStatus {
    /// Tokens the model can hold, if known.
    pub window: Option<i32>,
    /// Where `window` came from: "provider" (the model's own limit, or Ollama's num_ctx) or
    /// "preset" (the preset's max history tokens). `None` when unknown.
    pub window_source: Option<&'static str>,
    /// Tokens the next request would use.
    pub used: i32,
    /// `used` is an estimate from the text rather than the provider's own count of the last reply.
    pub estimated: bool,
    pub summary: Option<ConversationSummary>,
    /// How many messages the summary stands in for.
    pub summarized_count: usize,
    /// How many messages a compaction now would condense (0: nothing to compact).
    pub compactable_count: usize,
    pub auto_compact_percent: i32,
}

pub fn estimate_text_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(APPROX_CHARS_PER_TOKEN)
}

/// The whole request: system instruction plus every message, with a few tokens per message for the
/// role markers providers add.
pub fn estimate_request_tokens(request: &LlmRequest) -> i32 {
    let system = request.system_instruction.as_deref().map(estimate_text_tokens).unwrap_or(0);
    let history: usize = request.history.iter().map(|m| estimate_text_tokens(&m.text) + 4).sum();
    (system + history) as i32
}

/// Whether a request of `used` tokens should be compacted first.
pub fn needs_compaction(used: i32, window: Option<i32>) -> bool {
    window.is_some_and(|w| w > 0 && used as i64 * 100 >= w as i64 * AUTO_COMPACT_PERCENT as i64)
}

/// Which of `uncovered` (the messages not yet summarized) a compaction condenses: all but the newest few.
pub fn to_condense(uncovered: &[ChatMessage]) -> &[ChatMessage] {
    &uncovered[..uncovered.len().saturating_sub(KEEP_RECENT_MESSAGES)]
}

/// Who wrote each message, by name, so the summary keeps the player's and characters' actions apart.
pub struct Names<'a> {
    pub user: &'a str,
    pub characters: &'a [Character],
    pub roleplay: bool,
}

impl Names<'_> {
    fn author(&self, message: &ChatMessage) -> String {
        match message.role {
            MessageRole::User => self.user.to_string(),
            MessageRole::Tool => "Tool result".to_string(),
            MessageRole::System => "System".to_string(),
            MessageRole::Character => message
                .speaker_character_id
                .and_then(|id| self.characters.iter().find(|c| c.id == id))
                .or(match self.characters {
                    [only] => Some(only),
                    _ => None,
                })
                .map(|c| c.name.clone())
                .unwrap_or_else(|| if self.roleplay { "Narrator".into() } else { "Assistant".into() }),
        }
    }
}

/// Splits the messages to condense into transcripts that each fit `window`'s chunk share.
pub fn transcripts(messages: &[ChatMessage], names: &Names, window: Option<i32>) -> Vec<String> {
    let chunk_tokens = window
        .filter(|w| *w > 0)
        .map(|w| w as usize * CHUNK_SHARE_PERCENT / 100)
        .unwrap_or(DEFAULT_CHUNK_TOKENS)
        .max(1000);
    let chunk_chars = chunk_tokens * APPROX_CHARS_PER_TOKEN;

    let mut chunks = Vec::new();
    let mut current = String::new();
    for message in messages {
        let entry = format!("{}:\n{}\n\n", names.author(message), message.effective_text().trim());
        if !current.is_empty() && current.chars().count() + entry.chars().count() > chunk_chars {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(&entry);
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

/// The question that condenses one transcript into the running summary.
pub fn summary_prompt(previous: Option<&str>, transcript: &str, names: &Names) -> String {
    let previous = previous.filter(|p| !p.trim().is_empty());
    let earlier = match previous {
        Some(text) => format!("Summary of everything before these messages:\n{}\n\n", text.trim()),
        None => String::new(),
    };
    let what = if names.roleplay {
        let cast: Vec<&str> = names.characters.iter().map(|c| c.name.as_str()).collect();
        let cast = crate::prompting::roles::join_names(&cast, "and");
        let cast = if cast.is_empty() { "the characters".to_string() } else { cast };
        format!(
            "This is a roleplay between {user} (played by the user) and {cast}. Write the story so far: the important events in order, decisions and promises made, how the relationships stand, facts worth remembering (names, places, items, secrets) and the current situation - where everyone is and what is happening right now. Keep who did what exact: actions in {user}'s messages were done by {user}. Write in past tense, third person, as plain prose.",
            user = names.user,
        )
    } else {
        "This is a conversation between the user and an assistant. Write what it has covered so far: the topics and questions, what was decided or answered, facts the user gave about themselves or their work, important details (names, numbers, code, file paths, commands) and anything still open.".to_string()
    };
    let merge = if previous.is_some() {
        " Merge the earlier summary and the new messages into one updated summary, keeping what still matters from both."
    } else {
        ""
    };
    format!(
        "You are compacting the history of a long chat so it fits the model's memory. {what}{merge} Be concise but don't drop anything a later reply could need. At most about 800 words. Reply with only the summary.\n\n{earlier}Messages:\n{transcript}"
    )
}

/// The block that stands in for the summarized messages, appended to the system instruction.
pub fn summary_instruction(summary: &str, roleplay: bool) -> String {
    let label = if roleplay { "The story so far" } else { "Summary of the earlier conversation" };
    format!("{label} (older messages are no longer shown; this summary replaces them):\n{}", summary.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmMessage, LlmRole};
    use uuid::Uuid;

    fn message(role: MessageRole, text: &str) -> ChatMessage {
        ChatMessage::new(Uuid::new_v4(), role, text)
    }

    #[test]
    fn estimates_about_four_characters_per_token() {
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("abcd"), 1);
        assert_eq!(estimate_text_tokens("abcde"), 2);
        let request = LlmRequest {
            model: "m".into(),
            system_instruction: Some("x".repeat(40)),
            history: vec![LlmMessage::new(LlmRole::User, "y".repeat(8))],
            generation_config: Default::default(),
            enable_web_search: None,
        };
        assert_eq!(estimate_request_tokens(&request), 10 + 2 + 4);
    }

    #[test]
    fn compaction_kicks_in_at_the_threshold_and_never_without_a_window() {
        assert!(!needs_compaction(79, Some(100)));
        assert!(needs_compaction(80, Some(100)));
        assert!(!needs_compaction(1_000_000, None));
        assert!(!needs_compaction(10, Some(0)));
    }

    #[test]
    fn the_newest_messages_are_never_condensed() {
        let list: Vec<_> = (0..10).map(|i| message(MessageRole::User, &i.to_string())).collect();
        assert_eq!(to_condense(&list).len(), 10 - KEEP_RECENT_MESSAGES);
        assert!(to_condense(&list[..3]).is_empty());
    }

    #[test]
    fn transcripts_name_each_author_and_split_to_fit_the_window() {
        let nova = Character { name: "Nova".into(), ..Default::default() };
        let names = Names { user: "Alex", characters: std::slice::from_ref(&nova), roleplay: true };
        let list = vec![message(MessageRole::User, "*waves*"), message(MessageRole::Character, "Hello!")];

        let one = transcripts(&list, &names, Some(100_000));
        assert_eq!(one, ["Alex:\n*waves*\n\nNova:\nHello!\n\n"]);

        let long: Vec<_> = (0..10).map(|_| message(MessageRole::User, &"w".repeat(1000))).collect();
        // A 2000-token window allows 1000-token (4000-char) chunks: three messages each, then the rest.
        let chunks = transcripts(&long, &names, Some(2000));
        assert_eq!(chunks.len(), 4);
        assert!(chunks.iter().all(|c| c.chars().count() <= 4000), "{:?}", chunks.iter().map(|c| c.len()).collect::<Vec<_>>());
    }

    #[test]
    fn the_prompt_merges_an_earlier_summary_and_keeps_roleplay_authors_straight() {
        let nova = Character { name: "Nova".into(), ..Default::default() };
        let names = Names { user: "Alex", characters: std::slice::from_ref(&nova), roleplay: true };

        let first = summary_prompt(None, "Alex:\nhi\n\n", &names);
        assert!(first.contains("between Alex (played by the user) and Nova"), "{first}");
        assert!(first.contains("actions in Alex's messages were done by Alex"), "{first}");
        assert!(!first.contains("Summary of everything before"), "{first}");

        let next = summary_prompt(Some("They met."), "Alex:\nbye\n\n", &names);
        assert!(next.contains("Summary of everything before these messages:\nThey met."), "{next}");
        assert!(next.contains("Merge the earlier summary"), "{next}");

        let assistant = summary_prompt(None, "User:\nhi\n\n", &Names { user: "User", characters: &[], roleplay: false });
        assert!(assistant.contains("between the user and an assistant"), "{assistant}");
    }
}

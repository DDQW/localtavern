//! Catches a `<powershell-run>` tag in a reply as it streams, so the raw tag never flashes in the
//! chat bubble. Unlike the emotion tag - which is only looked for once a reply has fully arrived - a
//! tool call has to be caught mid-stream, so generation can stop and the command run before the
//! model says anything else. One filter per generated reply.
//!
//! It withholds only the shortest trailing suffix that could still turn into the opening tag, so
//! ordinary prose streams through immediately; once the opener is seen, everything is withheld until
//! the closer arrives.

use crate::prompting::powershell_tool::{try_parse, PowerShellToolCallRequest, CLOSE_TAG, OPEN_TAG};

#[derive(Default)]
pub struct ToolCallFilter {
    pending: String,
    detected: Option<PowerShellToolCallRequest>,
}

impl ToolCallFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set once a complete, well-formed tag has been seen.
    pub fn detected_call(&self) -> Option<&PowerShellToolCallRequest> {
        self.detected.as_ref()
    }

    /// Feeds a newly arrived piece of text; returns the part that is safe to show right away (which
    /// may be shorter than `delta`, or empty).
    pub fn feed(&mut self, delta: &str) -> String {
        if self.detected.is_some() || delta.is_empty() {
            return String::new();
        }
        self.pending.push_str(delta);
        let text = std::mem::take(&mut self.pending);

        let Some(open) = text.find(OPEN_TAG) else {
            let hold = longest_suffix_matching_prefix(&text, OPEN_TAG);
            let split = text.len() - hold;
            self.pending = text[split..].to_string();
            return text[..split].to_string();
        };

        let before = text[..open].to_string();
        let Some(close) = text[open..].find(CLOSE_TAG).map(|i| i + open) else {
            self.pending = text[open..].to_string();
            return before;
        };

        let tag_end = close + CLOSE_TAG.len();
        let tag = &text[open..tag_end];
        match try_parse(tag) {
            Some(call) => {
                self.detected = Some(call);
                before
            }
            // A well-formed open/close pair that isn't a valid tag (say, a stray attribute): show
            // it as written rather than silently dropping model output. What follows is fed again.
            None => {
                let rest = text[tag_end..].to_string();
                let mut shown = before;
                shown.push_str(tag);
                shown.push_str(&self.feed(&rest));
                shown
            }
        }
    }
}

/// Byte length of the longest proper suffix of `text` that is also a prefix of `marker` - how much
/// trailing text could still become the marker as more arrives.
fn longest_suffix_matching_prefix(text: &str, marker: &str) -> usize {
    (1..marker.len().min(text.len() + 1))
        .rev()
        .find(|&len| text.is_char_boundary(text.len() - len) && text.as_bytes()[text.len() - len..] == marker.as_bytes()[..len])
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `chunks` and returns everything shown, plus the filter.
    fn run(chunks: &[&str]) -> (String, ToolCallFilter) {
        let mut filter = ToolCallFilter::new();
        let shown = chunks.iter().map(|c| filter.feed(c)).collect::<String>();
        (shown, filter)
    }

    #[test]
    fn plain_text_passes_straight_through() {
        let (shown, filter) = run(&["Hello ", "there, ", "friend."]);
        assert_eq!(shown, "Hello there, friend.");
        assert!(filter.detected_call().is_none());
    }

    #[test]
    fn a_tag_is_swallowed_and_parsed_and_the_text_before_it_is_kept() {
        let (shown, filter) = run(&["Let me check. <powershell-run>Get-Date</powershell-run>"]);
        assert_eq!(shown, "Let me check. ");
        assert_eq!(filter.detected_call().unwrap().command, "Get-Date");
    }

    #[test]
    fn the_elevate_attribute_is_understood() {
        let (_, filter) = run(&["<powershell-run elevate=\"true\">Restart-Service x</powershell-run>"]);
        assert!(filter.detected_call().unwrap().elevate);
    }

    #[test]
    fn splitting_the_tag_at_every_position_never_leaks_any_of_it() {
        let full = "Sure!<powershell-run elevate=\"true\">Get-Process | Select -First 2</powershell-run>";
        for split in 1..full.len() {
            if !full.is_char_boundary(split) {
                continue;
            }
            let (shown, filter) = run(&[&full[..split], &full[split..]]);
            assert_eq!(shown, "Sure!", "split at {split}");
            assert_eq!(filter.detected_call().unwrap().command, "Get-Process | Select -First 2", "split at {split}");
        }
    }

    #[test]
    fn one_character_at_a_time_works_too() {
        let full = "Ok <powershell-run>ls</powershell-run>";
        let chunks: Vec<String> = full.chars().map(String::from).collect();
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let (shown, filter) = run(&refs);
        assert_eq!(shown, "Ok ");
        assert_eq!(filter.detected_call().unwrap().command, "ls");
    }

    #[test]
    fn a_lookalike_prefix_is_held_back_only_until_it_is_disproved() {
        let mut filter = ToolCallFilter::new();
        assert_eq!(filter.feed("a <power"), "a ");
        assert_eq!(filter.feed("point slides"), "<powerpoint slides");
        assert!(filter.detected_call().is_none());
        assert_eq!(filter.feed(" <"), " ");
        assert_eq!(filter.feed("b>"), "<b>");
    }

    #[test]
    fn anything_after_the_tag_is_dropped() {
        let (shown, filter) = run(&["<powershell-run>x</powershell-run> and then I", " keep talking"]);
        assert_eq!(shown, "");
        assert!(filter.detected_call().is_some());
    }

    #[test]
    fn a_malformed_pair_is_shown_literally_and_streaming_continues() {
        let (shown, filter) = run(&["a <powershell-run evil=\"1\">x</powershell-run> b"]);
        assert_eq!(shown, "a <powershell-run evil=\"1\">x</powershell-run> b");
        assert!(filter.detected_call().is_none());

        // ...and a good tag after a bad one is still caught.
        let (shown, filter) = run(&["<powershell-run></powershell-run>then<powershell-run>ls</powershell-run>"]);
        assert_eq!(shown, "<powershell-run></powershell-run>then");
        assert_eq!(filter.detected_call().unwrap().command, "ls");
    }

    #[test]
    fn an_unfinished_tag_stays_hidden() {
        let (shown, filter) = run(&["Hi <powershell-run>Get-Da", "te"]);
        assert_eq!(shown, "Hi ");
        assert!(filter.detected_call().is_none());
    }

    #[test]
    fn multibyte_text_never_splits_a_character() {
        let (shown, _) = run(&["Grüße ", "🙂 <", "b>ok"]);
        assert_eq!(shown, "Grüße 🙂 <b>ok");
    }
}

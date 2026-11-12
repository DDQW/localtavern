//! Pulls `<think>...</think>` out of the visible text channel.
//!
//! Large 4 streams reasoning as typed content parts (parsed in `openai_compat`). Older Magistral
//! checkpoints, and some gateways in front of them, still wrap the trace in think tags inside the
//! same string as the answer. Tags routinely split across SSE deltas, so this has to remember
//! whether it is inside one.

use futures::{Stream, StreamExt};

use crate::providers::error::ProviderError;
use crate::providers::LlmResponseChunk;

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

#[derive(Debug, Default)]
struct ThinkSplitter {
    pending: String,
    in_think: bool,
}

impl ThinkSplitter {
    /// Returns `(visible reply, reasoning)` for this delta. A tag that is only partly arrived is
    /// held back so it is not shown as reply text.
    fn push(&mut self, text: &str) -> (String, String) {
        self.pending.push_str(text);
        self.drain(false)
    }

    /// End of the stream: whatever is still held becomes visible text, or reasoning if a `<think>`
    /// was never closed.
    fn finish(&mut self) -> (String, String) {
        self.drain(true)
    }

    fn drain(&mut self, flush: bool) -> (String, String) {
        let mut visible = String::new();
        let mut thought = String::new();
        loop {
            if self.in_think {
                if let Some(end) = self.pending.find(CLOSE) {
                    thought.push_str(&self.pending[..end]);
                    self.pending = self.pending[end + CLOSE.len()..].to_string();
                    self.in_think = false;
                    continue;
                }
                let hold = if flush {
                    0
                } else {
                    held_suffix(&self.pending, CLOSE)
                };
                let emit = self.pending.len() - hold;
                thought.push_str(&self.pending[..emit]);
                self.pending = self.pending[emit..].to_string();
                break;
            }
            if let Some(start) = self.pending.find(OPEN) {
                visible.push_str(&self.pending[..start]);
                self.pending = self.pending[start + OPEN.len()..].to_string();
                self.in_think = true;
                continue;
            }
            let hold = if flush {
                0
            } else {
                held_suffix(&self.pending, OPEN)
            };
            let emit = self.pending.len() - hold;
            visible.push_str(&self.pending[..emit]);
            self.pending = self.pending[emit..].to_string();
            break;
        }
        (visible, thought)
    }
}

/// How many trailing bytes of `buf` are a proper prefix of `tag`, and so might still grow into it.
fn held_suffix(buf: &str, tag: &str) -> usize {
    let max = (tag.len() - 1).min(buf.len());
    for len in (1..=max).rev() {
        let start = buf.len() - len;
        if buf.is_char_boundary(start) && tag.starts_with(&buf[start..]) {
            return len;
        }
    }
    0
}

pub fn separate_think_tags<S>(
    chunks: S,
) -> impl Stream<Item = Result<LlmResponseChunk, ProviderError>>
where
    S: Stream<Item = Result<LlmResponseChunk, ProviderError>>,
{
    async_stream::try_stream! {
        let mut splitter = ThinkSplitter::default();
        futures::pin_mut!(chunks);
        while let Some(chunk) = chunks.next().await {
            let mut chunk = chunk?;
            let (text, thought) = splitter.push(&chunk.delta_text);
            chunk.delta_text = text;
            chunk.thought_delta_text.push_str(&thought);
            yield chunk;
        }
        let (text, thought) = splitter.finish();
        if !text.is_empty() || !thought.is_empty() {
            yield LlmResponseChunk { delta_text: text, thought_delta_text: thought, ..Default::default() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_split_across_deltas_becomes_reasoning() {
        let mut splitter = ThinkSplitter::default();
        let (visible, thought) = splitter.push("Hi <thi");
        assert_eq!(visible, "Hi ");
        assert_eq!(thought, "");
        let (visible, thought) = splitter.push("nk>because</think> there");
        assert_eq!(visible, " there");
        assert_eq!(thought, "because");
        assert_eq!(splitter.finish(), (String::new(), String::new()));
    }

    #[test]
    fn an_unclosed_tag_is_reasoning_not_the_reply() {
        let mut splitter = ThinkSplitter::default();
        assert_eq!(
            splitter.push("<think>still thinking"),
            (String::new(), "still thinking".into())
        );
        assert_eq!(splitter.finish(), (String::new(), String::new()));
    }

    #[test]
    fn a_partial_tag_that_never_finishes_is_shown_as_text() {
        let mut splitter = ThinkSplitter::default();
        // '<' could still become `<think>`, so it is held. The rest of "1 < 2" cannot.
        assert_eq!(splitter.push("1 <"), ("1 ".into(), String::new()));
        assert_eq!(splitter.finish(), ("<".into(), String::new()));
        assert_eq!(
            ThinkSplitter::default().push("1 < 2"),
            ("1 < 2".into(), String::new())
        );
    }
}

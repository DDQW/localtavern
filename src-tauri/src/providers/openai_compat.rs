//! The OpenAI-style chat-completions SSE stream, shared by every provider that speaks it (DeepSeek,
//! xAI). Each `data: {json}` event carries one incremental delta; the stream ends with a literal
//! `data: [DONE]` event, which carries no JSON and is simply the exit signal.

use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde::Deserialize;

use super::error::ProviderError;
use super::http::lines;
use super::LlmResponseChunk;
use crate::models::LlmUsage;

/// `reasoning_content` is a reasoning model's chain-of-thought, streamed separately from `content`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Delta {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Choice {
    pub delta: Option<Delta>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
}

/// One `data:` SSE event. With `stream_options.include_usage` set, an extra trailing chunk arrives
/// after the one carrying `finish_reason` - it has an empty `choices` array and only `usage`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChatCompletionChunk {
    pub choices: Option<Vec<Choice>>,
    pub usage: Option<Usage>,
}

/// `provider_name` only shapes the parse-error message ("Failed to parse DeepSeek stream chunk").
pub fn read_chunks<S, E>(
    bytes: S,
    provider_name: &'static str,
) -> impl Stream<Item = Result<LlmResponseChunk, ProviderError>>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: std::fmt::Display,
{
    async_stream::try_stream! {
        let lines = lines(bytes);
        futures::pin_mut!(lines);

        while let Some(line) = lines.next().await {
            let line = line?;
            let Some(payload) = line.strip_prefix("data:") else { continue };
            let payload = payload.trim();
            if payload.is_empty() {
                continue;
            }
            if payload == "[DONE]" {
                break;
            }

            let chunk: ChatCompletionChunk = serde_json::from_str(payload)
                .map_err(|e| ProviderError::unknown(format!("Failed to parse {provider_name} stream chunk: {e}")))?;

            let usage = chunk.usage.map(|u| LlmUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            });

            let Some(choice) = chunk.choices.and_then(|c| c.into_iter().next()) else {
                // The trailing usage-only chunk has no choice at all - still worth surfacing so the
                // caller picks up the final token counts, just with no text.
                if usage.is_some() {
                    yield LlmResponseChunk { usage, ..Default::default() };
                }
                continue;
            };

            let (text, thought) = match choice.delta {
                Some(d) => (d.content.unwrap_or_default(), d.reasoning_content.unwrap_or_default()),
                None => Default::default(),
            };

            yield LlmResponseChunk {
                delta_text: text,
                is_final: choice.finish_reason.is_some(),
                usage,
                finish_reason: choice.finish_reason,
                thought_delta_text: thought,
                images: None,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::error::ProviderErrorKind;
    use crate::providers::http::testing::chunked;

    async fn collect(sse: &str, chunk_size: usize) -> Result<Vec<LlmResponseChunk>, ProviderError> {
        read_chunks(chunked(sse, chunk_size), "Test").collect::<Vec<_>>().await.into_iter().collect()
    }

    const FULL: &str = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"lo!\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}\n\n",
        "data: [DONE]\n\n",
    );

    #[tokio::test]
    async fn parses_multiple_sse_events_into_ordered_chunks() {
        let chunks = collect(FULL, 4096).await.unwrap();

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].delta_text, "Hel");
        assert!(!chunks[0].is_final);
        assert!(chunks[0].usage.is_none());

        assert_eq!(chunks[1].delta_text, "lo!");
        assert!(chunks[1].is_final);
        assert_eq!(chunks[1].finish_reason.as_deref(), Some("stop"));
        assert!(chunks[1].usage.is_none());

        assert_eq!(chunks[2].delta_text, "");
        assert!(!chunks[2].is_final);
        let usage = chunks[2].usage.unwrap();
        assert_eq!((usage.prompt_tokens, usage.completion_tokens, usage.total_tokens), (Some(5), Some(2), Some(7)));
    }

    #[tokio::test]
    async fn is_indifferent_to_how_the_network_splits_the_bytes() {
        let whole = collect(FULL, 4096).await.unwrap();
        for size in [1, 2, 5, 11, 64] {
            assert_eq!(collect(FULL, size).await.unwrap(), whole, "chunk size {size}");
        }
    }

    #[tokio::test]
    async fn stops_at_the_done_marker_without_emitting_a_chunk_for_it() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: [DONE]\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"should never be seen\"}}]}\n\n",
        );
        let chunks = collect(sse, 4096).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta_text, "hi");
    }

    #[tokio::test]
    async fn separates_reasoning_content_from_the_visible_reply() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi there!\",\"reasoning_content\":\"Let me think...\"},\"finish_reason\":\"stop\"}]}\n\n";
        let chunks = collect(sse, 4096).await.unwrap();
        assert_eq!(chunks[0].delta_text, "Hi there!");
        assert_eq!(chunks[0].thought_delta_text, "Let me think...");
    }

    #[tokio::test]
    async fn a_reasoning_only_delta_and_an_empty_finishing_delta_both_parse() {
        // Shape xAI streams: reasoning first, then content, then an empty delta carrying the reason.
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hmm\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        );
        let chunks = collect(sse, 4096).await.unwrap();
        assert_eq!(chunks[0].thought_delta_text, "hmm");
        assert_eq!(chunks[0].delta_text, "");
        assert_eq!(chunks[1].delta_text, "Hello");
        assert_eq!(chunks[2].finish_reason.as_deref(), Some("stop"));
        assert!(chunks[2].is_final);
    }

    #[tokio::test]
    async fn ignores_non_data_lines_and_blank_lines() {
        let sse = "event: message\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n";
        let chunks = collect(sse, 4096).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta_text, "hi");
    }

    #[tokio::test]
    async fn a_delta_with_null_content_yields_an_empty_chunk() {
        let sse = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":null}}]}\n\n";
        assert_eq!(collect(sse, 4096).await.unwrap()[0].delta_text, "");
    }

    #[tokio::test]
    async fn malformed_json_is_a_provider_error_naming_the_provider() {
        let err = collect("data: {not valid json\n\n", 4096).await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Unknown);
        assert!(err.message.starts_with("Failed to parse Test stream chunk"), "{}", err.message);
    }
}

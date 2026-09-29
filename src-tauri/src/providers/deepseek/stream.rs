//! Parses a DeepSeek (OpenAI-compatible) chat-completions SSE stream. Each `data: {json}` event
//! carries one incremental delta; the stream ends with a literal `data: [DONE]` event, which carries
//! no JSON and is simply the exit signal.

use bytes::Bytes;
use futures::{Stream, StreamExt};

use super::wire::DeepSeekChatCompletionChunk;
use crate::models::LlmUsage;
use crate::providers::error::ProviderError;
use crate::providers::http::lines;
use crate::providers::LlmResponseChunk;

pub fn read_chunks<S, E>(bytes: S) -> impl Stream<Item = Result<LlmResponseChunk, ProviderError>>
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

            let chunk: DeepSeekChatCompletionChunk = serde_json::from_str(payload)
                .map_err(|e| ProviderError::unknown(format!("Failed to parse DeepSeek stream chunk: {e}")))?;

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
        read_chunks(chunked(sse, chunk_size)).collect::<Vec<_>>().await.into_iter().collect()
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
    async fn ignores_non_data_lines_and_blank_lines() {
        let sse = "event: message\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n";
        let chunks = collect(sse, 4096).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta_text, "hi");
    }

    #[tokio::test]
    async fn a_delta_with_null_content_yields_an_empty_chunk() {
        let sse = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":null}}]}\n\n";
        let chunks = collect(sse, 4096).await.unwrap();
        assert_eq!(chunks[0].delta_text, "");
    }

    #[tokio::test]
    async fn malformed_json_is_a_provider_error() {
        let err = collect("data: {not valid json\n\n", 4096).await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Unknown);
    }
}

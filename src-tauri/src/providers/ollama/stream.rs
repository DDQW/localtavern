//! Parses Ollama's `/api/chat` newline-delimited JSON stream. Unlike the SSE providers, every line
//! (not just ones prefixed `data:`) is its own complete JSON object, and the stream simply ends
//! after the object with `"done":true`.

use bytes::Bytes;
use futures::{Stream, StreamExt};

use super::wire::OllamaChatStreamEvent;
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
            if line.is_empty() {
                continue;
            }

            let event: OllamaChatStreamEvent = serde_json::from_str(&line)
                .map_err(|e| ProviderError::unknown(format!("Failed to parse Ollama stream chunk: {e}")))?;

            let (text, thought) = match event.message {
                Some(m) => (m.content.unwrap_or_default(), m.thinking.unwrap_or_default()),
                None => Default::default(),
            };

            let usage = event.done.then(|| LlmUsage {
                prompt_tokens: event.prompt_eval_count,
                completion_tokens: event.eval_count,
                total_tokens: event.prompt_eval_count.zip(event.eval_count).map(|(p, c)| p + c),
            });

            yield LlmResponseChunk {
                delta_text: text,
                is_final: event.done,
                usage,
                finish_reason: event.done.then(|| event.done_reason.clone().unwrap_or_else(|| "stop".to_string())),
                thought_delta_text: thought,
                images: None,
            };

            if event.done {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::error::ProviderErrorKind;
    use crate::providers::http::testing::chunked;

    async fn collect(ndjson: &str, chunk_size: usize) -> Result<Vec<LlmResponseChunk>, ProviderError> {
        read_chunks(chunked(ndjson, chunk_size)).collect::<Vec<_>>().await.into_iter().collect()
    }

    const FULL: &str = concat!(
        "{\"message\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"done\":false}\n",
        "{\"message\":{\"role\":\"assistant\",\"content\":\"lo!\"},\"done\":false}\n",
        "{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":5,\"eval_count\":2}\n",
    );

    #[tokio::test]
    async fn parses_multiple_ndjson_lines_into_ordered_chunks() {
        let chunks = collect(FULL, 4096).await.unwrap();

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].delta_text, "Hel");
        assert!(!chunks[0].is_final);
        assert!(chunks[0].usage.is_none());
        assert_eq!(chunks[1].delta_text, "lo!");
        assert!(!chunks[1].is_final);

        assert!(chunks[2].is_final);
        assert_eq!(chunks[2].finish_reason.as_deref(), Some("stop"));
        let usage = chunks[2].usage.unwrap();
        assert_eq!((usage.prompt_tokens, usage.completion_tokens, usage.total_tokens), (Some(5), Some(2), Some(7)));
    }

    #[tokio::test]
    async fn is_indifferent_to_how_the_network_splits_the_bytes() {
        let whole = collect(FULL, 4096).await.unwrap();
        for size in [1, 3, 10, 50] {
            assert_eq!(collect(FULL, size).await.unwrap(), whole, "chunk size {size}");
        }
    }

    #[tokio::test]
    async fn stops_after_done_true_without_reading_further_lines() {
        let ndjson = concat!(
            "{\"message\":{\"content\":\"hi\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\"}\n",
            "{\"message\":{\"content\":\"should never be seen\"},\"done\":false}\n",
        );
        let chunks = collect(ndjson, 4096).await.unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].delta_text, "hi");
        assert!(chunks[1].is_final);
    }

    #[tokio::test]
    async fn done_without_a_reason_defaults_to_stop_and_missing_counts_leave_total_unknown() {
        let chunks = collect("{\"message\":{\"content\":\"\"},\"done\":true}\n", 4096).await.unwrap();
        assert_eq!(chunks[0].finish_reason.as_deref(), Some("stop"));
        assert_eq!(chunks[0].usage.unwrap().total_tokens, None);
    }

    #[tokio::test]
    async fn separates_the_thinking_trace_from_the_visible_reply() {
        let chunks = collect("{\"message\":{\"content\":\"42\",\"thinking\":\"Let me compute...\"},\"done\":false}\n", 4096)
            .await
            .unwrap();
        assert_eq!(chunks[0].delta_text, "42");
        assert_eq!(chunks[0].thought_delta_text, "Let me compute...");
    }

    #[tokio::test]
    async fn ignores_blank_lines() {
        let chunks = collect("\n{\"message\":{\"content\":\"hi\"},\"done\":false}\n\n", 4096).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta_text, "hi");
    }

    #[tokio::test]
    async fn malformed_json_is_a_provider_error() {
        let err = collect("{not valid json\n", 4096).await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Unknown);
        assert!(err.message.contains("Failed to parse Ollama stream chunk"));
    }
}

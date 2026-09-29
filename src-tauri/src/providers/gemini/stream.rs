//! Parses a Gemini `alt=sse` response body into response chunks. Each SSE event
//! (`data: {json}`) carries one incremental delta.

use base64::Engine;
use bytes::Bytes;
use futures::{Stream, StreamExt};

use super::wire::GeminiGenerateContentResponse;
use crate::models::LlmUsage;
use crate::providers::error::ProviderError;
use crate::providers::http::lines;
use crate::providers::{LlmGeneratedImage, LlmResponseChunk};

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

            let response: GeminiGenerateContentResponse = serde_json::from_str(payload)
                .map_err(|e| ProviderError::unknown(format!("Failed to parse Gemini stream chunk: {e}")))?;

            let candidate = response.candidates.as_ref().and_then(|c| c.first());
            let parts = candidate
                .and_then(|c| c.content.as_ref())
                .and_then(|c| c.parts.as_deref())
                .unwrap_or_default();

            let text: String = parts
                .iter()
                .filter(|p| p.thought != Some(true))
                .filter_map(|p| p.text.as_deref())
                .collect();
            let thought_text: String = parts
                .iter()
                .filter(|p| p.thought == Some(true))
                .filter_map(|p| p.text.as_deref())
                .collect();

            let mut images = Vec::new();
            for inline in parts.iter().filter_map(|p| p.inline_data.as_ref()) {
                let data = base64::engine::general_purpose::STANDARD
                    .decode(&inline.data)
                    .map_err(|e| ProviderError::unknown(format!("Gemini returned an invalid image payload: {e}")))?;
                images.push(LlmGeneratedImage { data, mime_type: inline.mime_type.clone() });
            }

            // A prompt-level block (the request never even reached candidate generation) carries no
            // candidate/finishReason at all - promptFeedback.blockReason is the only signal. Prefixed
            // distinctly from a candidate's own finishReason so callers can tell the two apart.
            let finish_reason = match response.prompt_feedback.as_ref().and_then(|f| f.block_reason.as_ref()) {
                Some(reason) => Some(format!("PROMPT_BLOCKED:{reason}")),
                None => candidate.and_then(|c| c.finish_reason.clone()),
            };

            let usage = response.usage_metadata.map(|m| LlmUsage {
                prompt_tokens: m.prompt_token_count,
                completion_tokens: m.candidates_token_count,
                total_tokens: m.total_token_count,
            });

            yield LlmResponseChunk {
                delta_text: text,
                is_final: finish_reason.is_some(),
                usage,
                finish_reason,
                thought_delta_text: thought_text,
                images: if images.is_empty() { None } else { Some(images) },
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

    const TWO_EVENTS: &str = concat!(
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hel\"}],\"role\":\"model\"}}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"lo!\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],",
        "\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":2,\"totalTokenCount\":7}}\n\n",
    );

    #[tokio::test]
    async fn parses_multiple_sse_events_into_ordered_chunks() {
        let chunks = collect(TWO_EVENTS, 4096).await.unwrap();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].delta_text, "Hel");
        assert!(!chunks[0].is_final);
        assert!(chunks[0].usage.is_none());

        assert_eq!(chunks[1].delta_text, "lo!");
        assert!(chunks[1].is_final);
        assert_eq!(chunks[1].finish_reason.as_deref(), Some("STOP"));
        let usage = chunks[1].usage.unwrap();
        assert_eq!((usage.prompt_tokens, usage.completion_tokens, usage.total_tokens), (Some(5), Some(2), Some(7)));
    }

    #[tokio::test]
    async fn is_indifferent_to_how_the_network_splits_the_bytes() {
        let whole = collect(TWO_EVENTS, 4096).await.unwrap();
        for size in [1, 2, 3, 7, 13, 64] {
            assert_eq!(collect(TWO_EVENTS, size).await.unwrap(), whole, "chunk size {size}");
        }
    }

    #[tokio::test]
    async fn ignores_non_data_lines_and_blank_lines() {
        let sse = "event: message\n\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}],\"role\":\"model\"}}]}\n\n";
        let chunks = collect(sse, 4096).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta_text, "hi");
    }

    #[tokio::test]
    async fn separates_thought_parts_from_the_visible_reply() {
        let sse = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Let me think...\",\"thought\":true},{\"text\":\"Hi there!\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n";
        let chunks = collect(sse, 4096).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta_text, "Hi there!");
        assert_eq!(chunks[0].thought_delta_text, "Let me think...");
    }

    #[tokio::test]
    async fn thought_text_is_empty_when_no_part_is_flagged() {
        let sse = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}],\"role\":\"model\"}}]}\n\n";
        assert_eq!(collect(sse, 4096).await.unwrap()[0].thought_delta_text, "");
    }

    #[tokio::test]
    async fn parses_inline_data_parts_as_images() {
        let b64 = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]);
        let sse = format!(
            "data: {{\"candidates\":[{{\"content\":{{\"parts\":[{{\"inlineData\":{{\"mimeType\":\"image/png\",\"data\":\"{b64}\"}}}}],\"role\":\"model\"}},\"finishReason\":\"STOP\"}}]}}\n\n"
        );
        let chunks = collect(&sse, 4096).await.unwrap();
        let images = chunks[0].images.as_ref().unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].mime_type, "image/png");
        assert_eq!(images[0].data, [1, 2, 3]);
    }

    #[tokio::test]
    async fn images_are_none_when_no_part_carries_inline_data() {
        let sse = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}],\"role\":\"model\"}}]}\n\n";
        assert!(collect(sse, 4096).await.unwrap()[0].images.is_none());
    }

    #[tokio::test]
    async fn a_prompt_level_block_is_reported_as_a_distinct_finish_reason() {
        let sse = "data: {\"promptFeedback\":{\"blockReason\":\"SAFETY\"}}\n\n";
        let chunks = collect(sse, 4096).await.unwrap();
        assert_eq!(chunks[0].finish_reason.as_deref(), Some("PROMPT_BLOCKED:SAFETY"));
        assert!(chunks[0].is_final);
    }

    #[tokio::test]
    async fn malformed_json_is_a_provider_error() {
        let err = collect("data: {not valid json\n\n", 4096).await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Unknown);
        assert!(err.message.contains("Failed to parse Gemini stream chunk"));
    }

    #[tokio::test]
    async fn invalid_image_base64_is_a_provider_error() {
        let sse = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"inlineData\":{\"mimeType\":\"image/png\",\"data\":\"!!!\"}}]}}]}\n\n";
        assert!(collect(sse, 4096).await.is_err());
    }
}

//! Parses a Responses-API SSE stream. Every `data:` line is a JSON event with a `type`; only text
//! deltas, reasoning-summary deltas and the terminal events matter. Text and reasoning arrive as
//! separate chunks; the terminal event yields the finish reason and usage.

use bytes::Bytes;
use futures::{Stream, StreamExt};

use super::wire::{ResponsesEvent, ResponsesSummary};
use crate::models::LlmUsage;
use crate::providers::error::{ProviderError, ProviderErrorKind};
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

            let event: ResponsesEvent = serde_json::from_str(payload)
                .map_err(|e| ProviderError::unknown(format!("Failed to parse ChatGPT stream event: {e}")))?;

            match event.kind.as_deref() {
                Some("response.output_text.delta" | "response.refusal.delta") => {
                    if let Some(text) = delta_text(&event).filter(|t| !t.is_empty()) {
                        yield LlmResponseChunk::text(text);
                    }
                }
                Some("response.reasoning_summary_text.delta" | "response.reasoning_text.delta" | "response.reasoning.delta") => {
                    if let Some(thought) = delta_text(&event).filter(|t| !t.is_empty()) {
                        yield LlmResponseChunk { thought_delta_text: thought, ..Default::default() };
                    }
                }
                Some("response.completed") => {
                    yield terminal(event.response.as_ref(), "stop".to_string());
                    break;
                }
                Some("response.incomplete") => {
                    let reason = event.response.as_ref().and_then(|r| r.incomplete_details.as_ref()).and_then(|d| d.reason.as_deref());
                    yield terminal(event.response.as_ref(), incomplete_finish_reason(reason));
                    break;
                }
                Some("response.failed") => {
                    let error = event.response.as_ref().and_then(|r| r.error.as_ref());
                    Err(to_error(error.and_then(|e| e.message.as_deref()), error.and_then(|e| e.code.as_deref())))?;
                }
                Some("response.error") => {
                    let error = event.error.as_ref();
                    Err(to_error(error.and_then(|e| e.message.as_deref()), error.and_then(|e| e.code.as_deref())))?;
                }
                Some("error") => {
                    let error = event.error.as_ref();
                    let message = event.message.as_deref().or(error.and_then(|e| e.message.as_deref()));
                    let code = event.code.as_deref().or(error.and_then(|e| e.code.as_deref()));
                    Err(to_error(message, code))?;
                }
                _ => {}
            }
        }
    }
}

fn delta_text(event: &ResponsesEvent) -> Option<String> {
    event.delta.as_ref()?.as_str().map(String::from)
}

fn terminal(response: Option<&ResponsesSummary>, finish_reason: String) -> LlmResponseChunk {
    LlmResponseChunk {
        is_final: true,
        usage: response.and_then(|r| r.usage.as_ref()).map(|u| LlmUsage {
            prompt_tokens: u.input_tokens,
            completion_tokens: u.output_tokens,
            total_tokens: u.total_tokens,
        }),
        finish_reason: Some(finish_reason),
        ..Default::default()
    }
}

/// Maps the Responses API's incomplete reasons onto the finish-reason vocabulary the rest of the app
/// already understands.
fn incomplete_finish_reason(reason: Option<&str>) -> String {
    match reason {
        Some("max_output_tokens") | None | Some("") => "length".to_string(),
        Some(other) => other.to_string(),
    }
}

fn to_error(message: Option<&str>, code: Option<&str>) -> ProviderError {
    let text = message.filter(|m| !m.is_empty()).unwrap_or("ChatGPT reported an error while generating.");
    let kind = match code {
        Some("rate_limit_exceeded" | "usage_limit_reached" | "insufficient_quota") => ProviderErrorKind::RateLimit,
        Some("invalid_prompt" | "invalid_request_error" | "context_length_exceeded") => ProviderErrorKind::InvalidRequest,
        _ => ProviderErrorKind::Unknown,
    };
    ProviderError::new(text, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::http::testing::chunked;

    async fn read(sse: &str, chunk_size: usize) -> Result<Vec<LlmResponseChunk>, ProviderError> {
        read_chunks(chunked(sse, chunk_size)).collect::<Vec<_>>().await.into_iter().collect()
    }

    const FULL: &str = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"status\":\"in_progress\"}}\n\n",
        "event: response.reasoning_summary_text.delta\n",
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"r1\",\"summary_index\":0,\"delta\":\"thinking\"}\n\n",
        "event: response.output_text.delta\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"m1\",\"delta\":\"Hel\"}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"m1\",\"delta\":\"lo\"}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":{\"weird\":true}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":4,\"output_tokens\":2,\"total_tokens\":6}}}",
    );

    #[tokio::test]
    async fn parses_text_reasoning_and_completion() {
        let chunks = read(FULL, 4096).await.unwrap();

        assert_eq!(chunks.len(), 4); // the non-string-delta event was ignored, not a parse failure
        assert_eq!(chunks[0].thought_delta_text, "thinking");
        assert_eq!(chunks[1].delta_text, "Hel");
        assert_eq!(chunks[2].delta_text, "lo");
        let last = &chunks[3];
        assert!(last.is_final);
        assert_eq!(last.finish_reason.as_deref(), Some("stop"));
        let usage = last.usage.unwrap();
        assert_eq!((usage.prompt_tokens, usage.completion_tokens, usage.total_tokens), (Some(4), Some(2), Some(6)));
    }

    #[tokio::test]
    async fn is_indifferent_to_how_the_network_splits_the_bytes() {
        let whole = read(FULL, 4096).await.unwrap();
        for size in [1, 3, 17, 100] {
            assert_eq!(read(FULL, size).await.unwrap(), whole, "chunk size {size}");
        }
    }

    #[tokio::test]
    async fn maps_incomplete_reasons() {
        for (reason, expected) in [("max_output_tokens", "length"), ("content_filter", "content_filter"), ("other", "other")] {
            let sse = format!("data: {{\"type\":\"response.incomplete\",\"response\":{{\"incomplete_details\":{{\"reason\":\"{reason}\"}}}}}}");
            let chunks = read(&sse, 4096).await.unwrap();
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].finish_reason.as_deref(), Some(expected), "{reason}");
        }
        let chunks = read("data: {\"type\":\"response.incomplete\",\"response\":{}}", 4096).await.unwrap();
        assert_eq!(chunks[0].finish_reason.as_deref(), Some("length"));
    }

    #[tokio::test]
    async fn stops_after_the_terminal_event() {
        let sse = concat!(
            "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"never\"}\n\n",
        );
        assert_eq!(read(sse, 4096).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_response_is_a_provider_error() {
        let err = read(
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"boom\",\"code\":\"rate_limit_exceeded\"}}}",
            4096,
        )
        .await
        .unwrap_err();
        assert_eq!(err.message, "boom");
        assert_eq!(err.kind, ProviderErrorKind::RateLimit);
    }

    #[tokio::test]
    async fn a_generic_error_event_is_a_provider_error() {
        let err = read("data: {\"type\":\"error\",\"code\":\"invalid_prompt\",\"message\":\"bad prompt\"}", 4096)
            .await
            .unwrap_err();
        assert_eq!(err.message, "bad prompt");
        assert_eq!(err.kind, ProviderErrorKind::InvalidRequest);
    }

    #[tokio::test]
    async fn error_events_fall_back_to_a_generic_message() {
        let err = read("data: {\"type\":\"response.error\",\"error\":{}}", 4096).await.unwrap_err();
        assert_eq!(err.message, "ChatGPT reported an error while generating.");
        assert_eq!(err.kind, ProviderErrorKind::Unknown);
    }

    #[tokio::test]
    async fn chunks_delivered_before_an_error_still_arrive() {
        let sse = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
            "data: {\"type\":\"error\",\"message\":\"cut off\"}\n\n",
        );
        let items: Vec<_> = read_chunks(chunked(sse, 4096)).collect().await;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].as_ref().unwrap().delta_text, "partial");
        assert!(items[1].is_err());
    }

    #[tokio::test]
    async fn malformed_json_is_a_provider_error() {
        let err = read("data: {nope\n\n", 4096).await.unwrap_err();
        assert!(err.message.contains("Failed to parse ChatGPT stream event"));
    }
}

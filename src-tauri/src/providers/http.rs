//! HTTP plumbing shared by every provider: client construction, error mapping for transport
//! failures, and the byte-safe line splitter that SSE and NDJSON readers sit on.

use std::fmt::Display;
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::{RequestBuilder, Response, StatusCode};

use super::error::ProviderError;

/// A client suited to long-lived streaming replies: no overall timeout (a thinking model can stream
/// for minutes) but a bounded connect phase and a generous per-read stall limit.
pub fn build_client() -> reqwest::Client {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(180))
        .user_agent(concat!("LocalTavern/", env!("CARGO_PKG_VERSION")));
    // The platform verifier needs a JVM-side component on Android; the Mozilla roots do without.
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .filter_map(|der| reqwest::Certificate::from_der(der).ok()),
    );
    builder
        .build()
        .expect("reqwest client configuration is static and valid")
}

/// Sends a request, mapping transport failures to `Network` errors named after the provider.
pub async fn send(request: RequestBuilder, provider_name: &str) -> Result<Response, ProviderError> {
    request.send().await.map_err(|e| transport_error(provider_name, &e))
}

pub fn transport_error(provider_name: &str, e: &reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::network(format!("{provider_name} request timed out: {e}"))
    } else {
        ProviderError::network(format!("Network error calling {provider_name}: {e}"))
    }
}

/// Returns the response unchanged on success; otherwise reads the body and hands it to `map_error`.
pub async fn ensure_success(
    response: Response,
    map_error: impl FnOnce(StatusCode, &str) -> ProviderError,
) -> Result<Response, ProviderError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(map_error(status, &body))
}

/// "429 Too Many Requests"-style text for generic error messages.
pub fn describe_status(status: StatusCode) -> String {
    match status.canonical_reason() {
        Some(reason) => format!("{} {reason}", status.as_u16()),
        None => status.as_u16().to_string(),
    }
}

/// Splits a byte stream into lines on `\n` (dropping a trailing `\r`). Works on raw bytes so a
/// multi-byte UTF-8 character split across two network chunks is reassembled before decoding.
/// A final line without a trailing newline is still emitted. Transport errors mid-stream become
/// `Network` errors.
pub fn lines<S, E>(bytes: S) -> impl Stream<Item = Result<String, ProviderError>>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Display,
{
    async_stream::try_stream! {
        futures::pin_mut!(bytes);
        let mut buffer: Vec<u8> = Vec::new();

        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|e| ProviderError::network(format!("Connection lost while streaming: {e}")))?;
            buffer.extend_from_slice(&chunk);

            while let Some(newline) = buffer.iter().position(|b| *b == b'\n') {
                let mut line: Vec<u8> = buffer.drain(..=newline).collect();
                line.pop(); // '\n'
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                yield String::from_utf8_lossy(&line).into_owned();
            }
        }

        if !buffer.is_empty() {
            yield String::from_utf8_lossy(&buffer).into_owned();
        }
    }
}

/// Helpers for tests that need to feed readers a byte stream in awkward pieces.
#[cfg(test)]
pub mod testing {
    use std::convert::Infallible;

    use bytes::Bytes;
    use futures::Stream;

    /// Delivers `text` in chunks of at most `chunk_size` bytes, so tests exercise lines that are
    /// split across network reads (including in the middle of a UTF-8 character).
    pub fn chunked(text: &str, chunk_size: usize) -> impl Stream<Item = Result<Bytes, Infallible>> {
        let pieces: Vec<Result<Bytes, Infallible>> =
            text.as_bytes().chunks(chunk_size).map(|c| Ok(Bytes::copy_from_slice(c))).collect();
        futures::stream::iter(pieces)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::chunked;
    use super::*;

    async fn collect_lines(text: &str, chunk_size: usize) -> Vec<String> {
        lines(chunked(text, chunk_size)).map(|l| l.unwrap()).collect().await
    }

    #[tokio::test]
    async fn splits_on_newlines_and_strips_carriage_returns() {
        assert_eq!(collect_lines("a\r\nb\nc\n", 64).await, ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn emits_a_final_line_without_a_trailing_newline() {
        assert_eq!(collect_lines("a\nb", 64).await, ["a", "b"]);
    }

    #[tokio::test]
    async fn keeps_blank_lines() {
        assert_eq!(collect_lines("a\n\nb\n", 64).await, ["a", "", "b"]);
    }

    #[tokio::test]
    async fn reassembles_lines_split_across_chunks_of_any_size() {
        let text = "data: {\"x\":1}\n\ndata: [DONE]\n";
        for size in 1..=text.len() {
            assert_eq!(collect_lines(text, size).await, ["data: {\"x\":1}", "", "data: [DONE]"], "size {size}");
        }
    }

    #[tokio::test]
    async fn reassembles_multibyte_characters_split_across_chunks() {
        // "é" is 2 bytes, "日" is 3, the emoji is 4 - chunk size 1 splits every one of them.
        let text = "café 日本 😀\nnext\n";
        assert_eq!(collect_lines(text, 1).await, ["café 日本 😀", "next"]);
    }

    #[tokio::test]
    async fn transport_errors_become_network_errors() {
        let failing = futures::stream::iter(vec![
            Ok(Bytes::from_static(b"partial")),
            Err("connection reset"),
        ]);
        let results: Vec<_> = lines(failing).collect().await;
        let err = results.last().unwrap().as_ref().unwrap_err();
        assert_eq!(err.kind, super::super::error::ProviderErrorKind::Network);
        assert!(err.message.contains("connection reset"));
    }

    #[test]
    fn describe_status_includes_the_reason_phrase() {
        assert_eq!(describe_status(StatusCode::TOO_MANY_REQUESTS), "429 Too Many Requests");
    }
}

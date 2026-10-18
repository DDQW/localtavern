use reqwest::StatusCode;

use super::wire::OllamaErrorResponse;
use crate::providers::error::{ProviderError, ProviderErrorKind};
use crate::providers::http::describe_status;

pub fn to_provider_error(status: StatusCode, body: &str) -> ProviderError {
    let mut message = format!("Ollama request failed with status {}.", describe_status(status));

    // A body that isn't JSON just falls back to the generic message.
    if let Ok(parsed) = serde_json::from_str::<OllamaErrorResponse>(body) {
        if let Some(error) = parsed.error.filter(|e| !e.is_empty()) {
            message = error;
        }
    }

    let kind = match status {
        StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST => ProviderErrorKind::InvalidRequest,
        _ => ProviderErrorKind::Unknown,
    };
    ProviderError::with_status(message, kind, status.as_u16())
}

/// Ollama's 400 for a `think` value sent to a model without the thinking capability:
/// `"<model>" does not support thinking`.
pub fn is_thinking_unsupported(error: &ProviderError) -> bool {
    error.http_status == Some(400) && error.message.contains("does not support thinking")
}

/// Ollama has no auth concept, so the one error worth calling out specially is "server not
/// reachable" - a stopped or never-started local server, which is the common case rather than an
/// edge case.
pub fn connection_failed(base_url: &str) -> ProviderError {
    ProviderError::network(format!(
        "Couldn't reach Ollama at {base_url}. Is it running? (Start it with \"ollama serve\" or the Ollama app.)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_and_bad_request_are_invalid_requests() {
        let e = to_provider_error(StatusCode::NOT_FOUND, r#"{"error":"model 'ghost' not found"}"#);
        assert_eq!(e.kind, ProviderErrorKind::InvalidRequest);
        assert_eq!(e.message, "model 'ghost' not found");
        assert_eq!(to_provider_error(StatusCode::BAD_REQUEST, "").kind, ProviderErrorKind::InvalidRequest);
    }

    #[test]
    fn other_statuses_are_unknown_with_a_generic_message() {
        let e = to_provider_error(StatusCode::INTERNAL_SERVER_ERROR, "boom");
        assert_eq!(e.kind, ProviderErrorKind::Unknown);
        assert_eq!(e.message, "Ollama request failed with status 500 Internal Server Error.");
    }

    #[test]
    fn connection_failures_name_the_url_and_suggest_a_fix() {
        let e = connection_failed("http://localhost:11434");
        assert_eq!(e.kind, ProviderErrorKind::Network);
        assert!(e.message.contains("localhost:11434"));
        assert!(e.message.contains("ollama serve"));
    }
}

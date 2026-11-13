use reqwest::StatusCode;

use crate::providers::error::{ProviderError, ProviderErrorKind};
use crate::providers::http::describe_status;

/// oMLX is a FastAPI server: validation failures are `{"detail": ...}`, and it also speaks the
/// OpenAI `{"error": {"message": ...}}` shape.
fn message_in(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if let Some(message) = value.pointer("/error/message").and_then(|m| m.as_str()) {
        return Some(message.to_string());
    }
    if let Some(message) = value.get("message").and_then(|m| m.as_str()) {
        return Some(message.to_string());
    }
    match value.get("detail")? {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(items) => {
            let msgs: Vec<_> = items
                .iter()
                .filter_map(|i| i.get("msg").and_then(|m| m.as_str()))
                .collect();
            (!msgs.is_empty()).then(|| msgs.join("; "))
        }
        _ => None,
    }
}

pub fn to_provider_error(status: StatusCode, body: &str) -> ProviderError {
    let message = message_in(body).unwrap_or_else(|| {
        format!(
            "oMLX request failed with status {}.",
            describe_status(status)
        )
    });
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderErrorKind::Auth,
        StatusCode::TOO_MANY_REQUESTS => ProviderErrorKind::RateLimit,
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            ProviderErrorKind::InvalidRequest
        }
        _ => ProviderErrorKind::Unknown,
    };
    ProviderError::with_status(message, kind, status.as_u16())
}

/// What to do when a model rejects the thinking controls we sent.
#[derive(Debug, PartialEq, Eq)]
pub enum ThinkRetry {
    /// The template named the levels it accepts. Send this one instead.
    Level(String),
    /// The model has no thinking switch. Ask again with neither field.
    WithoutThinking,
}

/// A 400/422 that is about `reasoning_effort` or `enable_thinking`. Auth and "model not found"
/// are not this.
pub fn think_retry(error: &ProviderError) -> Option<ThinkRetry> {
    if error.kind != ProviderErrorKind::InvalidRequest {
        return None;
    }
    let msg = error.message.to_ascii_lowercase();
    if !msg.contains("reasoning") && !msg.contains("thinking") {
        return None;
    }
    // "Unexpected reasoning effort high. Supported types are xhigh (default), medium, and low."
    if let Some((_, tail)) = error.message.split_once("Supported types are ") {
        let list = tail.split(['.', '\n']).next().unwrap_or_default();
        let levels: Vec<&str> = list
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty() && *w != "and" && *w != "default")
            .collect();
        if levels.iter().any(|w| w.eq_ignore_ascii_case("xhigh")) {
            return Some(ThinkRetry::Level("xhigh".into()));
        }
        if let Some(level) = levels
            .iter()
            .find(|w| w.eq_ignore_ascii_case("high"))
            .copied()
        {
            return Some(ThinkRetry::Level(level.to_string()));
        }
    }
    Some(ThinkRetry::WithoutThinking)
}

pub fn connection_failed(base_url: &str) -> ProviderError {
    ProviderError::network(format!(
        "Couldn't reach oMLX at {base_url}. Is it running? (The menu-bar app, or `omlx serve`. It listens on port 8000.)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fastapi_and_openai_error_bodies() {
        assert_eq!(
            to_provider_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                r#"{"detail":[{"msg":"unexpected reasoning effort"}]}"#
            )
            .message,
            "unexpected reasoning effort"
        );
        assert_eq!(
            to_provider_error(StatusCode::BAD_REQUEST, r#"{"error":{"message":"nope"}}"#).kind,
            ProviderErrorKind::InvalidRequest
        );
        assert_eq!(
            to_provider_error(StatusCode::UNAUTHORIZED, "").kind,
            ProviderErrorKind::Auth
        );
    }

    #[test]
    fn a_template_that_names_its_levels_is_retried_at_the_top_one() {
        let err = to_provider_error(
            StatusCode::BAD_REQUEST,
            r#"{"detail":"Unexpected reasoning effort high. Supported types are xhigh (default), medium, and low."}"#,
        );
        match think_retry(&err) {
            Some(ThinkRetry::Level(level)) => assert_eq!(level, "xhigh"),
            other => panic!("expected a level retry, got {other:?}"),
        }
    }

    #[test]
    fn a_model_without_thinking_is_retried_without_the_fields() {
        let err = to_provider_error(
            StatusCode::BAD_REQUEST,
            r#"{"detail":"enable_thinking is not supported"}"#,
        );
        assert!(matches!(
            think_retry(&err),
            Some(ThinkRetry::WithoutThinking)
        ));
        let missing = to_provider_error(StatusCode::BAD_REQUEST, r#"{"detail":"model not found"}"#);
        assert!(think_retry(&missing).is_none());
    }
}

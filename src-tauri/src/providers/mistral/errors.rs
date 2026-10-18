use reqwest::StatusCode;

use crate::providers::error::{ProviderError, ProviderErrorKind};
use crate::providers::http::describe_status;

/// Mistral reports errors as `{"message": "...", "type": "..."}`, but validation failures (422) come
/// back FastAPI-style as `{"detail": [{"msg": "..."}]}` or `{"detail": "..."}`.
fn message_in(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if let Some(message) = value.get("message").and_then(|m| m.as_str()) {
        return Some(message.to_string());
    }
    if let Some(message) = value.pointer("/error/message").and_then(|m| m.as_str()) {
        return Some(message.to_string());
    }
    match value.get("detail")? {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(items) => {
            let msgs: Vec<_> = items.iter().filter_map(|i| i.get("msg").and_then(|m| m.as_str())).collect();
            (!msgs.is_empty()).then(|| msgs.join("; "))
        }
        _ => None,
    }
}

pub fn to_provider_error(status: StatusCode, body: &str) -> ProviderError {
    let message = message_in(body)
        .unwrap_or_else(|| format!("Mistral API request failed with status {}.", describe_status(status)));

    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderErrorKind::Auth,
        StatusCode::TOO_MANY_REQUESTS => ProviderErrorKind::RateLimit,
        StatusCode::BAD_REQUEST | StatusCode::PAYMENT_REQUIRED | StatusCode::UNPROCESSABLE_ENTITY => ProviderErrorKind::InvalidRequest,
        _ => ProviderErrorKind::Unknown,
    };

    ProviderError::with_status(message, kind, status.as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_status_codes_to_kinds() {
        for (status, kind) in [
            (StatusCode::UNAUTHORIZED, ProviderErrorKind::Auth),
            (StatusCode::FORBIDDEN, ProviderErrorKind::Auth),
            (StatusCode::TOO_MANY_REQUESTS, ProviderErrorKind::RateLimit),
            (StatusCode::BAD_REQUEST, ProviderErrorKind::InvalidRequest),
            (StatusCode::UNPROCESSABLE_ENTITY, ProviderErrorKind::InvalidRequest),
            (StatusCode::INTERNAL_SERVER_ERROR, ProviderErrorKind::Unknown),
        ] {
            assert_eq!(to_provider_error(status, "").kind, kind, "{status}");
        }
    }

    #[test]
    fn reads_the_message_from_each_error_shape() {
        for (body, expected) in [
            (r#"{"object":"error","message":"Unauthorized","type":"invalid_request_error"}"#, "Unauthorized"),
            (r#"{"error":{"message":"Nested"}}"#, "Nested"),
            (r#"{"detail":"Not allowed"}"#, "Not allowed"),
            (r#"{"detail":[{"msg":"field required"},{"msg":"bad value"}]}"#, "field required; bad value"),
        ] {
            assert_eq!(to_provider_error(StatusCode::BAD_REQUEST, body).message, expected, "{body}");
        }
    }

    #[test]
    fn a_non_json_body_falls_back_to_the_status() {
        let err = to_provider_error(StatusCode::BAD_GATEWAY, "<html>proxy</html>");
        assert_eq!(err.message, "Mistral API request failed with status 502 Bad Gateway.");
        assert_eq!(err.http_status, Some(502));
    }
}

use reqwest::StatusCode;
use serde_json::Value;

use super::wire::XaiErrorResponse;
use crate::providers::error::{ProviderError, ProviderErrorKind};
use crate::providers::http::describe_status;

pub const FORBIDDEN_MESSAGE: &str = "Login succeeded, but this Grok account can't use the API (HTTP 403) - some plans don't include this access. Upgrade the plan, or use an xAI API key instead.";

pub fn to_provider_error(status: StatusCode, body: &str) -> ProviderError {
    if status == StatusCode::FORBIDDEN {
        return ProviderError::with_status(FORBIDDEN_MESSAGE, ProviderErrorKind::Auth, 403).user_facing();
    }

    let mut message = format!("xAI API request failed with status {}.", describe_status(status));

    // A body that isn't JSON (e.g. a proxy error page) just falls back to the generic message.
    if let Ok(parsed) = serde_json::from_str::<XaiErrorResponse>(body) {
        let detail = match &parsed.error {
            Some(Value::String(text)) => Some(text.as_str()),
            Some(Value::Object(obj)) => obj.get("message").and_then(Value::as_str),
            _ => None,
        };
        if let Some(detail) = detail.filter(|d| !d.is_empty()) {
            message = detail.to_string();
        }
    }

    let kind = match status {
        StatusCode::UNAUTHORIZED => ProviderErrorKind::Auth,
        StatusCode::TOO_MANY_REQUESTS => ProviderErrorKind::RateLimit,
        StatusCode::BAD_REQUEST | StatusCode::PAYMENT_REQUIRED => ProviderErrorKind::InvalidRequest,
        _ => ProviderErrorKind::Unknown,
    };
    ProviderError::with_status(message, kind, status.as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_403_is_a_helpful_user_facing_auth_error() {
        let e = to_provider_error(StatusCode::FORBIDDEN, "{}");
        assert_eq!(e.kind, ProviderErrorKind::Auth);
        assert!(e.message_is_user_facing);
        assert!(e.message.contains("API key"));
        assert_eq!(e.http_status, Some(403));
    }

    #[test]
    fn reads_both_error_shapes() {
        let object = to_provider_error(StatusCode::BAD_REQUEST, r#"{"error":{"message":"bad model"}}"#);
        assert_eq!(object.message, "bad model");
        assert_eq!(object.kind, ProviderErrorKind::InvalidRequest);
        let text = to_provider_error(StatusCode::TOO_MANY_REQUESTS, r#"{"error":"slow down","code":"x"}"#);
        assert_eq!(text.message, "slow down");
        assert_eq!(text.kind, ProviderErrorKind::RateLimit);
    }

    #[test]
    fn maps_the_remaining_statuses() {
        assert_eq!(to_provider_error(StatusCode::UNAUTHORIZED, "").kind, ProviderErrorKind::Auth);
        assert_eq!(to_provider_error(StatusCode::PAYMENT_REQUIRED, "").kind, ProviderErrorKind::InvalidRequest);
        let e = to_provider_error(StatusCode::BAD_GATEWAY, "<html>");
        assert_eq!(e.kind, ProviderErrorKind::Unknown);
        assert_eq!(e.message, "xAI API request failed with status 502 Bad Gateway.");
    }
}

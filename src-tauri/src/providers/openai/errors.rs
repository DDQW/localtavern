use reqwest::StatusCode;
use serde_json::Value;

use crate::providers::error::{ProviderError, ProviderErrorKind};
use crate::providers::http::describe_status;

/// The Codex backend answers with `{"error": {"message": ...}}` or `{"detail": "..."}`.
pub fn to_provider_error(status: StatusCode, body: &str) -> ProviderError {
    let mut message = format!("ChatGPT request failed with status {}.", describe_status(status));

    // A body that isn't JSON (e.g. a Cloudflare challenge page) just falls back to the generic message.
    if let Ok(Value::Object(root)) = serde_json::from_str::<Value>(body) {
        if let Some(m) = root.get("error").and_then(|e| e.get("message")).and_then(Value::as_str) {
            message = m.to_string();
        } else if let Some(detail) = root.get("detail").and_then(Value::as_str) {
            message = detail.to_string();
        }
    }

    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderErrorKind::Auth,
        StatusCode::TOO_MANY_REQUESTS => ProviderErrorKind::RateLimit,
        StatusCode::BAD_REQUEST | StatusCode::PAYMENT_REQUIRED => ProviderErrorKind::InvalidRequest,
        _ => ProviderErrorKind::Unknown,
    };

    // A 401/403 here means the subscription session or plan is the problem, not a mistyped key.
    let user_facing = kind == ProviderErrorKind::Auth;
    if user_facing {
        message = format!("{message} Sign in again in Settings → ChatGPT, or check that your plan includes Codex access.");
    }

    let mut error = ProviderError::with_status(message, kind, status.as_u16());
    error.message_is_user_facing = user_facing;
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_error_body_shapes() {
        assert_eq!(
            to_provider_error(StatusCode::TOO_MANY_REQUESTS, r#"{"error":{"message":"You've hit your usage limit."}}"#).message,
            "You've hit your usage limit."
        );
        let unauthorized = to_provider_error(StatusCode::UNAUTHORIZED, r#"{"detail":"Unauthorized"}"#);
        assert!(unauthorized.message.starts_with("Unauthorized "), "{}", unauthorized.message);
    }

    #[test]
    fn auth_failures_are_user_facing_with_a_sign_in_hint() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            let e = to_provider_error(status, "{}");
            assert_eq!(e.kind, ProviderErrorKind::Auth);
            assert!(e.message_is_user_facing);
            assert!(e.message.contains("Settings"));
        }
    }

    #[test]
    fn other_errors_are_not_user_facing() {
        let e = to_provider_error(StatusCode::TOO_MANY_REQUESTS, "{}");
        assert_eq!(e.kind, ProviderErrorKind::RateLimit);
        assert!(!e.message_is_user_facing);
        assert_eq!(to_provider_error(StatusCode::PAYMENT_REQUIRED, "").kind, ProviderErrorKind::InvalidRequest);
        assert_eq!(to_provider_error(StatusCode::BAD_GATEWAY, "").kind, ProviderErrorKind::Unknown);
    }

    #[test]
    fn non_json_bodies_get_a_generic_message() {
        let e = to_provider_error(StatusCode::BAD_GATEWAY, "<html>challenge</html>");
        assert_eq!(e.message, "ChatGPT request failed with status 502 Bad Gateway.");
    }
}

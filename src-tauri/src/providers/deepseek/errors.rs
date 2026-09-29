use reqwest::StatusCode;

use super::wire::DeepSeekErrorResponse;
use crate::providers::error::{ProviderError, ProviderErrorKind};
use crate::providers::http::describe_status;

pub fn to_provider_error(status: StatusCode, body: &str) -> ProviderError {
    let mut message = format!("DeepSeek API request failed with status {}.", describe_status(status));

    // A body that isn't JSON (e.g. a proxy error page) just falls back to the generic message.
    if let Ok(parsed) = serde_json::from_str::<DeepSeekErrorResponse>(body) {
        if let Some(error) = parsed.error {
            message = error.message;
        }
    }

    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderErrorKind::Auth,
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
    fn maps_status_codes_to_kinds() {
        for (status, kind) in [
            (StatusCode::UNAUTHORIZED, ProviderErrorKind::Auth),
            (StatusCode::FORBIDDEN, ProviderErrorKind::Auth),
            (StatusCode::TOO_MANY_REQUESTS, ProviderErrorKind::RateLimit),
            (StatusCode::BAD_REQUEST, ProviderErrorKind::InvalidRequest),
            (StatusCode::PAYMENT_REQUIRED, ProviderErrorKind::InvalidRequest),
            (StatusCode::INTERNAL_SERVER_ERROR, ProviderErrorKind::Unknown),
        ] {
            assert_eq!(to_provider_error(status, "").kind, kind, "{status}");
        }
    }

    #[test]
    fn uses_the_api_error_message_when_present() {
        let e = to_provider_error(StatusCode::TOO_MANY_REQUESTS, r#"{"error":{"message":"Rate limit reached","type":"x"}}"#);
        assert_eq!(e.message, "Rate limit reached");
        assert_eq!(e.http_status, Some(429));
    }

    #[test]
    fn falls_back_to_a_generic_message() {
        let e = to_provider_error(StatusCode::BAD_GATEWAY, "<html>");
        assert_eq!(e.message, "DeepSeek API request failed with status 502 Bad Gateway.");
    }
}

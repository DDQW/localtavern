use reqwest::StatusCode;

use super::wire::GeminiErrorResponse;
use crate::providers::error::{ProviderError, ProviderErrorKind};
use crate::providers::http::describe_status;

pub fn to_provider_error(status: StatusCode, body: &str) -> ProviderError {
    let mut message = format!("Gemini API request failed with status {}.", describe_status(status));
    let mut api_status: Option<String> = None;

    // A body that isn't JSON (e.g. a proxy error page) just falls back to the generic message.
    if let Ok(parsed) = serde_json::from_str::<GeminiErrorResponse>(body) {
        if let Some(error) = parsed.error {
            message = error.message;
            api_status = error.status;
        }
    }

    let kind = match api_status.as_deref() {
        Some("UNAUTHENTICATED" | "PERMISSION_DENIED") => ProviderErrorKind::Auth,
        Some("RESOURCE_EXHAUSTED") => ProviderErrorKind::RateLimit,
        Some("INVALID_ARGUMENT" | "FAILED_PRECONDITION") => ProviderErrorKind::InvalidRequest,
        _ => match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderErrorKind::Auth,
            StatusCode::TOO_MANY_REQUESTS => ProviderErrorKind::RateLimit,
            StatusCode::BAD_REQUEST => ProviderErrorKind::InvalidRequest,
            _ => ProviderErrorKind::Unknown,
        },
    };

    ProviderError::with_status(message, kind, status.as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_status_takes_priority_over_the_http_code() {
        let body = r#"{"error":{"code":429,"message":"Quota exceeded","status":"RESOURCE_EXHAUSTED"}}"#;
        let e = to_provider_error(StatusCode::BAD_REQUEST, body);
        assert_eq!(e.kind, ProviderErrorKind::RateLimit);
        assert_eq!(e.message, "Quota exceeded");
        assert_eq!(e.http_status, Some(400));
    }

    #[test]
    fn maps_each_api_status() {
        for (status, kind) in [
            ("UNAUTHENTICATED", ProviderErrorKind::Auth),
            ("PERMISSION_DENIED", ProviderErrorKind::Auth),
            ("INVALID_ARGUMENT", ProviderErrorKind::InvalidRequest),
            ("FAILED_PRECONDITION", ProviderErrorKind::InvalidRequest),
            ("INTERNAL", ProviderErrorKind::Unknown),
        ] {
            let body = format!(r#"{{"error":{{"code":1,"message":"m","status":"{status}"}}}}"#);
            assert_eq!(to_provider_error(StatusCode::INTERNAL_SERVER_ERROR, &body).kind, kind, "{status}");
        }
    }

    #[test]
    fn falls_back_to_the_http_status_when_the_body_has_no_status() {
        assert_eq!(to_provider_error(StatusCode::UNAUTHORIZED, "").kind, ProviderErrorKind::Auth);
        assert_eq!(to_provider_error(StatusCode::TOO_MANY_REQUESTS, "").kind, ProviderErrorKind::RateLimit);
        assert_eq!(to_provider_error(StatusCode::BAD_REQUEST, "<html>").kind, ProviderErrorKind::InvalidRequest);
        assert_eq!(to_provider_error(StatusCode::BAD_GATEWAY, "<html>").kind, ProviderErrorKind::Unknown);
    }

    #[test]
    fn non_json_bodies_get_a_generic_message() {
        let e = to_provider_error(StatusCode::BAD_GATEWAY, "<html>oops</html>");
        assert_eq!(e.message, "Gemini API request failed with status 502 Bad Gateway.");
    }
}

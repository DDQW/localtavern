use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderErrorKind {
    Auth,
    RateLimit,
    InvalidRequest,
    Network,
    ContentBlocked,
    Unknown,
}

#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct ProviderError {
    pub message: String,
    pub kind: ProviderErrorKind,
    pub http_status: Option<u16>,
    /// True when `message` is already a complete, actionable sentence for the user (e.g. "sign in
    /// again"), so the UI shows it instead of a generic per-kind message.
    pub message_is_user_facing: bool,
}

impl ProviderError {
    pub fn new(message: impl Into<String>, kind: ProviderErrorKind) -> Self {
        Self { message: message.into(), kind, http_status: None, message_is_user_facing: false }
    }

    pub fn with_status(message: impl Into<String>, kind: ProviderErrorKind, status: u16) -> Self {
        Self { http_status: Some(status), ..Self::new(message, kind) }
    }

    pub fn user_facing(mut self) -> Self {
        self.message_is_user_facing = true;
        self
    }

    pub fn auth(message: impl Into<String>) -> Self {
        Self::new(message, ProviderErrorKind::Auth)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(message, ProviderErrorKind::InvalidRequest)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new(message, ProviderErrorKind::Network)
    }

    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new(message, ProviderErrorKind::Unknown)
    }
}

impl From<ProviderError> for crate::error::Error {
    fn from(e: ProviderError) -> Self {
        crate::error::Error::Other(e.message)
    }
}

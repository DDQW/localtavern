//! Where subscription-login providers (ChatGPT, SuperGrok) get their bearer token. The OAuth
//! session that refreshes and stores tokens implements this; providers only ever ask for the
//! current, valid access token.

use async_trait::async_trait;

use super::error::ProviderError;

#[async_trait]
pub trait AccessTokenSource: Send + Sync {
    /// A currently valid access token, refreshing it first if needed. Fails with a user-facing
    /// `Auth` error when nobody is signed in.
    async fn access_token(&self) -> Result<String, ProviderError>;

    /// The account the token belongs to, when the provider's API wants it as a header. Read after
    /// [`access_token`](Self::access_token), since a refresh may be what discovers it.
    fn account_id(&self) -> Option<String> {
        None
    }
}

/// A fixed token - for tests and simple callers.
pub struct StaticAccessToken {
    pub token: String,
    pub account_id: Option<String>,
}

impl StaticAccessToken {
    pub fn new(token: &str) -> Self {
        Self { token: token.to_string(), account_id: None }
    }

    pub fn with_account(token: &str, account_id: &str) -> Self {
        Self { token: token.to_string(), account_id: Some(account_id.to_string()) }
    }
}

#[async_trait]
impl AccessTokenSource for StaticAccessToken {
    async fn access_token(&self) -> Result<String, ProviderError> {
        Ok(self.token.clone())
    }

    fn account_id(&self) -> Option<String> {
        self.account_id.clone()
    }
}

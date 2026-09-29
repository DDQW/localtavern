//! Per-provider secret lookup. Kept as a trait so providers don't depend on how keys are stored
//! (the OS credential vault in the app, a fixed value in tests).

pub trait ApiKeyProvider: Send + Sync {
    fn try_get_api_key(&self, provider_id: &str) -> Option<String>;
}

/// Read/write/delete access to per-provider secrets. Subscription-login providers (OAuth) use this
/// to persist their refresh token, since - unlike an API key - it's written by the app itself.
pub trait SecretStore: ApiKeyProvider {
    fn save_api_key(&self, provider_id: &str, secret: &str) -> crate::error::Result<()>;
    fn delete_api_key(&self, provider_id: &str) -> crate::error::Result<()>;
}

/// A fixed key (or none) regardless of provider - for tests and simple callers.
pub struct StaticApiKey(pub Option<String>);

impl StaticApiKey {
    pub fn some(key: &str) -> Self {
        Self(Some(key.to_string()))
    }

    pub fn none() -> Self {
        Self(None)
    }
}

impl ApiKeyProvider for StaticApiKey {
    fn try_get_api_key(&self, _provider_id: &str) -> Option<String> {
        self.0.clone()
    }
}

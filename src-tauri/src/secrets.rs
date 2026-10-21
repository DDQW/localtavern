//! Per-provider API keys in the OS credential store (Windows Credential Manager; on Android,
//! SharedPreferences encrypted with a Keystore key). The C# app used the packaged-app PasswordVault,
//! which is sandboxed per package and unreadable from here, so keys are entered once in this app.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::providers::{ApiKeyProvider, SecretStore};

// keyring's own `Entry` only picks a store on desktop platforms, so Android sets keyring-core's
// default store itself and uses its `Entry` directly.
#[cfg(not(target_os = "android"))]
use keyring::{Entry, Error as KeyringError};
#[cfg(target_os = "android")]
use keyring_core::{Entry, Error as KeyringError};

const SERVICE: &str = "LocalTavern";

pub struct KeyringSecretStore;

fn entry(provider_id: &str) -> Result<Entry> {
    #[cfg(target_os = "android")]
    android_store()?;
    Entry::new(SERVICE, provider_id).map_err(|e| Error::Other(format!("credential store: {e}")))
}

#[cfg(target_os = "android")]
fn android_store() -> Result<()> {
    static STORE: std::sync::OnceLock<std::result::Result<(), String>> = std::sync::OnceLock::new();
    STORE
        .get_or_init(|| {
            let store = android_native_keyring_store::Store::new().map_err(|e| e.to_string())?;
            keyring_core::set_default_store(store);
            Ok(())
        })
        .clone()
        .map_err(|e| Error::Other(format!("credential store: {e}")))
}

impl ApiKeyProvider for KeyringSecretStore {
    fn try_get_api_key(&self, provider_id: &str) -> Option<String> {
        entry(provider_id).ok()?.get_password().ok().filter(|k| !k.is_empty())
    }
}

impl SecretStore for KeyringSecretStore {
    fn save_api_key(&self, provider_id: &str, secret: &str) -> Result<()> {
        entry(provider_id)?
            .set_password(secret)
            .map_err(|e| Error::Other(format!("Couldn't save the key to the credential store: {e}")))
    }

    fn delete_api_key(&self, provider_id: &str) -> Result<()> {
        match entry(provider_id)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(Error::Other(format!("Couldn't remove the key from the credential store: {e}"))),
        }
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, String>>);

impl ApiKeyProvider for MemorySecretStore {
    fn try_get_api_key(&self, provider_id: &str) -> Option<String> {
        self.0.lock().unwrap().get(provider_id).cloned()
    }
}

impl SecretStore for MemorySecretStore {
    fn save_api_key(&self, provider_id: &str, secret: &str) -> Result<()> {
        self.0.lock().unwrap().insert(provider_id.to_string(), secret.to_string());
        Ok(())
    }

    fn delete_api_key(&self, provider_id: &str) -> Result<()> {
        self.0.lock().unwrap().remove(provider_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_saves_replaces_and_deletes() {
        let store = MemorySecretStore::default();
        assert_eq!(store.try_get_api_key("gemini"), None);
        store.save_api_key("gemini", "a").unwrap();
        store.save_api_key("gemini", "b").unwrap();
        assert_eq!(store.try_get_api_key("gemini").as_deref(), Some("b"));
        store.delete_api_key("gemini").unwrap();
        assert_eq!(store.try_get_api_key("gemini"), None);
        store.delete_api_key("gemini").unwrap(); // deleting nothing is fine
    }

    /// Touches the real Windows Credential Manager, so it only runs on request:
    /// `cargo test real_credential_store -- --ignored`
    #[test]
    #[ignore = "uses the real OS credential store"]
    fn real_credential_store_round_trips() {
        let store = KeyringSecretStore;
        let id = "localtavern-test-provider";
        store.save_api_key(id, "secret-value").unwrap();
        assert_eq!(store.try_get_api_key(id).as_deref(), Some("secret-value"));
        store.delete_api_key(id).unwrap();
        assert_eq!(store.try_get_api_key(id), None);
    }
}
